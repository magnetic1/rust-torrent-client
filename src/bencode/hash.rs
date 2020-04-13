use std::ops::Deref;
use std::ptr;
use std::fmt::{Debug, Formatter, Error};
use core::fmt;
use crypto::digest::Digest;

#[derive(Eq, PartialEq, Clone)]
pub struct Sha1(pub [u8; 20]);

impl Sha1 {
    //todo: remove dependency
    pub fn calculate_sha1(input: &[u8]) -> Sha1 {
        let mut hasher = crypto::sha1::Sha1::new();
        hasher.input(input);

        let mut buf: Vec<u8> = vec![0; hasher.output_bytes()];
        hasher.result(&mut buf);

        let sha1 = Sha1::to_sha1list(&buf).remove(0);
        return sha1;
    }

    /// Returns the SHA1 hash as a string of hexadecimal digits.
    pub fn to_hex(&self) -> String {
        static HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";
        let mut buf = [0; 40];

        for (i, &b) in self.0.iter().enumerate() {
            buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
            buf[i * 2 + 1] = HEX_CHARS[(b & 0xf) as usize];
        }

        unsafe { String::from_utf8_unchecked(buf.to_vec()) }
    }


    pub fn to_sha1list(bytes: &[u8]) -> Vec<Sha1> {
        let len = 20;
        let mut i = 20;
        let mut res = Vec::with_capacity(bytes.len() / 20);

        while i <= bytes.len() {
            let mut t = [0; 20];
            unsafe {
                ptr::copy_nonoverlapping(
                    bytes.as_ptr().add( i - len), t.as_mut_ptr(), len);
            }

            let sha1 = Sha1(t);
            res.push(sha1);
            i += len;
        }
        res
    }
}

pub fn to_hex(bytes: &[u8]) -> String {
    static HEX_CHARS: &'static [u8; 16] = b"0123456789abcdef";
    let mut buf = vec![0; bytes.len() * 2];

    for (i, &b) in bytes.iter().enumerate() {
        buf[i * 2] = HEX_CHARS[(b >> 4) as usize];
        buf[i * 2 + 1] = HEX_CHARS[(b & 0xf) as usize];
    }

    unsafe { String::from_utf8_unchecked(buf.to_vec()) }
}

impl Deref for Sha1 {
    type Target = [u8; 20];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Debug for Sha1 {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Display for Sha1 {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), Error> {
        write!(f, "{}", self.to_hex())
    }
}

#[test]
fn hash_test() {
    let hash = Sha1([(16 * 16 - 1) as u8; 20]);
    assert_eq!(hash.to_hex(), String::from_utf8(vec![b'f'; 40]).unwrap());

    let vec = [(16 * 16 - 1) as u8; 60].to_vec();
    let sha1_vec = Sha1::to_sha1list(&vec);
    assert_eq!(sha1_vec, vec![hash.clone(); 3]);

    println!("{:#}", hash)
//
//    let mut vec = vec![1, 2, 3];
//    let s = (0..j).chain(1..3).all();
//    let t= unsafe { vec.as_mut_ptr().add(1).read() };
}

#[test]
fn list_test() {
//    let mut list = [["123", "123", "123"]];
//    let res :&[&str] = list.iter().flat_map(|elem| {*elem}).collect();
//    println!("{:?}", res);

    let mut list = ["1", "2", "3"];
    swap_to_head(&mut list, 0);
    assert_eq!(list, ["1", "2", "3"]);

    swap_to_head(&mut list, 1);
    assert_eq!(list, ["2", "1", "3"]);

    swap_to_head(&mut list, 2);
    assert_eq!(list, ["3", "2", "1", ]);
}


fn swap_to_head<T>(list: &mut [T], n: usize) {
    for i in 0..n {
        list.swap(n - i, n - i - 1);
    }
}

