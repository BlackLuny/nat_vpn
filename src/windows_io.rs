use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};

use async_compat::{Compat, CompatExt};
use futures::{AsyncRead, AsyncWrite};
use tunio::platform::wintun::AsyncTokioInterface;

use crate::cloned_io::IoHandler;

pub struct PlatformIO {
    f: Arc<Mutex<Compat<DefaultInterface>>>,
}
impl PlatformIO {
    pub fn new(t: DefaultInterface) -> Self {
        Self {
            f: Arc::new(Mutex::new(t.compat())),
        }
    }
}
pub struct SharedIO<T>(Arc<Mutex<T>>);
impl<T: AsyncRead + Unpin> AsyncRead for SharedIO<T> {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut [u8],
    ) -> std::task::Poll<futures_io::Result<usize>> {
        let mut l = self.0.lock().unwrap();
        Pin::new(&mut *l).poll_read(cx, buf)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for SharedIO<T> {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<futures_io::Result<usize>> {
        let mut l = self.0.lock().unwrap();
        Pin::new(&mut *l).poll_write(cx, buf)
    }
    fn poll_close(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<futures_io::Result<()>> {
        let mut l = self.0.lock().unwrap();
        Pin::new(&mut *l).poll_close(cx)
    }
    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<futures_io::Result<()>> {
        let mut l = self.0.lock().unwrap();
        Pin::new(&mut *l).poll_flush(cx)
    }
}
impl IoHandler for PlatformIO {
    type IO = SharedIO<Compat<DefaultInterface>>;
    fn new_io(&self) -> Self::IO {
        SharedIO(self.f.clone())
    }
}

pub type PlatformInterface = PlatformIO;
pub type DefaultInterface = AsyncTokioInterface;
