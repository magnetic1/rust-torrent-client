use crate::bencode::decode::{DecodeError, DecodeTo, Decoder};
use crate::bencode::hash::Sha1;
use core::fmt;
use std::collections::BTreeMap;
use std::fmt::{Error, Formatter};

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
    pub fn get_value(&self, key: &str) -> Result<&Value, DecodeError> {
        match self {
            Value::Dict(map) => {
                return map
                    .get(key)
                    .ok_or(DecodeError::MissingField(key.to_string()));
            }
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_option_value(&self, key: &str) -> Result<Option<&Value>, DecodeError> {
        match self {
            Value::Dict(map) => {
                return Ok(map.get(key));
            }
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_field<T: FromValue>(&self, key: &str) -> Result<T, DecodeError> {
        match self {
            Value::Dict(map) => {
                return match map
                    .get(key)
                    .ok_or(DecodeError::MissingField(key.to_string()))
                {
                    Ok(v) => T::from_value(v),
                    Err(e) => Err(e),
                };
            }
            _ => Err(DecodeError::InvalidDict),
        }
    }

    pub fn get_option_field<T: FromValue>(&self, key: &str) -> Result<Option<T>, DecodeError> {
        match self {
            Value::Dict(map) => {
                return match map.get(key) {
                    Some(v) => T::from_value(v).and_then(|s| Ok(Some(s))),
                    None => Ok(None),
                };
            }
            _ => Err(DecodeError::InvalidDict),
        }
    }
}

// TODO: pretty format
impl fmt::Display for Value {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        match self {
            Value::Bytes(s) => {
                //                let s = hex::encode(s);
                let sha1_list = Sha1::sha1_list(s);
                //                println!("{}", s);
                write!(f, "(\n")?;
                for sha1 in sha1_list {
                    write!(f, "{}\n", sha1.to_hex())?;
                }
                write!(f, ")\n")
            }
            Value::Integer(i) => write!(f, "{}", i),
            Value::String(s) => write!(f, "{}", s),
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

impl DecodeTo for Value {
    fn decode(d: &mut Decoder<'_>) -> Result<Value, DecodeError> {
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
            b => {
                println!("{}", String::from_utf8_lossy(d.get_ref()));
                Err(DecodeError::InvalidByte(b))
            },
        }
    }
}

pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, DecodeError>;
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<String, DecodeError> {
        if let Value::Bytes(bytes) = value {
            return String::from_utf8(bytes.clone())
                .ok()
                .ok_or(DecodeError::InvalidUtf8);
        };
        if let Value::String(s) = value {
            return Ok(s.to_string());
        };
        Err(DecodeError::ExtraneousData)
    }
}

impl FromValue for Vec<Sha1> {
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        if let Value::Bytes(bytes) = value {
            return Ok(Sha1::sha1_list(&bytes));
        };
        Err(DecodeError::ExtraneousData)
    }
}

impl<T: FromValue> FromValue for Vec<T> {
    fn from_value(value: &Value) -> Result<Self, DecodeError> {
        if let Value::List(list) = value {
            let mut res = Vec::with_capacity(list.len());

            for item in list {
                let t = FromValue::from_value(item)?;
                res.push(t);
            }
            return Ok(res);
        };
        Err(DecodeError::ExtraneousData)
    }
}

macro_rules! impl_from_value_integer {
    ( $( $ty:ident )* ) => {
        $(
            impl FromValue for $ty {
                fn from_value(value: &Value) -> Result<Self, DecodeError> {
                    if let Value::Integer(i) = value {
                        return Ok(*i as Self)
                    };
                    Err(DecodeError::ExtraneousData)
                }
            }
        )*
    }
}

impl_from_value_integer! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize }

pub trait IntoValue {
    fn into_value(self) -> Value;
}

impl IntoValue for String {
    fn into_value(self) -> Value {
        Value::String(self)
    }
}

impl IntoValue for Vec<Sha1> {
    fn into_value(self) -> Value {
        let len = 20;
        let mut bytes: Vec<u8> = Vec::with_capacity(self.len() * len);

        for sha1 in self {
            bytes.extend_from_slice(&(sha1[..]));
        }
        Value::Bytes(bytes)
    }
}

impl<T: IntoValue> IntoValue for Vec<T> {
    fn into_value(self) -> Value {
        let res = self.into_iter().map(|item| item.into_value()).collect();

        Value::List(res)
    }
}

macro_rules! impl_into_value_integer {
    ( $( $ty:ident )* ) => {
        $(
            impl IntoValue for $ty {
                fn into_value(self) -> Value {
                    Value::Integer(self as i64)
                }
            }
        )*
    }
}

impl_into_value_integer! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize }

#[cfg(test)]
mod test {
    use crate::base::meta_info::{Info, TorrentMetaInfo};
    use crate::bencode::decode::{DecodeError, DecodeTo, Decoder};
    use crate::bencode::value::{FromValue, IntoValue, Value};

    #[test]
    fn from_value() {
        let v = Value::Integer(4194304i64);
        let f: usize = FromValue::from_value(&v).unwrap();
        println!("{}", f);

        let bytes = vec![b'5', b'1'];
        let v = Value::Bytes(bytes);
        let f: String = FromValue::from_value(&v).unwrap();
        println!("{}", f);

        let values = vec![Value::Integer(1), Value::Integer(2)];
        let v = Value::List(values);
        let f: Vec<u8> = FromValue::from_value(&v).unwrap();
        println!("{}", f[1]);
    }

    #[test]
    fn into_value() {
        let f = std::fs::read(
            // "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent"
            // r#"C:\Users\12287\Downloads\[桜都字幕组][碧蓝航线_Azur Lane][01-12 END][GB][1080P].torrent"#
            r#"C:\Users\wzq\Downloads\[K&W][Gundam Build Divers Re-RISE][PV-Just before resumed!-][BIG5][720P][x264_AAC].mp4.torrent"#
        ).unwrap();

        let mut decoder = Decoder::new(f.as_slice());

        let v = Value::decode(&mut decoder).unwrap();

        let s: Result<TorrentMetaInfo, DecodeError> = FromValue::from_value(&v);

        match s {
            Ok(meta_info) => {
                println!("{:#?}", meta_info);
                if let Info::Multi(multi_info) = meta_info.info {
                    let _v = multi_info.pieces.into_value();
                    // println!("{}", v);
                    println!(
                        "{}",
                        (multi_info.files[0].length / multi_info.piece_length as u64)
                    );
                };
            }
            Err(e) => println!("error: {}", e),
        };
    }
}
