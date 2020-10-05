#![allow(dead_code)]
use ockam_common::commands::ockam_commands::*;
use ockam_message::message::*;
use ockam_router::router::*;
use ockam_transport::transport::*;
use ockam_channel::*;
use ockam_vault::software::DefaultVault;
use std::net::SocketAddr;
use std::str;
use std::str::FromStr;
use std::{thread, time};
use structopt::StructOpt;
use std::sync::{Arc, Mutex};
use ockam_message::message::Address::ChannelAddress;
use ockam_kex::xx::{XXInitiator, XXResponder};

#[derive(StructOpt, Debug)]
#[structopt(author = "Ockam Developers (ockam.io)")]
pub struct Args {
    /// Local address to bind socket
    #[structopt(short = "l", long = "local")]
    local_socket: Option<String>,

    /// Remote address to send to
    #[structopt(short = "r", long = "remote")]
    remote_socket: Option<String>,

    /// Via - intermediate router
    #[structopt(short = "v", long = "via")]
    via_socket: Option<String>,

    /// Worker
    #[structopt(short = "w", long = "worker")]
    worker_addr: Option<String>,

    /// Message - message to send
    #[structopt(short = "m", long = "message")]
    message: Option<String>,
}

pub struct TestWorker {
    rx: std::sync::mpsc::Receiver<OckamCommand>,
    _tx: std::sync::mpsc::Sender<OckamCommand>,
    router_tx: std::sync::mpsc::Sender<OckamCommand>,
    address: Address,
    channel_address: Address,
    pending_message: Option<Message>,
    toggle: u16,
}

impl TestWorker {
    pub fn new(
        rx: std::sync::mpsc::Receiver<OckamCommand>,
        tx: std::sync::mpsc::Sender<OckamCommand>,
        router_tx: std::sync::mpsc::Sender<OckamCommand>,
        channel_address: Option<Address>,
    ) -> Self {
        if let Err(_error) = router_tx.send(OckamCommand::Router(RouterCommand::Register(
            AddressType::Worker,
            tx.clone(),
        ))) {
            println!("send failed in TestChannel::new")
        }
        let mut channel = Address::ChannelAddress(vec![0,0,0,0]);
        match channel_address {
            Some(ca) => { channel = ca; }
            _ => {}
        }
        TestWorker {
            rx,
            _tx: tx,
            router_tx,
            address: Address::WorkerAddress(vec![0,1,2,3]),
            channel_address: channel,
            pending_message: None,
            toggle: 0,
        }
    }

    pub fn request_channel(&mut self, mut m: Message) -> Result<(), String> {
        let pending_message = Message {
            onward_route: Route{ addresses: vec![] },
            return_route: m.return_route.clone(),
            message_type: MessageType::Payload,
            message_body: m.message_body.clone()
        };
        self.pending_message = Some(pending_message);
        m.onward_route.addresses.insert(0, RouterAddress::from_address(self.channel_address.clone()).unwrap());
        m.return_route.addresses.insert(0, RouterAddress::from_address(self.address.clone()).unwrap());
        m.message_type = MessageType::None;
        m.message_body = vec![];
        self.router_tx.send(OckamCommand::Router(RouterCommand::SendMessage(m)));
        Ok(())
    }

    pub fn handle_send(&mut self, mut m: Message) -> Result<(), String> {
        if self.channel_address.as_string() == "00000000".to_string() {
            self.request_channel(m)
        } else {
            m.onward_route = Route { addresses: vec![RouterAddress::from_address(self.channel_address.clone()).unwrap()] };
            m.return_route.addresses.insert(0, RouterAddress::from_address(self.address.clone()).unwrap());
            match self
                .router_tx
                .send(OckamCommand::Router(RouterCommand::SendMessage(m)))
            {
                Ok(()) => Ok(()),
                Err(_unused) => {
                    println!("send to router failed");
                    Err("handle_send failed in TestWorker".into())
                }
            }
        }
    }

    pub fn receive_channel(&mut self, m: Message) -> Result<(), String> {
        // This should be an empty message with 1 return address, that being a secure channel
        if m.return_route.addresses.len() != 1 { return Err("bad return route".into()); }
        self.channel_address = m.return_route.addresses[0].address.clone();
        let m_opt = self.pending_message.clone();
        match m_opt {
            Some(mut m) => {
                let channel = RouterAddress::from_address(self.channel_address.clone()).unwrap();
                m.onward_route = Route { addresses: vec![channel, RouterAddress::worker_router_address_from_str("00010203").unwrap()] };
                m.return_route = Route { addresses: vec![RouterAddress::from_address(self.address.clone()).unwrap()]};
                self.router_tx.send(OckamCommand::Router(RouterCommand::SendMessage(m)));
                self.pending_message = None;
                return Ok(());
            },
            _ => { return Ok(()); }
        }
        Ok(())
    }

    pub fn handle_receive(&mut self, m: Message) -> Result<(), String> {
        match m.message_type {
            MessageType::None => {
                self.receive_channel(m)
            }
            MessageType::Payload => {
                let s: &str;
                if 0 == self.toggle % 2 {
                    s = "Hello Ockam";
                } else {
                    s = "Goodbye Ockam"
                };
                self.toggle += 1;
                let mut reply: Message = Message {
                    onward_route: Route { addresses: vec![] },
                    return_route: Route { addresses: vec![] },
                    message_type: MessageType::Payload,
                    message_body: s.as_bytes().to_vec(),
                };
                reply.onward_route.addresses = m.return_route.addresses.clone();
                if let Ok(r) = RouterAddress::worker_router_address_from_str("01020304") {
                    reply.return_route.addresses.push(r);
                } else {
                    return Err("handle_receive".into());
                }
                match self
                    .router_tx
                    .send(OckamCommand::Router(RouterCommand::SendMessage(reply)))
                {
                    Ok(()) => {}
                    Err(_unused) => {
                        println!("send to router failed");
                        return Err("send to router failed in TestWorker".into());
                    }
                }
                Ok(())
            }
            _ => {Err("worker got bad message type".into()) }
        }
    }

    pub fn poll(&mut self) -> bool {
        let mut keep_going = true;
        let mut got = true;
        while got {
            got = false;
            if let Ok(c) = self.rx.try_recv() {
                got = true;
                match c {
                    OckamCommand::Worker(WorkerCommand::Test) => {
                        println!("Worker got test command");
                    }
                    OckamCommand::Worker(WorkerCommand::SendMessage(mut m)) => {
                        println!("Worker got send");
                        self.handle_send(m).unwrap();
                    }
                    OckamCommand::Worker(WorkerCommand::ReceiveMessage(mut m)) => {
                        println!(
                            "Worker received message: {}",
                            str::from_utf8(&m.message_body).unwrap()
                        );
                        self.handle_receive(m).unwrap();
                    }
                    OckamCommand::Worker(WorkerCommand::Stop) => {
                        keep_going = false;
                    }
                    _ => println!("Worker got bad message"),
                }
            }
        }
        keep_going
    }
}

pub fn get_route() -> Option<Route> {
    let mut r = Route { addresses: vec![] };
    if let Ok(router_addr_0) = RouterAddress::udp_router_address_from_str("127.0.0.1:4051") {
        r.addresses.push(router_addr_0);
    } else {
        return None;
    };

    if let Ok(channel_addr) = RouterAddress::channel_router_address_from_str("01020304") {
        r.addresses.push(channel_addr);
    } else {
        return None;
    };

    Some(r)
}

pub fn parse_args(args: Args) -> Result<(RouterAddress, Route, String), String> {
    let mut local_socket: RouterAddress = RouterAddress {
        a_type: AddressType::Udp,
        length: 7,
        address: (Address::UdpAddress(SocketAddr::from_str("127.0.0.1:4050").unwrap())),
    };
    if let Some(l) = args.local_socket {
        if let Ok(sa) = SocketAddr::from_str(&l) {
            if let Some(ra) = RouterAddress::from_address(Address::UdpAddress(sa)) {
                local_socket = ra;
            }
        }
    } else {
        return Err("local socket address required: -l xxx.xxx.xxx.xxx:pppp".to_string());
    }

    let mut route = Route { addresses: vec![] };

    if let Some(vs) = args.via_socket {
        if let Ok(sa) = SocketAddr::from_str(&vs) {
            if let Some(ra) = RouterAddress::from_address(Address::UdpAddress(sa)) {
                route.addresses.push(ra);
            }
        }
    };

    if let Some(rs) = args.remote_socket {
        if let Ok(sa) = SocketAddr::from_str(&rs) {
            if let Some(ra) = RouterAddress::from_address(Address::UdpAddress(sa)) {
                route.addresses.push(ra);
            }
        }
    };

    if let Some(wa) = args.worker_addr {
        if let Ok(ra) = RouterAddress::worker_router_address_from_str(&wa) {
            route.addresses.push(ra);
        }
    };

    let mut message = "Hello Ockam".to_string();
    if let Some(m) = args.message {
        message = m;
    };

    Ok((local_socket, route, message))
}

pub fn start_node(local_socket: RouterAddress, onward_route: Route, payload: String) {
    let (transport_tx, transport_rx) = std::sync::mpsc::channel();
    let (router_tx, router_rx) = std::sync::mpsc::channel();
    let (worker_tx, worker_rx) = std::sync::mpsc::channel();
    let (channel_tx, channel_rx) = std::sync::mpsc::channel();
    let vault = Arc::new(Mutex::new(DefaultVault::default()));

    let mut router = Router::new(router_rx);

    let mut worker = TestWorker::new(
        worker_rx,
        worker_tx.clone(),
        router_tx.clone(),
        Some(Address::ChannelAddress(vec![0,0,0,0])));

    let sock_str: String;
    match local_socket.address {
        Address::UdpAddress(udp) => {
            sock_str = udp.to_string();
            println!("{}", udp.to_string());
        }
        _ => return,
    }

    let mut transport =
        UdpTransport::new(transport_rx, transport_tx, router_tx.clone(), &sock_str).unwrap();

    let _join_thread: thread::JoinHandle<_> = thread::spawn(move || {
        type XXChannelManager = ChannelManager<XXInitiator, XXResponder, XXInitiator>;
        let mut channel_handler = XXChannelManager::new(
            channel_rx,
            channel_tx.clone(),
            router_tx.clone(),
            vault).unwrap();

        while transport.poll() && router.poll() && channel_handler.poll().unwrap() && worker.poll() {
            thread::sleep(time::Duration::from_millis(100));
        }
    });

    let test_cmd = OckamCommand::Worker(WorkerCommand::Test);
    worker_tx.send(test_cmd);

    if !onward_route.addresses.is_empty() && !payload.is_empty() {
        let m = Message {
            onward_route,
            return_route: Route { addresses: vec![] },
            message_type: MessageType::Payload,
            message_body: payload.as_bytes().to_vec(),
        };
        let command = OckamCommand::Worker(WorkerCommand::SendMessage(m));
        match worker_tx.send(command) {
            Ok(_unused) => {}
            Err(_unused) => {
                println!("failed send to worker");
            }
        }
    }
}

fn main() {
    let args = Args::from_args();
    println!("{:?}", args);
    let local_socket: RouterAddress;
    let route: Route;
    let message: String;
    match parse_args(args) {
        Ok((ls, r, m)) => {
            local_socket = ls;
            route = r;
            message = m;
        }
        Err(s) => {
            println!("{}", s);
            return;
        }
    }

    start_node(local_socket, route, message);

    thread::sleep(time::Duration::from_millis(1000000));
}
