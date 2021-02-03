use crate::{
    base::{
        download::BLOCK_SIZE,
        ipc::{bytes_to_u32, Message, IPC},
        manager::ManagerEvent,
        spawn_and_log_error, Result,
    },
    bencode::{
        decode::DecodeError,
        hash::Sha1,
        value::{FromValue, Value},
    },
};
use async_std::{net::Ipv4Addr, net::TcpStream, prelude::*, task};
use futures::{
    channel::mpsc::{self, Receiver, Sender},
    select,
    sink::SinkExt,
    FutureExt, StreamExt,
};
use rand::Rng;
use std::collections::HashMap;

use async_std::task::JoinHandle;
use futures::channel::mpsc::UnboundedSender;
use crate::base::terminal;

const PROTOCOL: &'static str = "BitTorrent protocol";
const MAX_CONCURRENT_REQUESTS: u32 = 10;

#[derive(PartialEq, Debug, Eq, Hash, Clone)]
pub struct Peer {
    pub ip: String,
    pub port: u16,
}

impl Peer {
    pub fn from_bytes(v: &[u8]) -> Peer {
        let ip = Ipv4Addr::new(v[0], v[1], v[2], v[3]);
        let port = (v[4] as u16) * 256 + (v[5] as u16);
        Peer {
            ip: ip.to_string(),
            port,
        }
    }
}

impl FromValue for Peer {
    fn from_value(value: &Value) -> std::result::Result<Self, DecodeError> {
        let peer = Peer {
            ip: value.get_field("ip")?,
            port: value.get_field("port")?,
        };
        Ok(peer)
    }
}

pub struct PeerConnection {
    halt: bool,
    our_peer_id: String,
    info_hash: Sha1,
    // send_handshake_first: bool,
    stream: TcpStream,
    me: PeerMetadata,
    he: PeerMetadata,

    to_request: HashMap<(u32, u32), (u32, u32, u32)>,
    upload_in_progress: bool,

    writer_sender: Sender<Message>,
    manager_sender: UnboundedSender<ManagerEvent>,
    to_download: Sender<ManagerEvent>,
}

impl PeerConnection {
    async fn handshake(&mut self, send_handshake_first: bool) -> Result<()> {
        if send_handshake_first {
            self.send_handshake().await?;
            self.receive_handshake().await?;
        } else {
            self.receive_handshake().await?;
            self.send_handshake().await?;
        }

        Ok(())
    }

    async fn send_handshake(&mut self) -> Result<()> {
        let message = {
            let mut message = vec![];
            message.push(PROTOCOL.len() as u8);
            message.extend(PROTOCOL.bytes());
            message.extend(vec![0; 8].into_iter());
            message.extend(self.info_hash.iter());
            message.extend(self.our_peer_id.bytes());
            message
        };
        let stream = &mut self.stream;
        // stream.write_all(&[1,2]).await?;
        // let stream = &*self.stream;
        stream.write_all(message.as_slice()).await?;
        Ok(())
    }

    async fn receive_handshake(&mut self) -> Result<()> {
        let stream = &mut self.stream;

        terminal::print_log(format!("task {}: start receive", task::current().id(),)).await?;
        let pstrlen = read_n(stream, 1).await?;
        terminal::print_log(format!("{}: receive pstrlen", task::current().id(),)).await?;
        read_n(stream, pstrlen[0] as u32).await?; // ignore pstr
        read_n(stream, 8).await?; // ignore reserved
        let info_hash = read_n(stream, 20).await?;
        let peer_id = read_n(stream, 20).await?;

        {
            // validate info hash
            if &info_hash != &self.info_hash.0 {
                terminal::print_log(format!("{}", crate::bencode::hash::to_hex(&info_hash))).await?;
                terminal::print_log(format!("{}", crate::bencode::hash::to_hex(&self.info_hash.0))).await?;

                Err("Error::InvalidInfoHash")?;
            }

            // validate peer id
            let our_peer_id: Vec<u8> = self.our_peer_id.bytes().collect();
            if peer_id == our_peer_id {
                Err("Error::ConnectingToSelf")?;
            }
        }

        Ok(())
    }

    async fn request_more_blocks(&mut self) -> Result<()> {
        let is_choked = self.me.is_choked;
        let is_interested = self.me.is_interested;
        let len = self.to_request.len();

        if is_choked || !is_interested || len == 0 {
            return Ok(());
        }

        let mut req_len = self.me.requests.len();
        while req_len < MAX_CONCURRENT_REQUESTS as usize {
            let len = self.to_request.len();
            // terminal::print_log(format!("to_request {}", len)).await?;
            if len == 0 {
                return Ok(());
            }
            // remove a block at random from to_request
            let (piece_index, block_index, block_length) = {
                // todo: random index
                // let index = rand::thread_rng().gen_range(0, len);
                let index = 0;
                let target = self.to_request.keys().nth(index).unwrap().clone();
                self.to_request.remove(&target).unwrap()
            };
            // add a request
            let offset = block_index * BLOCK_SIZE;
            if self
                .me
                .requests
                .add(piece_index, block_index, offset, block_length)
            {
                self.writer_sender
                    .send(Message::Request(piece_index, offset, block_length))
                    .await?;
            };
            req_len = self.me.requests.len();
        }
        Ok(())
    }

    async fn upload_next_block(&mut self) -> Result<()> {
        if self.upload_in_progress || self.he.is_choked || !self.he.is_interested {
            return Ok(());
        }

        match self.he.requests.pop() {
            Some(r) => {
                let data = {
                    let (sender, receiver) = futures::channel::oneshot::channel();
                    self.to_download
                        .send(ManagerEvent::RequireData(r.clone(), sender))
                        .await?;
                    receiver.await?
                };
                self.upload_in_progress = true;
                self.writer_sender
                    .send(Message::Piece(r.piece_index, r.offset, data))
                    .await?
            }
            None => (),
        };
        Ok(())
    }

    async fn queue_blocks(&mut self, piece_index: u32) -> Result<()> {
        let incomplete_blocks = {
            let (sender, receiver) = futures::channel::oneshot::channel();
            self.to_download
                .send(ManagerEvent::RequireIncompleteBlocks(piece_index, sender))
                .await?;
            receiver.await?
        };

        for (block_index, block_length) in incomplete_blocks {
            if !self.me.requests.has(piece_index, block_index) {
                self.to_request.insert(
                    (piece_index, block_index),
                    (piece_index, block_index, block_length),
                );
            }
        }
        Ok(())
    }

    async fn update_my_interested_status(&mut self) -> Result<()> {
        let am_interested = self.me.requests.len() > 0 || self.to_request.len() > 0;
        let is_interested = self.me.is_interested;

        if is_interested != am_interested {
            self.me.is_interested = am_interested;
            let message = if am_interested {
                Message::Interested
            } else {
                Message::NotInterested
            };
            self.writer_sender.send(message).await?;
        }
        Ok(())
    }

    async fn send_bitfield(&mut self) -> Result<()> {
        let mut bytes: Vec<u8> =
            vec![0; (self.me.has_pieces.len() as f64 / 8 as f64).ceil() as usize];
        // todo:1 block here
        // bytes = vec![0; self.me.lock().await.has_pieces.len()];
        let l = self.me.has_pieces.len();
        for have_index in 0..l {
            let bytes_index = have_index / 8;
            let index_into_byte = have_index % 8;
            if self.me.has_pieces[have_index] {
                let mask = 1 << (7 - index_into_byte);
                bytes[bytes_index] |= mask;
            }
        }
        terminal::print_log(format!("{:?}: send bitfield", task::current().id())).await?;
        self.writer_sender.send(Message::Bitfield(bytes)).await?;
        Ok(())
    }
}

fn start_read_loop(stream: TcpStream, ipc_sender: Sender<IPC>) -> Receiver<Void> {
    let (shutdown_sender, shutdown) = mpsc::channel(1);
    spawn_and_log_error(async move {
        let res = conn_read_loop(stream, ipc_sender, shutdown_sender).await;
        terminal::print_log(format!("conn_read_loop over!")).await?;
        res
    });

    shutdown
}

fn start_write_loop(
    stream: TcpStream,
    ipc_sender: Sender<IPC>,
    writer_receiver: Receiver<Message>,
) -> JoinHandle<()> {
    let write_handle = spawn_and_log_error(async move {
        let e = conn_write_loop(writer_receiver, stream, ipc_sender).await;
        terminal::print_log(format!("conn_write_loop over!")).await?;
        e
    });

    write_handle
}

pub async fn peer_conn_loop(
    send_handshake_first: bool,
    our_peer_id: String,
    info_hash: Sha1,
    peer: Peer,
    ipc_sender: Sender<IPC>,
    ipcs: Receiver<IPC>,
    manager_sender: UnboundedSender<ManagerEvent>,
    mut require: Sender<ManagerEvent>,
) -> Result<()> {
    let have_pieces = {
        let (sender, receiver) = futures::channel::oneshot::channel();
        require
            .send(ManagerEvent::RequireHavePieces(sender))
            .await?;
        receiver.await?
    };
    let num_pieces = have_pieces.len();

    let stream = {
        let ip: Ipv4Addr = peer.ip.parse().unwrap();
        TcpStream::connect((ip, peer.port)).await?
    };

    let (writer_sender, writer_receiver) = mpsc::channel(10);

    let mut peer_conn = PeerConnection {
        halt: false,
        our_peer_id,
        info_hash,
        stream,
        me: PeerMetadata::new(have_pieces),
        he: PeerMetadata::new(vec![false; num_pieces]),
        to_request: Default::default(),
        upload_in_progress: false,
        writer_sender,
        manager_sender,
        to_download: require,
    };

    peer_conn.handshake(send_handshake_first).await?;

    let shutdown = start_read_loop(peer_conn.stream.clone(), ipc_sender.clone());
    let write_handle = start_write_loop(peer_conn.stream.clone(), ipc_sender, writer_receiver);

    // send a bitfield message letting peer know what we have
    peer_conn.send_bitfield().await?;

    let mut ipcs = ipcs.fuse();
    let mut shutdown = shutdown.fuse();
    while !peer_conn.halt {
        let ipc = select! {
            void = shutdown.next().fuse() => match void {
                Some(_void) => panic!("never reached!"),
                None => break,
            },
            ipc = ipcs.next().fuse() => match ipc {
                Some(ipc) => ipc,
                None => {
                    peer_conn.halt = true;
                    break;
                }
            },
        };
        // terminal::print_log(format!("ipc loop: {:?}", ipc)).await?;

        match ipc {
            IPC::Message(message) => process_message(&mut peer_conn, message).await?,
            IPC::BlockComplete(piece_index, block_index) => {
                peer_conn.to_request.remove(&(piece_index, block_index));
                match peer_conn.me.requests.remove(piece_index, block_index) {
                    Some(r) => {
                        peer_conn
                            .writer_sender
                            .send(Message::Cancel(r.piece_index, r.offset, r.block_length))
                            .await?
                    }
                    None => (),
                }
            }
            IPC::PieceComplete(piece_index) => {
                peer_conn.me.has_pieces[piece_index as usize] = true;
                peer_conn.update_my_interested_status().await?;
                peer_conn
                    .writer_sender
                    .send(Message::Have(piece_index))
                    .await?;
            }
            IPC::DownloadComplete => {
                // peer_conn.halt = true;
                peer_conn.update_my_interested_status().await?;
            }
            IPC::BlockUploaded => {
                peer_conn.upload_in_progress = false;
                peer_conn.upload_next_block().await?;
            }
        }
    }
    drop(peer_conn);
    write_handle.await;
    Ok(())
}

#[derive(Debug)]
enum Void {}

async fn conn_read_loop(
    mut stream: TcpStream,
    mut sender: Sender<IPC>,
    _shutdown: Sender<Void>,
) -> Result<()> {
    let stream = &mut stream;
    // let message_size = ;
    terminal::print_log(format!("task {}: conn_read_loop", task::current().id())).await?;
    // let mut buf = vec![0; 4];
    loop {
        let message_size = bytes_to_u32(&read_n(stream, 4).await?);
        let message = if message_size > 0 {
            // terminal::print_log(format!("{:?}: stream message len: {}", task::current().id(), message_size)).await?;
            let message = read_n(stream, message_size).await?;
            Message::new(&message[0], &message[1..])
        // let m = Message::new(&message[0], &message[1..]);
        // terminal::print_log(format!("{:?}", m)).await?;
        // m
        } else {
            Message::KeepAlive
        };

        sender.send(IPC::Message(message)).await?;
    }
}

async fn conn_write_loop(
    messages: Receiver<Message>,
    mut stream: TcpStream,
    mut sender: Sender<IPC>,
) -> Result<()> {
    // let mut stream = &mut stream;
    let mut messages = messages.fuse();
    terminal::print_log(format!("task {}: conn_write_loop", task::current().id())).await?;
    loop {
        select! {
            message = messages.next().fuse() => match message {
                Some(message) => {
                    let is_block_upload = match message {
                        Message::Piece(_, _, _) => true,
                        _ => false
                    };
                    stream.write_all(&message.clone().serialize()).await?;
                    // notify the main PeerConnection thread that this block is finished
                    if is_block_upload {
                        sender.send(IPC::BlockUploaded).await?;
                    }
                },
                None => break,
            }


        }
    }
    Ok(())
}

async fn process_message(peer_conn: &mut PeerConnection, message: Message) -> Result<()> {
    match message {
        Message::KeepAlive => {}
        Message::Choke => {
            peer_conn.me.is_choked = true;
        }
        Message::UnChoke => {
            let is_choked = peer_conn.me.is_choked;
            if is_choked {
                peer_conn.me.is_choked = false;
                peer_conn.request_more_blocks().await?;
            }
        }
        Message::Interested => {
            peer_conn.he.is_interested = true;
            let is_choked = peer_conn.he.is_choked;
            if is_choked {
                peer_conn.he.is_choked = false;
                peer_conn.writer_sender.send(Message::UnChoke).await?;
                peer_conn.upload_next_block().await?;
            }
        }
        Message::NotInterested => {
            peer_conn.he.is_interested = false;
        }
        Message::Have(have_index) => {
            peer_conn.he.has_pieces[have_index as usize] = true;
            peer_conn.queue_blocks(have_index).await?;
            peer_conn.update_my_interested_status().await?;
            peer_conn.request_more_blocks().await?;
        }
        Message::Bitfield(bytes) => {
            // terminal::print_log(format!("start bitfield")).await?;
            let l = peer_conn.he.has_pieces.len();
            for have_index in 0..l {
                let bytes_index = have_index / 8;
                let index_into_byte = have_index % 8;
                let byte = bytes[bytes_index];
                let mask = 1 << (7 - index_into_byte);
                let value = (byte & mask) != 0;
                peer_conn.he.has_pieces[have_index] = value;

                if value {
                    peer_conn.queue_blocks(have_index as u32).await?;
                }
            }
            peer_conn.update_my_interested_status().await?;
            peer_conn.request_more_blocks().await?;
            // terminal::print_log(format!("end bitfield")).await?;
        }
        Message::Request(piece_index, offset, length) => {
            let block_index = offset / BLOCK_SIZE;
            peer_conn
                .he
                .requests
                .add(piece_index, block_index, offset, length);
            peer_conn.upload_next_block().await?;
        }
        Message::Piece(piece_index, offset, data) => {
            let block_index = offset / BLOCK_SIZE;
            peer_conn.me.requests.remove(piece_index, block_index);
            // terminal::print_log(format!("Message::Piece start send")).await?;
            peer_conn
                .to_download
                .send(ManagerEvent::Download(Message::Piece(
                    piece_index,
                    offset,
                    data,
                )))
                .await?;
            // terminal::print_log(format!("Message::Piece finish send")).await?;
            peer_conn.update_my_interested_status().await?;
            peer_conn.request_more_blocks().await?;
        }
        Message::Cancel(piece_index, offset, _) => {
            let block_index = offset / BLOCK_SIZE;
            peer_conn.he.requests.remove(piece_index, block_index);
        }
        _ => return Err("Error UnknownRequestType(message)")?,
    }

    Ok(())
}

async fn read_n(stream: &mut TcpStream, bytes_to_read: u32) -> Result<Vec<u8>> {
    // let mut stream = &*stream;
    let mut buf = vec![0; bytes_to_read as usize];
    stream.read_exact(&mut buf).await?;
    // read_n_to_buf(stream, &mut buf, bytes_to_read).await?;
    Ok(buf)
}

struct PeerMetadata {
    has_pieces: Vec<bool>,
    is_choked: bool,
    is_interested: bool,
    requests: RequestQueue,
}

impl PeerMetadata {
    fn new(has_pieces: Vec<bool>) -> PeerMetadata {
        PeerMetadata {
            has_pieces,
            is_choked: true,
            is_interested: false,
            requests: RequestQueue::new(),
        }
    }
}

#[derive(Debug)]
pub struct RequestQueue {
    requests: Vec<RequestMetadata>,
}

impl RequestQueue {
    pub fn new() -> RequestQueue {
        RequestQueue { requests: vec![] }
    }

    pub fn has(&self, piece_index: u32, block_index: u32) -> bool {
        self.position(piece_index, block_index).is_some()
    }

    pub fn add(
        &mut self,
        piece_index: u32,
        block_index: u32,
        offset: u32,
        block_length: u32,
    ) -> bool {
        if !self.has(piece_index, block_index) {
            let r = RequestMetadata {
                piece_index,
                block_index,
                offset,
                block_length,
            };
            self.requests.push(r);
            true
        } else {
            false
        }
    }

    pub fn pop(&mut self) -> Option<RequestMetadata> {
        if self.requests.len() > 0 {
            Some(self.requests.remove(0))
        } else {
            None
        }
    }

    pub fn remove(&mut self, piece_index: u32, block_index: u32) -> Option<RequestMetadata> {
        match self.position(piece_index, block_index) {
            Some(i) => {
                let r = self.requests.remove(i);
                Some(r)
            }
            None => None,
        }
    }

    fn position(&self, piece_index: u32, block_index: u32) -> Option<usize> {
        self.requests
            .iter()
            .position(|r| r.matches(piece_index, block_index))
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }
}

#[derive(Debug, Clone)]
pub struct RequestMetadata {
    pub piece_index: u32,
    pub block_index: u32,
    pub offset: u32,
    pub block_length: u32,
}

impl RequestMetadata {
    pub fn matches(&self, piece_index: u32, block_index: u32) -> bool {
        self.piece_index == piece_index && self.block_index == block_index
    }
}
