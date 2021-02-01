use std::sync::Arc;
use url::Url;
use crate::tracker::tracker_supervisor::{TrackerStatus, TrackerSupervisor, TrackerResponse, TrackerMessage};
use crate::base::manager::ManagerEvent;
use crate::base::meta_info::TorrentMetaInfo;
use std::time::{Instant, Duration};
use futures::channel::mpsc::{UnboundedSender, Sender};
use async_std::task;
use crate::peer::peer_connection::Peer;
use crate::tracker::error::TrackerError;
use async_std::net::UdpSocket;
use url::percent_encoding::{percent_encode, FORM_URLENCODED_ENCODE_SET};
use crate::base::terminal;
use futures::SinkExt;
use crate::bencode::decode::DecodeError;

pub struct Tracker {
    pub(crate) metadata: Arc<TorrentMetaInfo>,
    to_manager: UnboundedSender<ManagerEvent>,
    pub(crate) peer_id: Arc<String>,
    url: Arc<Url>,
    tracker_supervisor: Sender<(Arc<Url>, Instant, TrackerStatus)>,

    interval: u64,
    listener_port: u16,
}

impl Tracker {
    pub fn new(ts: &TrackerSupervisor, url: Arc<Url>) -> Tracker {
        // url.scheme
        Tracker {
            metadata: Arc::new(ts.meta_info.clone()),
            to_manager: ts.to_manager.clone(),
            peer_id: Arc::new("".to_string()),
            url,
            tracker_supervisor: ts._sender.clone(),
            interval: 120,
            listener_port: ts.listener_port,
        }
    }

    pub async fn start(&mut self) {
        loop {
            let res = self.resolve_and_start().await;
            match res {
                Ok(_) => {}
                Err(TrackerError::Unresponsive) => {},
                Err(e) => {
                    terminal::print_log(format!("error: {}", e)).await.unwrap();
                    break;
                },
            }
            task::sleep(Duration::from_secs(self.interval)).await;
        }
    }

    async fn resolve_and_start(&mut self) -> Result<(), TrackerError> {
        use TrackerStatus::*;

        match self.try_announce().await {
            Ok(peer_addrs) => {
                self.send_to_supervisor(FoundPeers(peer_addrs.len())).await?;
                self.to_manager.send(ManagerEvent::Tracker(TrackerMessage::Peers(peer_addrs))).await?;
            }
            Err(TrackerError::Unresponsive) => {
                self.send_to_supervisor(HostUnresolved).await;
            }
            Err(e) => {
                self.send_to_supervisor(ErrorOccurred(Box::new(e))).await;
            }
        };

        Ok(())
    }

    async fn send_to_supervisor(&mut self, msg: TrackerStatus) -> Result<(), TrackerError> {
        self.tracker_supervisor
            .send((self.url.clone(), Instant::now(), msg))
            .await?;
        Ok(())
    }

    async fn try_announce(&mut self) -> Result<Vec<Peer>, TrackerError> {
        let res = match self.url.scheme.as_str() {
            "http" | "https" => self.http_announce().await,
            // "udp" => self.udp_announce().await,
            "udp" => Err(TrackerError::UnsupportedScheme),
            _ => Err(TrackerError::UnsupportedScheme),
        };

        res
    }

    async fn http_announce(&mut self) -> Result<Vec<Peer>, TrackerError> {
        let res = get_tracker_response_surf(
            self.peer_id.as_ref(),
            self.url.as_ref(),
            self.metadata.length(),
            self.metadata.info_hash().0,
            self.listener_port
        ).await?;
        self.interval = res.interval as u64;
        Ok(res.peers)
    }

    async fn udp_announce(&mut self) -> Result<Vec<Peer>, TrackerError> {
        let vec = Vec::new();
        let con = UdpConnection {
            url: Arc::clone(&self.url),
            state: None,
            buffer: vec![]
        };
        if con.state.is_none() {

        }


        Ok(vec)
    }
}

async fn get_tracker_response_surf(
    peer_id: &str,
    announce: &Url,
    length: u64,
    info_hash: [u8; 20],
    listener_port: u16,
) -> Result<TrackerResponse, TrackerError> {
    let length_string = length.to_string();
    let encoded_info_hash = percent_encode(&info_hash, FORM_URLENCODED_ENCODE_SET);
    let listener_port_string = listener_port.to_string();

    let params = vec![
        ("left", length_string.as_ref()),
        ("info_hash", encoded_info_hash.as_ref()),
        ("downloaded", "0"),
        ("uploaded", "0"),
        ("event", "started"),
        ("peer_id", peer_id),
        ("compact", "0"),
        ("port", listener_port_string.as_ref()),
    ];

    let url = format!("{}?{}", announce, encode_query_params(&params));
    terminal::print_log(format!("{}", url)).await.unwrap();

    let req = surf::get(&url)
        .header("Connection", "close")
        .body(surf::Body::empty())
        .await?;
    let body = surf::http::Body::from_reader(req, None);

    let buf = body.into_bytes().await?;
    terminal::print_log(format!("{}", buf.len())).await.unwrap();

    let res = TrackerResponse::parse(&buf);
    let res = match res {
        Ok(r) => Ok(r),
        Err(e) => {
            terminal::print_log(format!("TrackerResponse Decode error: {}", e)).await.unwrap();
            Err(TrackerError::InvalidInput)
        }
    };
    terminal::print_log(format!("{:#?}", res)).await.unwrap();

    res
}

fn encode_query_params(params: &[(&str, &str)]) -> String {
    let param_strings: Vec<String> = params
        .iter()
        .map(|&(k, v)| format!("{}={}", k, v))
        .collect();
    param_strings.join("&")
}

pub struct UdpConnection {
    pub url: Arc<Url>,
    pub state: Option<UdpState>,
    /// 16 bytes is just enough to send/receive Connect
    /// We allocate more after this request
    /// This avoid to allocate when the tracker is unreachable
    buffer: Vec<u8>,
}

pub struct UdpState {
    pub transaction_id: u32,
    pub connection_id: u64,
    connection_id_time: Instant,
    socket: UdpSocket,
}

#[cfg(test)]
mod test {
    use std::collections::HashMap;
    use std::sync::Arc;
    use url::Url;

    #[test]
    fn hash_map_arc_url() {
        let mut map: HashMap<Arc<Url>, i32> = Default::default();
        let url: Url = "https://google.com".parse().unwrap();
        let url = Arc::new(url);
        let url_clone = Arc::clone(&url);
        map.insert(url_clone, 1);

        println!("{} : {}", url, map[&url]);
    }
}