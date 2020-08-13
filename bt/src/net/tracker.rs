use crate::{
    base::Result,
    bencode::value::{FromValue, Value},
    bencode::decode::{Decoder, DecodeTo, DecodeError},
    net::peer_connection::Peer,
    base::meta_info::TorrentMetaInfo,
};
use hyper::{
    Client,
    Uri,
    body::Buf,
    Request,
    Response,
    Body,
};
use url::percent_encoding::{percent_encode, FORM_URLENCODED_ENCODE_SET};

async fn get_tracker_response(peer_id: &str, announce: &str, length: u64,
                              info_hash: &[u8], listener_port: u16) -> Result<TrackerResponse> {
    let length_string = length.to_string();
    let encoded_info_hash = percent_encode(info_hash, FORM_URLENCODED_ENCODE_SET);
    let listener_port_string = listener_port.to_string();

    let params = vec![("left", length_string.as_ref()),
                      ("info_hash", encoded_info_hash.as_ref()),
                      ("downloaded", "0"),
                      ("uploaded", "0"),
                      ("event", "started"),
                      ("peer_id", peer_id),
                      ("compact", "1"),
                      ("port", listener_port_string.as_ref())];

    let url = format!("{}?{}", announce, encode_query_params(&params));
    println!("{}", url);

    let client = Client::new();
    let request = Request::builder()
        .uri(url.as_str())
        .header("Connection", "close")
        .body(Body::empty())
        .unwrap();
    let http_res = client.request(request).await?;

    let buf = hyper::body::to_bytes(http_res).await?;
    let mut body = buf.bytes();
    println!("{}", body.len());

    let res = TrackerResponse::parse(body)?;

    println!("{:#?}", res);
    Ok(res)
}

pub async fn get_tracker_response_by_metainfo(peer_id: &str, metainfo: &TorrentMetaInfo, listener_port: u16) -> Result<TrackerResponse> {
    let announce = "http://explodie.org:6969/announce";
    let info_hash = &metainfo.info_hash().0;
    get_tracker_response(peer_id, announce, metainfo.length(), info_hash, listener_port).await
}

pub async fn get_peers(peer_id: &str, metainfo: &TorrentMetaInfo, listener_port: u16) -> Result<Vec<Peer>> {
    let res = get_tracker_response_by_metainfo(peer_id, metainfo, listener_port).await?;
    Ok(res.peers)
}

fn encode_query_params(params: &[(&str, &str)]) -> String {
    let param_strings: Vec<String> = params.iter().map(|&(k, v)| format!("{}={}", k, v)).collect();
    param_strings.join("&")
}

#[derive(PartialEq, Debug)]
pub struct TrackerResponse {
    pub interval: u32,
    pub min_interval: Option<u32>,
    pub complete: u32,
    pub incomplete: u32,
    pub peers: Vec<Peer>,
}

impl TrackerResponse {
    pub fn parse(bytes: &[u8]) -> Result<TrackerResponse> {
        let mut d = Decoder::new(bytes);
        let value: Value = DecodeTo::decode(&mut d)?;
        Ok(FromValue::from_value(&value)?)
    }
}

impl FromValue for TrackerResponse {
    fn from_value(value: &Value) -> std::result::Result<Self, DecodeError> {
        let peers_value = value.get_value("peers")?;

        let peers: Vec<Peer> = match peers_value {
            Value::List(_) => {
                FromValue::from_value(peers_value)?
            }
            Value::Bytes(b) => {
                b.chunks(6).map(Peer::from_bytes).collect()
            }
            _ => Err(DecodeError::ExtraneousData)?,
        };

        let response = TrackerResponse {
            interval: value.get_field("interval")?,
            min_interval: value.get_option_field("min interval")?,
            complete: value.get_field("complete")?,
            incomplete: value.get_field("incomplete")?,
            peers,
        };
        Ok(response)
    }
}

#[cfg(test)]
mod test {
    use std::fs;
    use crate::bencode::decode::{Decoder, DecodeTo};
    use crate::base::meta_info::TorrentMetaInfo;
    use crate::net::tracker::get_peers;
    use rand::Rng;

    const PEER_ID_PREFIX: &'static str = "-RC0001-";

    #[test]
    fn tracker_response() {
        let f = fs::read(
            r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        let res = tokio_test::block_on(async {
            get_peers(&generate_peer_id(), &torrent_meta_info, 8888).await
        });
        match res {
            Ok(_) => {}
            Err(e) => { println!("{:#?}", e) }
        }
    }

    fn generate_peer_id() -> String {
        let mut rng = rand::thread_rng();
        let rand_chars: String = rng.gen_ascii_chars().take(20 - PEER_ID_PREFIX.len()).collect();
        format!("{}{}", PEER_ID_PREFIX, rand_chars)
    }
}
