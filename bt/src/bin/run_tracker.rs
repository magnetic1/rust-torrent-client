use async_std::task;
use bt::{base::manager::manager_loop, base::meta_info::TorrentMetaInfo};
use rand::Rng;
use std::fs::File;
use std::io::Read;

pub const PEER_ID_PREFIX: &'static str = "-RC0001-";

fn main() {
    let filename = r#"torrent/ubuntu-22.10-desktop-amd64.iso.torrent"#;
    let mut bytes = Vec::new();
    File::open(filename).unwrap()
        .read_to_end(&mut bytes).unwrap();
    let meta_info = TorrentMetaInfo::parse(&bytes);

    let mut rng = rand::thread_rng();
    let rand_chars: String = rng
        .gen_ascii_chars()
        .take(20 - PEER_ID_PREFIX.len())
        .collect();
    let peer_id = format!("{}{}", PEER_ID_PREFIX, rand_chars);

    println!("start manager_loop");
    task::block_on(async {
        manager_loop(peer_id, meta_info).await
    }).unwrap();
}
