//! Asks the server whether it answers: `GET /_api/health` on the loopback.
//! The admin console is asked the same way, at its own address.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

const WAIT: Duration = Duration::from_millis(800);

pub fn answers(port: u16) -> bool {
    answers_at(port, "/_api/health")
}

/// Whether `GET path` on the loopback's `port` is answered with a 200.
pub fn answers_at(port: u16, path: &str) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, WAIT) else {
        return false;
    };
    let _ = stream.set_read_timeout(Some(WAIT));
    let _ = stream.set_write_timeout(Some(WAIT));
    let request = format!("GET {path} HTTP/1.0\r\nHost: 127.0.0.1:{port}\r\n\r\n");
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut head = [0u8; 16];
    let mut got = 0;
    while got < head.len() {
        match stream.read(&mut head[got..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => got += n,
        }
    }
    is_ok(&head[..got])
}

/// `HTTP/1.x 200`: anything else on the port is not a server that is up.
fn is_ok(head: &[u8]) -> bool {
    head.starts_with(b"HTTP/1.") && head.get(8..12) == Some(b" 200")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn only_a_200_counts() {
        assert!(is_ok(b"HTTP/1.1 200 OK\r"));
        assert!(!is_ok(b"HTTP/1.1 503 Ser"));
        assert!(!is_ok(b"SSH-2.0-OpenSSH"));
        assert!(!is_ok(b""));
    }

    #[test]
    fn a_closed_port_does_not_answer() {
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(!answers(port));
    }

    #[test]
    fn a_server_that_says_200_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0u8; 256];
            let _ = s.read(&mut buf);
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}");
        });
        assert!(answers(port));
    }
}
