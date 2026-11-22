//! Proxy support — HTTP/SOCKS proxy config + no-proxy bypass list.

/// Proxy configuration.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyConfig {
    /// Proxy URL (e.g. "http://proxy:8080" or "socks5://host:1080").
    pub url: Option<String>,
    /// Whether proxy is enabled.
    pub enabled: bool,
    /// Hosts that bypass the proxy.
    pub no_proxy: Vec<String>,
}

impl ProxyConfig {
    /// New disabled config.
    pub fn new() -> Self {
        Self::default()
    }
    /// Enable with URL.
    pub fn enable(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self.enabled = true;
        self
    }
    /// Add bypass host (suffix match).
    pub fn bypass(mut self, host: impl Into<String>) -> Self {
        self.no_proxy.push(host.into());
        self
    }

    /// Should `host` go through the proxy?
    pub fn use_proxy_for(&self, host: &str) -> bool {
        if !self.enabled || self.url.is_none() {
            return false;
        }
        !self
            .no_proxy
            .iter()
            .any(|p| host == p || host.ends_with(&format!(".{p}")) || host.ends_with(p.as_str()))
    }

    /// Resolve the effective proxy URL for a host.
    pub fn proxy_url_for(&self, host: &str) -> Option<&str> {
        if self.use_proxy_for(host) {
            self.url.as_deref()
        } else {
            None
        }
    }
}

/// No-proxy list helper (comma-separated env-style parsing).
pub struct NoProxy;

impl NoProxy {
    /// Parse "a.com, b.org ,*.local" → vec of patterns.
    pub fn parse(list: &str) -> Vec<String> {
        list.split(',')
            .map(|s| s.trim().trim_start_matches("*.").to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_routing() {
        let p = ProxyConfig::new()
            .enable("http://proxy:8080")
            .bypass("localhost")
            .bypass("internal.corp");
        assert!(p.use_proxy_for("example.com"));
        assert!(!p.use_proxy_for("localhost"));
        assert!(!p.use_proxy_for("sub.internal.corp"));
        assert_eq!(p.proxy_url_for("example.com"), Some("http://proxy:8080"));
        assert_eq!(p.proxy_url_for("localhost"), None);
    }

    #[test]
    fn disabled_proxy_never_used() {
        let p = ProxyConfig::new();
        assert!(!p.use_proxy_for("anything"));
    }

    #[test]
    fn no_proxy_parse() {
        let v = NoProxy::parse("a.com, *.local , b.org");
        assert_eq!(v, vec!["a.com", "local", "b.org"]);
    }
}
