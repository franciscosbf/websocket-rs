use base64::{Engine, prelude::BASE64_STANDARD};
use bytes::{BufMut, Bytes, BytesMut};
use rand::Rng;
use sha1::{Digest, Sha1};
use std::{
    net::SocketAddr,
    ops::{Deref, DerefMut},
};

use crate::error::WebSocketError;

pub const MAX_HEADERS: usize = 124;

const MAGIC_STRING: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

#[derive(Debug, Default)]
pub struct ParsedHeadersBuf<'h>(Vec<httparse::Header<'h>>);

impl ParsedHeadersBuf<'_> {
    pub fn new() -> Self {
        let headers = vec![
            httparse::Header {
                name: "",
                value: b""
            };
            MAX_HEADERS
        ];

        Self(headers)
    }
}

impl<'h> Deref for ParsedHeadersBuf<'h> {
    type Target = [httparse::Header<'h>];

    fn deref(&self) -> &Self::Target {
        self.0.as_slice()
    }
}

impl DerefMut for ParsedHeadersBuf<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut_slice()
    }
}

#[derive(Debug)]
pub struct ParsedResponse<'r>(httparse::Response<'r, 'r>);

pub fn parse_response<'r>(
    raw: &'r [u8],
    headers_buf: &'r mut ParsedHeadersBuf<'r>,
) -> Result<ParsedResponse<'r>, WebSocketError> {
    let mut response = httparse::Response::new(headers_buf);
    response
        .parse(raw)
        .map_err(WebSocketError::HttpResponseParser)?;

    Ok(ParsedResponse(response))
}

impl<'r> Deref for ParsedResponse<'r> {
    type Target = httparse::Response<'r, 'r>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct ParsedRequest<'r>(httparse::Request<'r, 'r>);

pub fn parse_request<'r>(
    raw: &'r [u8],
    headers_buf: &'r mut ParsedHeadersBuf<'r>,
) -> Result<ParsedRequest<'r>, WebSocketError> {
    let mut request = httparse::Request::new(headers_buf);
    request
        .parse(raw)
        .map_err(WebSocketError::HttpRequestParser)?;

    Ok(ParsedRequest(request))
}

#[derive(Debug)]
struct HeaderObserver<'h>(&'h httparse::Header<'h>);

impl HeaderObserver<'_> {
    fn is_key(&self, key: &str) -> bool {
        self.name.eq_ignore_ascii_case(key)
    }

    fn is_any_key(&self, keys: &[&str]) -> bool {
        keys.iter().any(|k| self.is_key(k))
    }

    fn is(&self, key: &str, value: &[u8]) -> bool {
        self.is_key(key) && self.value == value
    }
}

impl<'h> Deref for HeaderObserver<'h> {
    type Target = httparse::Header<'h>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'h> From<&'h httparse::Header<'h>> for HeaderObserver<'h> {
    fn from(header: &'h httparse::Header<'h>) -> Self {
        HeaderObserver(header)
    }
}

#[derive(Debug)]
struct Key(Bytes);

impl Key {
    fn generate() -> Self {
        let bts = rand::rng().random::<[u8; 16]>();
        let encoded = BASE64_STANDARD.encode(bts).into();

        Key(encoded)
    }

    fn encoded_hash(&self) -> Bytes {
        let mut hasher = Sha1::new();

        hasher.update(&self.0);
        hasher.update(MAGIC_STRING);
        let digest = hasher.finalize();

        BASE64_STANDARD.encode(digest).into()
    }
}

impl From<&[u8]> for Key {
    fn from(raw: &[u8]) -> Self {
        Key(Bytes::copy_from_slice(raw))
    }
}

impl Deref for Key {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
pub struct ClientHandshake {
    addr: SocketAddr,
    key: Key,
}

impl ClientHandshake {
    pub fn new(addr: SocketAddr) -> Self {
        let key = Key::generate();

        Self { addr, key }
    }

    pub fn raw_request(&self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.put(
            &b"\
                GET /chat HTTP/1.1\r\n\
                Upgrade: websocket\r\n\
                Connection: Upgrade\r\n\
                Sec-WebSocket-Version: 13\r\n\
                Sec-WebSocket-Key: "[..],
        );
        buf.put(&self.key[..]);
        buf.put(&b"\r\nHost: "[..]);
        buf.put(format!("{}", self.addr).as_bytes());
        buf.put(&b"\r\n\r\n"[..]);

        buf.into()
    }

    pub fn is_valid_response(&self, response: &ParsedResponse<'_>) -> bool {
        if !matches!((response.version, response.code), (Some(1), Some(101))) {
            return false;
        }

        let mut contains_headers = [0, 0];
        let mut valid_key = false;
        for h in response.headers.iter().map(HeaderObserver::from) {
            if h.is("Upgrade", b"websocket") {
                contains_headers[0] += 1;
            } else if h.is("Connection", b"Upgrade") {
                contains_headers[1] += 1;
            } else if h.is_key("Sec-WebSocket-Accept") {
                if self.key.encoded_hash() == h.value {
                    valid_key = true;
                } else {
                    return false;
                }
            } else if h.is_any_key(&["Sec-WebSocket-Extensions", "Sec-WebSocket-Protocol"]) {
                return false;
            }
        }
        contains_headers.iter().all(|&c| c > 0) && valid_key
    }
}

#[derive(Debug)]
pub struct ServerHanshake {
    key: Key,
}

impl ServerHanshake {
    fn new(key: Key) -> Self {
        Self { key }
    }

    pub fn from_request(request: &ParsedRequest<'_>) -> Option<Self> {
        if !matches!(
            (request.0.method, request.0.path, request.0.version),
            (Some("GET"), Some("/"), Some(1))
        ) {
            return None;
        }

        let mut contains_headers = [0, 0, 0, 0, 0];
        let mut encoded_key = &[0][..];
        request
            .0
            .headers
            .iter()
            .map(HeaderObserver::from)
            .for_each(|h| {
                if h.is_key("Host") {
                    contains_headers[0] += 1;
                } else if h.is("Upgrade", b"websocket") {
                    contains_headers[1] += 1;
                } else if h.is("Connection", b"Upgrade") {
                    contains_headers[2] += 1;
                } else if h.is_key("Sec-WebSocket-Key") && BASE64_STANDARD.decode(h.value).is_ok() {
                    encoded_key = h.value;
                    contains_headers[3] += 1;
                } else if h.is("Sec-WebSocket-Version", b"13") {
                    contains_headers[4] += 1;
                }
            });

        contains_headers
            .iter()
            .all(|&c| c > 0)
            .then(|| Self::new(encoded_key.into()))
    }

    pub fn into_raw_response(self) -> Bytes {
        let mut buf = BytesMut::new();

        buf.put(&b"HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: "[..]);
        buf.put(self.key.encoded_hash());
        buf.put(&b"\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\n"[..]);

        buf.into()
    }
}
