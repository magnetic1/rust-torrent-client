use crate::bencode::hash::Sha1;

use async_std::fs::{File, OpenOptions};
use async_std::io;
use async_std::task;

use futures::channel::mpsc;
use futures::sink::SinkExt;
use async_std::io::prelude::*;
use async_std::sync::{Arc, Mutex, MutexGuard};
use crate::base::ipc::{Message, IPC};
use crate::base::meta_info::{TorrentMetaInfo, Info};
use crate::base::Result;
use crate::require_oneshot;
use async_std::path::Path;
use std::fs;
use futures::prelude::stream::FuturesUnordered;
use futures::{StreamExt, AsyncWriteExt};
use crate::base::manager::ManagerEvent;
use futures::channel::mpsc::{Sender, Receiver};
use crate::net::peer_connection::RequestMetadata;

// type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub const BLOCK_SIZE: u32 = 16 * 1024;


struct Block {
    index: u32,
    length: u32,
    is_complete: bool,
}

impl Block {
    fn new(index: u32, length: u32) -> Block {
        Block {
            index,
            length,
            is_complete: false,
        }
    }
}

pub(crate) struct Piece {
    pub(crate) length: u32,
    offset: u64,
    hash: Sha1,
    blocks: Vec<Block>,
    is_complete: bool,
}

impl Piece {
    fn new(length: u32, offset: u64, hash: Sha1) -> Piece {
        // create blocks
        let num_blocks = (length as f64 / BLOCK_SIZE as f64).ceil() as usize;
        let mut blocks = Vec::with_capacity(num_blocks);

        for i in 0..num_blocks {
            let len = if i < (num_blocks - 1) {
                BLOCK_SIZE
            } else {
                length - (BLOCK_SIZE * (num_blocks - 1) as u32)
            };
            blocks.push(Block::new(i as u32, len));
        }

        Piece { length, offset, hash, blocks, is_complete: false }
    }

    fn has_block(&self, block_index: u32) -> bool {
        self.blocks[block_index as usize].is_complete
    }

    fn has_all_blocks(&self) -> bool {
        self.blocks.iter().all(|b| {
            b.is_complete
        })
    }

    fn reset_blocks(&mut self) {
        for block in self.blocks.iter_mut() {
            block.is_complete = false;
        }
    }
}

pub struct Download {
    pub our_peer_id: String,
    pub meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    files: Arc<Vec<Arc<Mutex<File>>>>,
    file_offsets: Vec<u64>,
    // file_paths: Vec<String>,
    manager_sender: Sender<ManagerEvent>,
}

impl Download {
    pub(crate) async fn store(&mut self, piece_index: u32, block_index: u32, data: Vec<u8>) -> Result<()> {
        let piece = &mut self.pieces[piece_index as usize];

        if piece.has_block(block_index) || piece.is_complete {
            // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
            return Ok(());
        }
        // println!("file_offsets: {} {} {}", self.file_offsets[0], self.file_offsets[1] ,self.file_offsets[2]);
        // println!("{}: {}", piece_index, block_index);
        store_block(&**self.files, &self.file_offsets, piece.offset, block_index, &data).await?;

        piece.blocks[block_index as usize].is_complete = true;

        if piece.has_all_blocks() {
            let valid = verify(piece, &**self.files, &self.file_offsets).await?;
            println!("verify {} {} {}", piece_index, piece.length, valid);
            if !valid {
                piece.reset_blocks();
            } else {
                piece.is_complete = true;
                // drop(piece);
                // verify files. rename it if finished.
                self.verify_file(piece_index).await?;
            }
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index)).await?;
        // println!("block {} complete", block_index);
        // notify peers if piece is complete
        if self.pieces[piece_index as usize].is_complete {
            println!("Piece {} complete", piece_index);
            self.broadcast(IPC::PieceComplete(piece_index)).await;
        }
        // notify peers if download is complete
        if self.is_complete() {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete).await;
        }

        Ok(())
    }

    pub(crate) async fn verify_file(&mut self, piece_index: u32) -> Result<()> {
        let piece = &self.pieces[piece_index as usize];

        let (index, v) = search_ptrs(&self.file_offsets, piece.offset, piece.length as usize);

        let piece_len = require_oneshot!(self.manager_sender, ManagerEvent::RequirePieceLength);

        for i in 0..v.len() {
            let low = self.file_offsets[i + index] as usize / piece_len;
            let high = (self.file_offsets[i + index + 1] - 1) as usize / piece_len;

            // let mut file_is_complete = true;
            let contained_pieces = &self.pieces[low..=high];
            println!("low{} high{} {} {}", low, high, self.pieces[low].is_complete, self.pieces[high].is_complete);
            let file_is_complete = contained_pieces.iter().all(|p| {
                p.is_complete
            });

            if file_is_complete {
                println!("{}: file_is_complete", i + index);
                self.manager_sender.send(ManagerEvent::FileFinish(i + index)).await?;
            }
            // if name.ends_with(".temp") {
            //     let low = self.file_offsets[i + index] as usize / piece_len;
            //     let high = (self.file_offsets[i + index + 1] - 1) as usize / piece_len;
            //
            //     // let mut file_is_complete = true;
            //     let contained_pieces = &self.pieces[low..=high];
            //     let file_is_complete = contained_pieces.iter().all(|p| {
            //         p.is_complete
            //     });
            //
            //     if file_is_complete {
            //         let new_name = &name[..name.len() - 5];
            //         async_std::fs::rename(name, new_name).await?;
            //
            //         let new_file = OpenOptions::new().create(true).read(true).write(true).open(new_name).await?;
            //         self.files[i + index] = Arc::new(Mutex::new(new_file));
            //         self.file_paths[i + index] = String::from(new_name);
            //     }
            // }
        }
        Ok(())
    }

    pub(crate) async fn retrieve_data(&self, request: RequestMetadata) -> Result<Vec<u8>> {
        let ref piece = self.pieces[request.piece_index as usize];

        if piece.is_complete {
            let offset = piece.offset + request.offset as u64;
            let buf = read(&**self.files, &self.file_offsets, offset, request.block_length).await?;
            Ok(buf)
        } else {
            Err("Error::MissingPieceData")?
        }
    }

    fn incomplete_blocks_for_piece(&self, piece_index: u32) -> Vec<(u32, u32)> {
        let ref piece = self.pieces[piece_index as usize];
        if !piece.is_complete {
            piece.blocks.iter()
                .filter(|&b| !b.is_complete)
                .map(|b| (b.index, b.length))
                .collect()
        } else {
            vec![]
        }
    }

    fn is_complete(&self) -> bool {
        self.pieces.iter().all(|piece| {
            piece.is_complete
        })
    }

    async fn broadcast(&mut self, ipc: IPC) -> Result<()> {
        self.manager_sender.send(ManagerEvent::Broadcast(ipc)).await?;
        Ok(())
    }

    pub fn have_pieces(&self) -> Vec<bool> {
        let mut res = Vec::with_capacity(self.pieces.len());
        for p in &self.pieces {
            res.push(p.is_complete);
        }
        res
    }
}

pub async fn download_loop(mut rx: Receiver<ManagerEvent>, mut manager_sender: Sender<ManagerEvent>,
                           files: Arc<Vec<Arc<Mutex<File>>>>, file_offsets: Vec<u64>,
                           our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<()> {
    // let file_infos = download_inline::create_file_infos(&meta_info.info).await;

    // let (file_offsets, file_paths, files)
    //     = download_inline::create_files(file_infos).await?;

    let len = file_offsets[file_offsets.len() - 1];
    let pieces = download_inline::create_pieces(len, &meta_info).await;

    let mut download = Download {
        our_peer_id,
        meta_info,
        pieces,
        files,
        file_offsets,
        manager_sender,
    };

    while let Some(event) = rx.next().await {
        match event {
            ManagerEvent::Download(Message::Piece(piece_index, offset, data)) => {
                let block_index = offset / BLOCK_SIZE;
                download.store(piece_index, block_index, data).await?;
                // println!("{:?} {:?}: store block {}", task::current().id(), piece_index, block_index);
            }
            ManagerEvent::RequireData(request_data, sender) => {
                let buf = download.retrieve_data(request_data).await?;
                sender.send(buf).unwrap();
            }
            ManagerEvent::RequireIncompleteBlocks(piece_index, sender) => {
                let res = download.incomplete_blocks_for_piece(piece_index);
                sender.send(res).unwrap();
            }
            ManagerEvent::RequireHavePieces(sender) => {
                let have_pieces = download.have_pieces();
                sender.send(have_pieces).unwrap();
            }
            _ => {}
        }
    };

    Ok(())
}

async fn verify(piece: &mut Piece, files: &[Arc<Mutex<File>>], file_offsets: &[u64]) -> Result<bool> {
    let buffer = read(files, file_offsets, piece.offset, piece.length).await?;

    // calculate the hash, verify it, and update is_complete
    piece.is_complete = piece.hash == Sha1::calculate_sha1(&buffer);

    if !piece.is_complete {
        println!("{:?}", Sha1::calculate_sha1(&buffer));
    }

    Ok(piece.is_complete)
}

pub async fn store_block(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<()> {
    let block_offset = piece_offset + (block_index * BLOCK_SIZE) as u64;

    // println!("search_ptrs {} {}", block_offset, data.len());
    let (i, ptr_vec) =
        search_ptrs(file_offsets, block_offset, data.len());
    // println!("finish search_ptrs {} {}", block_offset, data.len());
    if piece_offset == 216530944 {
        println!("{:?}", ptr_vec);
    }

    for (a, (block_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        // let mut file = files[i + a].clone();
        let mut file = files[i + a].lock().await;
        store(&mut file, file_ptr, block_ptr, len, data).await?;
    }

    Ok(())
}

async fn store(file: &mut File, file_ptr: u64,
               block_ptr: usize, len: usize,
               data: &[u8]) -> Result<()> {

    // async_std::io::seek::SeekExt::seek(&mut file, io::SeekFrom::Start(file_ptr)).await;
    // let s = async_std::io::SeekFrom::Start(file_ptr);
    file.seek(io::SeekFrom::Start(file_ptr)).await?;
    file.write_all(&data[block_ptr.. (block_ptr + len)]).await?;
    file.write()
    // println!("store success! {} {}", file_ptr, len);
    if file_ptr == 1595 {
        println!("block_ptr {} block_ptr + len {}=======================", block_ptr, (block_ptr + len));
        println!("{:?}", &data[14789..(14789 + 1595)]);
    }
    Ok(())
}

async fn read(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
              offset: u64, len: u32) -> Result<Vec<u8>> {
    let (i, ptr_vec) =
        search_ptrs(file_offsets, offset, len as usize);

    let mut buffer: Vec<u8> = vec![0; len as usize];

    for (a, (piece_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let file = files[i + a].clone();
        let mut file = file.lock().await;
        // read in the part of the file corresponding to the piece
        file.seek(io::SeekFrom::Start(file_ptr as u64)).await?;
        // let mut handle = file.take(len);
        println!("read file_ptr:{} piece_ptr:{} len:{}", file_ptr, piece_ptr, len);
        file.read(&mut buffer[piece_ptr..piece_ptr + len]).await?;
    }
    Ok(buffer)
}

fn search_index(file_offsets: &[u64], offset: u64) -> usize {
    let len = file_offsets.len();
    // println!("search_index: {}", offset);
    let (mut left, mut right) = (0usize, len - 1);

    while left <= right {
        let mid = (left + right) / 2;
        if offset == file_offsets[mid] {
            return mid;
        } else if offset < file_offsets[mid] {
            right = mid - 1;
        } else {
            left = mid + 1;
        }
    };

    return right;
}

fn search_ptrs(file_offsets: &[u64], offset: u64, len: usize) -> (usize, Vec<(usize, u64, usize)>) {
    let index = search_index(file_offsets, offset);
    // if len == 262144 {
    //     println!("{:?}", file_offsets);
    //     println!("search_index offset: {} len: {}", offset, len);
    //     println!("search_index index: {}", index);
    // }

    let mut vec = Vec::new();
    let mut i = index;
    let mut data_ptr = 0usize;
    let mut left = len as u64;
    let mut file_ptr = offset - file_offsets[i];

    while left + file_offsets[i] + file_ptr > file_offsets[i + 1] {
        let l = file_offsets[i + 1] - file_offsets[i] - file_ptr;
        vec.push((data_ptr, file_ptr, l as usize));

        data_ptr = data_ptr + l as usize;
        left = left - l;
        file_ptr = 0;
        i = i + 1;
        // if i == file_offsets.len() - 1 {
        //     return (index, vec);
        // };
    };

    vec.push((data_ptr, file_ptr, left as usize));
    return (index, vec);
}


pub mod download_inline {
    use async_std::sync::{Arc, Mutex};
    use async_std::fs::{File, OpenOptions};
    use std::fs;
    use async_std::path::Path;
    use crate::base::download::Piece;
    use crate::base::meta_info::{TorrentMetaInfo, Info};

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

    pub(crate) async fn create_file_infos(meta_info: &Info) -> Vec<(u64, String)> {
        let mut file_infos = Vec::new();
        let mut file_path = String::from("downloads/");
        match &meta_info {
            Info::Single(s) => {
                file_path.push_str(&s.name);
                file_path.push_str(".temp");

                file_infos.push((s.length, file_path));
            }
            Info::Multi(m) => {
                for f in &m.files {
                    let mut file_path = file_path.clone();
                    file_path.push_str(&f.path.join("/"));
                    file_path.push_str(".temp");

                    file_infos.push((f.length, file_path));
                }
            }
        };
        file_infos
    }

    pub(crate) async fn create_files(file_infos: Vec<(u64, String)>) -> Result<(Vec<u64>, Vec<String>, Vec<Arc<Mutex<File>>>)> {
        // create files and file_offsets
        let mut files = Vec::with_capacity(file_infos.len());
        let mut file_offsets = Vec::with_capacity(file_infos.len() + 1);
        let mut file_paths = Vec::with_capacity(file_infos.len());

        let mut file_offset = 0;
        for (length, file_path) in file_infos {
            let path = Path::new(&file_path);
            // create dirs
            let t: Vec<&str> = file_path.split("/").collect();
            if t.len() >= 2 {
                let s = t[..t.len() - 1].join("/");
                // match fs::metadata(s.clone()) {
                match fs::metadata(s.clone()) {
                    Ok(_) => {}
                    Err(_) => {
                        println!("create dir: {}", s);
                        fs::create_dir_all(s).unwrap()
                    }
                }
            }

            let mut file = OpenOptions::new().create(true).read(true).write(true).open(path).await?;
            file.set_len(length).await?;

            files.push(Arc::new(Mutex::new(file)));
            file_offsets.push(file_offset);
            file_paths.push(file_path);

            file_offset = file_offset + length;
        }
        file_offsets.push(file_offset);
        Ok((file_offsets, file_paths, files))
    }

    pub(crate) async fn create_pieces(files_length: u64, meta_info: &TorrentMetaInfo) -> Vec<Piece> {
        // create pieces
        let piece_length = meta_info.piece_length();
        let num_pieces = meta_info.num_pieces();

        let mut pieces = Vec::with_capacity(num_pieces);
        let shas = meta_info.pieces();
        for i in 0..num_pieces {
            let offset = i as u64 * piece_length as u64;
            let length = if i < (num_pieces - 1) {
                piece_length
            } else {
                assert!(files_length > offset);
                assert!(piece_length >= (files_length - offset) as usize);
                (files_length - offset) as usize
            };
            let mut piece = Piece::new(length as u32, offset, shas[i as usize].clone());

            // piece.verify(&files, &file_offsets)?;
            pieces.push(piece);
        }
        pieces
    }
}

#[cfg(test)]
mod test {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::io::{Seek, Read};
    use crate::bencode::hash::Sha1;

    #[test]
    fn file_test() {
        let my_path_1 = r#"D:\MyDocuments\rust\rustorrent\downloads\[Sakurato.sub] Yahari Ore no Seishun Love Come wa Machigatteiru Kan [03][HEVC-10Bit][1080P].mkv.temp"#;
        let my_path_2 = r#"D:\MyDocuments\rust\rustorrent\downloads\桜都字幕组招募中！.png.temp"#;
        let mut my_file_1 = OpenOptions::new().read(true).open(my_path_1).unwrap();
        let mut my_file_2 = OpenOptions::new().read(true).open(my_path_2).unwrap();

        let mut my_bytes = vec![0u8; 262_144];
        my_file_1.seek(io::SeekFrom::Start(216_530_944));
        my_file_1.read(&mut my_bytes[0..244_165]).unwrap();
        my_file_2.seek(io::SeekFrom::Start(0));
        my_file_2.read(&mut my_bytes[244_165..262_144]).unwrap();

        let path_1 = r#"D:\MyVideo\电影\动漫\[Sakurato.sub] Yahari Ore no Seishun Love Come wa Machigatteiru Kan [03][HEVC-10Bit][1080P]\[Sakurato.sub] Yahari Ore no Seishun Love Come wa Machigatteiru Kan [03][HEVC-10Bit][1080P].mkv"#;
        let path_2 = r#"D:\MyVideo\电影\动漫\[Sakurato.sub] Yahari Ore no Seishun Love Come wa Machigatteiru Kan [03][HEVC-10Bit][1080P]\桜都字幕组招募中！.png"#;
        let mut file_1 = OpenOptions::new().read(true).open(path_1).unwrap();
        let mut file_2 = OpenOptions::new().read(true).open(path_2).unwrap();
        let mut bytes = vec![0u8; 262_144];
        file_1.seek(io::SeekFrom::Start(216_530_944));
        file_1.read(&mut bytes[0..244_165]).unwrap();
        file_2.seek(io::SeekFrom::Start(0));
        file_2.read(&mut bytes[244_165..262_144]).unwrap();


        println!("{:?}", Sha1::calculate_sha1(&bytes));
        println!("{:?}", Sha1::calculate_sha1(&my_bytes));
        let mut indexs = vec![];
        for (i, &byte) in bytes.iter().enumerate() {
            // index = i;
            let my_byte = my_bytes[i];
            if byte != my_byte {
                indexs.push(i);
            }
        }
        println!("{:?}", indexs);
        println!("{:?}", &my_bytes[260_549..]);
        println!("{:?}", &bytes[260_549..]);
        // assert_eq!(&bytes, &my_bytes);
        // assert_eq!(Sha1::calculate_sha1(&bytes), Sha1::calculate_sha1(&my_bytes));

    }
}





