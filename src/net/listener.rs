use async_std::sync::Arc;
use crate::net::download::Download;
use crate::net::peer_connection;
use async_std::task::JoinHandle;
use async_std::net::{TcpListener, TcpStream, Incoming};
use futures::StreamExt;
use async_std::task;
use futures::prelude::stream::FuturesUnordered;
use futures::select;
use futures::pin_mut;
use futures::future::FutureExt;
use async_std::prelude::Future;
use futures::prelude::future::Fuse;
use futures::channel::mpsc::UnboundedSender;
use crate::net::peer_connection::Message;

async fn get_stream<'a>(mut incoming: Incoming<'a>) -> TcpStream {
    while let Some(stream) = incoming.next().await {
        match stream {
            Ok(s) => {return s},
            _ => panic!("TcpListener Error "),
        };
    }
    panic!("TcpListener None stream ");
}

pub async fn start(tx: UnboundedSender<Message>, port: u16, download: Arc<Download>) {
    println!("listener start!");

    let tcp_listener = TcpListener::bind(("0.0.0.0", port)).await.unwrap();
    let mut incoming = tcp_listener.incoming();

    let mut handle_connection_futs = FuturesUnordered::new();
    // let mut incoming = tcp_listener.incoming();

    let con_fut = Fuse::terminated();
    pin_mut!(con_fut);

    con_fut.set(get_stream(incoming).fuse());

    loop {
        select! {
            res = con_fut => {
                handle_connection_futs.push(
                    handle_connection(tx.clone(), res, download.clone()).fuse()
                );
                let mut incoming = tcp_listener.incoming();
                con_fut.set(get_stream(incoming).fuse());
            },
            () = handle_connection_futs.select_next_some() => {},
            complete => panic!("`interval_timer` completed unexpectedly"),
        }
    };
}

async fn handle_connection(tx: UnboundedSender<Message>, stream: TcpStream, download: Arc<Download>) {
    match peer_connection::accept(tx, stream, download).await {
        Ok(_) => println!("Peer done"),
        Err(e) => println!("Error: {:?}", e)
    };
}
