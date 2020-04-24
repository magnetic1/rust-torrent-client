use async_std::fs::{File, OpenOptions};
use async_std::path::{PathBuf, Path};
use async_std::{fs, io, task};
use async_std::sync::{Arc, RwLock, Mutex, MutexGuard};
use async_std::prelude::*;
use async_std::stream;
use std::convert;
use futures::{pin_mut, select, SinkExt, StreamExt, TryStreamExt};
use futures::channel::mpsc::{Sender, Receiver};
use futures::future::FutureExt;
use futures::prelude::stream::FuturesUnordered;
use crate::net::ipc::IPC;
use crate::bt::ipc::Message;
use crate::bencode::hash::Sha1;
use crate::base::meta_info::{TorrentMetaInfo, Info};
use std::option::Option::Some;

pub const BLOCK_SIZE: u32 = 16 * 1024;

pub struct MetaFile {
    name: String,
    dir:  String,
    pub file: File,
}

impl MetaFile {
    pub fn new(name: String, dir: String) -> MetaFile {
        let filepath = dir.clone() + &name;

        task::block_on(async move {
            let mut file = OpenOptions::new()
                .create(true).read(true).write(true).open(&filepath).await.unwrap();

            MetaFile {
                name,
                dir,
                file,
            }
        })
    }

    pub(crate) async fn rename(&mut self, new_name: String) -> Result<(), Error> {
        let old = self.dir.clone() + &self.name;
        let new = self.dir.clone() + &new_name;
        fs::rename(&old, &new).await?;
        self.name = new_name;

        let mut file = OpenOptions::new().create(true).read(true).write(true).open(&new).await?;
        self.file = file;
        Ok(())
    }
}

struct FilesManager {
    piece_length: usize,
    pieces: Vec<Piece>,
    files: Vec<Arc<Mutex<MetaFile>>>,
    file_offsets: Vec<u64>,
    peer_channels: Mutex<Vec<Sender<IPC>>>,

    store_receiver: Receiver<IPC>,
}

impl FilesManager {
    pub fn new(store_receiver: Receiver<IPC>, meta_info: &TorrentMetaInfo) -> Result<FilesManager, Error> {
        let mut file_infos = Vec::new();

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

        println!("{}", file_infos.len());

        // create files and file_offsets
        let mut files = Vec::with_capacity(file_infos.len());
        let mut file_offsets = Vec::with_capacity(file_infos.len() + 1);
        // let mut file_paths = Vec::with_capacity(file_infos.len());
        let mut file_offset = 0;

        task::block_on(async move {
            for (length, file_path) in file_infos {
                let path = Path::new(&file_path);

                // create dirs
                let t: Vec<String>= file_path.split("/").map(|s| s.to_string()).collect();
                let (dir, name): (String, String) = if t.len() > 1 {
                    let dir = t[..t.len()-1].join("/");
                    match fs::metadata(dir.clone()).await {
                        Ok(_) => {},
                        Err(_) => {
                            println!("{}", dir);
                            fs::create_dir_all(dir.clone()).await.unwrap();
                        },
                    }
                    (dir.to_string(), t[t.len()-1].to_string())
                } else {
                    ("".to_string(), t[0].to_string())
                };


                files.push(Arc::new(Mutex::new(MetaFile::new(name, dir))));
                file_offsets.push(file_offset);

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

            Ok(FilesManager {
                piece_length: meta_info.piece_length(),
                pieces,
                files,
                peer_channels: Mutex::new(Vec::new()),
                file_offsets,

                store_receiver
            })
        })
    }

    async fn run(&self) {
        // let mut futures = FuturesUnordered::new();
    }

    pub async fn store(&self, piece_index: u32, block_index: u32, data: Vec<u8>) -> Result<(), Error> {
        let piece = &self.pieces[piece_index as usize];

        if piece.has_block(block_index).await || *piece.is_complete.read().await {
            // if we already have this block, do an early return to avoid re-writing the piece, sending complete messages, etc
            return Ok(());
        }

        store_block(&self.files, &self.file_offsets, piece.offset, block_index, &data).await?;

        let mut piece = piece;
        piece.blocks[block_index as usize].write().await.is_complete = true;

        if piece.has_all_blocks().await {
            let valid = piece.verify(&self.files, &self.file_offsets).await?;
            if !valid {
                piece.reset_blocks();
            } else {
                // verify files. rename it if finished.
                self.file_verify_rename(piece.offset, piece.length, self.piece_length).await?;
            }
        }

        // notify peers that this block is complete
        self.broadcast(IPC::BlockComplete(piece_index, block_index)).await;
        // notify peers if piece is complete
        if *piece.is_complete.read().await {
            self.broadcast(IPC::PieceComplete(piece_index)).await;
        }
        // notify peers if download is complete
        if self.is_complete().await {
            println!("Download complete");
            self.broadcast(IPC::DownloadComplete).await;
        }

        Ok(())
    }

    pub async fn register_peer(&self, channel: Sender<IPC>) {
        self.peer_channels.lock().await.push(channel);
    }

    async fn file_verify_rename(&self, piece_offset: u64, length: u32, piece_length: usize) -> Result<(), Error> {
        let (index, v)
            = search_ptrs(&self.file_offsets, piece_offset, length as usize);

        for i in 0..v.len() {
            let mut file = self.files[i + index].lock().await;
            let name = &file.name;

            if name.ends_with(".temp") {
                let low = self.file_offsets[i + index] as usize / piece_length;
                let high = (self.file_offsets[i + index + 1] - 1) as usize / piece_length;

                let mut file_is_complete = true;
                for p in low..=high {
                    let b = match self.pieces[p].is_complete.try_read() {
                        Some(s) => {*s},
                        None => { false },
                    };
                    if !b {
                        file_is_complete = false;
                        break;
                    }
                };

                if file_is_complete {
                    let new = name[..name.len() - 5].to_string();
                    file.rename(new).await?;
                }
            }
        }

        Ok(())
    }

    async fn broadcast(&self, ipc: IPC) {
        // use futures::stream::TryStreamExt;
        // use futures::stream as s;
        // use futures::stream::{Stream, TryStream, TryStreamExt};

        let mut peer_channels = self.peer_channels.lock().await;
        let len = peer_channels.len();
        let mut stream = stream::from_iter(peer_channels.iter_mut()).enumerate();

        let mut keep = vec![false; len];

        // stream.try_for_each_concurrent(len, |n| async move {
        //
        // });
        while let  Some((i, channel)) = stream.next().await {
            match channel.send(ipc.clone()).await {
                Ok(_) => keep[i] = true,
                Err(_) => {},
            };
        }

        let mut i = 0;
        peer_channels.retain(|_| (keep[i], i += 1).0);
    }

    async fn is_complete(&self) -> bool {
        for piece in self.pieces.iter() {
            if !*piece.is_complete.read().await {
                return false;
            }
        }
        return true;
    }
}

struct Piece {
    length: u32,
    offset: u64,
    hash: Sha1,
    blocks: Vec<Arc<RwLock<Block>>>,
    is_complete: Arc<RwLock<bool>>,
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
            blocks.push(Arc::new(RwLock::new(Block::new(i as u32, len))));
        }

        Piece {
            length,
            offset,
            hash,
            blocks,
            is_complete: Arc::new(RwLock::new(false)),
        }
    }

    async fn verify(&self, files: &[Arc<Mutex<MetaFile>>], file_offsets: &[u64]) -> Result<bool, Error> {
        let buffer = read(files, file_offsets, self.offset, self.length).await?;

        // calculate the hash, verify it, and update is_complete
        if self.hash == Sha1::calculate_sha1(&buffer) {
            let mut is_complete = self.is_complete.write().await;
            *is_complete = true;
            return Ok(true);
        }
        return Ok(false);
    }

    async fn has_block(&self, block_index: u32) -> bool {
        self.blocks[block_index as usize].read().await.is_complete
    }

    async fn has_all_blocks(&self) -> bool {
        let mut futures = FuturesUnordered::new();

        for block in self.blocks.iter() {
            let b = block.clone();
            let future = async move {
                b.read().await.is_complete
            };
            futures.push(future.fuse());
        }

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
    }

    async fn reset_blocks(&self) {
        for block in self.blocks.iter() {
            (*block.write().await).is_complete = false;
        }
    }
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

pub async fn store_block(files: &[Arc<Mutex<MetaFile>>], file_offsets: &[u64],
                         piece_offset: u64, block_index: u32, data: &[u8]) -> Result<(), Error> {
    let block_offset = piece_offset + (block_index * BLOCK_SIZE) as u64;

    let (i, ptr_vec) =
        search_ptrs(file_offsets, block_offset, data.len());

    for (a, (block_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let mut file = files[i + a].clone();
        let mut file = &mut *file.lock().await;
        store(&mut file.file, file_ptr, block_ptr, len as u64, data).await?;
    }

    Ok(())
}

async fn store(file: &mut File,
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


async fn read(files: &[Arc<Mutex<MetaFile>>], file_offsets: &[u64],
              offset: u64, len: u32) -> Result<Vec<u8>, Error> {
    let (i, ptr_vec) =
        search_ptrs(file_offsets, offset, len as usize);

    let mut buffer: Vec<u8> = Vec::with_capacity(len as usize);

    for (a, (piece_ptr, file_ptr, len)) in ptr_vec.into_iter().enumerate() {
        let file = files[i + a].clone();
        let mut file = &mut *file.lock().await;
        let file = &mut file.file;
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
    use crate::bt::files_manager::MetaFile;
    use async_std::fs::OpenOptions;
    use async_std::task;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt, TryStreamExt};
    use async_std::stream;
    use async_std::prelude::*;


    #[test]
    fn rename_test() {
        task::block_on(async {
            let name = "old.docx".to_string();
            let file = OpenOptions::new().create(true).read(true).write(true).open(&name).await.unwrap();

            let mut mf = MetaFile {
                name,
                dir: "".to_string(),
                file,
            };

            mf.rename("new.docx".to_string()).await;
        });
    }

    #[test]
    fn mpsc_test() {
        let (mut s, mut r) = mpsc::unbounded();
        let mut s1 = s.clone();

        task::block_on(async move {
            s.send(1).await;
        });
        task::block_on(async move {
            s1.send(2).await;
        });

        task::block_on(async move {
            assert_eq!(r.next().await, Some(1));
            assert_eq!(r.next().await, Some(2));
            assert_eq!(r.next().await, None);
        })
    }

    #[test]
    fn stream_test() {

        let mut v = vec!["1".to_string(), "2".to_string()];
        let mut v1 = vec![];
        let mut stream = stream::from_iter(v.iter_mut());
        
        task::block_on(async move {
            while let Some(i) = stream.next().await {
                v1.push(i.push_str("23"));
            }
            println!("{:?}", v1);
        });

        task::block_on(async move {
            let mut v = vec!["1".to_string(), "2".to_string()];
            let stream = stream::from_iter(v.iter_mut());

            stream.for_each_concurrent(1, |n| async move {
                println!("{}", n);
                // 1
            }).await;


            println!("{:?}", "s");
        });

    }
}