use std::sync::Arc;
use url::Url;
use futures::channel::mpsc::Sender;
use crate::tracker::tracker_supervisor::TrackerStatus;
use crate::base::manager::ManagerEvent;
use crate::base::meta_info::TorrentMetaInfo;
use std::time::Instant;

// pub struct Tracker {
//     metadata: Arc<TorrentMetaInfo>,
//     to_manager: Sender<ManagerEvent>,
//     extern_id: Arc<PeerExternId>,
//     /// All addresses the host were resolved to
//     /// When we're connected to an address, it is moved to the first position
//     /// so later requests will use this address first.
//     address: Arc<Url>,
//     tracker_supervisor: Sender<(UrlHash, Instant, TrackerStatus)>,
// }