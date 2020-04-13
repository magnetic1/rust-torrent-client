use std::fs::OpenOptions;
use std::path::Path;
use std::io;
use std::io::{Seek, Write};

#[test]
fn io_test() {
    // create/open file
    let path = Path::new("download");
    let mut file = OpenOptions::new().create(true).read(true).write(true).open(path).unwrap();

    let s = file.seek(io::SeekFrom::Start(1<<12)).unwrap();

    file.write_all(&[b'i']).unwrap();
}