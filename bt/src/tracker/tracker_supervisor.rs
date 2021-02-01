use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_std::task::JoinHandle;
use futures::channel::mpsc::{Receiver, Sender, UnboundedSender};
use futures::channel::mpsc;
use futures::{SinkExt, StreamExt};
// use hyper::{body::Buf, Body, Client, Request};
// use parallel_stream::prelude::*;
use rand::Rng;
use url::percent_encoding::{FORM_URLENCODED_ENCODE_SET, percent_encode};
use url::Url;

use crate::{
    base::meta_info::TorrentMetaInfo,
    bencode::decode::{DecodeError, Decoder, DecodeTo},
    bencode::value::{FromValue, Value},
};
use crate::base::manager::{Manager, ManagerEvent};
use crate::base::terminal;
use crate::peer::peer_connection::Peer;
use crate::tracker::tracker::Tracker;
use async_std::future::TimeoutError;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Debug)]
pub struct TrackerSupervisor {
    pub(crate) meta_info: TorrentMetaInfo,
    pub(crate) to_manager: UnboundedSender<ManagerEvent>,
    pub(crate) peer_id: String,
    pub(crate) listener_port: u16,

    /// List of urls, by tier
    urls: Vec<Arc<Url>>,
    receiver: Receiver<(Arc<Url>, Instant, TrackerStatus)>,
    /// Keep a sender to not close the channel
    pub(crate) _sender: Sender<(Arc<Url>, Instant, TrackerStatus)>,
    pub(crate) tracker_states: HashMap<Arc<Url>, (Instant, TrackerStatus)>,
}

#[derive(Debug)]
pub enum TrackerStatus {
    FoundPeers(usize),
    HostUnresolved,
    ErrorOccurred(Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Debug)]
pub enum TrackerMessage {
    Peers(Vec<Peer>),
}

// #[derive(Debug, Eq)]
// pub struct TrackerUrl {
//     url: Url,
//     position: UrlHash,
//     tier: usize,
// }

impl TrackerSupervisor {

    pub(crate) fn from_manager(manager: &Manager) -> TrackerSupervisor {
        let urls = manager.meta_info.get_urls();

        let (_sender, receiver) = mpsc::channel(10);;
        TrackerSupervisor {
            meta_info: manager.meta_info.clone(),
            to_manager: manager.sender_unbounded.clone(),
            peer_id: manager.our_peer_id.clone(),
            listener_port: manager.listener_port,

            urls,
            _sender,
            receiver,
            tracker_states: Default::default(),
        }
    }

    fn has_announce_list(&self) -> bool {
        match &self.meta_info.announce_list {
            Some(_) => true,
            None => false,
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

    pub async fn run(&mut self) -> Result<()> {
        self.loop_until_connected().await?;
        self.wait_on_tracker_msg().await;
        Ok(())
    }

    async fn loop_until_connected(&mut self) -> Result<()> {
        let mut pending_status = Vec::with_capacity(10);

        for url in self.urls.as_slice() {
            self.spawn_tracker(url);

            let duration = Duration::from_secs(15);
            match async_std::future::timeout(duration, self.receiver.next()).await {
                Ok(Some((url, instant, TrackerStatus::FoundPeers(n)))) => {
                    pending_status.push((url, instant, TrackerStatus::FoundPeers(n)));
                    break;
                }
                Ok(Some((url, instant, msg))) => {
                    pending_status.push((url, instant, msg));
                }
                _ => {}
            }
        }

        for (url, instant, status) in pending_status {
            self.tracker_states.insert(url, (instant, status));
        }

        Ok(())
    }

    fn spawn_tracker(&self, url: &Arc<Url>) {
        let mut t = Tracker::new(&self, url.clone());
        async_std::task::spawn(async move { t.start().await });
    }

    async fn wait_on_tracker_msg(&mut self) {
        while let Some((url, instant, status)) = self.receiver.next().await {
            self.tracker_states.insert(url, (instant, status));

            if !self.is_one_active() {
                self.try_another_tracker().await;
            }
        }
    }

    async fn try_another_tracker(&self) {
        let spawned = { self.tracker_states.keys().cloned().collect::<Vec<_>>() };

        for url in self.urls.as_slice() {
            if !spawned.contains(url) {
                self.spawn_tracker(url);
                return;
            }
        }
    }

    fn is_one_active(&self) -> bool {
        for state in self.tracker_states.values() {
            if let TrackerStatus::FoundPeers(n) = state.1 {
                // We consider the tracker active if it found at least 2
                // peer addresses
                if n > 1 {
                    return true;
                }
            }
        }
        false
    }

    pub async fn announce_loop(&mut self) -> Result<()> {
        // let announce_list = &mut t_supervisor.meta_info.announce_list.unwrap();

        let peer_id = &mut self.peer_id;
        let meta_info = &mut self.meta_info;
        let info_hash = &meta_info.info_hash().0;
        let len = meta_info.length();
        let announce = meta_info.announce.as_mut().unwrap();

        loop {
            match try_announce(announce, peer_id, info_hash, len, self.listener_port).await {
                Ok(tracker_response) => {
                    let peers = tracker_response.peers;
                    self.to_manager
                        .send(ManagerEvent::Tracker(TrackerMessage::Peers(peers)))
                        .await?;
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

        let len = meta_info.length();
        let info_hash = meta_info.info_hash().0.clone();
        let listener_port = self.listener_port;

        let announce_list = meta_info.announce_list.as_mut().unwrap();
        shuffle_announce_list(announce_list);

        // let announce_list = Arc::new(Mutex::new(announce_list));
        // announce_list[0][0] = String::from("http://nyaa.tracker.wf:7777/announce");
        // let mut peers = Vec::new();
        // peers.push(Peer {
        //     ip: "127.0.0.1".to_string(),
        //     port: 54682
        // });
        // self.sender.send(ManagerEvent::Tracker(TrackerMessage::Peers(peers))).await?;

        let mut handles = Vec::new();
        for tier in announce_list {
            for (_i, announce) in tier.iter().enumerate() {
                let mut sender = self.to_manager.clone();
                let announce = announce.to_owned();
                let peer_id = peer_id.to_owned();
                // try announce
                let h: JoinHandle<Result<()>> = async_std::task::spawn(async move {
                    loop {
                        terminal::print_log(format!("try {}", announce)).await?;
                        let result =
                            try_announce(&announce, &peer_id, &info_hash, len, listener_port).await;
                        match result {
                            Err(e) => {
                                break;
                            }
                            Ok(response) => {
                                let peers = response.peers;
                                sender
                                    .send(ManagerEvent::Tracker(TrackerMessage::Peers(peers)))
                                    .await?
                            }
                        };
                        async_std::task::sleep(Duration::from_secs(60 * 60 * 5)).await;
                    }
                    Ok(())
                });
                handles.push(h);
            }
        }

        for h in handles {
            h.await;
        }

        Ok(())
        // loop {
        //     match announce_list_try(peer_id, meta_info, self.listener_port).await {
        //         Ok(tracker_response) => {
        //             let peers = tracker_response.peers;
        //             self.sender.send(ManagerEvent::Tracker(TrackerMessage::Peers(peers))).await?;
        //         },
        //         Err(_) => {
        //             async_std::task::sleep(Duration::from_secs(5)).await;
        //             continue;
        //         },
        //     }
        //     async_std::task::sleep(Duration::from_secs(30)).await;
        // }
    }
}

fn shuffle_announce_list(announce_list: &mut Vec<Vec<String>>) {
    for tier in announce_list {
        knuth_shuffle(tier);
    }
}

async fn try_announce(
    announce: &str,
    peer_id: &str,
    info_hash: &[u8; 20],
    length: u64,
    listener_port: u16,
) -> Result<TrackerResponse> {
    get_tracker_response_surf(peer_id, announce, length, info_hash, listener_port).await
}

fn knuth_shuffle<T>(list: &mut [T]) {
    let mut rng = rand::thread_rng();

    for i in 0..list.len() - 1 {
        let k = list.len() - i - 1;
        list.swap(k, rng.gen_range(0, k));
    }
}

async fn get_tracker_response_surf(
    peer_id: &str,
    announce: &str,
    length: u64,
    info_hash: &[u8],
    listener_port: u16,
) -> Result<TrackerResponse> {
    let length_string = length.to_string();
    let encoded_info_hash = percent_encode(info_hash, FORM_URLENCODED_ENCODE_SET);
    let listener_port_string = listener_port.to_string();

    let params = vec![
        ("left", length_string.as_ref()),
        ("info_hash", encoded_info_hash.as_ref()),
        ("downloaded", "0"),
        ("uploaded", "0"),
        ("event", "started"),
        ("peer_id", peer_id),
        ("compact", "1"),
        ("port", listener_port_string.as_ref()),
    ];

    let url = format!("{}?{}", announce, encode_query_params(&params));
    terminal::print_log(format!("{}", url)).await?;

    let req = surf::get(&url)
        .header("Connection", "close")
        .body(surf::Body::empty())
        .await?;
    let body = surf::http::Body::from_reader(req, None);

    let buf = body.into_bytes().await?;
    terminal::print_log(format!("{}", buf.len())).await?;

    let res = TrackerResponse::parse(&buf)?;
    terminal::print_log(format!("{:#?}", res)).await?;

    Ok(res)
}

fn encode_query_params(params: &[(&str, &str)]) -> String {
    let param_strings: Vec<String> = params
        .iter()
        .map(|&(k, v)| format!("{}={}", k, v))
        .collect();
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
    pub fn parse(bytes: &[u8]) -> std::result::Result<Self, DecodeError> {
        let mut d = Decoder::new(bytes);
        let value: Value = DecodeTo::decode(&mut d)?;
        Ok(FromValue::from_value(&value)?)
    }
}

impl FromValue for TrackerResponse {
    fn from_value(value: &Value) -> std::result::Result<Self, DecodeError> {
        let peers_value = value.get_value("peers")?;

        let peers: Vec<Peer> = match peers_value {
            Value::List(_) => FromValue::from_value(peers_value)?,
            Value::Bytes(b) => b.chunks(6).map(Peer::from_bytes).collect(),
            v => {
                // terminal::print_log(format!("{:#?}", v)).await?;
                Err(DecodeError::ExtraneousData)?
            }
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

    use rand::Rng;

    use crate::base::meta_info::TorrentMetaInfo;
    use crate::bencode::decode::{Decoder, DecodeTo};
    use crate::tracker::tracker_supervisor::get_tracker_response_surf;

    const PEER_ID_PREFIX: &'static str = "-RC0001-";

    #[test]
    fn tracker_response() {
        let f = fs::read(
            r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        let announce = "http://explodie.org:6969/announce";
        let tracker_response = async_std::task::block_on(async move {
            get_tracker_response_surf(
                &generate_peer_id(),
                &announce,
                torrent_meta_info.length(),
                &torrent_meta_info.info_hash().0,
                8888,
            ).await
        });
        let res = tracker_response.map(|r| r.peers);
        // get_peers(&generate_peer_id(), &torrent_meta_info, 8888).await
        match res {
            Ok(_) => {}
            Err(e) => println!("{:#?}", e),
        }
    }

    fn generate_peer_id() -> String {
        let mut rng = rand::thread_rng();
        let rand_chars: String = rng
            .gen_ascii_chars()
            .take(20 - PEER_ID_PREFIX.len())
            .collect();
        format!("{}{}", PEER_ID_PREFIX, rand_chars)
    }
}
