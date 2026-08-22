//! Linux/WSL2 hosted multi-process prototype for Court Kernel MVP-0B.
//!
//! The protocol types are always available so the crate can compile on
//! non-Unix hosts. Unix sockets, mmap, and demo orchestration are compiled only
//! on Unix targets.

pub mod manifest;
pub mod protocol;

pub type LinuxResult<T> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[cfg(unix)]
pub mod app;
#[cfg(unix)]
pub mod control;
#[cfg(unix)]
pub mod net;
#[cfg(unix)]
pub mod root;
#[cfg(unix)]
pub mod shm_ring;
