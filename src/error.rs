use thiserror::Error;

use crate::connection::{MAX_FRAME_PAYLOAD_SIZE, MAX_MESSAGE_SIZE};

#[derive(Debug, Error)]
pub enum InvalidFrame {
    #[error("unknown opcode `{0}`")]
    Opcode(u8),
    #[error("unknown status code `{0}`")]
    Code(u16),
    #[error("payload surpasses size limit: {MAX_FRAME_PAYLOAD_SIZE}")]
    PayloadSize,
    #[error("text isn't UTF-8 compliant: {0}")]
    Text(#[from] std::str::Utf8Error),
    #[error("inconsistent data")]
    Inconsistent,
}

#[derive(Debug, Error)]
pub enum InvalidHandshake {
    #[error("failed to parse request: {0}")]
    HttpRequestParser(#[source] httparse::Error),
    #[error("failed to parse response: {0}")]
    HttpResponseParser(#[source] httparse::Error),
    #[error("does not meet the specified requirements")]
    NonConformant,
}

#[derive(Debug, Error)]
pub enum WebSocketError {
    #[error("handshake is invalid: {0}")]
    InvalidHandshake(#[from] InvalidHandshake),
    #[error("something went wrong with the connection: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid frame: {0}")]
    InvalidFrame(#[from] InvalidFrame),
    #[error("message surpasses size limit: {MAX_MESSAGE_SIZE}")]
    InvalidMessageSize,
    #[error("connection is closed")]
    ConnectionClosed,
}
