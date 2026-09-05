//! The parts of HomeCloud that are the same on every platform.
//!
//! HomeCloud does not implement folder synchronisation. It drives a Syncthing
//! process over its local REST API and hides everything about that engine that
//! a person should never have to care about. This crate is that translation
//! layer: a small, opinionated vocabulary of shared folders and invitations on
//! top of Syncthing's much larger one of devices, folders and cluster config.

pub mod client;
pub mod device_id;
pub mod error;
pub mod model;
pub mod pairing;
pub mod supervisor;

pub use client::Syncthing;
pub use device_id::DeviceId;
pub use error::{Error, Result};
pub use pairing::PairingCode;
