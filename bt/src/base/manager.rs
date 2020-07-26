use crate::base::meta_info::TorrentMetaInfo;
use crate::base::download::Piece;
use async_std::sync::{Arc, Mutex};
use async_std::fs::File;
use crate::base::ipc::Message;
use futures::channel::mpsc::{Receiver, Sender};
use futures::{StreamExt, SinkExt};
use futures::channel::mpsc;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub struct Manager {
    our_peer_id: String,
    meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    files: Vec<Arc<Mutex<File>>>,
    file_offsets: Vec<u64>,
    file_paths: Vec<String>,
}

pub async fn manager_loop(mut manager: Manager, mut rx: Receiver<Message>, our_peer_id: String) -> Result<()> {
    let (s, mut r): (Sender<ManagerEvent>, Receiver<ManagerEvent>) = mpsc::channel(10);

    while let Some(e) = r.next().await {
        match e {
            ManagerEvent::Download(Message) => {

            },
            ManagerEvent::RequirePieceLength(mut sender) => {
                sender.send(manager.meta_info.piece_length()).await?;
            }
        }
    }

    Ok(())
}
#[derive(Clone)]
pub enum ManagerEvent {
    Download(Message),
    RequirePieceLength(Sender<usize>),
}