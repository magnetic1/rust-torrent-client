use std::fmt;

#[derive(Clone, Debug)]
pub enum IPC {
    BlockComplete(u32, u32),
    PieceComplete(u32),
    DownloadComplete,
    Message(Message),
    BlockUploaded,

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
    pub fn new(id: &u8, body: &[u8]) -> Message {
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

    pub fn serialize(self) -> Vec<u8> {
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

const BYTE_0: u32 = 256 * 256 * 256;
const BYTE_1: u32 = 256 * 256;
const BYTE_2: u32 = 256;
const BYTE_3: u32 = 1;

pub(crate) fn bytes_to_u32(bytes: &[u8]) -> u32 {
    bytes[0] as u32 * BYTE_0 +
        bytes[1] as u32 * BYTE_1 +
        bytes[2] as u32 * BYTE_2 +
        bytes[3] as u32 * BYTE_3
}

pub(crate) fn u32_to_bytes(integer: u32) -> Vec<u8> {
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

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Message::KeepAlive => write!(f, "KeepAlive"),
            Message::Choke => write!(f, "Choke"),
            Message::Unchoke => write!(f, "Unchoke"),
            Message::Interested => write!(f, "Interested"),
            Message::NotInterested => write!(f, "NotInterested"),
            Message::Have(ref index) => write!(f, "Have({})", index),
            Message::Bitfield(ref bytes) => write!(f, "Bitfield({:?})", crate::bencode::hash::to_hex(bytes)),
            Message::Request(ref index, ref offset, ref length) => write!(f, "Request({}, {}, {})", index, offset, length),
            Message::Piece(ref index, ref offset, ref data) => write!(f, "Piece({}, {}, size={})", index, offset, data.len()),
            Message::Cancel(ref index, ref offset, ref length) => write!(f, "Cancel({}, {}, {})", index, offset, length),
            Message::Port => write!(f, "Port"),
        }
    }
}