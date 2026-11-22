//! Network abstraction — transport trait + request/response types.

use std::collections::HashMap;
use std::time::Duration;

/// HTTP method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Delete,
    Head,
}

impl HttpMethod {
    /// As string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
        }
    }
}

/// A request.
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: HttpMethod,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

impl HttpRequest {
    /// GET.
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: HttpMethod::Get,
            url: url.into(),
            headers: HashMap::new(),
            body: Vec::new(),
            timeout: Duration::from_secs(10),
        }
    }
    /// POST with body.
    pub fn post(url: impl Into<String>, body: Vec<u8>) -> Self {
        Self {
            method: HttpMethod::Post,
            url: url.into(),
            headers: HashMap::new(),
            body,
            timeout: Duration::from_secs(30),
        }
    }
    /// Add header.
    pub fn header(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.headers.insert(k.into(), v.into());
        self
    }
}

/// A response.
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    /// Success status?
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// Errors.
#[derive(Debug)]
pub enum NetError {
    /// Transport failure (DNS, connect, TLS).
    Transport(String),
    /// Timeout.
    Timeout,
    /// HTTP error status.
    Http(u16),
    /// Offline mode and no cached response.
    Offline,
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(e) => write!(f, "transport: {e}"),
            Self::Timeout => write!(f, "timeout"),
            Self::Http(s) => write!(f, "http {s}"),
            Self::Offline => write!(f, "offline"),
        }
    }
}
impl std::error::Error for NetError {}

/// Transport trait — implement for reqwest/curl/socket backends.
pub trait Transport: Send + Sync {
    /// Execute a request.
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, NetError>;
}

/// Loopback transport for tests: url → queued responses.
#[derive(Debug, Default)]
pub struct LoopbackTransport {
    routes: std::sync::Mutex<
        HashMap<String, std::collections::VecDeque<Result<HttpResponse, NetError>>>,
    >,
    pub seen: std::sync::Mutex<Vec<HttpRequest>>,
}

impl LoopbackTransport {
    /// New loopback.
    pub fn new() -> Self {
        Self::default()
    }
    /// Queue a response for `METHOD url`.
    pub fn push(&self, method: HttpMethod, url: &str, resp: HttpResponse) {
        self.routes
            .lock()
            .unwrap()
            .entry(format!("{} {}", method.as_str(), url))
            .or_default()
            .push_back(Ok(resp));
    }
    /// Queue an error.
    pub fn push_err(&self, method: HttpMethod, url: &str, err: NetError) {
        self.routes
            .lock()
            .unwrap()
            .entry(format!("{} {}", method.as_str(), url))
            .or_default()
            .push_back(Err(err));
    }
}

impl Transport for LoopbackTransport {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, NetError> {
        self.seen.lock().unwrap().push(req.clone());
        let key = format!("{} {}", req.method.as_str(), req.url);
        let mut routes = self.routes.lock().unwrap();
        if let Some(q) = routes.get_mut(&key) {
            if let Some(r) = q.pop_front() {
                return r;
            }
        }
        Err(NetError::Transport("no route".into()))
    }
}

/// Simple client wrapper with offline/cache awareness lives in
/// `network_cache`; this file provides primitives only.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn method_strings() {
        assert_eq!(HttpMethod::Get.as_str(), "GET");
        assert_eq!(HttpMethod::Delete.as_str(), "DELETE");
    }

    #[test]
    fn loopback_routes() {
        let t = LoopbackTransport::new();
        t.push(
            HttpMethod::Get,
            "http://x/",
            HttpResponse {
                status: 200,
                headers: HashMap::new(),
                body: b"hi".to_vec(),
            },
        );
        let r = t.send(&HttpRequest::get("http://x/")).unwrap();
        assert!(r.ok());
        assert_eq!(r.body, b"hi");
    }
}
