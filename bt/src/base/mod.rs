use async_std::{prelude::*, task};
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod download;
pub mod ipc;
pub mod manager;
pub mod meta_info;
pub mod terminal;

pub fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            terminal::print_log(format!("task {} {}", task::current().id(), e)).unwrap();
            // eprintln!("task {} {}", task::current().id(), e)
        }
    })
}

#[macro_export]
macro_rules! require_buf1 {
    ($sender:expr, $event:path) => {{
        let (mut sender, mut receiver) = mpsc::channel(1);
        $sender.send($event(sender)).await?;
        receiver.next().await.ok_or("ManagerEvent Require error")?
    };};
}

#[macro_export]
macro_rules! require_oneshot {
    ($sender:expr, $event:path) => {{
        let (sender, receiver) = futures::channel::oneshot::channel();
        $sender.send($event(sender)).await?;
        receiver.await?
    };};
}
