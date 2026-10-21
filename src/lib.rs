//! Core of the Posthorn Rust port of Mako.
//!
//! MVP scope is strict Mako behavior parity. The D-Bus service and Wayland
//! compositor integration are not yet implemented; this library is not a
//! notification daemon by itself.

pub mod config;
pub mod dbus;
pub mod control;
pub mod format;
pub mod notification;
pub mod timers;
pub mod runtime;
pub mod wayland;
pub mod raster;
pub mod grouping;

pub const MAKO_BASELINE: &str = "v1.11.0";
