use std::{sync::Arc, task::Poll};
use futures::{AsyncRead, AsyncWrite, FutureExt};
use libp2p::{
    core::upgrade::ReadyUpgrade,
    swarm::{
        handler::{ConnectionEvent, FullyNegotiatedInbound, FullyNegotiatedOutbound},
        ConnectionHandler, ConnectionHandlerEvent, IntoConnectionHandler, NegotiatedSubstream,
        NetworkBehaviour, NetworkBehaviourAction, SubstreamProtocol,
    },
    PeerId,
};

pub struct Handler<U> {
    tun: Option<U>,
    fut: Option<CopyFuture<U, NegotiatedSubstream>>,
    init_stream: bool,
}
use thiserror::Error;

use crate::{cloned_io::IoHandler, copy_future::CopyFuture};

#[derive(Error, Debug)]
pub enum Error {}

impl<U: AsyncRead + AsyncWrite + 'static + Send + Unpin> ConnectionHandler for Handler<U> {
    type Error = Error;
    type InEvent = ();
    type OutEvent = ();
    type InboundOpenInfo = ();
    type OutboundOpenInfo = ();
    type InboundProtocol = ReadyUpgrade<&'static [u8]>;
    type OutboundProtocol = ReadyUpgrade<&'static [u8]>;
    fn connection_keep_alive(&self) -> libp2p::swarm::KeepAlive {
        libp2p::swarm::KeepAlive::Yes
    }
    fn on_behaviour_event(&mut self, _event: Self::InEvent) {
        if self.fut.is_none() {
            self.init_stream = true;
        }
    }
    fn listen_protocol(
        &self,
    ) -> libp2p::swarm::SubstreamProtocol<Self::InboundProtocol, Self::InboundOpenInfo> {
        SubstreamProtocol::new(ReadyUpgrade::new("/tun/1.0".as_bytes()), ())
    }
    fn poll(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<
        libp2p::swarm::ConnectionHandlerEvent<
            Self::OutboundProtocol,
            Self::OutboundOpenInfo,
            Self::OutEvent,
            Self::Error,
        >,
    > {
        if self.init_stream {
            self.init_stream = false;
            return Poll::Ready(ConnectionHandlerEvent::OutboundSubstreamRequest {
                protocol: SubstreamProtocol::new(ReadyUpgrade::new("/tun/1.0".as_bytes()), ()),
            });
        }
        self.fut.as_mut().map(|f| f.poll_unpin(cx));
        Poll::Pending
    }
    fn on_connection_event(
        &mut self,
        event: libp2p::swarm::handler::ConnectionEvent<
            Self::InboundProtocol,
            Self::OutboundProtocol,
            Self::InboundOpenInfo,
            Self::OutboundOpenInfo,
        >,
    ) {
        match event {
            ConnectionEvent::FullyNegotiatedInbound(FullyNegotiatedInbound {
                protocol, ..
            })
            | ConnectionEvent::FullyNegotiatedOutbound(FullyNegotiatedOutbound {
                protocol, ..
            }) => {
                self.fut = Some(CopyFuture::new(self.tun.take().unwrap(), protocol));
            }
            _ => {
                println!("other");
            }
        }
    }
}

pub struct Prototype<T> {
    io: Arc<T>,
}
impl<T: 'static + IoHandler<IO = U> + Send + Sync, U: 'static + Send + AsyncRead + AsyncWrite + Unpin>
    IntoConnectionHandler for Prototype<T>
{
    type Handler = Handler<U>;
    fn into_handler(
        self,
        _remote_peer_id: &PeerId,
        _connected_point: &libp2p::core::ConnectedPoint,
    ) -> Self::Handler {
        Handler {
            tun: Some(self.io.new_io()),
            fut: None,
            init_stream: false,
        }
    }
    fn inbound_protocol(&self) -> <Self::Handler as ConnectionHandler>::InboundProtocol {
        ReadyUpgrade::new("/tun/1.0".as_bytes())
    }
}

pub struct Behaviour<T> {
    init_stream: Option<PeerId>,
    tun: Arc<T>,
}

impl<T> Behaviour<T> {
    pub fn new(tun: T) -> Self {
        Self {
            // itf: tun,
            init_stream: None,
            tun: Arc::new(tun),
        }
    }
    pub fn open_stream(&mut self, peer_id: PeerId) {
        self.init_stream = Some(peer_id);
    }
}

impl<T: 'static + IoHandler<IO = U> + Send + Sync, U: 'static + Send + AsyncRead + AsyncWrite + Unpin> NetworkBehaviour for Behaviour<T> {
    type ConnectionHandler = Prototype<T>;
    type OutEvent = ();
    fn new_handler(&mut self) -> Self::ConnectionHandler {
        Prototype {
            io: self.tun.clone(),
        }
    }
    fn poll(
        &mut self,
        _cx: &mut std::task::Context<'_>,
        _params: &mut impl libp2p::swarm::PollParameters,
    ) -> std::task::Poll<
        libp2p::swarm::NetworkBehaviourAction<Self::OutEvent, Self::ConnectionHandler>,
    > {
        if let Some(peer_id) = self.init_stream.take() {
            return Poll::Ready(NetworkBehaviourAction::NotifyHandler {
                peer_id,
                handler: libp2p::swarm::NotifyHandler::Any,
                event: (),
            });
        }
        Poll::Pending
    }
}
