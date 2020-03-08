use std::collections::BTreeMap;
use core::fmt;
use std::fmt::{Formatter, Error};
use crate::bencode::hash::Sha1;
use crate::bencode::decode::{DecodeTo, Decoder, DecodeError};

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

// TODO: pretty format
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