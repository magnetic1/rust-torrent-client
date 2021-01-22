use core::fmt;
use std::str::FromStr;

pub mod decode;
pub mod encode;
pub mod hash;
pub mod value;

/// An integer type that can be encoded and decoded.
pub unsafe trait Integer: Copy + fmt::Display + FromStr {}

macro_rules! impl_integer {
    ( $( $ty:ident )* ) => {
        $( unsafe impl Integer for $ty {} )*
    }
}

impl_integer! { u8 u16 u32 u64 usize i8 i16 i32 i64 isize }
