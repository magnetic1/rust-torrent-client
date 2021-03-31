use std::sync::{Mutex, Arc};
use std::thread;

#[test]
fn test() {
    #[derive(Debug)]
    struct Person {
        name: Arc<Mutex<String>>,
        address: Arc<Mutex<String>>
    }

    let mut person = Person {
        name: Arc::new(Mutex::new("".to_string())),
        address: Arc::new(Mutex::new("".to_string()))
    };
    let name = person.name.clone();
    let address = person.address.clone();

    let handle1 = thread::spawn(move || {
        let mut a = address.lock().unwrap();
        a.push_str("beijing")
    });
    let handle2 = thread::spawn(move || {
        let mut a = name.lock().unwrap();
        a.push_str("Bob")
    });
    handle1.join();
    handle2.join();

    println!("{:?}", person);
}