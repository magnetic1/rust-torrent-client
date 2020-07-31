use std::fs::File;
use std::io::Read;
use bt::{
    base::meta_info::TorrentMetaInfo,
    base::manager::manager_loop
};
use rand::Rng;
use async_std::task;

pub const PEER_ID_PREFIX: &'static str = "-RC0001-";

fn main() {
    let filename = r#"C:\Users\12287\Downloads\03ffe2d471d52a832ea02f2de06c82a14a7cfbcb.torrent"#;
    let mut bytes = Vec::new();
    File::open(filename).unwrap().read_to_end(&mut bytes).unwrap();
    let metainfo = TorrentMetaInfo::parse(&bytes);

    let mut rng = rand::thread_rng();
    let rand_chars: String = rng.gen_ascii_chars().take(20 - PEER_ID_PREFIX.len()).collect();
    let peer_id = format!("{}{}", PEER_ID_PREFIX, rand_chars);

    println!("start manager_loop");
    task::block_on(async {
        manager_loop(peer_id, metainfo).await;
    })
}