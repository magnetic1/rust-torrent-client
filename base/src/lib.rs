use std::collections::BTreeMap;
use core::fmt;
use std::io::{Cursor, Read};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::fmt::{Formatter, Error, Debug};
use crate::hash::Sha1;

pub mod hash;

pub const PROJECT_NAME: &'static str = "rustorrent";

pub struct Decoder<'a> {
    data: Cursor<&'a [u8]>,
}

impl<'a> Deref for Decoder<'a> {
    type Target = Cursor<&'a [u8]>;
    fn deref(&self) -> &Self::Target {
        &self.data
    }
}

impl<'a> DerefMut for Decoder<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.data
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Value {
    /// Integer value
    Integer(i64),
    /// Byte string value
    Bytes(Vec<u8>),
    /// UTF-8 string value
    String(String),
    /// List value
    List(Vec<Value>),
    /// Dictionary value
    Dict(BTreeMap<String, Value>),
}

impl Value {
    pub fn get_map(&self) -> Result<&BTreeMap<String, Value>, DecodeError> {
        match self {
            Value::Dict(map) => Ok(map),
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_list(&self) -> Result<&Vec<Value>, DecodeError> {
        match self {
            Value::List(list) => Ok(list),
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_bytes(&self) -> Result<&Vec<u8>, DecodeError> {
        match self {
            Value::Bytes(bytes) => Ok(bytes),
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_string(&self) -> Result<&str, DecodeError> {
        match self {
            Value::String(s) => Ok(s),
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_usize(&self) -> Result<usize, DecodeError> {
        match self {
            Value::Integer(i) => if *i > 0 {
                Ok(1)
            } else {
                Err(DecodeError::InvalidDict)
            }
            _ => Err(DecodeError::InvalidDict),
        }
    }
}

/// Represents an error in a decoding operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecodeError {
    /// End of bytes reached before expected
    Eof,
    /// Extraneous data at the end of the stream
    ExtraneousData,
    /// Unexpected byte
    InvalidByte(u8),
    /// Duplicate or out-of-order key in a dict
    InvalidDict,
    /// Invalid formatted number
    InvalidNumber,
    /// Invalid UTF-8 in a string
    InvalidUtf8,
    /// Field not found while decoding `struct`
    MissingField(String),
    /// Unexpected byte encountered
    UnexpectedByte {
        /// Byte expected
        expected: u8,
        /// Byte found
        found: u8,
    },
}

impl fmt::Display for DecodeError {
    // TODO: pretty format
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            DecodeError::Eof => f.write_str("unexpected end-of-file"),
            DecodeError::ExtraneousData => f.write_str("extraneous data"),
            DecodeError::InvalidByte(b) => write!(f, "invalid byte {:?}", b),
            DecodeError::InvalidDict => f.write_str("invalid dict"),
            DecodeError::InvalidNumber => f.write_str("invalid number"),
            DecodeError::InvalidUtf8 => f.write_str("invalid utf-8"),
            DecodeError::MissingField(s) => write!(f, "missing field: {}", s),
            DecodeError::UnexpectedByte { expected, found } =>
                write!(f, "expected byte {:?}, found {:?}", *expected as char, *found as char),
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        match self {
            Value::Bytes(s) => {

//                let s = hex::encode(s);
                let sha1_list = Sha1::to_sha1list(s);
//                println!("{}", s);
                write!(f, "(\n")?;
                for sha1 in sha1_list {
                    write!(f, "{}\n", sha1.to_hex())?;
                }
                write!(f, ")\n")
            }
            Value::Integer(i) => {
                write!(f, "{}", i)
            }
            Value::String(s) => {
                write!(f, "{}", s)
            }
            Value::List(list) => {
                write!(f, "List(\n")?;
                for item in list {
                    write!(f, "\t\t{}\n", item)?;
                }
//                f.pad("\t");
                write!(f, "\t)\n")
            }
            Value::Dict(map) => {
                write!(f, "Dict(\n")?;
                for (k, v) in map {
                    write!(f, "\t\t{}: {}\n", k, v)?;
                }
                write!(f, "\t)\n")
            }
        }
    }
}


impl<'a> Decoder<'a> {
    pub fn new(data: &[u8]) -> Decoder {
        Decoder { data: Cursor::new(data) }
    }

    /// Returns the number of bytes remaining in the stream.
    pub fn remaining(&self) -> usize {
        self.get_ref().len() - self.position() as usize
    }

    /// Returns an error if there is data remaining in the stream.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(DecodeError::ExtraneousData)
        }
    }

    /// Returns an error if the next byte is not `byte`.
    pub fn expect(&mut self, byte: u8) -> Result<(), DecodeError> {
        let b = self.read_byte()?;
        if b == byte {
            Ok(())
        } else {
            Err(DecodeError::UnexpectedByte { expected: byte, found: b })
        }
    }

    /// Reads a series of bytes from the stream equal to `buf.len()`.
    /// If fewer bytes are available to read, an error is returned.
    pub fn read(&mut self, buf: &mut [u8]) -> Result<(), DecodeError> {
        match Cursor::read(self, buf) {
            Ok(n) if n == buf.len() => Ok(()),
            _ => Err(DecodeError::Eof)
        }
    }

    /// Reads a single byte from the stream. If no bytes are available to read,
    /// an error is returned.
    pub fn read_byte(&mut self) -> Result<u8, DecodeError> {
        let mut b = [0];
        self.read(&mut b)?;
        Ok(b[0])
    }

    /// Returns a slice of bytes without advancing the cursor.
    /// If fewer than `n` bytes are available, an error is returned.
    pub fn peek_bytes(&self, n: usize) -> Result<&[u8], DecodeError> {
        let pos = self.position() as usize;
        let data = self.get_ref();

        if data.len() < pos + n {
            Err(DecodeError::Eof)
        } else {
            Ok(&data[pos..pos + n])
        }
    }

    /// Reads a single byte from the stream without advancing the cursor.
    pub fn peek_byte(&self) -> Result<u8, DecodeError> {
        let byte = self.peek_bytes(1)?;
        Ok(byte[0])
    }

    /// Reads bytes from the stream until `predicate` returns `false`.
    pub fn read_while<P>(&mut self, predicate: P) -> Result<Vec<u8>, DecodeError>
        where P: Fn(u8) -> bool
    {
        let mut res = Vec::new();

        loop {
            let b = self.peek_byte()?;
            if !predicate(b) { break; }
            res.push(self.read_byte()?);
        }

        Ok(res)
    }

    /// Reads a number from the stream.
    /// This does not include the `i` prefix and `e` suffix.
    pub fn read_number<T: FromStr>(&mut self) -> Result<T, DecodeError> {
        let buf = self.read_while(is_number)?;

        if buf.is_empty() || (buf.len() > 1 && buf[0] == b'0') || buf == b"-0" {
            return Err(DecodeError::InvalidNumber);
        }
        String::from_utf8(buf).ok().and_then(|s| s.parse().ok())
            .ok_or(DecodeError::InvalidNumber)
    }

    /// Advance bytes in the stream until `predicate` returns `false`.
    pub fn skip_while<P>(&mut self, predicate: P) -> Result<(), DecodeError>
        where P: Fn(u8) -> bool {
        while predicate(self.peek_byte()?) {
            self.read_byte()?;
        }
        Ok(())
    }

    /// Advances the cursor `n` bytes.
    pub fn skip(&mut self, n: usize) -> Result<(), DecodeError> {
        let pos = self.position();
        if self.get_ref().len() < pos as usize + n {
            Err(DecodeError::Eof)
        } else {
            self.set_position(pos + n as u64);
            Ok(())
        }
    }

    /// Advances the cursor beyond the current value.
    pub fn skip_item(&mut self) -> Result<(), DecodeError> {
        match self.peek_byte()? {
            b'd' => {
                self.read_byte()?;
                while self.peek_byte()? != b'e' {
                    self.skip_item()?;
                    self.skip_item()?;
                }
                self.expect(b'e')
            }
            b'l' => {
                self.read_byte()?;
                while self.peek_byte()? != b'e' {
                    self.skip_item()?;
                }
                self.expect(b'e')
            }
            b'i' => {
                self.expect(b'i')?;
                self.skip_while(is_number)?;
                self.expect(b'e')
            }
            b'0'..=b'9' => {
                let n = self.read_number()?;
                self.expect(b':')?;
                self.skip(n)?;
                Ok(())
            }
            b => Err(DecodeError::InvalidByte(b))
        }
    }

    /// Reads a byte string from the stream.
    pub fn read_byte_string(&mut self) -> Result<Vec<u8>, DecodeError> {
        let n: usize = self.read_number()?;
        self.expect(b':')?;

        if self.remaining() < n {
            return Err(DecodeError::Eof);
        }

        let mut buf = vec![0; n];
        self.read(&mut buf)?;
        Ok(buf)
    }

    /// Reads a UTF-8 encoded string from the stream.
    pub fn read_str(&mut self) -> Result<String, DecodeError> {
        String::from_utf8(self.read_byte_string()?)
            .map_err(|_| DecodeError::InvalidUtf8)
    }

    /// Reads an integer value from the stream.
    pub fn read_integer<T: FromStr>(&mut self) -> Result<T, DecodeError> {
        self.expect(b'i')?;
        let n = self.read_number()?;
        self.expect(b'e')?;

        Ok(n)
    }

    /// Reads a key value mapping from the stream.
    pub fn read_dict<T: DecodeTo>(&mut self)
                                  -> Result<BTreeMap<String, T>, DecodeError> {
        self.expect(b'd')?;
        let mut map = BTreeMap::new();

        while self.peek_byte()? != b'e' {
            let key = self.read_str()?;

            // Ensure that this key is greater than the greatest existing key
//            if !map.is_empty() {
//                let last: &String = map.keys().last().unwrap();
//                if key.as_bytes() <= last.as_bytes() {
//                    return Err(DecodeError::InvalidDict);
//                }
//            }

            let value = DecodeTo::decode(self)?;
            map.insert(key, value);
        }

        self.expect(b'e')?;
        Ok(map)
    }

    /// Reads a series of values from the stream.
    pub fn read_list<T: DecodeTo>(&mut self) -> Result<Vec<T>, DecodeError> {
        self.expect(b'l')?;
        let mut list = Vec::new();

        while self.peek_byte()? != b'e' {
            list.push(DecodeTo::decode(self)?);
        }

        self.expect(b'e')?;
        Ok(list)
    }

    /// Reads a key value mapping from the stream as a `struct`.
    ///
    /// The given callable is expected to call `read_field` for each field
    /// and `read_option` for any optional fields, in lexicographical order.
    pub fn read_struct<T, F>(&mut self, f: F) -> Result<T, DecodeError>
        where F: FnOnce(&mut Self) -> Result<T, DecodeError> {
        self.expect(b'd')?;
        let res = f(self)?;

        // Skip any additional fields
        while self.peek_byte()? != b'e' {
            self.skip_item()?;
            self.skip_item()?;
        }

        self.expect(b'e')?;
        Ok(res)
    }

    /// Reads a single field from the stream. Get the first one with the same name.
    pub fn read_field<T: DecodeTo>(&mut self, name: &str) -> Result<T, DecodeError> {
        let pos = self.position();

        while self.peek_byte()? != b'e' {
            let key = self.read_str()?;

            if name == key {
                return DecodeTo::decode(self)
            } else if &key[..] < name {
                // This key is less than name. name may be found later.
                self.skip_item()?;
            } else {
                // This key is greater than name.
                // We won't find name, so bail out now.
                break;
            }
        }

        self.set_position(pos);
        Err(DecodeError::MissingField(name.to_string()))
    }

    /// Reads a single field from the stream. Get the last one with the same name.
    pub fn read_last_field<T: DecodeTo>(&mut self, name: &str) -> Result<T, DecodeError> {
        let mut pos = self.position();
        let mut res = None;

        while self.peek_byte()? != b'e' {
            let key = self.read_str()?;

            if name == key {
                res = Some(DecodeTo::decode(self));
                pos = self.position();
            } else if &key[..] < name {
                // This key is less than name. name may be found later.
                self.skip_item()?;
            } else {
                // This key is greater than name.
                // We won't find name, so bail out now.
                break;
            }
        }

        self.set_position(pos);

        match res {
            Some(s) => s,
            None => Err(DecodeError::MissingField(name.to_string())),
        }
    }

    /// Reads a single field from the stream. Combine the fields with the same name.
    pub fn read_field_combine<T: DecodeTo>(&mut self, name: &str) -> Result<Option<Vec<T>>, DecodeError> {
        let mut pos = self.position();
        let mut res = Vec::new();

        while self.peek_byte()? != b'e' {
            let key = self.read_str()?;

            if name == key {
                let s: Vec<T> = DecodeTo::decode(self)?;
                let _temp : Vec<()> = s.into_iter().map(|item| {
                    res.push(item);
                }).collect();
                pos = self.position();
            } else if &key[..] < name {
                // This key is less than name. name may be found later.
                self.skip_item()?;
            } else {
                // This key is greater than name.
                // We won't find name, so bail out now.
                break;
            }
        }

        self.set_position(pos);

        if res.len() == 0 {
            Err(DecodeError::MissingField(name.to_string()))
        } else {
            Ok(Some(res))
        }
    }

    /// Reads an optional field from the stream.
    pub fn read_option<T: DecodeTo>(&mut self, name: &str)
                                    -> Result<Option<T>, DecodeError> {
        match self.read_field(name) {
            Ok(t) => Ok(Some(t)),
            Err(DecodeError::MissingField(_)) => Ok(None),
            Err(e) => Err(e)
        }
    }
}

pub trait DecodeTo: Sized {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError>;
}

impl DecodeTo for Value {
    fn decode(d: &mut Decoder) -> Result<Value, DecodeError> {
        match d.peek_byte()? {
            b'd' => Ok(Value::Dict(d.read_dict()?)),
            b'l' => Ok(Value::List(d.read_list()?)),
            b'i' => Ok(Value::Integer(d.read_integer()?)),
            b'0'..=b'9' => match String::from_utf8(d.read_byte_string()?) {
                Ok(s) => Ok(Value::String(s)),
                Err(e) => {
                    let bytes = e.into_bytes();
                    Ok(Value::Bytes(bytes))
                }
            },
            b => Err(DecodeError::InvalidByte(b))
        }
    }
}

/// Returns whether the given byte may appear in a number.
fn is_number(b: u8) -> bool {
    match b {
        b'-' | b'0'..=b'9' => true,
        _ => false
    }
}

impl<T: DecodeTo> DecodeTo for Vec<T> {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
        d.read_list()
    }
}

//impl DecodeTo for usize {
//    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
//        d.read_integer()
//    }
//}

impl DecodeTo for String {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
        d.read_str()
    }
}

impl DecodeTo for Vec<Sha1> {
    fn decode(d: &mut Decoder) -> Result<Self, DecodeError> {
        let bytes = d.read_byte_string()?;
        Ok(Sha1::to_sha1list(&bytes))
    }
}

macro_rules! impl_decodable_integer {
    ( $( $ty:ident )* ) => {
        $(
            impl DecodeTo for $ty {
                fn decode(d: &mut Decoder) -> Result<$ty, DecodeError> {
                    d.read_integer()
                }
            }
        )*
    }
}

impl_decodable_integer!{ u8 u16 u32 u64 usize i8 i16 i32 i64 isize }

pub mod meta_info {
    use std::time::Instant;
    use crate::{DecodeTo, DecodeError, Decoder};
    use crate::hash::Sha1;

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
}


#[cfg(test)]
mod tests {
    use std::fs;
    use crate::{Decoder, DecodeTo};
    use crate::meta_info::{TorrentMetaInfo, Info};
    use std::fs::File;
    use std::io::Read;
    use bencode::util::ByteString;

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
