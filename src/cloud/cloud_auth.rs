//! Cloud auth — token management (no real IdP).

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Auth token with expiry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken {
    /// Opaque token string.
    pub token: String,
    /// Expiry (unix secs); `None` = never expires.
    pub expires_at: Option<u64>,
}

/// Auth errors.
#[derive(Debug, PartialEq)]
pub enum AuthError {
    /// Token expired.
    Expired,
    /// No token set.
    Missing,
    /// Bad token format.
    Malformed,
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Expired => write!(f, "token expired"),
            Self::Missing => write!(f, "no token"),
            Self::Malformed => write!(f, "malformed token"),
        }
    }
}
impl std::error::Error for AuthError {}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Token store + refresh logic.
#[derive(Debug, Default)]
pub struct CloudAuth {
    token: Option<AuthToken>,
    /// Refresh margin (expire this early).
    pub refresh_margin: Duration,
}

impl CloudAuth {
    /// New auth store.
    pub fn new() -> Self {
        Self {
            refresh_margin: Duration::from_secs(60),
            ..Default::default()
        }
    }

    /// Set a token (validates non-empty).
    pub fn set_token(
        &mut self,
        token: impl Into<String>,
        ttl: Option<Duration>,
    ) -> Result<(), AuthError> {
        let t = token.into();
        if t.trim().is_empty() {
            return Err(AuthError::Malformed);
        }
        self.token = Some(AuthToken {
            token: t,
            expires_at: ttl.map(|d| now_secs() + d.as_secs()),
        });
        Ok(())
    }

    /// Current valid token (checks expiry).
    pub fn token(&self) -> Result<&str, AuthError> {
        let t = self.token.as_ref().ok_or(AuthError::Missing)?;
        if let Some(exp) = t.expires_at {
            if now_secs() + self.refresh_margin.as_secs() >= exp {
                return Err(AuthError::Expired);
            }
        }
        Ok(&t.token)
    }

    /// Is a refresh needed soon?
    pub fn needs_refresh(&self) -> bool {
        match &self.token {
            None => true,
            Some(t) => match t.expires_at {
                None => false,
                Some(exp) => now_secs() + self.refresh_margin.as_secs() >= exp,
            },
        }
    }

    /// Clear token (logout).
    pub fn clear(&mut self) {
        self.token = None;
    }

    /// Deserialize-safe snapshot (token string only, no secrets in logs).
    pub fn fingerprint(&self) -> Option<String> {
        self.token.as_ref().map(|t| {
            let h = t.token.bytes().fold(0xcbf29ce484222325u64, |a, b| {
                (a ^ b as u64).wrapping_mul(0x100000001b3)
            });
            format!("{:016x}", h)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_lifecycle() {
        let mut a = CloudAuth::new();
        assert!(matches!(a.token(), Err(AuthError::Missing)));
        a.set_token("abc", Some(Duration::from_secs(3600))).unwrap();
        assert_eq!(a.token().unwrap(), "abc");
        assert!(!a.needs_refresh());
        assert!(a.fingerprint().is_some());
        a.clear();
        assert!(matches!(a.token(), Err(AuthError::Missing)));
    }

    #[test]
    fn expired_token_rejected() {
        let mut a = CloudAuth::new();
        a.set_token("old", Some(Duration::from_secs(0))).unwrap();
        // ttl=0 → expires immediately
        assert!(matches!(a.token(), Err(AuthError::Expired)));
        assert!(a.needs_refresh());
    }

    #[test]
    fn malformed_rejected() {
        let mut a = CloudAuth::new();
        assert!(matches!(
            a.set_token("   ", None),
            Err(AuthError::Malformed)
        ));
    }
}
