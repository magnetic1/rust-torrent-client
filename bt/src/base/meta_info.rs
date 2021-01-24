use crate::base::meta_info::Info::{Multi, Single};
use crate::bencode::decode::{DecodeError, DecodeTo, Decoder};
use crate::bencode::encode::{EncodeTo, Encoder};
use crate::bencode::hash::Sha1;
use crate::bencode::value::{FromValue, IntoValue, Value};
use std::collections::BTreeMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TorrentMetaInfo {
    pub info: Info,
    pub announce: Option<String>,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub creation_date: Option<Instant>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

impl TorrentMetaInfo {
    pub fn parse(bytes: &[u8]) -> TorrentMetaInfo {
        let mut d = Decoder::new(bytes);
        let v: Value = DecodeTo::decode(&mut d).unwrap();

        TorrentMetaInfo::from_value(&v).unwrap()
    }

    pub fn piece_length(&self) -> usize {
        match &self.info {
            Single(s) => s.piece_length,
            Multi(m) => m.piece_length,
        }
    }

    pub fn num_pieces(&self) -> usize {
        match &self.info {
            Single(s) => s.pieces.len(),
            Multi(m) => m.pieces.len(),
        }
    }

    pub fn pieces(&self) -> &[Sha1] {
        match &self.info {
            Single(s) => &s.pieces,
            Multi(m) => &m.pieces,
        }
    }

    pub fn length(&self) -> u64 {
        let info = &self.info;
        match info {
            Single(i) => i.length,
            Multi(m) => {
                let mut len = 0;
                for item in &m.files {
                    len += item.length;
                }
                len
            }
        }
    }

    pub fn info_hash(&self) -> Sha1 {
        let info = &self.info;
        // let s: Info = info.;
        let v = IntoValue::into_value(info.clone());

        let mut encoder = Encoder::new();
        v.encode(&mut encoder).unwrap();

        Sha1::calculate_sha1(&encoder.into_bytes())
    }

    pub fn announce(&mut self) -> String {
        match &self.announce_list {
            Some(_v) => self.announce.clone().unwrap(),
            None => self.announce.clone().unwrap(),
        }
    }
}

#[derive(Debug, Clone)]
pub enum Info {
    Single(SingleInfo),
    Multi(MultiInfo),
}

#[derive(Debug, Clone)]
pub struct SingleInfo {
    pub length: u64,
    pub md5sum: Option<String>,
    // 字符串,BitTorrent下载路径中最上层的目录名
    pub name: String,
    // 整数,是BitTorrent文件块的大小
    pub piece_length: usize,
    // 字符串,连续的存放着所有块的SHA1杂凑值,每一个文件块的杂凑值为20字节
    pub pieces: Vec<Sha1>,
}

#[derive(Debug, Clone)]
pub struct MultiInfo {
    pub files: Vec<FileInfo>,
    pub name: String,
    pub piece_length: usize,
    pub pieces: Vec<Sha1>,
}

#[derive(Debug, Clone)]
pub struct FileInfo {
    pub length: u64,
    pub md5sum: Option<String>,
    pub path: Vec<String>,
}

impl DecodeTo for FileInfo {
    fn decode(d: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        d.read_struct(|d| {
            Ok(FileInfo {
                length: d.read_field("length")?,
                md5sum: d.read_option("md5sum")?,
                path: d.read_field("path")?,
            })
        })
    }
}

impl DecodeTo for Info {
    fn decode(d: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        d.read_struct(|d| {
            let files_result: Result<Vec<FileInfo>, DecodeError> = d.read_field("files");
            match files_result {
                Ok(files) => Ok(Info::Multi(MultiInfo {
                    files,
                    name: d.read_field("name")?,
                    piece_length: d.read_field("piece length")?,
                    pieces: d.read_field("pieces")?,
                })),
                _ => Ok(Info::Single(SingleInfo {
                    length: d.read_field("length")?,
                    md5sum: d.read_option("md5sum")?,
                    name: d.read_field("name")?,
                    piece_length: d.read_field("piece length")?,
                    pieces: d.read_field("pieces")?,
                })),
            }
        })
    }
}

impl DecodeTo for TorrentMetaInfo {
    fn decode(d: &mut Decoder<'_>) -> Result<Self, DecodeError> {
        d.read_struct(|d| {
            Ok(TorrentMetaInfo {
                announce: d.read_option("announce")?,
                //                    announce_list: d.read_option("announce-list")?,
                announce_list: d.read_field_combine("announce-list")?,
                // TODO: time
                creation_date: None,
                comment: d.read_option("comment")?,
                created_by: d.read_option("created by")?,
                encoding: d.read_option("encoding")?,

                info: d.read_field("info")?,
            })
        })
    }
}

impl FromValue for FileInfo {
    fn from_value(value: &Value) -> Result<FileInfo, DecodeError> {
        Ok(FileInfo {
            length: value.get_field("length")?,
            md5sum: value.get_option_field("md5sum")?,
            path: value.get_field("path")?,
        })
    }
}

fn insert<T: IntoValue>(map: &mut BTreeMap<String, Value>, s: &str, t: T) {
    map.insert(s.to_string(), IntoValue::into_value(t));
}

fn insert_option<T: IntoValue>(map: &mut BTreeMap<String, Value>, s: &str, t: Option<T>) {
    match t {
        Some(v) => map.insert(s.to_string(), IntoValue::into_value(v)),
        None => None,
    };
}

impl IntoValue for FileInfo {
    fn into_value(self) -> Value {
        let mut map = BTreeMap::new();

        insert(&mut map, "length", self.length);
        insert_option(&mut map, "md5sum", self.md5sum);
        insert(&mut map, "path", self.path);

        Value::Dict(map)
    }
}

impl FromValue for Info {
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        match value.get_option_value("files")? {
            None => Ok(Single(SingleInfo {
                length: value.get_field("length")?,
                md5sum: value.get_option_field("md5sum")?,
                name: value.get_field("name")?,
                piece_length: value.get_field("piece length")?,
                pieces: value.get_field("pieces")?,
            })),
            Some(_) => Ok(Multi(MultiInfo {
                files: value.get_field("files")?,
                name: value.get_field("name")?,
                piece_length: value.get_field("piece length")?,
                pieces: value.get_field("pieces")?,
            })),
        }
    }
}

impl IntoValue for Info {
    fn into_value(self) -> Value {
        let mut map = BTreeMap::new();

        match self {
            Multi(multi) => {
                insert(&mut map, "files", multi.files);
                insert(&mut map, "pieces", multi.pieces);
                insert(&mut map, "name", multi.name);
                insert(&mut map, "piece length", multi.piece_length);
            }
            Single(single) => {
                insert_option(&mut map, "md5sum", single.md5sum);
                insert(&mut map, "length", single.length);
                insert(&mut map, "pieces", single.pieces);
                insert(&mut map, "name", single.name);
                insert(&mut map, "piece length", single.piece_length);
            }
        };
        Value::Dict(map)
    }
}

impl FromValue for TorrentMetaInfo {
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        Ok(TorrentMetaInfo {
            info: value.get_field("info")?,
            announce: value.get_option_field("announce")?,
            announce_list: value.get_option_field("announce-list")?,
            creation_date: None,
            comment: value.get_option_field("comment")?,
            created_by: value.get_option_field("created by")?,
            encoding: value.get_option_field("encoding")?,
        })
    }
}

impl IntoValue for TorrentMetaInfo {
    fn into_value(self) -> Value {
        let mut map = BTreeMap::new();

        insert_option(&mut map, "announce", self.announce);
        insert_option(&mut map, "announce-list", self.announce_list);
        insert_option(&mut map, "comment", self.comment);
        insert_option(&mut map, "created by", self.created_by);
        // Todo: instant
        //        insert_option(&mut map, "creation date", None);
        insert_option(&mut map, "encoding", self.encoding);
        insert(&mut map, "info", self.info);

        Value::Dict(map)
    }
}

#[cfg(test)]
mod tests {
    use crate::base::meta_info::{Info, TorrentMetaInfo};
    use crate::bencode::decode::{DecodeError, DecodeTo, Decoder};
    use crate::bencode::value::{FromValue, IntoValue, Value};
    use std::fs;

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn decode() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        ).unwrap();

        //        let f = fs::read(
        //            r#"D:\MyVideo\电影\里\3_xunlei\[脸肿字幕组][魔人]euphoria 目指す楽園は神聖なる儀式の先に。救世主の母は……白夜凛音！？ 編\.0D787B3AF81663F1CF1FC1EDF397DFE6F012829A.torrent"#
        //        ).unwrap();

        let mut decoder = Decoder::new(f.as_slice());

        let v = TorrentMetaInfo::decode(&mut decoder);

        match v {
            Ok(value) => println!("{:#?}", value),
            Err(e) => println!("{}", e.to_string()),
        };
    }

    #[test]
    fn torrent_meta_info() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        ).unwrap();

        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        if let Info::Multi(multi_info) = torrent_meta_info.info {
            println!("piece length: {}", multi_info.piece_length / 1024 / 1024)
        }
    }

    #[test]
    fn from_value() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let v = Value::decode(&mut decoder).unwrap();

        let s: Result<TorrentMetaInfo, DecodeError> = FromValue::from_value(&v);

        match s {
            Ok(value) => println!("{:#?}", value),
            Err(e) => println!("error: {}", e),
        };
    }

    #[test]
    fn into_value() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let v = Value::decode(&mut decoder).unwrap();

        let s: TorrentMetaInfo = FromValue::from_value(&v).unwrap();
        let v = IntoValue::into_value(s);

        let s: TorrentMetaInfo = FromValue::from_value(&v).unwrap();
        println!("{:#?}", s);
    }

    #[test]
    fn pieces_files() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        ).unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let v = Value::decode(&mut decoder).unwrap();

        let s: TorrentMetaInfo = FromValue::from_value(&v).unwrap();

        if let Info::Multi(m) = s.info {
            let piece_num = m.pieces.len();
            let piece_len = m.piece_length;

            let per_file_piece_sum = m
                .files
                .iter()
                .map(|f| (f.length as f64 / piece_len as f64).ceil() as usize)
                .sum::<usize>();

            let files_piece_sum = m
                .files
                .iter()
                .map(|f| (f.length as usize / piece_len) + 1)
                .sum::<usize>();

            println!("{}, {}, {}", per_file_piece_sum, files_piece_sum, piece_num)
        }
    }
}
