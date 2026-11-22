//! Multiplayer — Phase 11.5 stub.
//!
//! Lobby + session model with state replication *concepts*.
//! No real networking: peers are identified by id, state sync is a
//! deterministic merge of key/value blobs. Netcode (prediction,
//! interpolation) is modeled but not wire-real.

#[allow(clippy::module_inception)]
pub mod multiplayer;
pub mod multiplayer_netcode;
pub mod multiplayer_sync;

pub use multiplayer::{Lobby, LobbyError, Peer, PeerId, Session, SessionState};
pub use multiplayer_netcode::{Interpolator, NetcodeConfig, Predictor};
pub use multiplayer_sync::{ConsistencyMode, ReplicatedState, SyncEngine};

#[cfg(test)]
mod tests {
    #[test]
    fn modules_link() {
        let _ = super::ConsistencyMode::Lockstep;
    }
}
