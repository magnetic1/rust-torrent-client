use std::{fs, convert, any, env, process};
use crate::bencode::decode::{Decoder, DecodeTo, DecodeError};
use crate::bencode::value::Value;
use crate::net::{download, listener, peer_connection, tracker_connection};
use crate::base::meta_info;
use rand::Rng;
use crate::net::download::Download;
use async_std::sync::{Mutex, Arc};
use async_std::task::JoinHandle;
use async_std::task;
use std::fs::File;
use std::io::Read;
use getopts::Options;
use futures::prelude::stream::FuturesUnordered;
use async_std::prelude::Future;
use futures::StreamExt;
use crate::net::peer_connection::Peer;

mod bencode;
mod base;
mod net;


const PEER_ID_PREFIX: &'static str = "-RC0001-";

fn main() {
    // parse command-line arguments & options
    let args: Vec<String> = env::args().collect();
    let program = &args[0];
    let mut opts = Options::new();
    opts.optopt("p", "port", "set listen port to", "6881");
    opts.optflag("h", "help", "print this help menu");

    let matches = match opts.parse(&args[1..]) {
        Ok(m) => { m }
        Err(f) => { panic!(f.to_string()) }
    };

    if matches.opt_present("h") {
        print_usage(&program, opts);
        return;
    }

    let port = match matches.opt_str("p") {
        Some(port_string) => {
            let port: Result<u16,_> = port_string.parse();
            match port {
                Ok(p) => p,
                Err(_) => return abort(&program, opts, format!("Bad port number: {}", port_string))
            }
        },
        None => 6881
    };

    let rest = matches.free;
    // if rest.len() != 1 {
    //     abort(&program, opts, format!("You must provide exactly 1 argument to rusty_torrent: {:?}", rest))
    // }


    // let filename = &rest[0];
    let filename = r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#;

    println!("start!");
    task::block_on(async move {
        match run(filename, port).await {
            Ok(_) => {},
            Err(e) => println!("main Error: {:?}", e)
        };
    });
}

fn print_usage(program: &str, opts: Options) {
    let brief = format!("Usage: {} [options] path/to/myfile.torrent", program);
    print!("{}", opts.usage(&brief));
}

fn abort(program: &str, opts: Options, err: String) {
    println!("{}", err);
    print_usage(program, opts);
    process::exit(1);
}

async fn run(filename: &str, listener_port: u16) -> Result<(), Error> {
    let our_peer_id = generate_peer_id();
    println!("Using peer id: {}", our_peer_id);

    // parse .torrent file
    let mut bytes = Vec::new();
    File::open(filename).unwrap().read_to_end(&mut bytes).unwrap();
    let metainfo = meta_info::TorrentMetaInfo::parse(&bytes);

    // connect to tracker and download list of peers
    // let mut peers = tracker_connection::get_peers(&our_peer_id, &metainfo, listener_port)?;
    /// 116.249.137.177:51636
    /// 104.152.209.30:64223
    /// 205.185.122.158:54794
    let mut peers = Vec::new();
    peers.push(Peer {
        ip: "116.249.137.177".to_string(),
        port: 51636
    });
    peers.push(Peer {
        ip: "104.152.209.30".to_string(),
        port: 64223
    });
    peers.push(Peer {
        ip: "205.185.122.158".to_string(),
        port: 54794
    });
    println!("Found {} peers", peers.len());

    // create the download metadata object and stuff it inside a reference-counted mutex
    let download = Download::new(our_peer_id, metainfo)?;
    let download = Arc::new(download);

    // spawn thread to listen for incoming request
    let d = download.clone();


    let listener_task = task::spawn(async move {
        listener::start(listener_port, d).await;
    });

    // spawn threads to connect to peers and start the download
    // let mut futs = FuturesUnordered::new();
    let mut peer_tasks: Vec<JoinHandle<()>> = Vec::new();
    for peer in peers {
        let download = download.clone();

        let t = task::spawn(async move {
            match peer_connection::connect(&peer, download).await {
                Ok(_) => println!("Peer done"),
                Err(e) => println!("peer connect Error: {:?}", e)
            };
        });
        peer_tasks.push(t);
    };

    let t = task::spawn(async move {
        futures::future::join_all(peer_tasks).await;
    });
    futures::join!(listener_task, t);
    // futs.push(listener_task);

    // task::block_on(async {
    //     loop {
    //         futs.select_next_some().await
    //     };
    // });

    // wait for peers to complete
    // futures::future::join_all(peer_tasks).await;
    Ok(())
}

fn generate_peer_id() -> String {
    let mut rng = rand::thread_rng();
    let rand_chars: String = rng.gen_ascii_chars().take(20 - PEER_ID_PREFIX.len()).collect();
    format!("{}{}", PEER_ID_PREFIX, rand_chars)
}

#[derive(Debug)]
pub enum Error {
    DecoderError(DecodeError),
    DownloadError(download::Error),
    TrackerError(tracker_connection::Error),
    Any(Box<dyn any::Any + Send>),
}

impl convert::From<DecodeError> for Error {
    fn from(err: DecodeError) -> Error {
        Error::DecoderError(err)
    }
}

impl convert::From<download::Error> for Error {
    fn from(err: download::Error) -> Error {
        Error::DownloadError(err)
    }
}

impl convert::From<tracker_connection::Error> for Error {
    fn from(err: tracker_connection::Error) -> Error {
        Error::TrackerError(err)
    }
}

impl convert::From<Box<dyn any::Any + Send>> for Error {
    fn from(err: Box<dyn any::Any + Send>) -> Error {
        Error::Any(err)
    }
}
