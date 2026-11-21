//! Runtime subsystem: JVM/Scala bridge and compatibility shims.

pub mod scala_bridge;

pub use scala_bridge::{BridgeMessage, ScalaBridge};
