use futures::channel::mpsc;
use futures::sink::SinkExt;
use futures::{select, FutureExt, StreamExt};
use async_std::task;
use async_std::sync::Arc;
use async_std::prelude::*;
use async_std::net::TcpStream;
use async_std::net::Ipv4Addr;

use crate::base::ipc::{Message, IPC, bytes_to_u32};
use crate::base::meta_info::TorrentMetaInfo;
use crate::bencode::hash::Sha1;
use crate::bencode::value::{FromValue, Value};
use crate::base::spawn_and_log_error;
use crate::bencode::decode::DecodeError;
use std::option::Option::Some;
use std::collections::HashMap;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::Sender<T>;
type Receiver<T> = mpsc::Receiver<T>;

const PROTOCOL: &'static str = "BitTorrent protocol";
const MAX_CONCURRENT_REQUESTS: u32 = 100;

#[derive(PartialEq, Debug)]
pub struct Peer {
    pub ip: String,
    pub port: u16,
}

impl Peer {
    pub fn from_bytes(v: &[u8]) -> Peer {
        let ip = Ipv4Addr::new(v[0], v[1], v[2], v[3]);
        let port = (v[4] as u16) * 256 + (v[5] as u16);
        Peer { ip: ip.to_string(), port }
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
    our_peer_id: String,
    info_hash: Sha1,
    send_handshake_first: bool,

    stream: Arc<TcpStream>,
    me: PeerMetadata,
    he: PeerMetadata,

    to_request: HashMap<(u32, u32), (u32, u32, u32)>,
    // stream_sender: Arc<TcpStream>,
    // halt: bool,
    // download: Arc<Download>,
    //
    // me: PeerMetadata,
    // he: PeerMetadata,
    // incoming_tx: Arc<Mutex<Sender<IPC>>>,
    // outgoing_tx: Arc<Mutex<Sender<Message>>>,
    // upload_in_progress: bool,
    // to_request: Arc<Mutex<HashMap<(u32, u32), (u32, u32, u32)>>>,
    //
    // tx_down: Mutex<TX<Message>>
}

impl PeerConnection {
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
        let mut stream = &*self.stream;
        // stream.write_all(&[1,2]).await?;
        // let stream = &*self.stream;
        stream.write_all(message.as_slice()).await?;
        Ok(())
    }

    async fn receive_handshake(&self) -> Result<()> {
        let stream = &*self.stream;

        let pstrlen = read_n(stream, 1).await?;
        read_n(stream, pstrlen[0] as u32).await?; // ignore pstr
        read_n(stream, 8).await?; // ignore reserved
        let info_hash = read_n(stream, 20).await?;
        let peer_id = read_n(stream, 20).await?;

        {

            // validate info hash
            if &info_hash != &self.info_hash.0 {
                println!("{}", crate::bencode::hash::to_hex(&info_hash));
                println!("{}", crate::bencode::hash::to_hex(&self.info_hash.0));

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
}

pub async fn peer_conn_loop(mut peer_conn: PeerConnection, mut download_sender: Sender<Message>) -> Result<()> {
    if peer_conn.send_handshake_first {
        peer_conn.send_handshake().await?;
        peer_conn.receive_handshake().await?;
    } else {
        peer_conn.receive_handshake().await?;
        peer_conn.send_handshake().await?;
    }

    let (mut ipc_sender, mut ipcs) = futures::channel::mpsc::channel(5);

    spawn_and_log_error(
        conn_read_loop(Arc::clone(&peer_conn.stream), ipc_sender.clone())
    );

    while let Some(ipc) = ipcs.next().await {
        match ipc {
            IPC::Message(message) => {

            },
            IPC::BlockComplete(_, _) => {},
            IPC::PieceComplete(_) => {},
            IPC::DownloadComplete => {},
            IPC::BlockUploaded => {},
        }
    }

    // let stream = Arc::new(stream);
    // let _handler = spawn_and_log_error(conn_write_loop())
    // (s: futures::channel::mpsc::Sender<u32>, v: futures::channel::mpsc::Receiver<u32>) = futures::channel::mpsc::channel(0);
    // s.

    Ok(())
}

async fn conn_read_loop(stream: Arc<TcpStream>, mut sender: Sender<IPC>) -> Result<()> {
    let mut stream = &*stream;
    // let message_size = ;

    while let message_size = bytes_to_u32(&read_n(stream, 4).await?) {
        let message = if message_size > 0 {
            println!("{:?}: stream message len: {}", task::current().id(), message_size);

            let message = read_n(stream, message_size).await?;
            Message::new(&message[0], &message[1..])
        } else {
            Message::KeepAlive
        };

        sender.send(IPC::Message(message)).await?;
    }

    Ok(())
}

async fn process_message(peer_conn: &mut PeerConnection, message: Message) -> Result<()> {
    match message {
        Message::KeepAlive => {},
        Message::Choke => {
            peer_conn.me.is_choked = true;
        }
        Message::Unchoke => {
            let is_choked = peer_conn.me.is_choked;
            if is_choked {
                peer_conn.me.is_choked = false;
                // peer_conn.request_more_blocks().await?;
            }
        }
        Message::Interested => {}
        Message::NotInterested => {}
        Message::Have(_) => {}
        Message::Bitfield(_) => {}
        Message::Request(_, _, _) => {}
        Message::Piece(_, _, _) => {}
        Message::Cancel(_, _, _) => {}
        Message::Port => {}
    }

    Ok(())
}

async fn request_more_blocks(peer_conn: &mut PeerConnection) {
    let is_choked = peer_conn.me.is_choked;
    let is_interested = peer_conn.me.is_interested;
    let len = peer_conn.to_request.len();
}

async fn conn_write_loop(messages: &mut Receiver<Message>, stream: Arc<TcpStream>, mut sender: Sender<IPC>) -> Result<()> {
    let mut stream = &*stream;
    let mut messages = messages.fuse();

    loop {
        select! {
            message = messages.next().fuse() => match message {
                Some(message) => {
                    let is_block_upload = match message {
                        Message::Piece(_, _, _) => true,
                        _ => false
                    };
                    stream.write_all(&message.clone().serialize()).await?;
                    println!("{:?}: outgoing reciever have recv message: {:?}", task::current().id(), message);

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


async fn read_n(mut stream: &TcpStream, bytes_to_read: u32) -> Result<Vec<u8>> {
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

    pub fn add(&mut self, piece_index: u32, block_index: u32, offset: u32, block_length: u32) -> bool {
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
            },
            None => None
        }
    }

    fn position(&self, piece_index: u32, block_index: u32) -> Option<usize> {
        self.requests.iter().position(|r| r.matches(piece_index, block_index))
    }

    pub fn len(&self) -> usize {
        self.requests.len()
    }
}

#[derive(Debug)]
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