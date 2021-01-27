use crate::net::tracker::{TrackerMessage, TrackerSupervisor};
use crate::{
    base::download::{download_inline, download_loop, Piece},
    base::ipc::{Message, IPC},
    base::meta_info::TorrentMetaInfo,
    base::spawn_and_log_error,
    net::peer_connection::{peer_conn_loop, Peer, RequestMetadata},
};
use async_std::{
    fs::{File, OpenOptions},
    sync::{Arc, Mutex},
};
use futures::channel::mpsc::UnboundedSender;
use futures::{channel::mpsc, channel::mpsc::Sender, select, SinkExt, StreamExt};
use std::collections::{HashMap, VecDeque};
use crate::base::terminal;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Manager {
    our_peer_id: String,
    meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    files: Arc<Vec<Arc<Mutex<File>>>>,
    file_offsets: Vec<u64>,
    file_paths: Vec<String>,
}

fn connect(
    send_handshake_first: bool,
    peer: Peer,
    peers: &mut HashMap<Peer, Sender<IPC>>,
    our_peer_id: String,
    manager: &mut Manager,
    mut disconnect_sender: Sender<Peer>,
    sender_unbounded: UnboundedSender<ManagerEvent>,
    sender_to_download: Sender<ManagerEvent>,
) {
    let (peer_sender, peer_receiver) = mpsc::channel(10);
    let params = (
        send_handshake_first,
        our_peer_id,
        manager.meta_info.info_hash(),
        peer.clone(),
        peer_sender.clone(),
        peer_receiver,
        sender_unbounded,
        sender_to_download,
    );
    // start peer conn loop
    spawn_and_log_error(async move {
        let peer = params.3.clone();
        let res = peer_conn_loop(
            params.0, params.1, params.2, params.3, params.4, params.5, params.6, params.7,
        )
        .await;
        disconnect_sender.send(peer).await.unwrap();
        terminal::print_log(format!("peer connection finished")).await?;
        res
    });

    assert!(peers.insert(peer, peer_sender.clone()).is_none());
}

pub async fn manager_loop(our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<()> {
    let (mut sender_to_download, download_receiver) = mpsc::channel(10);
    let (mut sender_unbounded, mut events_unbounded) = mpsc::unbounded();
    // let (mut sender_unbounded, mut events_unbounded) = mpsc::unbounded();
    let mut peers: HashMap<Peer, Sender<IPC>> = HashMap::new();
    let mut peers_deque: VecDeque<(bool, Peer)> = VecDeque::new();

    let file_infos = download_inline::create_file_infos(&meta_info.info).await;
    terminal::print_log(format!("create_file_infos finished")).await?;

    let (file_offsets, file_paths, files) = download_inline::create_files(file_infos).await?;
    terminal::print_log(format!("create_files finished")).await?;

    let len = file_offsets[file_offsets.len() - 1];
    let pieces = download_inline::create_pieces(len, &meta_info).await;
    terminal::print_log(format!("create_pieces finished")).await?;
    terminal::print_log(format!("file_offsets: {:?} len: {}", file_offsets, len)).await?;

    let ps: Vec<u32> = pieces.iter().map(|p| p.length).collect();
    terminal::print_log(format!("pieces len: {:?}", ps)).await?;

    let mut manager = Manager {
        our_peer_id: our_peer_id.clone(),
        meta_info,
        pieces,
        files: Arc::new(files),
        file_offsets,
        file_paths,
    };

    let _down_handle = spawn_and_log_error(download_loop(
        download_receiver,
        sender_unbounded.clone(),
        Arc::clone(&manager.files),
        manager.file_offsets.clone(),
        manager.our_peer_id.clone(),
        manager.meta_info.clone(),
    ));

    let mut tracker_supervisor = TrackerSupervisor::new(
        manager.meta_info.clone(),
        manager.our_peer_id.clone(),
        54654,
        sender_unbounded.clone(),
    );
    // let _tracker_handle = spawn_and_log_error(async move {
    //     let res = tracker_supervisor.start().await;
    //     terminal::print_log(format!("tracker supervisor finished")).await?;
    //     res
    // });

    {
        let mut ps = Vec::new();
        ps.push(Peer {
            ip: "127.0.0.1".to_string(),
            port: 54682,
        });
        for p in ps {
            sender_unbounded.send(ManagerEvent::Connection(true, p)).await?;
        }
    }

    let (disconnect_sender, mut disconnect_receiver) = mpsc::channel(10);

    loop {
        let event = select! {
            event = events_unbounded.next() => match event {
                Some(event) => event,
                None => break,
            },
            // event = events_unbounded.next() => match event {
            //     Some(event) => event,
            //     None => break,
            // },
            disconnect = disconnect_receiver.next() => {
                let peer = disconnect.unwrap();
                terminal::print_log(format!("remove {:?}", peer)).await?;
                assert!(peers.remove(&peer).is_some());
                terminal::print_log(format!("{:?}: disconnected", peer)).await?;
                continue;
            },
        };
        // terminal::print_log(format!("manger loop: {:?}", event)).await?;

        match event {
            ManagerEvent::Broadcast(ipc) => {
                terminal::print_log(format!("manger loop: Broadcast start")).await?;
                let mut delete_keys = Vec::with_capacity(peers.len());
                for (p, s) in peers.iter_mut() {
                    match s.send(ipc.clone()).await {
                        Ok(()) => (),
                        Err(_) => {
                            delete_keys.push(p.clone());
                        }
                    }
                }
                terminal::print_log(format!("manger loop: Broadcast end")).await?;
                let _r: Vec<()> = delete_keys
                    .iter()
                    .map(|p| {
                        peers.remove(p);
                        match peers_deque.pop_front() {
                            Some((send_handshake_first, peer)) => {
                                // peers.
                                connect(
                                    send_handshake_first,
                                    peer,
                                    &mut peers,
                                    our_peer_id.clone(),
                                    &mut manager,
                                    disconnect_sender.clone(),
                                    sender_unbounded.clone(),
                                    sender_to_download.clone(),
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
                if peers.get(peer).is_some() || peers_deque.contains(&pair) {
                    continue;
                } else if peers.len() < 10 {
                    connect(
                        send_handshake_first,
                        pair.1,
                        &mut peers,
                        our_peer_id.clone(),
                        &mut manager,
                        disconnect_sender.clone(),
                        sender_unbounded.clone(),
                        sender_to_download.clone(),
                    );

                // let (peer_sender, peer_receiver) = mpsc::channel(10);
                // let params = (send_handshake_first, our_peer_id.clone(),
                //               manager.meta_info.info_hash(), peer.clone(), peer_sender.clone(),
                //               peer_receiver, sender_unbounded.clone(), sender_to_download.clone());
                // let mut disconnect_sender = disconnect_sender.clone();
                // // start peer conn loop
                // spawn_and_log_error(async move {
                //     let peer = params.3.clone();
                //     let res = peer_conn_loop(params.0, params.1,
                //                              params.2, params.3, params.4,
                //                              params.5, params.6, params.7).await;
                //     disconnect_sender.send(peer).await.unwrap();
                //     terminal::print_log(format!("peer connection finished")).await?;
                //     res
                // });
                // let peer = pair.1;
                // assert!(peers.insert(peer, peer_sender.clone()).is_none());
                } else {
                    let peer = pair.1;
                    peers_deque.push_back((send_handshake_first, peer));
                }
                // if peers.len() < 10 && peers.get(&peer).is_none() {
                //     let (peer_sender, peer_receiver) = mpsc::channel(10);
                //     let params = (send_handshake_first, our_peer_id.clone(),
                //                   manager.meta_info.info_hash(), peer.clone(), peer_sender.clone(),
                //                   peer_receiver, sender_unbounded.clone(), sender_to_download.clone());
                //     let mut disconnect_sender = disconnect_sender.clone();
                //     // start peer conn loop
                //     spawn_and_log_error(async move {
                //         let peer = params.3.clone();
                //         let res = peer_conn_loop(params.0, params.1,
                //                                  params.2, params.3, params.4,
                //                                  params.5, params.6, params.7).await;
                //         disconnect_sender.send(peer).await.unwrap();
                //         terminal::print_log(format!("peer connection finished")).await?;
                //         res
                //     });
                //     assert!(peers.insert(peer, peer_sender.clone()).is_none());
                // }
            }

            ManagerEvent::RequirePieceLength(sender) => {
                sender.send(manager.meta_info.piece_length()).unwrap();
            }
            ManagerEvent::FileFinish(file_index) => {
                let name = &manager.file_paths[file_index];
                if name.ends_with(".temp") {
                    let new_name = &name[..name.len() - 5];
                    async_std::fs::rename(name, new_name).await?;
                    let new_file = OpenOptions::new()
                        .create(true)
                        .read(true)
                        .write(true)
                        .open(new_name)
                        .await?;
                    let mut file = manager.files[file_index].lock().await;
                    *file = new_file;
                    // = Arc::new(Mutex::new(new_file));
                    manager.file_paths[file_index] = String::from(new_name);
                }
            }

            ManagerEvent::Tracker(tracker_message) => match tracker_message {
                TrackerMessage::Peers(peers) => {
                    let mut sender = sender_unbounded.clone();
                    async_std::task::spawn(async move {
                        for p in peers {
                            sender
                                .send(ManagerEvent::Connection(true, p))
                                .await;
                        }
                    });
                }
            },

            e => sender_to_download.send(e).await?,
            // ManagerEvent::RequireData(_, _) => {}
            // ManagerEvent::RequireIncompleteBlocks(_, _) => {}
            // ManagerEvent::Download(message) => {
            // }
            // ManagerEvent::RequireHavePieces(_) => {}
        }
    }

    Ok(())
}

#[derive(Debug)]
pub enum ManagerEvent {
    Broadcast(IPC),
    Connection(bool, Peer),

    RequirePieceLength(futures::channel::oneshot::Sender<usize>),
    FileFinish(usize),

    Download(Message),
    RequireData(RequestMetadata, futures::channel::oneshot::Sender<Vec<u8>>),
    RequireIncompleteBlocks(u32, futures::channel::oneshot::Sender<Vec<(u32, u32)>>),
    RequireHavePieces(futures::channel::oneshot::Sender<Vec<bool>>),

    Tracker(TrackerMessage),
}
