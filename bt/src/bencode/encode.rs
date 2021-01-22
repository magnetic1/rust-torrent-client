use crate::bencode::value::Value;
use crate::bencode::Integer;
use std::collections::BTreeMap;
use std::fmt::Debug;
use std::io::Write;

/// Encodes values into a stream of bytes.
#[derive(Clone)]
pub struct Encoder {
    data: Vec<u8>,
}

/// Represents an error in an encoding operation.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EncodeError {
    // There are no encoding errors, but this exists in case we ever have any.
}

pub trait EncodeTo {
    fn encode(&self, encoder: &mut Encoder) -> Result<(), EncodeError>;
}

impl Encoder {
    pub fn new() -> Encoder {
        Encoder { data: Vec::new() }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }

    /// Writes a single byte to the stream.
    pub fn write_byte(&mut self, b: u8) -> Result<(), EncodeError> {
        self.data.push(b);
        Ok(())
    }

    /// Writes a single byte to the stream.
    pub fn write(&mut self, b: &[u8]) -> Result<(), EncodeError> {
        self.data.write(b).unwrap();
        Ok(())
    }

    /// Writes an integer value to the stream.
    pub fn write_integer<T: Integer>(&mut self, t: T) -> Result<(), EncodeError> {
        self.write_byte(b'i')?;
        self.write_number(t)?;
        self.write_byte(b'e')
    }

    /// Writes a number to the stream.
    /// This does not include `i` prefix and `e` suffix.
    pub fn write_number<T: Integer>(&mut self, t: T) -> Result<(), EncodeError> {
        self.write(format!("{}", t).as_bytes())
    }

    /// Writes a byte string to the stream.
    pub fn write_bytes(&mut self, b: &[u8]) -> Result<(), EncodeError> {
        self.write_number(b.len())?;
        self.write_byte(b':')?;
        self.write(b)
    }

    /// Writes a UTF-8 encoded string to the stream.
    pub fn write_str(&mut self, s: &str) -> Result<(), EncodeError> {
        self.write_bytes(s.as_bytes())
    }

    /// Writes a key value mapping to the stream.
    pub fn write_dict<K, V>(&mut self, map: &BTreeMap<K, V>) -> Result<(), EncodeError>
    where
        K: Ord + AsRef<str>,
        V: EncodeTo,
    {
        self.write_byte(b'd')?;
        for (k, v) in map.iter() {
            self.write_str(k.as_ref())?;
            v.encode(self)?;
        }
        self.write_byte(b'e')
    }

    pub fn write_list<T: EncodeTo>(&mut self, t: &[T]) -> Result<(), EncodeError> {
        self.write_byte(b'l')?;

        for v in t {
            v.encode(self)?;
        }

        self.write_byte(b'e')
    }
}

impl EncodeTo for Value {
    fn encode(&self, encoder: &mut Encoder) -> Result<(), EncodeError> {
        match self {
            Value::Integer(i) => encoder.write_integer(*i),
            Value::Bytes(bytes) => encoder.write_bytes(bytes),
            Value::String(s) => encoder.write_str(s),
            Value::List(list) => encoder.write_list(list),
            Value::Dict(d) => encoder.write_dict(d),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::bencode::decode::{DecodeTo, Decoder};
    use crate::bencode::encode::{EncodeTo, Encoder};
    use crate::bencode::value::Value;
    use std::fs;

    #[test]
    fn encode_test() {
        let f = fs::read(
            "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent",
        )
        .unwrap();
        let mut decoder = Decoder::new(f.as_slice());

        let mut encoder = Encoder::new();

        let mut value = Value::decode(&mut decoder).unwrap();
        match &mut value {
            Value::Dict(map) => map.remove("info"),
            _ => None,
        };
        value.encode(&mut encoder).unwrap();
        println!("{}", String::from_utf8(encoder.into_bytes()).unwrap())
    }
}
