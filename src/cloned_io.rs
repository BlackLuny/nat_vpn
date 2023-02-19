use futures::{AsyncRead, AsyncWrite};

pub trait IoHandler:'static {
    type IO: AsyncRead + AsyncWrite +Unpin;
    fn new_io(&self) ->Self::IO;
}