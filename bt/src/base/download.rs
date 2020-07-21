use crate::bencode::hash::Sha1;

use async_std::fs::{File, OpenOptions};
use async_std::io;

use futures::channel::mpsc;
use futures::sink::SinkExt;
use async_std::io::prelude::*;
use async_std::sync::{Arc, Mutex};
use crate::base::ipc::Message;
use crate::base::meta_info::{TorrentMetaInfo, Info};
use async_std::path::Path;
use std::fs;
use futures::prelude::stream::FuturesUnordered;
use futures::StreamExt;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;

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
    length: u32,
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

pub async fn download_loop(rx: Receiver<Message>, our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<()> {
    let file_infos = download_inline::create_file_infos(&meta_info.info).await;
    let piece_len = meta_info.piece_length();
    let (file_offsets, file_paths, files)
        = download_inline::create_files(file_infos).await?;

    let len = file_offsets[file_offsets.len() - 1];
    let pieces = download_inline::create_pieces(len, &meta_info).await;


    // let mut store_futs = FuturesUnordered::new();
    let mut messages = rx.fuse();

    Ok(())
}

async fn verify(piece: &mut Piece, files: &mut [Arc<Mutex<File>>], file_offsets: &[u64]) -> Result<bool> {
    let buffer = read(files, file_offsets, piece.offset, piece.length).await?;

    // calculate the hash, verify it, and update is_complete
    piece.is_complete = piece.hash == Sha1::calculate_sha1(&buffer);
    Ok(piece.is_complete)
}

pub async fn store_block(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<()> {
    let block_offset = piece_offset + (block_index * BLOCK_SIZE) as u64;

    let (i, ptr_vec) =
        search_ptrs(file_offsets, block_offset, data.len());

    for (a, (block_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let mut file = files[i + a].clone();
        let mut file = file.lock().await;
        store(&mut file, file_ptr, block_ptr, len as u64, data).await?;
    }

    Ok(())
}

async fn store(file: &mut File, file_ptr: u64,
               block_ptr: usize, len: u64,
               data: &[u8]) -> Result<()> {
    // async_std::io::seek::SeekExt::seek(&mut file, io::SeekFrom::Start(file_ptr)).await;
    file.seek(io::SeekFrom::Start(file_ptr)).await?;
    file.write(&data[block_ptr..block_ptr + len as usize]).await?;

    Ok(())
}

async fn read(files: &mut [Arc<Mutex<File>>], file_offsets: &[u64],
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
    println!("search_index: {}", offset);
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
    };

    vec.push((data_ptr, file_ptr, left as usize));
    return (index, vec);
}


mod download_inline {
    use async_std::sync::{Arc, Mutex};
    use async_std::fs::{File, OpenOptions};
    use std::fs;
    use async_std::path::Path;
    use crate::base::download::{Piece, store_block, search_ptrs};
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
            // file.set_len(length).await;

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
        let offset = (num_pieces - 1) as u64 * piece_length as u64;
        let shas = meta_info.pieces();
        for i in 0..num_pieces {
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

    // pub(crate) async fn store(pieces: &mut [Piece], files: &[Arc<Mutex<File>>],
    //                    file_offsets: &[u64], piece_index: u32,
    //                    block_index: u32, data: Vec<u8>) -> Result<()> {
    //     let piece = &mut pieces[piece_index as usize];
    //
    //     if piece.has_block(block_index) || piece.is_complete {
    //         // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
    //         return Ok(());
    //     }
    //
    //     store_block(files, file_offsets, piece.offset, block_index, &data).await?;
    //
    //
    //     let mut piece = piece;
    //     piece.blocks[block_index as usize].is_complete = true;
    //
    //     if piece.has_all_blocks() {
    //         let valid =  piece.verify(&self.files, &self.file_offsets)?;
    //         if !valid {
    //             piece.reset_blocks();
    //         } else {
    //             // verify files. rename it if finished.
    //             let (index, v)
    //                 = search_ptrs(&self.file_offsets, piece.offset, piece.length as usize);
    //             let piece_len = self.meta_info.piece_length();
    //
    //             for i in 0..v.len() {
    //                 let name = &self.file_paths[i + index];
    //
    //                 if name.ends_with(".temp") {
    //                     let low = file_offsets[i + index] as usize / piece_len;
    //                     let high = (file_offsets[i + index + 1] - 1) as usize / piece_len;
    //
    //                     let mut file_is_complete = true;
    //                     for p in low..=high {
    //                         // let b = *self.pieces[p].is_complete.clone().lock().await;
    //                         // if !b {
    //                         //     file_is_complete = false;
    //                         //     break;
    //                         // }
    //
    //                         let b = match self.pieces[p].is_complete.try_lock() {
    //                             Some(s) => {*s},
    //                             None => { false },
    //                         };
    //
    //                         if !b {
    //                             file_is_complete = false;
    //                             break;
    //                         }
    //                     };
    //
    //                     if file_is_complete {
    //                         async_std::fs::rename(name, &name[..name.len() - 5]).await;
    //                     }
    //                 }
    //             }
    //         }
    //     }
    //
    //     // notify peers that this block is complete
    //     self.broadcast(IPC::BlockComplete(piece_index, block_index)).await;
    //
    //     // notify peers if piece is complete
    //     if *piece.is_complete.clone().lock().await {
    //         self.broadcast(IPC::PieceComplete(piece_index)).await;
    //     }
    //
    //     // notify peers if download is complete
    //     if self.is_complete() {
    //         println!("Download complete");
    //         self.broadcast(IPC::DownloadComplete).await;
    //     }
    //
    //     Ok(())
    // }
}


