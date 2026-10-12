//! Test-only ingress to an offline container. The destination is fixed by the runner.
use std::net::{TcpListener, TcpStream};
fn main() {
    let destination = std::env::args().nth(1).expect("fixed destination");
    for connection in TcpListener::bind("0.0.0.0:3000").unwrap().incoming() {
        let destination = destination.clone();
        std::thread::spawn(move || {
            let mut incoming = connection.unwrap();
            let mut outgoing = TcpStream::connect(destination).unwrap();
            let mut reverse_in = incoming.try_clone().unwrap();
            let mut reverse_out = outgoing.try_clone().unwrap();
            let reverse = std::thread::spawn(move || {
                let _ = std::io::copy(&mut reverse_out, &mut reverse_in);
                let _ = reverse_in.shutdown(std::net::Shutdown::Write);
            });
            let _ = std::io::copy(&mut incoming, &mut outgoing);
            let _ = outgoing.shutdown(std::net::Shutdown::Write);
            let _ = reverse.join();
        });
    }
}
