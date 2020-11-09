use crate::{
    bencode::value::{FromValue, Value},
    bencode::decode::{Decoder, DecodeTo, DecodeError},
    net::peer_connection::Peer,
    base::meta_info::TorrentMetaInfo,
};
use hyper::{
    Client,
    Uri,
    body::Buf,
    Request,
    Response,
    Body,
};
use url::percent_encoding::{percent_encode, FORM_URLENCODED_ENCODE_SET};
use parallel_stream::prelude::*;
use rand::Rng;
use std::error::Error;
use std::convert::TryFrom;
use std::time::Duration;
use futures::channel::mpsc::UnboundedSender;
use crate::base::manager::ManagerEvent;
use futures::SinkExt;
use std::sync::Arc;


type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug)]
pub struct TrackerSupervisor {
    meta_info: TorrentMetaInfo,
    sender: UnboundedSender<ManagerEvent>,
    peer_id: String,
    listener_port: u16,
}

#[derive(Debug)]
pub enum TrackerMessage {
    Peers(Vec<Peer>),
}

impl TrackerSupervisor {
    pub fn new(meta_info: TorrentMetaInfo, peer_id: String,
               listener_port: u16, sender: UnboundedSender<ManagerEvent>) -> TrackerSupervisor {
        TrackerSupervisor {
            meta_info,
            sender,
            peer_id,
            listener_port,
        }
    }

    fn has_announce_list(&self) -> bool {
        match &self.meta_info.announce_list {
            Some(_) => true,
            None => false
        }
    }

    pub async fn start(&mut self) -> Result<()> {

        if self.has_announce_list() {
            self.announce_list_loop().await?;
        } else {
            // panic!("announce unfinished!")
            self.announce_loop().await?;
        }
        Ok(())
    }

    pub async fn announce_loop(&mut self) -> Result<()> {
        // let announce_list = &mut t_supervisor.meta_info.announce_list.unwrap();

        let peer_id = &mut self.peer_id;
        let meta_info = &mut self.meta_info;
        let info_hash = &meta_info.info_hash().0;
        let len = meta_info.length();
        let announce = meta_info.announce.as_mut().unwrap();

        loop {
            match try_announce(announce, peer_id, info_hash, len,  self.listener_port).await {
                Ok(tracker_response) => {
                    let peers = tracker_response.peers;
                    self.sender.send(ManagerEvent::Tracker(TrackerMessage::Peers(peers))).await?;
                }
                Err(_) => {
                    async_std::task::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            }
            async_std::task::sleep(Duration::from_secs(30)).await;
        }
    }

    pub async fn announce_list_loop(&mut self) -> Result<()> {
        // let announce_list = &mut t_supervisor.meta_info.announce_list.unwrap();
        let peer_id = &mut self.peer_id;
        let meta_info = &mut self.meta_info;
        let announce_list = meta_info.announce_list.as_mut().unwrap();

        shuffle_announce_list(announce_list);
        announce_list[0][0] = String::from("http://nyaa.tracker.wf:7777/announce");
        loop {
            match announce_list_try(peer_id, meta_info, self.listener_port).await {
                Ok(tracker_response) => {
                    let peers = tracker_response.peers;
                    // let mut peers = Vec::new();
                    // peers.push(Peer {
                    //     ip: "127.0.0.1".to_string(),
                    //     port: 54682
                    // });
                    self.sender.send(ManagerEvent::Tracker(TrackerMessage::Peers(peers))).await?;
                },
                Err(_) => {
                    async_std::task::sleep(Duration::from_secs(5)).await;
                    continue;
                },
            }
            async_std::task::sleep(Duration::from_secs(30)).await;
        }
    }
}

fn shuffle_announce_list(announce_list: &mut Vec<Vec<String>>) {
    for tier in announce_list {
        knuth_shuffle(tier);
    }
}

async fn announce_list_try(peer_id: &str, meta_info: &mut TorrentMetaInfo, listener_port: u16) -> Result<TrackerResponse> {
    let info_hash = &meta_info.info_hash().0;
    let len = meta_info.length();
    let announce_list = meta_info.announce_list.as_mut().unwrap();
    let mut error: Box<dyn Error + Send + Sync> = Box::try_from("announce_list is empty").unwrap();

    for tier in announce_list {
        for (i, announce) in tier.iter().enumerate() {
            // try announce
            println!("try {}", announce);
            let result = try_announce(announce, peer_id, info_hash, len,  listener_port).await;
            error = match result {
                Err(e) => {
                    println!("error: {}", e);
                    e
                }
                Ok(response) => {
                    crate::bencode::hash::swap_to_head(tier, i);
                    return Ok(response);
                }
            };
        }
    }
    Err(error)
}

async fn try_announce(announce: &str, peer_id: &str, info_hash: &[u8; 20], length: u64, listener_port: u16) -> Result<TrackerResponse> {
    // let announce = announce.to_owned();
    // let peer_id = peer_id.to_owned();
    // let info_hash = info_hash.to_vec();
    //
    // async_std::task::spawn(async move {
    //     tokio::block_on(async move {
    //         get_tracker_response(&peer_id, &announce, length, &info_hash, listener_port).await
    //     })
    // }).await

    get_tracker_response_surf(peer_id, announce, length, info_hash, listener_port).await
}

fn knuth_shuffle<T>(list: &mut [T]) {
    let mut rng = rand::thread_rng();

    for i in 0..list.len()-1 {
        let k = list.len() - i - 1;
        list.swap(k, rng.gen_range(0, k));
    }
}

pub async fn get_peer(t: &TorrentMetaInfo) {
    let announces = match t.announce_list {
        Some(ref v) => {
            v[0].iter().map(|s| s.clone()).collect::<Vec<String>>()
        }
        None => {
            let s = t.announce.as_ref().unwrap().clone();
            vec![s]
        }
    };
    let v = vec![1, 2, 3, 4];
    let mut out: Vec<usize> = v
        .into_par_stream()
        .map(|n| async move { n * n })
        .collect()
        .await;

    // let mut out: Vec<String> = announces
    //     .into_par_stream()
    //     .map(|n: String| async move {
    //         get_peers()
    //     }).collect()
    //     .await;
}

async fn get_tracker_response_async(peer_id: &str, announce: &str, length: u64,
                              info_hash: &[u8], listener_port: u16) -> Result<TrackerResponse> {
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

    let stream = async_std::net::TcpStream::connect("127.0.0.1:48582").await?;
    let url = http_types::Url::parse(&format!("{}?{}", announce, encode_query_params(&params)))?;
    println!("{}", url.as_str());

    let mut req = http_types::Request::new(http_types::Method::Get, url);
    req.insert_header("Connection", "close");

    let mut http_res = async_h1::connect( stream.clone(), req).await?;;

    let body = http_res.body_bytes().await?;
    // let buf = hyper::body::to_bytes(http_res).await?;
    // let mut body = buf.bytes();
    println!("{}", body.len());

    let res = TrackerResponse::parse(&body)?;

    println!("{:#?}", res);
    Ok(res)
}

async fn get_tracker_response_surf(peer_id: &str, announce: &str, length: u64,
                              info_hash: &[u8], listener_port: u16) -> Result<TrackerResponse> {
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

    let req = surf::get(&url)
        .header("Connection", "close")
        .body(surf::Body::empty()).await?;
    let body = surf::http::Body::from_reader(req, None);
    let buf = body.into_bytes().await?;
    //
    // let client = Client::new();
    // let request = Request::builder()
    //     .uri(url.as_str())
    //     .header("Connection", "close")
    //     .body(Body::empty())
    //     .unwrap();
    // let http_res = client.request(request).await?;
    //
    // let buf = hyper::body::to_bytes(http_res).await?;
    // let mut body = buf.bytes();
    println!("{}", buf.len());

    let res = TrackerResponse::parse(&buf)?;

    println!("{:#?}", res);
    Ok(res)
}

async fn get_tracker_response(peer_id: &str, announce: &str, length: u64,
                              info_hash: &[u8], listener_port: u16) -> Result<TrackerResponse> {
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
    let request = Request::builder()
        .uri(url.as_str())
        .header("Connection", "close")
        .body(Body::empty())
        .unwrap();
    let http_res = client.request(request).await?;

    let buf = hyper::body::to_bytes(http_res).await?;
    let mut body = buf.bytes();
    println!("{}", body.len());

    let res = TrackerResponse::parse(body)?;

    println!("{:#?}", res);
    Ok(res)
}

pub async fn get_tracker_response_by_meta_info(announce: &str, peer_id: &str,
                                               meta_info: &TorrentMetaInfo,
                                               listener_port: u16) -> Result<TrackerResponse> {
    let info_hash = &meta_info.info_hash().0;
    get_tracker_response(peer_id, announce, meta_info.length(), info_hash, listener_port).await
}

pub async fn get_peers(peer_id: &str, metainfo: &TorrentMetaInfo, listener_port: u16) -> Result<Vec<Peer>> {
    let announce = "http://explodie.org:6969/announce";
    let res = get_tracker_response_by_meta_info(announce, peer_id, metainfo, listener_port).await?;
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
    pub fn parse(bytes: &[u8]) -> Result<TrackerResponse> {
        let mut d = Decoder::new(bytes);
        let value: Value = DecodeTo::decode(&mut d)?;
        Ok(FromValue::from_value(&value)?)
    }
}

impl FromValue for TrackerResponse {
    fn from_value(value: &Value) -> std::result::Result<Self, DecodeError> {
        let peers_value = value.get_value("peers")?;

        let peers: Vec<Peer> = match peers_value {
            Value::List(_) => {
                FromValue::from_value(peers_value)?
            }
            Value::Bytes(b) => {
                b.chunks(6).map(Peer::from_bytes).collect()
            }
            _ => Err(DecodeError::ExtraneousData)?,
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

#[cfg(test)]
mod test {
    use std::fs;
    use crate::bencode::decode::{Decoder, DecodeTo};
    use crate::base::meta_info::TorrentMetaInfo;
    use crate::net::tracker::get_peers;
    use rand::Rng;

    const PEER_ID_PREFIX: &'static str = "-RC0001-";

    #[test]
    fn tracker_response() {
        let f = fs::read(
            r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        let res = tokio_test::block_on(async {
            get_peers(&generate_peer_id(), &torrent_meta_info, 8888).await
        });
        match res {
            Ok(_) => {}
            Err(e) => { println!("{:#?}", e) }
        }
    }

    fn generate_peer_id() -> String {
        let mut rng = rand::thread_rng();
        let rand_chars: String = rng.gen_ascii_chars().take(20 - PEER_ID_PREFIX.len()).collect();
        format!("{}{}", PEER_ID_PREFIX, rand_chars)
    }
}
