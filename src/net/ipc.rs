use crate::net::peer_connection::Message;

#[derive(Clone, Debug)]
pub enum IPC {
    BlockComplete(u32, u32),
    PieceComplete(u32),
    DownloadComplete,
    Message(Message),
    BlockUploaded,
}
