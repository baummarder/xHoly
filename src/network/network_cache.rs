//! Network cache — offline mode + response caching.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::network::{HttpRequest, HttpResponse, NetError, Transport};

/// Cache policy per request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CachePolicy {
    /// Always fetch fresh.
    NoCache,
    /// Use cache if present (offline-friendly).
    CacheFirst,
    /// Fetch, then update cache.
    CacheAndFetch,
}

/// Cached response entry.
#[derive(Debug, Clone)]
struct Entry {
    resp: HttpResponse,
    stored_at: u64,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Offline-aware caching client wrapper.
pub struct NetCache<T: Transport> {
    inner: T,
    cache: HashMap<String, Entry>,
    /// Global offline flag.
    pub offline: bool,
    /// TTL seconds for cached responses (0 = forever).
    pub ttl_secs: u64,
    /// Default policy.
    pub policy: CachePolicy,
}

impl<T: Transport> NetCache<T> {
    /// New cache wrapper.
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            cache: HashMap::new(),
            offline: false,
            ttl_secs: 3600,
            policy: CachePolicy::CacheAndFetch,
        }
    }

    /// Borrow inner transport.
    pub fn transport(&self) -> &T {
        &self.inner
    }

    fn cache_key(req: &HttpRequest) -> String {
        format!("{} {}", req.method.as_str(), req.url)
    }

    fn fresh(&self, e: &Entry) -> bool {
        self.ttl_secs == 0 || now().saturating_sub(e.stored_at) <= self.ttl_secs
    }

    /// Send with caching + offline fallback.
    pub fn send(&mut self, req: &HttpRequest) -> Result<HttpResponse, NetError> {
        let key = Self::cache_key(req);
        if self.offline {
            return self
                .cache
                .get(&key)
                .filter(|e| self.fresh(e))
                .map(|e| e.resp.clone())
                .ok_or(NetError::Offline);
        }
        match self.inner.send(req) {
            Ok(resp) => {
                if resp.ok() || resp.status < 500 {
                    self.cache.insert(
                        key,
                        Entry {
                            resp: resp.clone(),
                            stored_at: now(),
                        },
                    );
                }
                Ok(resp)
            }
            Err(NetError::Transport(_)) | Err(NetError::Timeout) => {
                // Network down → serve stale cache if any.
                self.cache
                    .get(&key)
                    .map(|e| e.resp.clone())
                    .ok_or(NetError::Offline)
            }
            Err(e) => Err(e),
        }
    }

    /// Force-refresh: bypass cache read, still writes.
    pub fn fetch_fresh(&mut self, req: &HttpRequest) -> Result<HttpResponse, NetError> {
        let r = self.inner.send(req)?;
        self.cache.insert(
            Self::cache_key(req),
            Entry {
                resp: r.clone(),
                stored_at: now(),
            },
        );
        Ok(r)
    }

    /// Invalidate a URL.
    pub fn invalidate(&mut self, url: &str) {
        self.cache.retain(|k, _| !k.ends_with(url));
    }

    /// Clear cache.
    pub fn clear(&mut self) {
        self.cache.clear();
    }

    /// Cached entry count.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::network::{HttpMethod, LoopbackTransport};
    use std::collections::HashMap;

    fn resp(body: &str) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: HashMap::new(),
            body: body.as_bytes().to_vec(),
        }
    }

    #[test]
    fn caches_successful_get() {
        let t = LoopbackTransport::new();
        t.push(HttpMethod::Get, "http://x/a", resp("one"));
        let mut c = NetCache::new(t);
        let r1 = c.send(&HttpRequest::get("http://x/a")).unwrap();
        assert_eq!(r1.body, b"one");
        // Second send: transport has no route → falls back to cache.
        let r2 = c.send(&HttpRequest::get("http://x/a")).unwrap();
        assert_eq!(r2.body, b"one");
    }

    #[test]
    fn offline_serves_cache() {
        let t = LoopbackTransport::new();
        t.push(HttpMethod::Get, "http://x/b", resp("cached"));
        let mut c = NetCache::new(t);
        let _ = c.send(&HttpRequest::get("http://x/b")).unwrap();
        c.offline = true;
        let r = c.send(&HttpRequest::get("http://x/b")).unwrap();
        assert_eq!(r.body, b"cached");
        // Uncached URL offline → Offline error.
        assert!(matches!(
            c.send(&HttpRequest::get("http://x/never")),
            Err(NetError::Offline)
        ));
    }

    #[test]
    fn transport_error_falls_back_to_stale() {
        let t = LoopbackTransport::new();
        t.push(HttpMethod::Get, "http://x/c", resp("v1"));
        let mut c = NetCache::new(t);
        let _ = c.send(&HttpRequest::get("http://x/c")).unwrap();
        // Next request: route exhausted → transport error → stale cache.
        let r = c.send(&HttpRequest::get("http://x/c")).unwrap();
        assert_eq!(r.body, b"v1");
    }
}
