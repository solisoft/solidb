//! How the client reads response frames off the socket, against a server
//! that writes them the awkward ways a real one only does sometimes: in
//! pieces, several at once, larger than the client's buffer, cut short.
//!
//! The client reads each response into a buffer it keeps per connection
//! and decodes it there. These pin down that what callers get — rows and
//! errors — is what they got when it read the length and then the payload
//! with two `read_exact`s.

use serde_json::{json, Value};
use solidb_client::protocol::{
    decode_message, encode_response, Command, DriverError, Response, DRIVER_MAGIC, MAX_MESSAGE_SIZE,
};
use solidb_client::SoliDBClient;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// What the fake server does when request `n` (from 0) arrives.
enum Reply {
    /// Write these bytes, in pieces of these sizes (the rest in one).
    Pieces(Vec<u8>, Vec<usize>),
    /// Write nothing.
    Nothing,
    /// Close the connection after writing these bytes.
    Close(Vec<u8>),
}

fn frame(resp: &Response) -> Vec<u8> {
    encode_response(resp).unwrap()
}

/// A server for one connection, answering its requests with `replies`.
async fn serve(replies: Vec<Reply>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut magic = [0u8; 14];
        stream.read_exact(&mut magic).await.unwrap();
        assert_eq!(&magic[..], DRIVER_MAGIC);
        for reply in replies {
            read_request(&mut stream).await;
            match reply {
                Reply::Nothing => {}
                Reply::Close(bytes) => {
                    stream.write_all(&bytes).await.unwrap();
                    return;
                }
                Reply::Pieces(bytes, sizes) => {
                    let mut rest = &bytes[..];
                    for size in sizes {
                        let (piece, tail) = rest.split_at(size.min(rest.len()));
                        stream.write_all(piece).await.unwrap();
                        stream.flush().await.unwrap();
                        tokio::time::sleep(Duration::from_millis(5)).await;
                        rest = tail;
                    }
                    stream.write_all(rest).await.unwrap();
                }
            }
        }
        // Hold the connection open until the client is done with it.
        let mut sink = Vec::new();
        let _ = stream.read_to_end(&mut sink).await;
    });
    addr
}

async fn read_request(stream: &mut TcpStream) -> Command {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).await.unwrap();
    let mut payload = vec![0u8; u32::from_be_bytes(len) as usize];
    stream.read_exact(&mut payload).await.unwrap();
    decode_message(&payload).unwrap()
}

async fn client(addr: &str) -> SoliDBClient {
    SoliDBClient::connect_with_pool(addr, 1).await.unwrap()
}

async fn rows(c: &mut SoliDBClient) -> Result<Vec<Value>, DriverError> {
    c.query_with_cache("db", "FOR d IN c RETURN d", None, false)
        .await
}

fn docs(n: usize, filler: usize) -> Value {
    (0..n)
        .map(|i| json!({"_key": format!("k{i}"), "n": i, "text": "é".repeat(filler)}))
        .collect()
}

#[tokio::test]
async fn a_frame_written_in_pieces_is_read_whole() {
    let a = docs(3, 10);
    let b = docs(50, 100);
    let addr = serve(vec![
        // The length itself split, then the payload.
        Reply::Pieces(frame(&Response::ok(a.clone())), vec![1, 2, 1, 7]),
        // The length whole, the payload in two.
        Reply::Pieces(frame(&Response::ok(b.clone())), vec![4, 3000]),
    ])
    .await;
    let mut c = client(&addr).await;
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), a);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), b);
}

#[tokio::test]
async fn frames_larger_than_the_buffer_and_after() {
    // 40 KB, past the buffer the connection starts with; 3 MB, past what it
    // keeps; then small ones again.
    let medium = docs(200, 100);
    let large = docs(3000, 500);
    let small = docs(2, 1);
    let addr = serve(vec![
        Reply::Pieces(frame(&Response::ok(medium.clone())), vec![]),
        Reply::Pieces(frame(&Response::ok(large.clone())), vec![100_000]),
        Reply::Pieces(frame(&Response::ok(small.clone())), vec![]),
        Reply::Pieces(frame(&Response::ok(medium.clone())), vec![]),
    ])
    .await;
    let mut c = client(&addr).await;
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), medium);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), large);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), small);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), medium);
}

/// Two frames in one write: the second is the answer to the next request,
/// as it was when it waited in the socket instead of in the client.
#[tokio::test]
async fn bytes_past_a_frame_wait_for_the_next_request() {
    let (a, b, c3) = (docs(1, 1), docs(2, 2), docs(3, 3));
    let mut both = frame(&Response::ok(a.clone()));
    both.extend(frame(&Response::ok(b.clone())));
    let addr = serve(vec![
        Reply::Pieces(both, vec![]),
        Reply::Nothing,
        Reply::Pieces(frame(&Response::ok(c3.clone())), vec![]),
    ])
    .await;
    let mut c = client(&addr).await;
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), a);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), b);
    assert_eq!(Value::Array(rows(&mut c).await.unwrap()), c3);
}

#[tokio::test]
async fn errors_and_other_responses_come_through() {
    let addr = serve(vec![
        Reply::Pieces(
            frame(&Response::error(DriverError::DatabaseError(
                "Parse error: nope".into(),
            ))),
            vec![2],
        ),
        Reply::Pieces(frame(&Response::Pong { timestamp: 7 }), vec![]),
        Reply::Pieces(frame(&Response::ok_count(3)), vec![]),
    ])
    .await;
    let mut c = client(&addr).await;
    let err = rows(&mut c).await.unwrap_err();
    assert_eq!(err.to_string(), "Database error: Parse error: nope");
    assert_eq!(c.ping().await.unwrap(), 7);
    let err = rows(&mut c).await.unwrap_err();
    assert_eq!(err.to_string(), "Protocol error: Expected data");
}

/// The messages `read_exact` gave when the socket closed early.
#[tokio::test]
async fn a_connection_closed_early_fails_as_it_did() {
    let payload = frame(&Response::ok(docs(5, 5)));
    let cases = [
        (
            Vec::new(),
            "Connection error: Read length failed: early eof",
        ),
        (
            payload[..2].to_vec(),
            "Connection error: Read length failed: early eof",
        ),
        (
            payload[..30].to_vec(),
            "Connection error: Read payload failed: early eof",
        ),
    ];
    for (bytes, expected) in cases {
        let addr = serve(vec![Reply::Close(bytes)]).await;
        let mut c = client(&addr).await;
        assert_eq!(rows(&mut c).await.unwrap_err().to_string(), expected);
    }
}

#[tokio::test]
async fn a_frame_over_the_limit_is_refused() {
    let len = (MAX_MESSAGE_SIZE as u32 + 1).to_be_bytes().to_vec();
    let addr = serve(vec![Reply::Pieces(len, vec![])]).await;
    let mut c = client(&addr).await;
    assert!(matches!(
        rows(&mut c).await.unwrap_err(),
        DriverError::MessageTooLarge
    ));
}

#[tokio::test]
async fn query_as_reads_from_the_same_buffer() {
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct Row {
        n: u64,
        text: String,
    }
    let addr = serve(vec![Reply::Pieces(
        frame(&Response::ok(docs(3, 2))),
        vec![5],
    )])
    .await;
    let mut c = client(&addr).await;
    let got: Vec<Row> = c
        .query_as("db", "FOR d IN c RETURN d", None, false)
        .await
        .unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(
        got[2],
        Row {
            n: 2,
            text: "éé".into()
        }
    );
}
