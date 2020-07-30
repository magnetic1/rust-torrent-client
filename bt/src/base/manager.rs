use async_std::{
    sync::{Arc, Mutex},
    fs::{File, OpenOptions}
};
use futures::{
    StreamExt,
    SinkExt,
    channel::mpsc,
    channel::mpsc::{Receiver, Sender},
};
use crate::{
    net::peer_connection::{RequestMetadata, Peer, peer_conn_loop},
    base::ipc::{Message, IPC},
    base::download::{Piece, download_loop, download_inline},
    base::meta_info::TorrentMetaInfo,
    base::spawn_and_log_error
};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Manager {
    our_peer_id: String,
    meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    files: Arc<Vec<Arc<Mutex<File>>>>,
    file_offsets: Vec<u64>,
    file_paths: Vec<String>,
}

pub async fn manager_loop(our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<()> {
    let (mut sender, mut client_receiver) = mpsc::channel(10);
    let (mut client_sender, mut events) = mpsc::channel(10);
    let mut peers: HashMap<Peer, Sender<IPC>> = HashMap::new();

    let file_infos = download_inline::create_file_infos(&meta_info.info).await;
    println!("create_file_infos finished");

    let (file_offsets, file_paths, files)
        = download_inline::create_files(file_infos).await?;
    println!("create_files finished");

    let len = file_offsets[file_offsets.len() - 1];
    let pieces = download_inline::create_pieces(len, &meta_info).await;
    println!("create_pieces finished");
    println!("file_offsets: {:?} len: {}", file_offsets, len);

    let ps: Vec<u32> = pieces.iter().map(|p| p.length).collect();
    println!("pieces len: {:?}", ps);

    let mut manager = Manager {
        our_peer_id: our_peer_id.clone(),
        meta_info,
        pieces,
        files: Arc::new(files),
        file_offsets,
        file_paths,
    };

    let _down_handle = spawn_and_log_error(
        download_loop(client_receiver, client_sender.clone(),
                      Arc::clone(&manager.files), manager.file_offsets.clone(),
                      manager.our_peer_id.clone(), manager.meta_info.clone())
    );

    {
        let mut ps = Vec::new();
        // ps.push(Peer {
        //     ip: "42.98.69.212".to_string(),
        //     port: 10379
        // });
        // ps.push(Peer {
        //     ip: "51.158.148.85".to_string(),
        //     port: 58579
        // });
        ps.push(Peer {
            ip: "127.0.0.1".to_string(),
            port: 57463
        });
        // ps.push(Peer {
        //     ip: "205.185.122.158".to_string(),
        //     port: 54794
        // });
        for p in ps {
            client_sender.send(ManagerEvent::Connection(true, p)).await?;
        }
    }


    while let Some(event) = events.next().await {
        match event {
            ManagerEvent::Broadcast(ipc) => {
                peers.retain(|peer, sender| {
                    async_std::task::block_on(async {
                        match sender.send(ipc.clone()).await {
                            Ok(_) => true,
                            Err(_) => false,
                        }
                    })
                });
            }
            ManagerEvent::Connection(send_handshake_first, peer) => {
                if peers.get(&peer).is_none() {
                    let (peer_sender, peer_receiver) = mpsc::channel(10);
                    spawn_and_log_error(
                        peer_conn_loop(send_handshake_first, our_peer_id.clone(),
                                       manager.meta_info.info_hash(), peer.clone(), peer_sender.clone(),
                                       peer_receiver, client_sender.clone())
                    );
                    assert!(peers.insert(peer, peer_sender.clone()).is_none());
                }
            }

            ManagerEvent::RequirePieceLength(mut sender) => {
                sender.send(manager.meta_info.piece_length()).unwrap();
            }
            ManagerEvent::FileFinish(file_index) => {
                let name = &manager.file_paths[file_index];
                if name.ends_with(".temp") {
                    let new_name = &name[..name.len() - 5];
                    async_std::fs::rename(name, new_name).await?;
                    let new_file = OpenOptions::new().create(true).read(true).write(true).open(new_name).await?;
                    let mut file = manager.files[file_index].lock().await;
                    *file = new_file;
                     // = Arc::new(Mutex::new(new_file));
                    manager.file_paths[file_index] = String::from(new_name);
                }
            }

            e => sender.send(e).await?,

            // ManagerEvent::RequireData(_, _) => {}
            // ManagerEvent::RequireIncompleteBlocks(_, _) => {}
            // ManagerEvent::Download(message) => {
            // }
            // ManagerEvent::RequireHavePieces(_) => {}
        }
    }

    Ok(())
}

pub enum ManagerEvent {
    Broadcast(IPC),
    Connection(bool, Peer),

    RequirePieceLength(futures::channel::oneshot::Sender<usize>),
    FileFinish(usize),

    Download(Message),
    RequireData(RequestMetadata, futures::channel::oneshot::Sender<Vec<u8>>),
    RequireIncompleteBlocks(u32, futures::channel::oneshot::Sender<Vec<(u32, u32)>>),
    RequireHavePieces(futures::channel::oneshot::Sender<Vec<bool>>),
}