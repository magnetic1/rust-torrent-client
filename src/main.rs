use base::{Value, Decoder, DecodeTo};
use std::fs;


fn main() {
    println!("Hello, world!");

//    let s = "d8:announce34:http://tracker.ydy.com:86/announce10:createdby13:BitComet/0.5813:creationdatei1117953113e8:encoding3:GBK4:infod6:lengthi474499162e4:name51:05.262005.StarWars Episode IV A New Hope-Rv9.rmvb10:name.utf-851:05.26.2005.Star WasEpisode IV A New Hope-Rv9.rmvb12:piecelengthi262144e6:pieces36220:XXXXXXXXXXXXXXX";
//    let mut _decoder = Decoder::new(s.as_bytes());

    let f = fs::read(
        "D:/MyVideo/犬夜叉部剧场版[全]/F767AB595A8E5E2162A881D4FE9BF3B4330BF603.torrent"
    ).unwrap();
    let mut decoder = Decoder::new(f.as_slice());

    let v = Value::decode(&mut decoder);

    match v {
        Ok(value) => println!("{:#}", value),
        Err(e) => println!("error: {}", e),
    };
}
