use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebSocketError {
    #[error("failed to parse request: {0}")]
    HttpRequestParser(#[source] httparse::Error),
    #[error("failed to parse response: {0}")]
    HttpResponseParser(#[source] httparse::Error),
    #[error("handshake is invalid")]
    InvalidHandshake,
    #[error("something went wrong with the connection: {0}")]
    Io(#[from] std::io::Error),
}
