use std::net::{Ipv4Addr, Shutdown};
use crate::bencode::value::{FromValue, Value};
use crate::bencode::decode::DecodeError;
use crate::net::download::{Download, BLOCK_SIZE};
use async_std::net::TcpStream;
use std::sync::mpsc::{Sender, RecvError, SendError, Receiver, channel};
use crate::net::ipc::IPC;
use std::collections::HashMap;
use std::{convert, any, fmt};
use crate::net::download;
use async_std::io;
use async_std::task;
use crate::net::request_queue::RequestQueue;
use rand::Rng;
use async_std::sync::{Arc, Mutex};
use futures::{FutureExt, AsyncReadExt};
use futures::prelude::future::BoxFuture;
use async_std::io::prelude::WriteExt;

const PROTOCOL: &'static str = "BitTorrent protocol";
const MAX_CONCURRENT_REQUESTS: u32 = 10;

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
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        let peer = Peer {
            ip: value.get_field("ip")?,
            port: value.get_field("port")?,
        };
        Ok(peer)
    }
}

#[derive(Clone)]
pub enum Message {
    KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(u32, u32, u32),
    Piece(u32, u32, Vec<u8>),
    Cancel(u32, u32, u32),
    Port,
}

impl Message {
    fn new(id: &u8, body: &[u8]) -> Message {
        match *id {
            0 => Message::Choke,
            1 => Message::Unchoke,
            2 => Message::Interested,
            3 => Message::NotInterested,
            4 => Message::Have(bytes_to_u32(body)),
            5 => Message::Bitfield(body.to_owned()),
            6 => {
                let index = bytes_to_u32(&body[0..4]);
                let offset = bytes_to_u32(&body[4..8]);
                let length = bytes_to_u32(&body[8..12]);
                Message::Request(index, offset, length)
            }
            7 => {
                let index = bytes_to_u32(&body[0..4]);
                let offset = bytes_to_u32(&body[4..8]);
                let data = body[8..].to_owned();
                Message::Piece(index, offset, data)
            }
            8 => {
                let index = bytes_to_u32(&body[0..4]);
                let offset = bytes_to_u32(&body[4..8]);
                let length = bytes_to_u32(&body[8..12]);
                Message::Cancel(index, offset, length)
            }
            9 => Message::Port,
            _ => panic!("Bad message id: {}", id)
        }
    }

    fn serialize(self) -> Vec<u8> {
        let mut payload = vec![];
        match self {
            Message::KeepAlive => {}
            Message::Choke => payload.push(0),
            Message::Unchoke => payload.push(1),
            Message::Interested => payload.push(2),
            Message::NotInterested => payload.push(3),
            Message::Have(index) => {
                payload.push(4);
                payload.extend(u32_to_bytes(index).into_iter());
            }
            Message::Bitfield(bytes) => {
                payload.push(5);
                payload.extend(bytes);
            }
            Message::Request(index, offset, length) => {
                payload.push(6);
                payload.extend(u32_to_bytes(index).into_iter());
                payload.extend(u32_to_bytes(offset).into_iter());
                payload.extend(u32_to_bytes(length).into_iter());
            }
            Message::Piece(index, offset, data) => {
                payload.push(6);
                payload.extend(u32_to_bytes(index).into_iter());
                payload.extend(u32_to_bytes(offset).into_iter());
                payload.extend(data);
            }
            Message::Cancel(index, offset, length) => {
                payload.push(8);
                payload.extend(u32_to_bytes(index).into_iter());
                payload.extend(u32_to_bytes(offset).into_iter());
                payload.extend(u32_to_bytes(length).into_iter());
            }
            Message::Port => payload.push(9),
        };

        // prepend size
        let mut size = u32_to_bytes(payload.len() as u32);
        size.extend(payload);
        size
    }
}

pub struct PeerConnection {
    halt: bool,
    download: Arc<Download>,
    stream: Arc<Mutex<TcpStream>>,
    me: Arc<Mutex<PeerMetadata>>,
    them: Arc<Mutex<PeerMetadata>>,
    incoming_tx: Arc<Mutex<Sender<IPC>>>,
    outgoing_tx: Arc<Mutex<Sender<Message>>>,
    upload_in_progress: bool,
    to_request: Arc<Mutex<HashMap<(u32, u32), (u32, u32, u32)>>>,
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

impl PeerConnection {
    async fn connect(peer: &Peer, download: Arc<Download>) -> Result<(), Error> {
        println!("Connecting to {}:{}", peer.ip, peer.port);
        let ip: Ipv4Addr = peer.ip.parse().unwrap();
        let stream = TcpStream::connect((ip, peer.port)).await?;
        PeerConnection::new(stream, download.clone(), true).await?;
        println!("{}:{} Disconnected", &peer.ip, peer.port);
        Ok(())
    }

    async fn accept(stream: TcpStream, download: Arc<Download>) -> Result<(), Error> {
        println!("Received connection from a peer!");
        PeerConnection::new(stream, download.clone(), false).await
    }

    async fn new(stream: TcpStream, download: Arc<Download>, send_handshake_first: bool) -> Result<(), Error> {
        let have_pieces = download.have_pieces().await;

        let num_pieces = have_pieces.len();

        // create & register incoming IPC channel with Download
        let (incoming_tx, incoming_rx) = channel::<IPC>();
        download.register_peer(incoming_tx.clone()).await;

        // create outgoing Message channel
        let (outgoing_tx, outgoing_rx) = channel::<Message>();

        let mut conn = PeerConnection {
            halt: false,
            download,
            stream: Arc::new(Mutex::new(stream)),
            me: Arc::new(Mutex::new(PeerMetadata::new(have_pieces))),
            them: Arc::new(Mutex::new(PeerMetadata::new(vec![false; num_pieces]))),
            incoming_tx: Arc::new(Mutex::new(incoming_tx)),
            outgoing_tx: Arc::new(Mutex::new(outgoing_tx)),
            upload_in_progress: false,
            to_request: Arc::new(Mutex::new(HashMap::new())),
        };

        conn.run(send_handshake_first, incoming_rx, outgoing_rx).await?;

        Ok(())
    }

    async fn run(&mut self, send_handshake_first: bool, incoming_rx: Receiver<IPC>, outgoing_rx: Receiver<Message>) -> Result<(), Error> {
        if send_handshake_first {
            self.send_handshake().await?;
            self.receive_handshake().await?;
        } else {
            self.receive_handshake().await?;
            self.send_handshake().await?;
        }

        println!("Handshake complete");

        // spawn a thread to funnel incoming messages from the socket into the incoming message channel
        let downstream_funnel_task = {
            let stream = self.stream.clone();
            let tx = self.incoming_tx.clone();
            task::spawn(async move {
                DownLoadMessageFunnel::start(stream, tx).await
            })
        };

        // spawn a thread to funnel outgoing messages from the outgoing message channel into the socket
        let upstream_funnel_task = {
            let stream = self.stream.clone();
            let tx = self.incoming_tx.clone();
            task::spawn(async move {
                UpLoadMessageFunnel::start(stream, Arc::new(Mutex::new(outgoing_rx)), tx)
            })
            // task::spawn(async move {
            //     UpLoadMessageFunnel::start(stream, outgoing_rx, tx).await
            // })
        };

        // send a bitfield message letting peer know what we have
        self.send_bitfield().await?;

        // process messages received on the channel (both from the remote peer, and from Download)
        while !self.halt {
            // let message = task::spawn_blocking(|| {
            let message =  incoming_rx.recv()?;
            // }).await?;
            self.process(message).await?;
        }

        println!("Disconnecting");
        self.stream.lock().await.shutdown(Shutdown::Both)?;
        futures::join!(downstream_funnel_task, upstream_funnel_task);
        Ok(())
    }

    async fn send_handshake(&self) -> Result<(), Error> {
        let message = {
            let download = self.download.clone();
            let mut message = vec![];
            message.push(PROTOCOL.len() as u8);
            message.extend(PROTOCOL.bytes());
            message.extend(vec![0; 8].into_iter());
            message.extend(download.meta_info.info_hash().iter().cloned());
            message.extend(download.our_peer_id.bytes());
            message
        };
        self.stream.lock().await.write_all(&message).await?;
        Ok(())
    }

    async fn receive_handshake(&self) -> Result<(), Error> {
        let pstrlen = read_n(self.stream.clone(), 1).await?;
        read_n(self.stream.clone(), pstrlen[0] as u32).await?; // ignore pstr
        read_n(self.stream.clone(), 8).await?; // ignore reserved
        let info_hash = read_n(self.stream.clone(), 20).await?;
        let peer_id = read_n(self.stream.clone(), 20).await?;

        {
            let download = self.download.clone();

            // validate info hash
            if &info_hash != &download.meta_info.info_hash().0 {
                return Err(Error::InvalidInfoHash);
            }

            // validate peer id
            let our_peer_id: Vec<u8> = download.our_peer_id.bytes().collect();
            if peer_id == our_peer_id {
                return Err(Error::ConnectingToSelf);
            }
        }

        Ok(())
    }

    async fn send_message(&self, message: Message) -> Result<(), Error> {
        // println!("Sending: {:?}", message);
        self.outgoing_tx.clone().lock().await.send(message)?;
        Ok(())
    }

    async fn process(&mut self, ipc: IPC) -> Result<(), Error> {
        match ipc {
            IPC::Message(message) => self.process_message(message).await,
            IPC::BlockComplete(piece_index, block_index) => {
                self.to_request.lock().await.remove(&(piece_index, block_index));
                match self.me.lock().await.requests.remove(piece_index, block_index) {
                    Some(r) =>
                        self.send_message(
                            Message::Cancel(r.piece_index, r.offset, r.block_length)
                        ).await,
                    None => Ok(())
                }
            }
            IPC::PieceComplete(piece_index) => {
                self.me.lock().await.has_pieces[piece_index as usize] = true;
                self.update_my_interested_status().await?;
                self.send_message(Message::Have(piece_index)).await?;
                Ok(())
            }
            IPC::DownloadComplete => {
                self.halt = true;
                self.update_my_interested_status().await?;
                Ok(())
            }
            IPC::BlockUploaded => {
                self.upload_in_progress = false;
                self.upload_next_block().await?;
                Ok(())
            }
        }
    }

    async fn process_message(&mut self, message: Message) -> Result<(), Error> {
        // println!("Received: {:?}", message);
        match message {
            Message::KeepAlive => {}
            Message::Choke => {
                self.me.lock().await.is_choked = true;
            }
            Message::Unchoke => {
                if self.me.lock().await.is_choked {
                    self.me.lock().await.is_choked = false;
                    self.request_more_blocks().await?;
                }
            }
            Message::Interested => {
                self.them.lock().await.is_interested = true;
                self.unchoke_them().await?;
            }
            Message::NotInterested => {
                self.them.lock().await.is_interested = false;
            }
            Message::Have(have_index) => {
                self.them.lock().await.has_pieces[have_index as usize] = true;
                self.queue_blocks(have_index).await;
                self.update_my_interested_status().await?;
                self.request_more_blocks().await?;
            }
            Message::Bitfield(bytes) => {
                for have_index in 0..self.them.lock().await.has_pieces.len() {
                    let bytes_index = have_index / 8;
                    let index_into_byte = have_index % 8;
                    let byte = bytes[bytes_index];
                    let mask = 1 << (7 - index_into_byte);
                    let value = (byte & mask) != 0;
                    self.them.lock().await.has_pieces[have_index] = value;
                    if value {
                        self.queue_blocks(have_index as u32).await;
                    }
                };
                self.update_my_interested_status().await?;
                self.request_more_blocks().await?;
            }
            Message::Request(piece_index, offset, length) => {
                let block_index = offset / BLOCK_SIZE;
                self.them.lock().await.requests.add(piece_index, block_index, offset, length);
                self.upload_next_block().await?;
            }
            Message::Piece(piece_index, offset, data) => {
                let block_index = offset / BLOCK_SIZE;
                self.me.lock().await.requests.remove(piece_index, block_index);
                {
                    let download = self.download.clone();
                    download.store(piece_index, block_index, data).await?
                }
                self.update_my_interested_status().await?;
                self.request_more_blocks().await?;
            }
            Message::Cancel(piece_index, offset, _) => {
                let block_index = offset / BLOCK_SIZE;
                self.them.lock().await.requests.remove(piece_index, block_index);
            }
            _ => return Err(Error::UnknownRequestType(message))
            // Message::Port => {}
        };
        Ok(())
    }

    async fn queue_blocks(&self, piece_index: u32) {
        let incomplete_blocks = {
            let download = self.download.clone();
            download.incomplete_blocks_for_piece(piece_index).await
        };

        for (block_index, block_length) in incomplete_blocks {
            if !self.me.lock().await.requests.has(piece_index, block_index) {
                self.to_request.lock().await.insert((piece_index, block_index),
                                       (piece_index, block_index, block_length));
            }
        }
    }

    async fn update_my_interested_status(&self) -> Result<(), Error> {
        let am_interested = self.me.lock().await.requests.len() > 0 || self.to_request.lock().await.len() > 0;

        if self.me.lock().await.is_interested != am_interested {
            self.me.lock().await.is_interested = am_interested;
            let message = if am_interested {
                Message::Interested
            } else {
                Message::NotInterested
            };
            self.send_message(message).await
        } else {
            Ok(())
        }
    }

    async fn send_bitfield(&self) -> Result<(), Error> {
        let mut bytes: Vec<u8> = vec![0; (self.me.lock().await.has_pieces.len() as f64 / 8 as f64).ceil() as usize];
        for have_index in 0..self.me.lock().await.has_pieces.len() {
            let bytes_index = have_index / 8;
            let index_into_byte = have_index % 8;
            if self.me.lock().await.has_pieces[have_index] {
                let mask = 1 << (7 - index_into_byte);
                bytes[bytes_index] |= mask;
            }
        };
        self.send_message(Message::Bitfield(bytes)).await
    }

    async fn request_more_blocks(&self) -> Result<(), Error> {
        if self.me.lock().await.is_choked || !self.me.lock().await.is_interested || self.to_request.lock().await.len() == 0 {
            return Ok(());
        }

        while self.me.lock().await.requests.len() < MAX_CONCURRENT_REQUESTS as usize {
            let len = self.to_request.clone().lock().await.len();

            if len == 0 {
                return Ok(());
            }
            // remove a block at random from to_request
            let (piece_index, block_index, block_length) = {
                let index = rand::thread_rng().gen_range(0, len);
                let target = self.to_request.lock().await.keys().nth(index).unwrap().clone();
                self.to_request.lock().await.remove(&target).unwrap()
            };

            // add a request
            let offset = block_index * BLOCK_SIZE;
            if self.me.lock().await.requests.add(piece_index, block_index, offset, block_length) {
                self.send_message(Message::Request(piece_index, offset, block_length)).await?;
            };
        }

        Ok(())
    }

    async fn unchoke_them(&mut self) -> Result<(), Error> {
        if self.them.lock().await.is_choked {
            self.them.lock().await.is_choked = false;
            self.send_message(Message::Unchoke).await?;
            self.upload_next_block().await?;
        }
        Ok(())
    }

    async fn upload_next_block(&mut self) -> Result<(), Error> {
        if self.upload_in_progress || self.them.lock().await.is_choked || !self.them.lock().await.is_interested {
            return Ok(());
        }

        match self.them.lock().await.requests.pop() {
            Some(r) => {
                let data = {
                    let download = self.download.clone();
                    download.retrieve_data(&r).await?
                };
                self.upload_in_progress = true;
                self.send_message(Message::Piece(r.piece_index, r.offset, data)).await
            }
            None => Ok(())
        }
    }
}

struct DownLoadMessageFunnel {
    stream: Arc<Mutex<TcpStream>>,
    tx: Arc<Mutex<Sender<IPC>>>,
}

impl DownLoadMessageFunnel {
    async fn start(stream: Arc<Mutex<TcpStream>>, tx: Arc<Mutex<Sender<IPC>>>) {
        let mut funnel = DownLoadMessageFunnel {
            stream,
            tx,
        };
        match funnel.run().await {
            Ok(_) => {}
            Err(e) => println!("DownLoadMessageFunnel Error: {:?}", e)
        }
    }

    async fn run(&self) -> Result<(), Error> {
        loop {
            let message: Message = self.receive_message().await?;
            self.tx.clone().lock().await.send(IPC::Message(message))?;
        }
    }

    async fn receive_message(&self) -> Result<Message, Error> {
        let message_size = bytes_to_u32(&read_n(self.stream.clone(), 4).await?);
        if message_size > 0 {
            let message = read_n(self.stream.clone(), message_size).await?;
            Ok(Message::new(&message[0], &message[1..]))
        } else {
            Ok(Message::KeepAlive)
        }
    }
}

struct UpLoadMessageFunnel {
    stream: Arc<Mutex<TcpStream>>,
    rx: Arc<Mutex<Receiver<Message>>>,
    tx: Arc<Mutex<Sender<IPC>>>,
}

impl UpLoadMessageFunnel {
    async fn start(stream: Arc<Mutex<TcpStream>>, rx: Arc<Mutex<Receiver<Message>>>, tx: Arc<Mutex<Sender<IPC>>>) {
        let mut funnel = UpLoadMessageFunnel {
            stream,
            rx,
            tx,
        };

        match funnel.run().await {
            Ok(_) => {},
            Err(e) => println!("UpLoadMessageFunnel Error : {:?}", e),
            // _ => {}
        }
    }

    async fn run(&self) -> Result<(), Error> {
        loop {
            let rx = self.rx.clone();
            let message = task::spawn_blocking(move || {
                let rx_fut = rx.lock();
                let r = task::block_on(async move {
                    rx_fut.await
                });
                r.recv()
            }).await?;

            let is_block_upload = match message {
                Message::Piece(_, _, _) => true,
                _ => false
            };

            // do a blocking write to the TCP stream
            task::block_on(async {
                self.stream.clone().lock().await.write_all(&message.serialize()).await
            })?;

            // notify the main PeerConnection thread that this block is finished
            if is_block_upload {
                self.tx.clone().lock().await.send(IPC::BlockUploaded)?;
            }
        }
    }
}

async fn read_n(stream: Arc<Mutex<TcpStream>>, bytes_to_read: u32) -> Result<Vec<u8>, Error> {
    let mut buf = vec![];
    read_n_to_buf(stream, &mut buf, bytes_to_read).await?;
    Ok(buf)
}

fn read_n_to_buf(stream: Arc<Mutex<TcpStream>>, buf: &mut Vec<u8>, bytes_to_read: u32) -> BoxFuture<Result<(), Error>> {
    async move {
        if bytes_to_read == 0 {
            return Ok(());
        }

        let mut take = stream.lock().await.clone().take(bytes_to_read as u64);
        let bytes_read = take.read_to_end(buf).await;
        match bytes_read {
            Ok(0) => Err(Error::SocketClosed),
            Ok(n) if n == bytes_to_read as usize => Ok(()),
            Ok(n) => read_n_to_buf(stream.clone(), buf, bytes_to_read - n as u32).await,
            Err(e) => Err(e)?
        }
    }.boxed()
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Message::KeepAlive => write!(f, "KeepAlive"),
            Message::Choke => write!(f, "Choke"),
            Message::Unchoke => write!(f, "Unchoke"),
            Message::Interested => write!(f, "Interested"),
            Message::NotInterested => write!(f, "NotInterested"),
            Message::Have(ref index) => write!(f, "Have({})", index),
            Message::Bitfield(ref bytes) => write!(f, "Bitfield({:?})", bytes),
            Message::Request(ref index, ref offset, ref length) => write!(f, "Request({}, {}, {})", index, offset, length),
            Message::Piece(ref index, ref offset, ref data) => write!(f, "Piece({}, {}, size={})", index, offset, data.len()),
            Message::Cancel(ref index, ref offset, ref length) => write!(f, "Cancel({}, {}, {})", index, offset, length),
            Message::Port => write!(f, "Port"),
        }
    }
}

const BYTE_0: u32 = 256 * 256 * 256;
const BYTE_1: u32 = 256 * 256;
const BYTE_2: u32 = 256;
const BYTE_3: u32 = 1;

fn bytes_to_u32(bytes: &[u8]) -> u32 {
    bytes[0] as u32 * BYTE_0 +
        bytes[1] as u32 * BYTE_1 +
        bytes[2] as u32 * BYTE_2 +
        bytes[3] as u32 * BYTE_3
}

fn u32_to_bytes(integer: u32) -> Vec<u8> {
    let mut rest = integer;
    let first = rest / BYTE_0;
    rest -= first * BYTE_0;
    let second = rest / BYTE_1;
    rest -= second * BYTE_1;
    let third = rest / BYTE_2;
    rest -= third * BYTE_2;
    let fourth = rest;
    vec![first as u8, second as u8, third as u8, fourth as u8]
}

#[derive(Debug)]
pub enum Error {
    InvalidInfoHash,
    ConnectingToSelf,
    DownloadError(download::Error),
    IoError(io::Error),
    SocketClosed,
    UnknownRequestType(Message),
    ReceiveError(RecvError),
    SendMessageError(SendError<Message>),
    SendIPCError(SendError<IPC>),
    Any(Box<dyn any::Any + Send>),
}

impl convert::From<download::Error> for Error {
    fn from(err: download::Error) -> Error {
        Error::DownloadError(err)
    }
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl convert::From<RecvError> for Error {
    fn from(err: RecvError) -> Error {
        Error::ReceiveError(err)
    }
}

impl convert::From<SendError<Message>> for Error {
    fn from(err: SendError<Message>) -> Error {
        Error::SendMessageError(err)
    }
}

impl convert::From<SendError<IPC>> for Error {
    fn from(err: SendError<IPC>) -> Error {
        Error::SendIPCError(err)
    }
}

impl convert::From<Box<dyn any::Any + Send>> for Error {
    fn from(err: Box<dyn any::Any + Send>) -> Error {
        Error::Any(err)
    }
}

#[cfg(test)]
mod tests {
    use crate::net::peer_connection::Peer;
    use std::net::Ipv4Addr;

    #[test]
    fn peer_from_bytes() {
        let bytes: [u8; 6] = [192, 128, 0, 1, 1, 1];
        let peer = Peer::from_bytes(&bytes);

        let ip: Ipv4Addr = "192.128.0.1".parse().unwrap();
        assert_eq!(peer.ip, ip.to_string());

        let port: u16 = (1 << 8) + 1;
        assert_eq!(peer.port, port);
    }
}