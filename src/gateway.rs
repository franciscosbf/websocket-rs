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

impl From<Buf> for TcpStream {
    fn from(buf: Buf) -> Self {
        buf.bstream.into_inner()
    }
}

pub async fn accept(stream: TcpStream) -> Result<Connection, WebSocketError> {
    let mut buf = Buf::new(stream);

    let raw_request = buf.read_raw_http().await?;

    let mut headers = ParsedHeadersBuf::new();
    let request = parse_request(&raw_request, &mut headers)?;

    let handshake = match ServerHanshake::try_from_request(&request) {
        Ok(handshake) => handshake,
        Err(e) => {
            buf.write_raw_http(&b"HTTP/1.1 400 Bad Request\r\n\r\n"[..])
                .await?;

            return Err(e.into());
        }
    };

    let raw_response = handshake.into_raw_response();

    buf.write_raw_http(&raw_response).await?;

    let stream = buf.into();
    let connection = Connection::server_side(stream);

    Ok(connection)
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

    handshake.validate_response(&response)?;

    let stream = buf.into();
    let connection = Connection::client_side(stream);

    Ok(connection)
}
