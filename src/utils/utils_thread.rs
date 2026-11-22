//! Thread utilities — named threads, simple pool, semaphore.

use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};

/// Worker count helper (respects a cap).
pub struct WorkerCount;

impl WorkerCount {
    /// Recommended workers = available_parallelism, capped.
    pub fn recommended(cap: usize) -> usize {
        thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
            .min(cap)
            .max(1)
    }
}

/// Spawn a named thread (better panic messages/debugging).
pub fn spawn_named<F, T>(name: &str, f: F) -> JoinHandle<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    thread::Builder::new()
        .name(name.to_string())
        .spawn(f)
        .expect("failed to spawn named thread")
}

/// Re-exported alias.
pub type NamedThread = JoinHandle<()>;

/// Simple bounded thread pool (mpsc-based, no external deps).
///
/// Jobs are `Box<dyn FnOnce() + Send>`. `shutdown` drops the sender
/// and joins workers. Not work-stealing; fine for IO-ish tasks.
pub struct ThreadPool {
    tx: Option<mpsc::Sender<Box<dyn FnOnce() + Send>>>,
    workers: Vec<JoinHandle<()>>,
}

impl ThreadPool {
    /// New pool with `n` workers.
    pub fn new(n: usize) -> Self {
        let n = n.max(1);
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        let mut workers = Vec::with_capacity(n);
        for i in 0..n {
            let rx = Arc::clone(&rx);
            workers.push(spawn_named(&format!("xholy-pool-{i}"), move || {
                loop {
                    // Lock briefly to recv; channel is MPMC-safe via Mutex.
                    let job = {
                        let guard = rx.lock().unwrap();
                        guard.recv()
                    };
                    match job {
                        Ok(j) => j(),
                        Err(_) => break, // sender dropped → exit
                    }
                }
            }));
        }
        Self {
            tx: Some(tx),
            workers,
        }
    }

    /// Submit a job.
    pub fn submit<F>(&self, f: F) -> Result<(), String>
    where
        F: FnOnce() + Send + 'static,
    {
        let tx = self.tx.as_ref().ok_or("pool shut down")?;
        tx.send(Box::new(f)).map_err(|e| e.to_string())
    }

    /// Submit and return a oneshot-ish result via channel.
    pub fn submit_result<T, F>(&self, f: F) -> Result<mpsc::Receiver<T>, String>
    where
        F: FnOnce() -> T + Send + 'static,
        T: Send + 'static,
    {
        let (rtx, rrx) = mpsc::channel();
        self.submit(move || {
            let _ = rtx.send(f());
        })?;
        Ok(rrx)
    }

    /// Worker count.
    pub fn workers(&self) -> usize {
        self.workers.len()
    }

    /// Shutdown: close channel, join workers (with timeout best-effort).
    pub fn shutdown(mut self) {
        self.tx = None; // drop sender → workers see Err and exit
        for w in self.workers.drain(..) {
            // Can't timeout-join; rely on jobs being finite.
            let _ = w.join();
        }
    }
}

/// Counting semaphore (std-only implementation).
#[derive(Debug)]
pub struct Semaphore {
    count: std::sync::Mutex<usize>,
    cv: std::sync::Condvar,
    max: usize,
}

impl Semaphore {
    /// New semaphore with `max` permits.
    pub fn new(max: usize) -> Self {
        Self {
            count: std::sync::Mutex::new(max),
            cv: std::sync::Condvar::new(),
            max,
        }
    }

    /// Acquire a permit (blocks).
    pub fn acquire(&self) {
        let mut c = self.count.lock().unwrap();
        while *c == 0 {
            c = self.cv.wait(c).unwrap();
        }
        *c -= 1;
    }

    /// Try acquire without blocking.
    pub fn try_acquire(&self) -> bool {
        let mut c = self.count.lock().unwrap();
        if *c > 0 {
            *c -= 1;
            true
        } else {
            false
        }
    }

    /// Release a permit.
    pub fn release(&self) {
        let mut c = self.count.lock().unwrap();
        if *c < self.max {
            *c += 1;
            self.cv.notify_one();
        }
    }

    /// Available permits.
    pub fn available(&self) -> usize {
        *self.count.lock().unwrap()
    }
}

/// RAII permit guard.
pub struct Permit<'a> {
    sem: &'a Semaphore,
}

impl<'a> Permit<'a> {
    /// Acquire or return None if unavailable.
    pub fn try_new(sem: &'a Semaphore) -> Option<Self> {
        if sem.try_acquire() {
            Some(Self { sem })
        } else {
            None
        }
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        self.sem.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn worker_count_at_least_one() {
        assert!(WorkerCount::recommended(64) >= 1);
    }

    #[test]
    fn named_thread_runs() {
        let h = spawn_named("test-thread", || 42);
        assert_eq!(h.join().unwrap(), 42);
    }

    #[test]
    fn pool_executes_jobs() {
        let pool = ThreadPool::new(2);
        let counter = Arc::new(AtomicUsize::new(0));
        for _ in 0..10 {
            let c = Arc::clone(&counter);
            pool.submit(move || {
                c.fetch_add(1, Ordering::SeqCst);
            })
            .unwrap();
        }
        // Give workers a moment.
        thread::sleep(Duration::from_millis(50));
        pool.shutdown();
        assert_eq!(counter.load(Ordering::SeqCst), 10);
    }

    #[test]
    fn pool_result_channel() {
        let pool = ThreadPool::new(1);
        let rx = pool.submit_result(|| 7 * 6).unwrap();
        assert_eq!(rx.recv().unwrap(), 42);
        pool.shutdown();
    }

    #[test]
    fn semaphore_permits() {
        let sem = Semaphore::new(2);
        let p1 = Permit::try_new(&sem).unwrap();
        let p2 = Permit::try_new(&sem).unwrap();
        assert!(Permit::try_new(&sem).is_none());
        drop(p2);
        assert!(Permit::try_new(&sem).is_some());
        drop(p1);
    }
}
