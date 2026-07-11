#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

#[cfg(unix)]
pub use unix::{bind_and_spawn, serve_until, socket_path};
#[cfg(windows)]
pub use windows::{bind_and_spawn, serve_until, socket_path};
