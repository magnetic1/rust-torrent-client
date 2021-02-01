use futures::channel::mpsc::SendError;
use TrackerError::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TrackerError {
    Unresponsive,
    UnsupportedScheme,
    InvalidInput,
    SendError(SendError),
}

impl std::fmt::Display for TrackerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Unresponsive => write!(f, "Unresponsive!"),
            UnsupportedScheme => write!(f, "UnsupportedScheme! Only Support http/https. "),
            InvalidInput => write!(f, "InvalidInput!"),
            SendError(e) => write!(f, "TrackerError({})", e),
        }
    }
}

impl std::error::Error for TrackerError {}

impl From<SendError> for TrackerError {
    fn from(e: SendError) -> Self {
        SendError(e)
    }
}

impl From<surf::Error> for TrackerError {
    fn from(_: surf::Error) -> Self {
        Unresponsive
    }
}
