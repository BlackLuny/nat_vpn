use std::os::fd::{FromRawFd, RawFd, AsRawFd};

use async_io::Async;

use crate::cloned_io::IoHandler;
use nix::unistd::dup;
pub struct LinuxIoHandler(RawFd, DefaultInterface);
impl IoHandler for LinuxIoHandler {
    type IO = Async<std::fs::File>;
    fn new_io(&self) -> Self::IO {
        let io = Async::new(unsafe { std::fs::File::from_raw_fd(dup(self.0).unwrap()) }).unwrap();
        io
    }
}
impl LinuxIoHandler {
    pub fn new(t: DefaultInterface) -> Self {
        Self(t.as_raw_fd(), t)
    }
}

pub type DefaultInterface = tunio::DefaultInterface;
pub type PlatformInterface = LinuxIoHandler;
