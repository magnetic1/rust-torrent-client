use std::fs::File;
use std::io::Read;
use bt::{
    base::meta_info::TorrentMetaInfo,
    base::manager::manager_loop,
};
use rand::Rng;
use async_std::task;
use bt::net::tracker::get_peer;

pub const PEER_ID_PREFIX: &'static str = "-RC0001-";

fn main() {
    // let filename = r#"C:\Users\wzq\Downloads\03ffe2d471d52a832ea02f2de06c82a14a7cfbcb.torrent"#;
    let filename = r#"C:\Users\wzq\Downloads\[K&W][Gundam Build Divers Re-RISE][PV-Just before resumed!-][BIG5][720P][x264_AAC].mp4.torrent"#;

    let mut bytes = Vec::new();
    File::open(filename).unwrap().read_to_end(&mut bytes).unwrap();
    let metainfo = TorrentMetaInfo::parse(&bytes);
    // get_peer(&metainfo);

    let mut rng = rand::thread_rng();
    let rand_chars: String = rng.gen_ascii_chars().take(20 - PEER_ID_PREFIX.len()).collect();
    let peer_id = format!("{}{}", PEER_ID_PREFIX, rand_chars);

    println!("start manager_loop");
    task::block_on(async {
        manager_loop(peer_id, metainfo).await;
    })
}