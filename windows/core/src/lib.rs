//! mimic-core: everything needed to talk to discord's local ipc socket, with
//! no ui and no platform specific code outside of [`pipe`].
//!
//! The pieces, bottom to top:
//!
//! - [`frame`]: the 8 byte header + json framing, with a buffering decoder
//! - [`message`]: handshake / ping / close / dispatch parsing and building
//! - [`activity`]: validating user input into a discord activity payload
//! - [`debounce`]: coalescing and rate limiting of presence updates
//! - [`manager`]: the connection manager that ties it together and exposes
//!   a [`manager::ConnectionState`] for a ui
//! - [`app`]: the app's in-memory state and the operations on it (go live,
//!   save a preset, restore on launch), shared by the ui commands and the tray
//! - [`games`], [`preset`], [`settings`], [`storage`]: the game list, saved
//!   presets and settings, as plain data + json files
//! - [`pipe`]: the windows named pipe connector (cfg(windows) only)
//!
//! The wire protocol is documented in `shared/README.md`.

#![forbid(unsafe_code)]

pub mod activity;
pub mod app;
pub mod debounce;
pub mod frame;
pub mod games;
pub mod manager;
pub mod message;
pub mod pipe;
pub mod preset;
pub mod settings;
pub mod storage;

pub use activity::{
    build_activity, set_activity_payload, Activity, ActivityError, ActivityInput, ButtonInput,
    PartyInput, Timer,
};
pub use debounce::{DebounceConfig, Debouncer};
pub use frame::{Frame, FrameDecoder, FrameError, FrameReader, Opcode};
pub use manager::{
    spawn, Connector, ConnectionState, Manager, ManagerConfig, PresenceHandle, PIPE_COUNT,
};
pub use message::{Incoming, ProtocolError};
