use async_std::task;
use bt::{base::manager::manager_loop, base::meta_info::TorrentMetaInfo};
use rand::Rng;
use std::fs::File;
use std::io::Read;

pub const PEER_ID_PREFIX: &'static str = "-RC0001-";

fn main() {
    // let filename = r#"C:\Users\wzq\Downloads\03ffe2d471d52a832ea02f2de06c82a14a7cfbcb.torrent"#;
    let filename = r#"C:\Users\wzq\Downloads\【喵萌奶茶屋】★10月新番★[前说!_まえせつ！_Maesetsu! Opening Act][04][720p][简体][招募翻译].torrent"#;

    let mut bytes = Vec::new();
    File::open(filename)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    let metainfo = TorrentMetaInfo::parse(&bytes);
    // get_peer(&metainfo);

    let mut rng = rand::thread_rng();
    let rand_chars: String = rng
        .gen_ascii_chars()
        .take(20 - PEER_ID_PREFIX.len())
        .collect();
    let peer_id = format!("{}{}", PEER_ID_PREFIX, rand_chars);

    println!("start manager_loop");
    task::block_on(async { manager_loop(peer_id, metainfo).await }).unwrap();
}
