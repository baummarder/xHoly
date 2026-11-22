//! Multiplayer stub — lobby/session model.

use std::collections::{BTreeMap, BTreeSet};

/// Peer identifier.
pub type PeerId = u64;

/// A connected peer.
#[derive(Debug, Clone)]
pub struct Peer {
    pub id: PeerId,
    /// Display name (not used for identity).
    pub name: String,
    /// Is host?
    pub is_host: bool,
    /// Last-seen tick.
    pub last_seen: u64,
}

/// Session lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    /// Lobby open, no game running.
    Lobby,
    /// Loading map/assets.
    Loading,
    /// In-game.
    Playing,
    /// Paused.
    Paused,
    /// Ended.
    Ended,
}

/// Lobby errors.
#[derive(Debug, PartialEq)]
pub enum LobbyError {
    /// Lobby full.
    Full,
    /// Peer not found.
    UnknownPeer(PeerId),
    /// Session not in expected state.
    BadState(SessionState),
    /// Duplicate peer id.
    Duplicate(PeerId),
}

impl std::fmt::Display for LobbyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => write!(f, "lobby full"),
            Self::UnknownPeer(id) => write!(f, "unknown peer {id}"),
            Self::BadState(s) => write!(f, "bad state {s:?}"),
            Self::Duplicate(id) => write!(f, "duplicate peer {id}"),
        }
    }
}
impl std::error::Error for LobbyError {}

/// A multiplayer lobby + session.
pub struct Lobby {
    /// Lobby id.
    pub id: String,
    /// Max peers.
    pub max_peers: usize,
    /// Peers.
    peers: BTreeMap<PeerId, Peer>,
    /// Session state.
    pub state: SessionState,
    /// Ready set (peer ids who readied).
    ready: BTreeSet<PeerId>,
    /// Host peer id (first joiner).
    pub host: Option<PeerId>,
}

impl Lobby {
    /// New lobby.
    pub fn new(id: impl Into<String>, max_peers: usize) -> Self {
        Self {
            id: id.into(),
            max_peers,
            peers: BTreeMap::new(),
            state: SessionState::Lobby,
            ready: BTreeSet::new(),
            host: None,
        }
    }

    /// Join a peer; first joiner becomes host.
    pub fn join(&mut self, id: PeerId, name: impl Into<String>) -> Result<(), LobbyError> {
        if self.peers.len() >= self.max_peers {
            return Err(LobbyError::Full);
        }
        if self.peers.contains_key(&id) {
            return Err(LobbyError::Duplicate(id));
        }
        let is_host = self.host.is_none();
        if is_host {
            self.host = Some(id);
        }
        self.peers.insert(
            id,
            Peer {
                id,
                name: name.into(),
                is_host,
                last_seen: 0,
            },
        );
        Ok(())
    }

    /// Leave.
    pub fn leave(&mut self, id: PeerId) -> Result<(), LobbyError> {
        if self.peers.remove(&id).is_none() {
            return Err(LobbyError::UnknownPeer(id));
        }
        self.ready.remove(&id);
        if self.host == Some(id) {
            // Promote lowest remaining id.
            self.host = self.peers.keys().next().copied();
            if let Some(h) = self.host {
                if let Some(p) = self.peers.get_mut(&h) {
                    p.is_host = true;
                }
            }
        }
        Ok(())
    }

    /// Mark ready.
    pub fn set_ready(&mut self, id: PeerId, ready: bool) -> Result<(), LobbyError> {
        if !self.peers.contains_key(&id) {
            return Err(LobbyError::UnknownPeer(id));
        }
        if ready {
            self.ready.insert(id);
        } else {
            self.ready.remove(&id);
        }
        Ok(())
    }

    /// All peers ready?
    pub fn all_ready(&self) -> bool {
        !self.peers.is_empty() && self.ready.len() == self.peers.len()
    }

    /// Transition to Playing (requires all ready + host).
    pub fn start(&mut self, by: PeerId) -> Result<(), LobbyError> {
        if self.host != Some(by) {
            return Err(LobbyError::UnknownPeer(by)); // not host — reuse error loosely
        }
        if self.state != SessionState::Lobby {
            return Err(LobbyError::BadState(self.state));
        }
        if !self.all_ready() {
            return Err(LobbyError::BadState(self.state));
        }
        self.state = SessionState::Playing;
        Ok(())
    }

    /// Peer count.
    pub fn len(&self) -> usize {
        self.peers.len()
    }
    /// Empty?
    pub fn is_empty(&self) -> bool {
        self.peers.is_empty()
    }
    /// Peer names sorted.
    pub fn names(&self) -> Vec<&str> {
        self.peers.values().map(|p| p.name.as_str()).collect()
    }
}

/// A session wraps a lobby + tick counter.
pub struct Session {
    pub lobby: Lobby,
    /// Monotonic tick.
    pub tick: u64,
}

impl Session {
    /// New session.
    pub fn new(lobby: Lobby) -> Self {
        Self { lobby, tick: 0 }
    }
    /// Advance one tick.
    pub fn tick(&mut self) {
        self.tick += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lobby_lifecycle() {
        let mut l = Lobby::new("room1", 4);
        l.join(1, "alice").unwrap();
        l.join(2, "bob").unwrap();
        assert_eq!(l.host, Some(1));
        assert_eq!(l.len(), 2);
        l.set_ready(1, true).unwrap();
        l.set_ready(2, true).unwrap();
        assert!(l.all_ready());
        l.start(1).unwrap();
        assert_eq!(l.state, SessionState::Playing);
    }

    #[test]
    fn host_migration_on_leave() {
        let mut l = Lobby::new("r", 4);
        l.join(1, "a").unwrap();
        l.join(2, "b").unwrap();
        l.leave(1).unwrap();
        assert_eq!(l.host, Some(2));
        assert!(l.peers.get(&2).unwrap().is_host);
    }

    #[test]
    fn full_lobby_rejects() {
        let mut l = Lobby::new("r", 2);
        l.join(1, "a").unwrap();
        l.join(2, "b").unwrap();
        assert_eq!(l.join(3, "c"), Err(LobbyError::Full));
    }

    #[test]
    fn duplicate_rejected() {
        let mut l = Lobby::new("r", 4);
        l.join(1, "a").unwrap();
        assert_eq!(l.join(1, "a2"), Err(LobbyError::Duplicate(1)));
    }
}
