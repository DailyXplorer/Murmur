//! The on-device speech models, shared by dictation and meetings: every
//! file pinned and checked by SHA-256 (`manifest`), fetched by `download`.

pub mod download;
pub mod manifest;
mod pinned;
pub mod verify;
