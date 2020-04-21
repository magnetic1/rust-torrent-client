use crate::base::meta_info::{TorrentMetaInfo, Info};
use crate::net::ipc::IPC;
use crate::bencode::hash::Sha1;
use crate::net::request_metadata::RequestMetadata;
use std::{convert, io, fs};
use async_std::sync::{Mutex, Arc, MutexGuard};
use async_std::fs::{File, OpenOptions};
use async_std::task;
use async_std::prelude::*;
use futures::stream::FuturesUnordered;
use futures::future::FutureExt;
use futures::StreamExt;
use async_std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, SendError};


pub const BLOCK_SIZE: u32 = 16 * 1024;


pub struct Download {
    pub our_peer_id: String,
    pub meta_info: TorrentMetaInfo,
    pieces: Vec<Piece>,
    files: Vec<Arc<Mutex<File>>>,
    file_offsets: Vec<u64>,
    file_paths: Vec<String>,
    peer_channels: Arc<Mutex<Vec<Sender<IPC>>>>,
}

struct Piece {
    length: u32,
    offset: u64,
    hash: Sha1,
    blocks: Vec<Arc<Mutex<Block>>>,
    is_complete: Arc<Mutex<bool>>,
}

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
            blocks.push(Arc::new(Mutex::new(Block::new(i as u32, len))));
        }

        Piece {
            length,
            offset,
            hash,
            blocks,
            is_complete: Arc::new(Mutex::new(false)),
        }
    }

    fn verify(&self, files: &[Arc<Mutex<File>>], file_offsets: &[u64]) -> Result<bool, Error> {
        task::block_on(async move {
            let buffer = read(files, file_offsets, self.offset, self.length).await?;

            // calculate the hash, verify it, and update is_complete
            let s = self.is_complete.clone();
            let mut is_complete = s.lock().await;
            *is_complete = self.hash == Sha1::calculate_sha1(&buffer);
            Ok(*is_complete)
        })
    }

    fn has_block(&self, block_index: u32) -> bool {
        task::block_on(async {
            // todo: is work?
            self.blocks[block_index as usize].lock().await.is_complete
        })
    }

    fn has_all_blocks(&self) -> bool {
        let mut futures = FuturesUnordered::new();

        for block in self.blocks.iter() {
            let b = block.clone();
            let future = async move {
                b.lock().await.is_complete
            };
            futures.push(future.fuse());
        }

        return task::block_on(async move {
            loop {
                match futures.next().await {
                    Some(b) => {
                        if !b {
                            return false;
                        }
                    }
                    None => {
                        return true;
                    }
                }
            };
        });
    }

    fn reset_blocks(&self) {
        task::block_on(async {
            for block in self.blocks.iter() {
                block.clone().lock().await.is_complete = false;
            }
        })
    }
}

impl Download {
    pub fn new(our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<Download, Error> {
        let mut file_infos = Vec::new();
        {
            match &meta_info.info {
                Info::Single(s) => {
                    let mut file_path = String::from("downloads/");
                    file_path.push_str(&s.name);
                    file_path.push_str(".temp");

                    file_infos.push((s.length, file_path));
                }
                Info::Multi(m) => {
                    for f in &m.files {
                        let mut file_path = String::from("downloads/");
                        file_path.push_str(&f.path.join("/"));
                        file_path.push_str(".temp");

                        file_infos.push((f.length, file_path));
                    }
                }
            };
        }
        println!("{}", file_infos.len());

        // create files and file_offsets
        let mut files = Vec::with_capacity(file_infos.len());
        let mut file_offsets = Vec::with_capacity(file_infos.len() + 1);
        let mut file_paths = Vec::with_capacity(file_infos.len());
        let mut file_offset = 0;

        task::block_on(async move {
            for (length, file_path) in file_infos {
                let path = Path::new(&file_path);
                // create dirs
                let t: Vec<&str>= file_path.split("/").collect();
                if t.len() > 1 {
                    let s = t[..t.len()-1].join("/");
                    match fs::metadata(s.clone()) {
                        Ok(_) => {},
                        Err(_) => {
                            println!("{}", s);
                            fs::create_dir_all(s).unwrap()
                        },
                    }
                }


                let mut file = OpenOptions::new().create(true).read(true).write(true).open(path).await?;

                files.push(Arc::new(Mutex::new(file)));
                file_offsets.push(file_offset);
                file_paths.push(file_path);

                file_offset = file_offset + length;
            }
            file_offsets.push(file_offset);

            let file_length = file_offset;

            println!("file_offsets {}", file_offsets.len());
            println!("file_offsets: {:?}", file_offsets);
            // create pieces
            let piece_length = meta_info.piece_length();
            let num_pieces = meta_info.num_pieces();

            let mut pieces = Vec::with_capacity(num_pieces);

            for i in 0..num_pieces {
                let offset = i as u64 * piece_length as u64;
                let length = if i < (num_pieces - 1) {
                    piece_length
                } else {
                    (file_length - offset) as usize
                };
                let mut piece = Piece::new(length as u32, offset, meta_info.pieces()[i as usize].clone());

                // piece.verify(&files, &file_offsets)?;
                pieces.push(piece);
            }



            Ok(Download {
                our_peer_id,
                meta_info,
                pieces,
                files,
                file_paths,
                peer_channels: Arc::new(Mutex::new(Vec::new())),
                file_offsets,
            })
        })

    }

    pub async fn store(&self, piece_index: u32, block_index: u32, data: Vec<u8>) -> Result<(), Error> {
        let piece = &self.pieces[piece_index as usize];

        if piece.has_block(block_index) || *piece.is_complete.clone().lock().await {
            // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
            return Ok(());
        }

        store_block(&self.files, &self.file_offsets, piece.offset, block_index, &data).await?;


        let mut piece = piece;
        piece.blocks[block_index as usize].clone().lock().await.is_complete = true;

        if piece.has_all_blocks() {
            let valid = piece.verify(&self.files, &self.file_offsets)?;
            if !valid {
                piece.reset_blocks();
            } else {
                // verify files. rename it if finished.
                let (index, v)
                    = search_ptrs(&self.file_offsets, piece.offset, piece.length as usize);
                let piece_len = self.meta_info.piece_length();

                for i in 0..v.len() {
                    let name = &self.file_paths[i + index];

                    if name.ends_with(".temp") {
                        let low = self.file_offsets[i + index] as usize / piece_len;
                        let high = (self.file_offsets[i + index + 1] - 1) as usize / piece_len;

                        let mut file_is_complete = true;
                        for p in low..=high {
                            // let b = *self.pieces[p].is_complete.clone().lock().await;
                            // if !b {
                            //     file_is_complete = false;
                            //     break;
                            // }

                            let b = match self.pieces[p].is_complete.try_lock() {
                                Some(s) => {*s},
                                None => { false },
                            };

                            if !b {
                                file_is_complete = false;
                                break;
                            }
                        };

                        if file_is_complete {
                            async_std::fs::rename(name, &name[..name.len() - 5]).await;
                        }
                    }
                }
            }
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index)).await;

        // notify peers if piece is complete
        if *piece.is_complete.clone().lock().await {
            self.broadcast(IPC::PieceComplete(piece_index)).await;
        }

        // notify peers if download is complete
        if self.is_complete() {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete).await;
        }

        Ok(())
    }

    pub async fn register_peer(&self, channel: Sender<IPC>) {
        self.peer_channels.clone().lock().await.push(channel);
    }

    pub async fn retrieve_data(&self, request: &RequestMetadata) -> Result<Vec<u8>, Error> {
        let ref piece = self.pieces[request.piece_index as usize];
        let p = piece.is_complete.clone();

        if *p.lock().await {
            let offset = piece.offset + request.offset as u64;
            let buf =
                read(&self.files, &self.file_offsets, offset, request.block_length).await?;
            Ok(buf)
        } else {
            Err(Error::MissingPieceData)
        }
    }

    pub async fn have_pieces(&self) -> Vec<bool> {
        let mut res = Vec::with_capacity(self.pieces.len());

        for p in &self.pieces {
            res.push(*p.is_complete.clone().lock().await);
        }
        res
    }

    pub async fn incomplete_blocks_for_piece(&self, piece_index: u32) -> Vec<(u32, u32)> {
        let ref piece = self.pieces[piece_index as usize];
        if { !*piece.is_complete.clone().lock().await } {
            piece.blocks.iter()
                .filter(|b| {
                    task::block_on(async {
                        !b.clone().lock().await.is_complete
                    })
                }).map(|b| {
                task::block_on(async {
                    let b = b.clone();
                    let b = b.lock().await;
                    (b.index, b.length)
                })
            }).collect()
        } else {
            vec![]
        }
    }

    fn is_complete(&self) -> bool {
        task::block_on(async {
            for piece in self.pieces.iter() {
                if !*piece.is_complete.clone().lock().await {
                    return false;
                }
            }
            true
        })
    }

    async fn broadcast(&self, ipc: IPC) {
        self.peer_channels.clone().lock().await.retain(|channel| {
            task::block_on(async {
                // async_std::sync::Sender
                match channel.send(ipc.clone()) {
                    Ok(_) => true,
                    Err(_) => false,
                }
            })
        });
    }
}

pub async fn store_block(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<(), Error> {
    let block_offset = piece_offset + (block_index * BLOCK_SIZE) as u64;

    let (i, ptr_vec) =
        search_ptrs(file_offsets, block_offset, data.len());

    for (a, (block_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let mut file = files[i + a].clone();
        let mut file = file.lock().await;
        store(file, file_ptr, block_ptr, len as u64, data).await?;
    }

    Ok(())
}

async fn store(mut file: MutexGuard<'_, File>,
               file_ptr: u64, block_ptr: usize, len: u64,
               data: &[u8]) -> Result<(), Error> {
    // async_std::io::seek::SeekExt::seek(&mut file, io::SeekFrom::Start(file_ptr)).await;
    file.seek(io::SeekFrom::Start(file_ptr)).await?;
    file.write(&data[block_ptr..block_ptr + len as usize]).await?;

    Ok(())
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


async fn read(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
              offset: u64, len: u32) -> Result<Vec<u8>, Error> {
    let (i, ptr_vec) =
        search_ptrs(file_offsets, offset, len as usize);

    let mut buffer: Vec<u8> = Vec::with_capacity(len as usize);

    for (a, (piece_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let file = files[i + a].clone();
        let mut file = file.lock().await;
        // read in the part of the file corresponding to the piece
        file.seek(io::SeekFrom::Start(file_ptr as u64)).await?;
        // let mut handle = file.take(len);
        file.read(&mut buffer[piece_ptr..piece_ptr + len]).await?;
    }
    Ok(buffer)
}


#[derive(Debug)]
pub enum Error {
    MissingPieceData,
    IoError(io::Error),
}

impl convert::From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

#[cfg(test)]
mod tests {
    use crate::net::download::search_ptrs;
    use async_std::task;
    use async_std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn search_ptrs_test() {
        let file_offsets = [0, 8, 16, 24];
        let a = search_ptrs(&file_offsets, 15, 2);

        assert_eq!(search_ptrs(&file_offsets, 15, 1), (1, vec![(0, 7, 1)]));
        assert_eq!(search_ptrs(&file_offsets, 15, 2), (1, vec![(0, 7, 1), (1, 0, 1)]));
        assert_eq!(search_ptrs(&file_offsets, 0, 8), (0, vec![(0, 0, 8)]));
        assert_eq!(search_ptrs(&file_offsets, 8, 8), (1, vec![(0, 0, 8)]));
        assert_eq!(search_ptrs(&file_offsets, 8, 9), (1, vec![(0, 0, 8), (8, 0, 1)]));
        assert_eq!(search_ptrs(&file_offsets, 8, 16), (1, vec![(0, 0, 8), (8, 0, 8)]));
    }

    #[test]
    fn lock_test() {
        let t = Arc::new(Mutex::new(true));
        let f = Arc::new(Mutex::new(false));
        let t1 = t.clone();
        let f1 = f.clone();

        let task1 = task::spawn(async move {
            if !*f1.lock().await {
                // let p = f1.lock().await;
                println!("task2 f locked");
                thread::sleep(Duration::from_millis(500));
                let s = t1.lock().await;
                println!("task2 t locked");
            }
        });

        let task2 = task::spawn(async move {
            let p = t.lock().await;
            println!("task1 t locked");

            thread::sleep(Duration::from_secs(1));
            let s = f.lock().await;
            println!("task1 f locked");
        });

        task::block_on(async move {
            futures::join!(task1, task2);
        });
    }
}
