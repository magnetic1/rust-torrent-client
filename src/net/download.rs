use crate::base::meta_info::{TorrentMetaInfo, Info};
use crate::bencode::hash::Sha1;
use std::{convert, io};
use async_std::sync::{Mutex, Arc, MutexGuard, Sender};
use async_std::fs::{File, OpenOptions};
use async_std::task;
use async_std::prelude::*;
use futures::stream::FuturesUnordered;
use futures::future::{FutureExt};
use futures::StreamExt;

use crate::net::ipc::IPC;
use async_std::path::Path;
use crate::net::request_metadata::RequestMetadata;


pub const BLOCK_SIZE: u32 = 16 * 1024;


pub struct Download {
    pub our_peer_id: String,
    pub meta_info:       TorrentMetaInfo,
    pieces:          Vec<Piece>,
    files:           Vec<Arc<Mutex<File>>>,
    file_offsets:    Vec<u64>,
    peer_channels:   Arc<Mutex<Vec<Sender<IPC>>>>,
}

struct Piece {
    length:      u32,
    offset:      u64,
    hash:        Sha1,
    blocks:      Vec<Arc<Mutex<Block>>>,
    is_complete: Arc<Mutex<bool>>,
}

struct Block {
    index:       u32,
    length:      u32,
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
            let piece_offset = self.offset;
            let piece_len = self.length as usize;
            let (i, ptr_vec) =
                search_ptrs(file_offsets, piece_offset, piece_len);

            let mut buffer = Vec::with_capacity(self.length as usize);
            // let mut consumed = 0;
            for (a, (piece_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
                let mut file = files[i + a].clone();
                let mut file = file.lock().await;
                // read in the part of the file corresponding to the piece
                file.seek(io::SeekFrom::Start(file_ptr as u64)).await?;
                // let mut handle = file.take(len);
                file.read(&mut buffer[piece_ptr..piece_ptr + len]).await?;
            }

            // calculate the hash, verify it, and update is_complete
            let s = self.is_complete.clone();
            let mut s = s.lock().await;
            *s = (self.hash == Sha1::calculate_sha1(&buffer));
            Ok(*s)
        })
    }

    fn has_block(&self, block_index: u32) -> bool {
        task::block_on(async {
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
                    },
                    None => {
                        return true;
                    },
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
    pub async fn store(&self, piece_index: u32, block_index: u32, data: Vec<u8>) -> Result<(), Error> {

        let piece = &self.pieces[piece_index as usize];

        if  piece.has_block(block_index) || *piece.is_complete.clone().lock().await {
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
            }
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index));

        // notify peers if piece is complete
        if *piece.is_complete.clone().lock().await {
            self.broadcast(IPC::PieceComplete(piece_index));
        }

        // notify peers if download is complete
        if self.is_complete() {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete);
        }

        Ok(())
    }

    pub fn new(our_peer_id: String, meta_info: TorrentMetaInfo) -> Result<Download, Error> {
        
        let mut file_infos = Vec::new();
        {
            match &meta_info.info {
                Info::Single(s) => {
                    file_infos.push((s.length, s.name.clone()));
                },
                Info::Multi(m) => {
                    for f in &m.files {
                        file_infos.push((f.length, f.path.join("/")));
                    }
                },
            };
        }
        
        // create files and file_offsets
        let mut files = Vec::with_capacity(file_infos.len());
        let mut file_offsets = Vec::with_capacity(file_infos.len() + 1);
        let mut file_offset = 0;
        let _s: Result<(), Error> = task::block_on(async  {
            for (length, file_path) in file_infos {
                let path = Path::new("downloads").join(&file_path);
                let mut file = OpenOptions::new().create(true).read(true).write(true).open(path).await?;
                files.push(Arc::new(Mutex::new(file)));
                file_offsets.push(file_offset);

                file_offset = file_offset + length;
            }
            file_offsets.push(file_offset);
            Ok(())
        });
        let file_length = file_offset;


        // create pieces
        let piece_length = meta_info.piece_length();
        let num_pieces = meta_info.num_pieces() ;

        let mut pieces = Vec::with_capacity(num_pieces);

        for i in 0..num_pieces {
            let offset = i as u64 * piece_length as u64;
            let length = if i < (num_pieces - 1) {
                piece_length
            } else {
                (file_length - offset) as usize
            };
            let mut piece = Piece::new(length as u32, offset, meta_info.pieces()[i as usize].clone());

            piece.verify(&files, &file_offsets)?;
            pieces.push(piece);
        }

        Ok(Download {
            our_peer_id,
            meta_info,
            pieces,
            files,
            peer_channels: Arc::new(Mutex::new(Vec::new())),
            file_offsets
        })
    }

    pub fn register_peer(&self, channel: Sender<IPC>) {
        task::block_on(async {
            self.peer_channels.clone().lock().await.push(channel);
        });

    }

    pub async fn retrive_data(&self, request: &RequestMetadata) -> Result<Vec<u8>, Error> {
        // let ref piece = self.pieces[request.piece_index as usize];
        //
        // if *piece.is_complete.clone().lock().await {
        //     let offset = piece.offset + request.offset as u64;
        //     let file = &mut self.file;
        //     file.seek(io::SeekFrom::Start(offset))?;
        //     let mut buf = vec![];
        //     file.take(request.block_length as u64).read_to_end(&mut buf)?;
        //     Ok(buf)
        // } else {
        //     Err(Error::MissingPieceData)
        // }
        Err(Error::MissingPieceData)
    }

    pub fn have_pieces(&self) -> Vec<bool> {
        task::block_on(async {
            let mut res = Vec::with_capacity(self.pieces.len());

            for p in &self.pieces {
                let b = p.is_complete.clone();
                res.push(*b.lock().await);
            }
            res
        })
    }

    pub fn incomplete_blocks_for_piece(&self, piece_index: u32) -> Vec<(u32,u32)> {
        let ref piece = self.pieces[piece_index as usize];
        task::block_on(async {
            if !*piece.is_complete.clone().lock().await {
                piece.blocks.iter()
                    .filter(|b| {
                        task::block_on(async {
                            !b.clone().lock().await.is_complete
                        })
                    })
                    .map(|b| {
                        task::block_on(async {
                            let b = b.clone();
                            let b = b.lock().await;
                            (b.index, b.length)
                        })
                    }).collect()
            } else {
                vec![]
            }
        })
    }

    fn is_complete(&self) -> bool {
        task::block_on(async {
            for piece in self.pieces.iter() {
                if !*piece.is_complete.clone().lock().await {
                    return false
                }
            }
            true
        })
    }

    async fn broadcast(&self, ipc: IPC) {
        self.peer_channels.clone().lock().await.retain(|channel| {
            task::block_on(async {
                channel.send(ipc.clone()).await;
                true
            })
        });
    }
}

pub async fn store_block(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<(), Error> {

    let block_offset = piece_offset + (block_index  * BLOCK_SIZE) as u64;

    let (i, ptr_vec) =
        search_ptrs(file_offsets, block_offset, data.len());

    for (a, (block_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let mut file = files[i + a].clone();
        let mut file= file.lock().await;
        store(file, file_ptr, block_ptr, len as u64, data).await?;
    }

    Ok(())
}

//todo: return (i, num)
fn search_index(file_offsets: &[u64], offset: u64) -> usize {
    let len = file_offsets.len();
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

//todo: return (i, Vec<(data_ptr, file_ptr, len)>)
fn search_ptrs(file_offsets: &[u64], offset: u64, len: usize) -> (usize, Vec<(usize, u64, usize)>) {
    let index = search_index(file_offsets, offset);

    let mut vec = Vec::new();
    let mut i = index;
    let mut data_ptr = 0usize;
    let mut left = len as u64;
    let mut file_ptr = offset - file_offsets[i];

    while left + file_offsets[i] >= file_offsets[i + 1] {
        let l =  file_offsets[i + 1] - file_offsets[i];
        vec.push((data_ptr, file_ptr, l as usize));

        data_ptr = data_ptr + l as usize;
        left = left - l;
        file_ptr = 0;
        i = i + 1;
    };

    vec.push((data_ptr, 0, left as usize));
    return (index, vec);
}

async fn store(mut file: MutexGuard<'_, File>,
               file_ptr: u64, block_ptr: usize, len: u64,
               data: &[u8]) -> Result<(), Error> {
    // async_std::io::seek::SeekExt::seek(&mut file, io::SeekFrom::Start(file_ptr)).await;
    file.seek(io::SeekFrom::Start(file_ptr)).await;
    file.write(&data[block_ptr..block_ptr + len as usize]).await?;

    Ok(())
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
    #[test]
    fn test() {
        assert_eq!(4, 4);
    }
}
