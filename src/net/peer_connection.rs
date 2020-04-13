use std::net::Ipv4Addr;
use crate::bencode::value::{FromValue, Value};
use crate::bencode::decode::DecodeError;

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
    // KeepAlive,
    Choke,
    Unchoke,
    Interested,
    NotInterested,
    Have(u32),
    Bitfield(Vec<u8>),
    Request(u32, u32, u32),
    Piece(u32, u32, Vec<u8>),
    Cancel(u32, u32, u32),
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