use std::fs::OpenOptions;
use std::path::Path;
use std::{io, fs};
use std::io::{Seek, Write};
use async_std::fs::Metadata;
use async_std::io::Error;

#[test]
fn io_test() {
    // create/open file
    let path = Path::new("download");
    let file = Path::new("download/123.temp");

    fs::create_dir_all(path).unwrap();
    // fs::create_dir_all(path).unwrap();
    let mut file = OpenOptions::new().create(true).read(true).write(true).open(file).unwrap();


    let s = file.seek(io::SeekFrom::Start(1<<12)).unwrap();

    file.write_all(&[b'i']).unwrap();
}

#[test]
fn dirs_test() {
    let files = vec!["download/we/123.temp", "download/sr/567.temp"];

    for path in files {
        let t: Vec<&str>= path.split("/").collect();
        if t.len() > 1 {
            let s = t[..t.len()-1].join("/");
            match fs::metadata(s.clone()) {
                Ok(_) => {},
                Err(_) => {
                    println!("{}", s);
                    fs::create_dir_all(s).unwrap()
                },
            }
        }
    }
}

#[test]
fn read_test() {
    let w = vec![2, 3];
    let buf = vec![0, 4];

    w.readv()
}