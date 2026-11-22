//! Network helpers — Phase 11.5.
//!
//! HTTP/REST client abstraction (transport-injected, no real sockets),
//! offline-aware cache, proxy support. Real `reqwest` integration is
//! intentionally deferred — everything is testable headless.

#[allow(clippy::module_inception)]
pub mod network;
pub mod network_cache;
pub mod network_proxy;

pub use network::{HttpMethod, HttpRequest, HttpResponse, LoopbackTransport, NetError, Transport};
pub use network_cache::{CachePolicy, NetCache};
pub use network_proxy::{NoProxy, ProxyConfig};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::HttpMethod::Get;
    }
}
