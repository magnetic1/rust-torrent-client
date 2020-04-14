use crate::base::meta_info::TorrentMetaInfo;
use crate::bencode::hash::Sha1;
use std::{convert, io};
use async_std::sync::{Mutex, Arc, MutexGuard};
use async_std::fs::File;
use async_std::task;
use futures::{AsyncSeekExt, AsyncWriteExt};


pub const BLOCK_SIZE: u32 = 16 * 1024;


pub struct Download {
    pieces:          Vec<Piece>,
    meta_info:       TorrentMetaInfo,
    files:           Vec<Arc<Mutex<File>>>,
    file_offsets:    Vec<u64>,
    peer_channels:   Vec<async_std::sync::Sender<IPC>>,
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
                length - (BLOCK_SIZE * (num_blocks - 1))
            };
            blocks.push(Block::new(i as u32, len));
        }

        Piece {
            length,
            offset,
            hash,
            blocks,
            is_complete: false,
        }
    }



    fn verify(&mut self, file: &mut File) -> Result<bool, Error> {
        // read in the part of the file corresponding to the piece
        try!(file.seek(io::SeekFrom::Start(self.offset)));
        let mut data = vec![];
        try!(file.take(self.length as u64).read_to_end(&mut data));

        // calculate the hash, verify it, and update is_complete
        self.is_complete = self.hash == calculate_sha1(&data);
        Ok(self.is_complete)
    }

    fn has_block(&self, block_index: u32) -> bool {
        task::block_on(async {
            self.blocks[block_index as usize].lock().await.is_complete
        })
    }

    fn has_all_blocks(&self) -> bool {
        for block in self.blocks.iter() {
            task::block_on(async {
                if !block.clone().lock().await.is_complete {
                    false
                }
            })
        }
        true
    }

    fn reset_blocks(&mut self) {
        for block in self.blocks.iter_mut() {
            block.is_complete = false;
        }
    }
}

impl Download {
    pub async fn store(&self, piece_index: u32, block_index: u32, data: Vec<u8>) -> Result<(), Error> {

        let piece = &self.pieces[piece_index as usize];
        if piece.is_complete.clone().lock().await || piece.has_block(block_index) {
            // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
            return Ok(());
        }

        store_block(&self.files, &self.file_offsets, piece.offset, block_index, &data)?;


        let mut piece = piece;
        piece.blocks[block_index as usize].is_complete = true;

        if piece.has_all_blocks() {
            let valid = piece.verify(file)?;
            if !valid {
                piece.reset_blocks();
            }
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index));

        // notify peers if piece is complete
        if piece.is_complete {
            self.broadcast(IPC::PieceComplete(piece_index));
        }

        // notify peers if download is complete
        if self.is_complete() {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete);
        }

        Ok(())
    }
}

pub async fn store_block(files: &[Arc<Mutex<File>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<(), Error> {

    let block_offset = piece_offset + (block_index  * BLOCK_SIZE) as usize;
    let (i, num) = search_index(file_offsets, block_offset, data.len());

    let mut block_ptr = 0usize;
    let mut file_ptr = block_offset - file_offset;

    for a in 0..num {

        let file_length = file_offsets[i + a + 1] - file_offsets[i + a];
        let mut file = files[1].clone();

        let write_len;
        if data.len() - block_ptr >= file_length - file_ptr  {
            write_len = file_length - file_ptr;
        } else {
            write_len = (data.len() - block_ptr) as u64;
        }

        store(file.lock().await, file_ptr, block_ptr, (data.len() - block_ptr) as u64, data).await?;

        block_ptr = block_ptr + write_len;
        file_ptr = 0;
    }

    Ok(())
}

//todo: return (i, num)
fn search_index(file_offsets: &[u64], offset: u64, len: usize) -> (usize, usize) {


    (0, 2)
}

async fn store(mut file: MutexGuard<File>,
               file_ptr: u64, block_ptr: usize, len: u64,
               data: &[u8]) -> Result<(), Error> {
    file.seek(io::SeekFrom::Start(file_ptr));
    file.write(&data[block_ptr..block_ptr + len]).await?;

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
