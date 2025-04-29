use std::net::SocketAddr;

use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufStream},
    net::TcpStream,
};

use crate::{
    connection::Connection,
    error::WebSocketError,
    handshake::{ClientHandshake, ParsedHeadersBuf, ServerHanshake, parse_request, parse_response},
};

struct Buf {
    bstream: BufStream<TcpStream>,
}

impl Buf {
    fn new(stream: TcpStream) -> Self {
        let bstream = BufStream::new(stream);

        Self { bstream }
    }

    async fn read_raw_http(&mut self) -> Result<Vec<u8>, tokio::io::Error> {
        let mut raw = Vec::new();
        loop {
            if self.bstream.read_until(b'\n', &mut raw).await? == 0 {
                break;
            }

            if raw.ends_with(b"\r\n\r\n") {
                break;
            }
        }

        Ok(raw)
    }

    async fn write_raw_http(&mut self, raw: &[u8]) -> Result<(), tokio::io::Error> {
        self.bstream.write_all(raw).await?;
        self.bstream.flush().await?;

        Ok(())
    }
}

impl From<Buf> for Connection {
    fn from(buf: Buf) -> Self {
        let stream = buf.bstream.into_inner();

        Connection::new(stream)
    }
}

pub async fn accept(stream: TcpStream) -> Result<Connection, WebSocketError> {
    let mut buf = Buf::new(stream);

    let raw_request = buf.read_raw_http().await?;

    let mut headers = ParsedHeadersBuf::new();
    let request = parse_request(&raw_request, &mut headers)?;

    let handshake = match ServerHanshake::from_request(&request) {
        Some(handshake) => Ok(handshake),
        None => {
            buf.write_raw_http(&b"HTTP/1.1 400 Bad Request\r\n\r\n"[..])
                .await?;

            Err(WebSocketError::InvalidHandshake)
        }
    }?;

    let raw_response = handshake.to_raw_response();

    buf.write_raw_http(&raw_response).await?;

    Ok(buf.into())
}

pub async fn connect(addr: SocketAddr) -> Result<Connection, WebSocketError> {
    let stream = TcpStream::connect(addr).await?;

    let mut buf = Buf::new(stream);

    let handshake = ClientHandshake::new(addr);
    let request = handshake.raw_request();

    buf.write_raw_http(&request).await?;

    let raw_response = buf.read_raw_http().await?;

    let mut headers = ParsedHeadersBuf::new();
    let response = parse_response(&raw_response, &mut headers)?;

    if !handshake.is_valid_response(&response) {
        return Err(WebSocketError::InvalidHandshake);
    }

    Ok(buf.into())
}
