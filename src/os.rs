pub use osimpl::*;

#[cfg(unix)]
#[path = "os/unix.rs"]
mod osimpl;
#[cfg(windows)]
#[path = "os/windows.rs"]
mod osimpl;
