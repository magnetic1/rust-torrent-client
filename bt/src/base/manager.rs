use std::collections::{HashMap, VecDeque};

use async_std::sync::Arc;
use futures::{channel::mpsc, channel::mpsc::Sender, select, SinkExt, StreamExt};
use futures::channel::mpsc::UnboundedSender;

use crate::{
    base::download::{download_inline, download_loop, Piece},
    base::ipc::{IPC, Message},
    base::meta_info::TorrentMetaInfo,
    base::spawn_and_log_error,
};
use crate::base::terminal;
use crate::peer::peer_connection::{Peer, peer_conn_loop, RequestMetadata};
use crate::tracker::tracker_supervisor::{TrackerMessage, TrackerSupervisor};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Manager {
    pub(crate) our_peer_id: String,
    pub(crate) meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    file_offsets: Vec<u64>,
    file_paths: Vec<Arc<String>>,

    peers: HashMap<Peer, Sender<IPC>>,
    peers_deque: VecDeque<(bool, Peer)>,

    sender_to_download: Sender<ManagerEvent>,
    pub(crate) sender_unbounded: UnboundedSender<ManagerEvent>,

    pub(crate) listener_port: u16
}

impl Manager {
    async fn process_event(&mut self, event: ManagerEvent, disconnect_sender: &Sender<Peer>) -> Result<()> {
        match event {
            ManagerEvent::Continue => {}
            ManagerEvent::Broadcast(ipc) => {
                // terminal::print_log(format!("Broadcast {:?}", ipc))?;
                let mut delete_keys = Vec::with_capacity(self.peers.len());
                for (p, s) in self.peers.iter_mut() {
                    match s.send(ipc.clone()).await {
                        Ok(()) => (),
                        Err(_) => {
                            delete_keys.push(p.clone());
                        }
                    }
                }
                // terminal::print_log(format!("manger loop: Broadcast end"))?;
                let _r: Vec<()> = delete_keys
                    .iter()
                    .map(|p| {
                        self.peers.remove(p);
                        match self.peers_deque.pop_front() {
                            Some((send_handshake_first, peer)) => {
                                // manager.peers.
                                connect(
                                    send_handshake_first,
                                    peer,
                                    self,
                                    disconnect_sender.clone(),
                                );
                            }
                            None => {}
                        }
                    })
                    .collect::<Vec<()>>();
            }
            ManagerEvent::Connection(send_handshake_first, peer) => {
                let pair = (send_handshake_first, peer);
                let ref peer = pair.1;
                if self.peers.get(peer).is_some() || self.peers_deque.contains(&pair) {
                    // continue;
                } else if self.peers.len() < 30 {
                    connect(
                        send_handshake_first,
                        pair.1,
                        self,
                        disconnect_sender.clone(),
                    );
                } else {
                    let peer = pair.1;
                    self.peers_deque.push_back((send_handshake_first, peer));
                }
            }
            ManagerEvent::RequirePieceLength(sender) => {
                sender.send(self.meta_info.piece_length()).unwrap();
            }
            ManagerEvent::FileFinish(file_index, file_path) => {
                    // = Arc::new(Mutex::new(new_file));
                    self.file_paths[file_index] = file_path;
            }
            ManagerEvent::Tracker(tracker_message) => match tracker_message {
                TrackerMessage::Peers(peers) => {
                    let mut sender = self.sender_unbounded.clone();
                    async_std::task::spawn(async move {
                        for p in peers {
                            sender.send(ManagerEvent::Connection(true, p)).await.unwrap();
                        }
                    });
                }
            },
            e => self.sender_to_download.send(e).await?,
        }

        Ok(())
    }
}

fn connect(
    send_handshake_first: bool,
    peer: Peer,
    manager: &mut Manager,
    mut disconnect_sender: Sender<Peer>,
) {
    let (peer_sender, peer_receiver) = mpsc::channel(10);
    let params = (
        send_handshake_first,
        manager.our_peer_id.clone(),
        manager.meta_info.info_hash(),
        peer.clone(),
        peer_sender.clone(),
        peer_receiver,
        manager.sender_unbounded.clone(),
        manager.sender_to_download.clone(),
    );
    assert!(manager.peers.insert(peer, peer_sender.clone()).is_none());
    // start peer conn loop
    spawn_and_log_error(async move {
        let peer = params.3.clone();
        let res = peer_conn_loop(
            params.0, params.1, params.2, params.3, params.4, params.5, params.6, params.7,
        )
            .await;
        disconnect_sender.send(peer).await.unwrap();
        terminal::print_log(format!("peer connection finished"))?;
        res
    });

}

//noinspection RsTypeCheck
pub async fn manager_loop(our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<()> {
    let (sender_to_download, download_receiver) = mpsc::channel(10);
    let (sender_unbounded, mut events_unbounded) = mpsc::unbounded();
    // let (mut sender_unbounded, mut events_unbounded) = mpsc::unbounded();
    let peers: HashMap<Peer, Sender<IPC>> = HashMap::new();
    let peers_deque: VecDeque<(bool, Peer)> = VecDeque::new();

    let file_infos = download_inline::create_file_infos(&meta_info.info).await;
    terminal::print_log(format!("create_file_infos finished"))?;

    // terminal::fresh_state(State::Magenta("2█".to_string()))?;

    let (file_offsets, file_paths, files) = download_inline::create_files(file_infos).await?;
    terminal::print_log(format!("create_files finished"))?;

    let len = file_offsets[file_offsets.len() - 1];
    let pieces = download_inline::create_pieces(len, &meta_info).await;
    terminal::print_log(format!("create_pieces finished"))?;
    terminal::print_log(format!("file_offsets: {:?} len: {}", file_offsets, len))?;

    let ps: Vec<u32> = pieces.iter().map(|p| p.length).collect();
    terminal::print_log(format!("pieces len: {:?}", ps[0]))?;

    let mut manager = Manager {
        our_peer_id: our_peer_id.clone(),
        meta_info,
        pieces,
        file_offsets,
        file_paths: file_paths.clone(),

        peers,
        peers_deque,
        sender_to_download,
        sender_unbounded,

        listener_port: 54654,
    };

    let _down_handle = spawn_and_log_error(download_loop(
        download_receiver,
        manager.sender_unbounded.clone(),
        file_paths,
        files,
        manager.file_offsets.clone(),
        manager.our_peer_id.clone(),
        manager.meta_info.clone(),
    ));

    let mut tracker_supervisor = TrackerSupervisor::from_manager(&manager);
    let _tracker_handle = spawn_and_log_error(async move {
        let res = tracker_supervisor.run().await;
        terminal::print_log(format!("tracker supervisor finished"))?;
        res
    });

    // {
    //     let mut ps = Vec::new();
    //     // ps.push(Peer {
    //     //     ip: "127.0.0.1".to_string(),
    //     //     port: 54682,
    //     // });
    //     for p in ps {
    //         manager.sender_unbounded.send(ManagerEvent::Connection(true, p)).await?;
    //     }
    // }

    let (disconnect_sender, mut disconnect_receiver) = mpsc::channel(10);

    loop {
        // to fix the warning of intellij-rust
        let mut event = ManagerEvent::Continue;
        select! {
            e = events_unbounded.next() => match e {
                Some(e) => event = e,
                None => break,
            },
            // peer disconnect
            disconnect = disconnect_receiver.next() => {
                let peer = disconnect.unwrap();
                // terminal::print_log(format!("remove {:?}", peer))?;
                // assert!(manager.peers.remove(&peer).is_some());
                manager.peers.remove(&peer);
                terminal::print_log(format!("{:?}: disconnected", peer))?;
                continue;
            },
        };
        // terminal::print_log(format!("manger loop: {:?}", event))?;

        manager.process_event(event, &disconnect_sender).await?;
    }

    Ok(())
}

#[derive(Debug)]
pub enum ManagerEvent {
    Continue,

    Broadcast(IPC),
    Connection(bool, Peer),

    RequirePieceLength(futures::channel::oneshot::Sender<usize>),
    FileFinish(usize, Arc<String>),

    Download(Message),
    RequireData(RequestMetadata, futures::channel::oneshot::Sender<Vec<u8>>),
    RequireIncompleteBlocks(u32, futures::channel::oneshot::Sender<Vec<(u32, u32)>>),
    RequireHavePieces(futures::channel::oneshot::Sender<Vec<bool>>),

    Tracker(TrackerMessage),
}


#[cfg(test)]
mod test {
    use std::mem::size_of;
    use crate::base::manager::ManagerEvent;

    #[test]
    fn size_of_manager_event() {
        println!("{}", size_of::<ManagerEvent>())
    }
}