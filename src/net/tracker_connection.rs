use crate::net::peer_connection::Peer;
use url::percent_encoding::{percent_encode, FORM_URLENCODED_ENCODE_SET};
use hyper::Client;
use hyper::header::Connection;
use std::io::Read;
use crate::base::meta_info::TorrentMetaInfo;
use std::{convert, io};
use crate::bencode::decode::{Decoder, DecodeTo, DecodeError};
use crate::bencode::value::{Value, FromValue};

// async fn get()


pub fn get_tracker_response(peer_id: &str, announce: &str, length: u64, info_hash: &[u8], listener_port: u16) -> Result<TrackerResponse, Error> {
    let length_string = length.to_string();
    let encoded_info_hash = percent_encode(info_hash, FORM_URLENCODED_ENCODE_SET);
    let listener_port_string = listener_port.to_string();

    let params = vec![("left", length_string.as_ref()),
                      ("info_hash", encoded_info_hash.as_ref()),
                      ("downloaded", "0"),
                      ("uploaded", "0"),
                      ("event", "started"),
                      ("peer_id", peer_id),
                      ("compact", "1"),
                      ("port", listener_port_string.as_ref())];

    let url = format!("{}?{}", announce, encode_query_params(&params));
    println!("{}", url);

    let client = Client::new();
    let mut http_res = client.get(&url).header(Connection::close()).send()?;

    let mut body = Vec::new();
    http_res.read_to_end(&mut body)?;
    println!("{}", body.len());

    let res = TrackerResponse::parse(&body)?;
    // println!("{:#?}", res);
    Ok(res)
}

pub fn get_tracker_response_by_metainfo(peer_id: &str, metainfo: &TorrentMetaInfo, listener_port: u16) -> Result<TrackerResponse, Error> {
    let announce = "http://explodie.org:6969/announce";
    let info_hash = &metainfo.info_hash().0;
    get_tracker_response(peer_id, announce, metainfo.length(), info_hash, listener_port)
}
/// 116.249.137.177:51636
/// 104.152.209.30:64223
/// 205.185.122.158:54794
pub fn get_peers(peer_id: &str, metainfo: &TorrentMetaInfo, listener_port: u16) -> Result<Vec<Peer>, Error> {
    let res = get_tracker_response_by_metainfo(peer_id, metainfo, listener_port)?;
    Ok(res.peers)
}

fn encode_query_params(params: &[(&str, &str)]) -> String {
    let param_strings: Vec<String> = params.iter().map(|&(k, v)| format!("{}={}", k, v)).collect();
    param_strings.join("&")
}

#[derive(PartialEq, Debug)]
pub struct TrackerResponse {
    pub interval: u32,
    pub min_interval: Option<u32>,
    pub complete: u32,
    pub incomplete: u32,
    pub peers: Vec<Peer>,
}

impl TrackerResponse {
    pub fn parse(bytes: &[u8]) -> Result<TrackerResponse, DecodeError> {
        let mut d = Decoder::new(bytes);
        let value: Value = DecodeTo::decode(&mut d)?;
        FromValue::from_value(&value)
    }
}

impl FromValue for TrackerResponse {
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        let peers_value = value.get_value("peers")?;

        let peers: Vec<Peer> = match peers_value {
            Value::List(_) => {
                FromValue::from_value(peers_value)?
            }
            Value::Bytes(b) => {
                b.chunks(6).map(Peer::from_bytes).collect()
            }
            _ => { return Err(DecodeError::ExtraneousData); }
        };

        let response = TrackerResponse {
            interval: value.get_field("interval")?,
            min_interval: value.get_option_field("min interval")?,
            complete: value.get_field("complete")?,
            incomplete: value.get_field("incomplete")?,
            peers,
        };
        Ok(response)
    }
}

mod tests {
    use std::fs;
    use crate::bencode::decode::{Decoder, DecodeTo};
    use crate::base::meta_info::TorrentMetaInfo;
    use crate::net::tracker_connection::{get_peers, Error};
    use rand::Rng;
    use crate::net::peer_connection::Peer;
    use crate::generate_peer_id;

    const PEER_ID_PREFIX: &'static str = "-RC0001-";

    #[test]
    fn tracker_response() {
        let f = fs::read(
            r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        match get_peers(&generate_peer_id(), &torrent_meta_info, 8888) {
            Ok(_) => {}
            Err(e) => { println!("{:#?}", e) }
        }
    }

    #[test]
    fn tracker_response_chain() {
        let f = fs::read(
            r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
        ).map_err(Error::IoError).and_then(|f| {
                let mut decoder = Decoder::new(f.as_slice());
                let torrent_meta_info =
                    TorrentMetaInfo::decode(&mut decoder)?;
                Ok(torrent_meta_info)
        }).and_then(|torrent_meta_info| -> Result<Vec<Peer>, Error> {
            get_peers(&generate_peer_id(), &torrent_meta_info, 8888)
        });
    }
}

#[derive(Debug)]
pub enum Error {
    DecoderError(DecodeError),
    HyperError(hyper::Error),
    IoError(io::Error),
}

impl convert::From<DecodeError> for Error {
    fn from(err: DecodeError) -> Error {
        Error::DecoderError(err)
    }
}

impl convert::From<hyper::Error> for Error {
    fn from(err: hyper::Error) -> Error {
        Error::HyperError(err)
    }
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}