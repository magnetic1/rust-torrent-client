use std::time::Instant;
use crate::bencode::decode::{DecodeTo, Decoder, DecodeError};
use crate::bencode::hash::Sha1;

#[derive(Debug)]
pub struct TorrentMetaInfo {
    pub info: Info,
    pub announce: Option<String>,
    pub announce_list: Option<Vec<Vec<String>>>,
    pub creation_date: Option<Instant>,
    pub comment: Option<String>,
    pub created_by: Option<String>,
    pub encoding: Option<String>,
}

#[derive(Debug)]
pub enum Info {
    Single(SingleInfo),
    Multi(MultiInfo),
}

#[derive(Debug)]
pub struct SingleInfo {
    pub length: usize,
    pub md5sum: Option<String>,
    pub name: String,
    // 字符串,BitTorrent下载路径中最上层的目录名
    pub piece_length: usize,
    // 整数,是BitTorrent文件块的大小
    pub pieces: Vec<Sha1>, // 字符串,连续的存放着所有块的SHA1杂凑值,每一个文件块的杂凑值为20字节
}

#[derive(Debug)]
pub struct MultiInfo {
    pub files: Vec<FileInfo>,
    pub name: String,
    pub piece_length: usize,
    pub pieces: Vec<Sha1>,
}

#[derive(Debug)]
pub struct FileInfo {
    pub length: usize,
    pub md5sum: Option<String>,
    pub path: Vec<String>,
}

impl DecodeTo for FileInfo {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
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
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
        d.read_struct(|d| {
            let files_result: Result<Vec<FileInfo>, DecodeError> = d.read_field("files");
            match files_result {
                Ok(files) => {
                    Ok(Info::Multi(MultiInfo {
                        files,
                        name: d.read_field("name")?,
                        piece_length: d.read_field("piece length")?,
                        pieces: d.read_field("pieces")?,
                    }))
                },
                _ => {
                    Ok(Info::Single(SingleInfo {
                        length: d.read_field("length")?,
                        md5sum: d.read_option("md5sum")?,
                        name: d.read_field("name")?,
                        piece_length: d.read_field("piece length")?,
                        pieces: d.read_field("pieces")?,
                    }))
                }
            }
        })
    }
}

impl DecodeTo for TorrentMetaInfo {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::fs::File;
    use std::io::Read;
    use bencode::util::ByteString;
    use crate::bencode::decode::{Decoder, DecodeTo};
    use crate::base::meta_info::{TorrentMetaInfo, Info};

    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }

    #[test]
    fn decode() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent"
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
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent"
        ).unwrap();

        let mut decoder = Decoder::new(f.as_slice());

        let torrent_meta_info = TorrentMetaInfo::decode(&mut decoder).unwrap();

        if let Info::Multi(multi_info) =  torrent_meta_info.info {

            println!("piece length: {}", multi_info.piece_length/1024/1024)
        }
    }

    #[test]
    fn bencode() {
        let filename = r#"D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent"#;
        println!("Loading {}", filename);

        // read the torrent file into a byte vector
        let mut f = File::open(filename).unwrap();
        let mut v = Vec::new();
        f.read_to_end(&mut v).unwrap();

        // decode the byte vector into a struct
        let bencode = bencode::from_vec(v).unwrap();

        if let bencode::Bencode::Dict(ref map) = bencode {
            let key: ByteString = bencode::util::ByteString::from_str("announce-list");
            let s = map.get(&key).unwrap();
            println!("{}", String::from_utf8(s.to_bytes().unwrap()).unwrap())
        }
//        let result = FromBencode::from_bencode(&bencode).unwrap();

    }
}