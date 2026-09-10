//! Screen capture, replay buffer and screenshots for FiveMClip.
//!
//! Kept as its own crate so the Windows-specific parts can be type-checked
//! against `x86_64-pc-windows-gnu` without dragging in the whole Tauri app.

pub mod audio;
pub mod config;
pub mod ffmpeg;
pub mod reaper;
pub mod ring;
pub mod shot;
pub mod sysprobe;

pub use config::{MicMode, Settings};
pub use ffmpeg::{Pipeline, ProbeReport};
pub use ring::{Recorder, RecorderStatus};

pub mod disk;
