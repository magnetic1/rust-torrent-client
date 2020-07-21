use async_std::{
    prelude::*,
    task,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

pub mod meta_info;
pub mod ipc;
pub mod download;


fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
    where
        F: Future<Output=Result<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            eprintln!("{}", e)
        }
    })
}