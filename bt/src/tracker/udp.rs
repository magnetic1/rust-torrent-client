// use std::convert::TryFrom;
// use crate::tracker::error::TrackerError;
// use std::sync::Arc;
// use crate::tracker::tracker::{Tracker, UdpConnection};
// use crate::bencode::hash::Sha1;
// use crate::peer::peer_connection::Peer;
//
// #[derive(Debug)]
// pub enum Action {
//     Connect,
//     Announce,
//     Scrape,
//     Error,
// }
//
// impl TryFrom<u32> for Action {
//     type Error = TorrentError;
//
//     fn try_from(n: u32) -> Result<Action, TrackerError> {
//         match n {
//             0 => Ok(Action::Connect),
//             1 => Ok(Action::Announce),
//             2 => Ok(Action::Scrape),
//             3 => Ok(Action::Error),
//             _ => Err(TrackerError::InvalidInput),
//         }
//     }
// }
//
// impl From<&Action> for u32 {
//     fn from(action: &Action) -> Self {
//         match action {
//             Action::Connect => 0,
//             Action::Announce => 1,
//             Action::Scrape => 2,
//             Action::Error => 3,
//         }
//     }
// }
//
// #[derive(Debug)]
// pub struct ConnectRequest {
//     pub protocol_id: u64,
//     pub action: Action,
//     pub transaction_id: u32,
// }
//
// impl ConnectRequest {
//     pub fn new(transaction_id: u32) -> ConnectRequest {
//         ConnectRequest {
//             transaction_id,
//             protocol_id: 0x0417_2710_1980,
//             action: Action::Connect,
//         }
//     }
// }
//
// #[derive(Debug)]
// pub struct ConnectResponse {
//     pub action: Action,
//     pub transaction_id: u32,
//     pub connection_id: u64,
// }
//
// #[derive(Debug)]
// pub enum Event {
//     None,
//     Completed,
//     Started,
//     Stopped,
// }
//
// impl TryFrom<u32> for Event {
//     type Error = TorrentError;
//
//     fn try_from(n: u32) -> Result<Event, TrackerError> {
//         match n {
//             0 => Ok(Event::None),
//             1 => Ok(Event::Completed),
//             2 => Ok(Event::Started),
//             3 => Ok(Event::Stopped),
//             _ => Err(TrackerError::InvalidInput),
//         }
//     }
// }
//
// impl From<&Event> for u32 {
//     fn from(e: &Event) -> Self {
//         match e {
//             Event::None => 0,
//             Event::Completed => 1,
//             Event::Started => 2,
//             Event::Stopped => 3,
//         }
//     }
// }
//
// #[derive(Debug)]
// pub struct AnnounceRequest {
//     pub connection_id: u64,
//     pub action: Action,
//     pub transaction_id: u32,
//     pub info_hash: Arc<Sha1>,
//     //    pub info_hash: [u8; 20],
//     pub peer_id: Arc<String>,
//     //    pub peer_id: [u8; 20],
//     pub downloaded: u64,
//     pub left: u64,
//     pub uploaded: u64,
//     pub event: Event,
//     pub ip_address: u32,
//     pub key: u32,
//     pub num_want: u32,
//     pub port: u16,
// }
//
// impl AnnounceRequest {
//     fn new(t: &Tracker, u: UdpConnection) -> AnnounceRequest {
//         let state = u.state.as_ref().unwrap();
//         AnnounceRequest {
//             connection_id: state.connection_id,
//             action: Action::Announce,
//             transaction_id: state.transaction_id,
//             info_hash: Arc::new(t.metadata.info_hash()),
//             peer_id: Arc::clone(&t.peer_id),
//             downloaded: 0,
//             left: t.metadata.length() as u64,
//             uploaded: 0,
//             event: Event::Started,
//             ip_address: 0,
//             key: 0,
//             num_want: 100,
//             port: 6881,
//         }
//     }
// }
//
// #[derive(Debug)]
// pub struct AnnounceResponse {
//     pub action: Action,
//     pub transaction_id: u32,
//     pub interval: u32,
//     pub leechers: u32,
//     pub seeders: u32,
//     pub addrs: Vec<Peer>,
// }
//
//
//
