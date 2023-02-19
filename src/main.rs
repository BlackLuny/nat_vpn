use anyhow::anyhow;
use libp2p::core::muxing::StreamMuxerBox;
use libp2p::core::transport::upgrade;
use libp2p::multiaddr::Protocol;
use libp2p::swarm::derive_prelude::EitherOutput;
use libp2p::swarm::SwarmBuilder;
use libp2p::swarm::{NetworkBehaviour, SwarmEvent};
use libp2p::yamux::YamuxConfig;
use libp2p::{identity, ping, Multiaddr, PeerId, Swarm};
use libp2p::{relay, Transport};

use std::env::set_var;
use std::error::Error;
use tokio::time::sleep;
use tunio::{
    traits::{DriverT, InterfaceT},
    DefaultDriver,
};
mod cloned_io;
mod copy_future;
mod data_protoc_v2;

cfg_if::cfg_if! {
    if #[cfg(target_os = "windows")] {
        mod windows_io;
        use windows_io::{DefaultInterface, PlatformInterface};
    }else if #[cfg(unix)] {
        mod linux_io;
        use linux_io::{DefaultInterface, PlatformInterface};
    }
}

// mod copy_fut;
// Copyright 2021 Protocol Labs.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

use clap::Parser;
use data_protoc_v2::Behaviour as DataBehaviour;
use futures::future::FutureExt;
use futures::stream::StreamExt;
use log::info;
use std::convert::TryInto;
use std::str::FromStr;
#[derive(Debug, Parser)]
#[clap(name = "libp2p DCUtR client")]
struct Opts {
    /// The mode (client-listen, client-dial).
    #[clap(long)]
    itf: String,

    #[clap(long)]
    ip: String,

    #[clap(long)]
    mode: Mode,

    /// Fixed value to generate deterministic peer id.
    #[clap(long)]
    secret_key_seed: u8,

    /// The listening address
    #[clap(long)]
    relay_address: Multiaddr,

    /// Peer ID of the remote peer to hole punch to.
    #[clap(long)]
    remote_peer_id: Option<PeerId>,
}

#[derive(Clone, Debug, PartialEq, Parser)]
enum Mode {
    Dial,
    Listen,
}

impl FromStr for Mode {
    type Err = String;
    fn from_str(mode: &str) -> Result<Self, Self::Err> {
        match mode {
            "dial" => Ok(Mode::Dial),
            "listen" => Ok(Mode::Listen),
            _ => Err("Expected either 'dial' or 'listen'".to_string()),
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    set_var("RUST_LOG", "info");
    env_logger::init();
    let opts = Opts::parse();
    let mut driver = DefaultDriver::new().map_err(|_| anyhow!("create driver error"))?;
    let if_config = DefaultInterface::config_builder().name(opts.itf).build()?;
    let tun = DefaultInterface::new_up(&mut driver, if_config)
        .map_err(|_| anyhow!("new_up error"))?;
    tun.handle().add_address(opts.ip.parse()?)?;

    let local_key = generate_ed25519(opts.secret_key_seed);
    let local_peer_id = PeerId::from(local_key.public());
    info!("Local peer id: {:?}", local_peer_id);

    let (relay_transport, client) =
        relay::v2::client::Client::new_transport_and_behaviour(local_peer_id);
    let transport = relay_transport
        .upgrade(upgrade::Version::V1)
        .authenticate(libp2p::plaintext::PlainText2Config {
            local_public_key: local_key.public(),
        })
        .multiplex(YamuxConfig::default())
        .or_transport(libp2p::quic::tokio::Transport::new(
            libp2p::quic::Config::new(&local_key),
        ))
        .map(|either, _| match either {
            EitherOutput::First((p, y)) => (p, StreamMuxerBox::new(y)),
            EitherOutput::Second((p, c)) => (p, StreamMuxerBox::new(c)),
        })
        .boxed();

    #[derive(NetworkBehaviour)]
    #[behaviour(out_event = "Event", event_process = false)]
    struct Behaviour {
        relay_client: relay::v2::client::Client,
        ping: ping::Behaviour,
        identify: libp2p::identify::Behaviour,
        data: DataBehaviour<PlatformInterface>,
    }

    #[derive(Debug)]
    #[allow(clippy::large_enum_variant)]
    enum Event {
        Ping(ping::Event),
        Identify(libp2p::identify::Event),
        Relay(relay::v2::client::Event),
        DataEvent(()),
    }
    impl From<()> for Event {
        fn from(_value: ()) -> Self {
            Event::DataEvent(())
        }
    }
    impl From<ping::Event> for Event {
        fn from(e: ping::Event) -> Self {
            Event::Ping(e)
        }
    }

    impl From<libp2p::identify::Event> for Event {
        fn from(e: libp2p::identify::Event) -> Self {
            Event::Identify(e)
        }
    }

    impl From<relay::v2::client::Event> for Event {
        fn from(e: relay::v2::client::Event) -> Self {
            Event::Relay(e)
        }
    }
    let behaviour = Behaviour {
        relay_client: client,
        ping: ping::Behaviour::new(ping::Config::new()),
        identify: libp2p::identify::Behaviour::new(libp2p::identify::Config::new(
            "/TODO/0.0.1".to_string(),
            local_key.public(),
        )),
        data: DataBehaviour::new(PlatformInterface::new(tun)),
    };

    let mut swarm = SwarmBuilder::with_tokio_executor(transport, behaviour, local_peer_id)
        .dial_concurrency_factor(10_u8.try_into().unwrap())
        .build();

    swarm
        .listen_on("/ip4/0.0.0.0/udp/0/quic-v1".parse().unwrap())
        .unwrap();

    // Wait to listen on all interfaces.

    loop {
        futures::select! {
            event = swarm.next() => {
                match event.unwrap() {
                    SwarmEvent::NewListenAddr { address, .. } => {
                        info!("Listening on {:?}", address);
                    }
                    event => panic!("{event:?}"),
                }
            }
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)).fuse() => {
                // Likely listening on all interfaces now, thus continuing by breaking the loop.
                break;
            }
        }
    }

    // Connect to the relay server. Not for the reservation or relayed connection, but to (a) learn
    // our local public address and (b) enable a freshly started relay to learn its public address.
    // swarm.dial(opts.relay_address.clone()).unwrap();

    async fn try_to_dial_to_relay(
        swarm: &mut Swarm<Behaviour>,
        relay_addr: Multiaddr,
    ) -> anyhow::Result<()> {
        loop {
            if let Ok(_) = swarm.dial(relay_addr.clone()) {
                let mut learned_observed_addr = false;
                let mut told_relay_observed_addr = false;
                loop {
                    match swarm.next().await.unwrap() {
                        SwarmEvent::NewListenAddr { .. } => {}
                        SwarmEvent::Dialing { .. } => {}
                        SwarmEvent::ConnectionEstablished { .. } => {}
                        SwarmEvent::Behaviour(Event::Ping(_)) => {}
                        SwarmEvent::Behaviour(Event::Identify(libp2p::identify::Event::Sent {
                            ..
                        })) => {
                            info!("Told relay its public address.");
                            told_relay_observed_addr = true;
                        }
                        SwarmEvent::Behaviour(Event::Identify(
                            libp2p::identify::Event::Received {
                                info: libp2p::identify::Info { observed_addr, .. },
                                ..
                            },
                        )) => {
                            info!("Relay told us our public address: {:?}", observed_addr);
                            learned_observed_addr = true;
                        }
                        event => println!("{event:?}"),
                    }

                    if learned_observed_addr && told_relay_observed_addr {
                        return Ok(());
                    }
                }
            } else {
                sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }

    async fn connect_to_remote(
        swarm: &mut Swarm<Behaviour>,
        relay_addr: Multiaddr,
        remote: Option<PeerId>,
        mode: &Mode,
    ) {
        match mode {
            &Mode::Dial => {
                while let Err(_e) = swarm.dial(
                    relay_addr
                        .clone()
                        .with(Protocol::P2pCircuit)
                        .with(Protocol::P2p(remote.unwrap().into())),
                ) {
                    sleep(std::time::Duration::from_secs(5)).await;
                }
            }
            &Mode::Listen => {
                while let Err(_e) = swarm.listen_on(relay_addr.clone().with(Protocol::P2pCircuit)) {
                    sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
    let _ = try_to_dial_to_relay(&mut swarm, opts.relay_address.clone()).await;
    let _ = connect_to_remote(
        &mut swarm,
        opts.relay_address.clone(),
        opts.remote_peer_id.clone(),
        &opts.mode,
    )
    .await;
    loop {
        let event = swarm.next().await;
        match event.unwrap() {
            SwarmEvent::NewListenAddr { address, .. } => {
                println!("Listening on {:?}", address);
            }
            SwarmEvent::Behaviour(Event::Relay(
                relay::v2::client::Event::ReservationReqAccepted { .. },
            )) => {
                assert!(opts.mode == Mode::Listen);
                info!("Relay accepted our reservation request.");
            }
            SwarmEvent::Behaviour(Event::Relay(event)) => {
                info!("{:?}", event)
            }
            SwarmEvent::Behaviour(Event::Identify(event)) => {
                info!("{:?}", event)
            }
            SwarmEvent::Behaviour(Event::Ping(_)) => {}
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                println!("Established connection to {:?} via {:?}", peer_id, endpoint);
                if matches!(opts.mode, Mode::Dial) {
                    swarm.behaviour_mut().data.open_stream(peer_id);
                }
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error } => {
                println!("Outgoing connection error to {:?}: {:?}", peer_id, error);
            }
            SwarmEvent::ConnectionClosed {
                peer_id,
                endpoint: _,
                num_established: _,
                cause,
            } => {
                println!("ConnectionClosed {peer_id:?}, {cause:?}");
                println!("retry connect");
                let _ = try_to_dial_to_relay(&mut swarm, opts.relay_address.clone()).await;
                let _ = connect_to_remote(
                    &mut swarm,
                    opts.relay_address.clone(),
                    opts.remote_peer_id.clone(),
                    &opts.mode,
                )
                .await;
            }
            o => {
                println!("{o:?}");
            }
        }
    }
}

fn generate_ed25519(secret_key_seed: u8) -> identity::Keypair {
    let mut bytes = [0u8; 32];
    bytes[0] = secret_key_seed;

    let secret_key = identity::ed25519::SecretKey::from_bytes(&mut bytes)
        .expect("this returns `Err` only if the length is wrong; the length is correct; qed");
    identity::Keypair::Ed25519(secret_key.into())
}
