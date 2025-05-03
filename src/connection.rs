#![allow(dead_code)]

use bytes::{Bytes, BytesMut};
use rayon::iter::{IndexedParallelIterator, IntoParallelRefMutIterator, ParallelIterator};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};

use crate::{
    error::{InvalidFrame, WebSocketError},
    message::{Binary, Message, Text},
};

pub(crate) const MAX_FRAME_PAYLOAD_SIZE: usize = 16 * 1024 * 1024;
pub(crate) const MAX_MESSAGE_SIZE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Copy)]
enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl TryFrom<u8> for Opcode {
    type Error = InvalidFrame;
    fn try_from(raw: u8) -> Result<Self, Self::Error> {
        let opcode = match raw {
            0x0 => Self::Continuation,
            0x1 => Self::Text,
            0x2 => Self::Binary,
            0x8 => Self::Close,
            0x9 => Self::Ping,
            0xA => Self::Pong,
            _ => return Err(InvalidFrame::Opcode(raw)),
        };

        Ok(opcode)
    }
}

impl From<Opcode> for u8 {
    fn from(opcode: Opcode) -> Self {
        match opcode {
            Opcode::Continuation => 0x0,
            Opcode::Text => 0x1,
            Opcode::Binary => 0x2,
            Opcode::Close => 0x8,
            Opcode::Ping => 0x9,
            Opcode::Pong => 0xA,
        }
    }
}

#[derive(Debug)]
struct RawFrame {
    fin: bool,
    opcode: Opcode,
    payload: Bytes,
}

#[derive(Debug)]
enum StatusCode {
    NormalClosure,
    GoingAway,
    ProtocolError,
    UnkownType,
    InconsistentData,
    PolicyViolation,
    MessageTooBig,
    UnexpectedCondition,
}

impl TryFrom<u16> for StatusCode {
    type Error = InvalidFrame;

    fn try_from(raw: u16) -> Result<Self, Self::Error> {
        let code = match raw {
            1000 => Self::NormalClosure,
            1001 => Self::GoingAway,
            1002 => Self::ProtocolError,
            1003 => Self::UnkownType,
            1007 => Self::InconsistentData,
            1008 => Self::PolicyViolation,
            1009 => Self::MessageTooBig,
            1011 => Self::UnexpectedCondition,
            _ => return Err(InvalidFrame::Code(raw)),
        };

        Ok(code)
    }
}

impl From<StatusCode> for u16 {
    fn from(status: StatusCode) -> Self {
        match status {
            StatusCode::NormalClosure => 1000,
            StatusCode::GoingAway => 1001,
            StatusCode::ProtocolError => 1002,
            StatusCode::UnkownType => 1003,
            StatusCode::InconsistentData => 1007,
            StatusCode::PolicyViolation => 1008,
            StatusCode::MessageTooBig => 1009,
            StatusCode::UnexpectedCondition => unreachable!(),
        }
    }
}

#[derive(Debug)]
struct TextContent {
    pub fin: bool,
    pub text: Text,
}

#[derive(Debug)]
struct BinaryContent {
    pub fin: bool,
    pub binary: Binary,
}

type PingContent = Binary;

type PongContent = Binary;

#[derive(Debug)]
struct CloseContent {
    pub status: StatusCode,
    pub reason: Option<Text>,
}

#[derive(Debug)]
enum Frame {
    Text(TextContent),
    Binary(BinaryContent),
    Ping(Option<PingContent>),
    Pong(Option<PongContent>),
    Close(Option<CloseContent>),
}

impl Frame {
    fn size(&self) -> usize {
        match self {
            Self::Text(TextContent { text, .. }) => text.len(),
            Self::Binary(BinaryContent { binary, .. }) => binary.len(),
            Self::Ping(Some(binary)) => binary.len(),
            Self::Pong(Some(binary)) => binary.len(),
            Self::Pong(None) => 0,
            Self::Close(Some(CloseContent { reason, .. })) => {
                16 + reason.as_ref().map_or(0, |reason| reason.len())
            }
            _ => unreachable!(),
        }
    }
}

#[derive(Debug)]
enum Mask {
    Client,
    Server,
}

fn xor_payload(masking_key: u32, payload: &mut [u8]) {
    let masking_key = masking_key.to_be_bytes();
    payload
        .par_iter_mut()
        .enumerate()
        .for_each(|(i, b)| *b ^= masking_key[i % 4]);
}

#[derive(Debug)]
struct Controller {
    send_tx: flume::Sender<Message>,
    receive_rx: flume::Receiver<Message>,
    stop_tx: tokio::sync::oneshot::Sender<()>,
    manager_handler: tokio::task::JoinHandle<()>,
}

impl Controller {
    async fn send(&self, message: Message) -> Result<(), WebSocketError> {
        self.send_tx
            .send_async(message)
            .await
            .map_err(|_| WebSocketError::ConnectionClosed)
    }

    async fn receive(&self) -> Result<Message, WebSocketError> {
        self.receive_rx
            .recv_async()
            .await
            .map_err(|_| WebSocketError::ConnectionClosed)
    }

    #[allow(unused_must_use)]
    async fn stop(self) {
        self.stop_tx.send(());

        self.manager_handler.await;
    }
}

#[derive(Debug)]
struct Manager {
    stream: TcpStream,
    mask: Mask,
}

impl Manager {
    async fn decode(&mut self) -> Result<RawFrame, WebSocketError> {
        let octet = self.stream.read_u8().await?;
        let fin = (octet >> 7) & 1 != 0;
        if (octet >> 4) & 0b111 != 0 {
            return Err(InvalidFrame::Inconsistent.into());
        }
        let opcode: Opcode = (octet & 0xF).try_into()?;
        match (fin, opcode) {
            (false, Opcode::Ping | Opcode::Pong | Opcode::Close) | (true, Opcode::Continuation) => {
                return Err(InvalidFrame::Inconsistent.into());
            }
            _ => (),
        }

        let octet = self.stream.read_u8().await?;
        let masked = (octet >> 7) & 1 != 0;
        match self.mask {
            Mask::Client if !masked => (),
            Mask::Server if masked => (),
            _ => return Err(InvalidFrame::PayloadSize.into()),
        }
        let possible_payload_length = octet ^ (1 << 8);
        let payload_length = match possible_payload_length {
            (0..=125) => possible_payload_length as usize,
            126 => self.stream.read_u16().await? as usize,
            127 => self.stream.read_u64().await? as usize,
            _ => return Err(InvalidFrame::Inconsistent.into()),
        };
        if payload_length > MAX_FRAME_PAYLOAD_SIZE {
            return Err(InvalidFrame::PayloadSize.into());
        }

        let masking_key = if let Mask::Server = self.mask {
            Some(self.stream.read_u32().await?)
        } else {
            None
        };

        let payload = if payload_length > 0 {
            let mut payload = BytesMut::with_capacity(payload_length);
            self.stream.read_exact(&mut payload).await?;

            if let Mask::Server = self.mask {
                xor_payload(masking_key.unwrap(), &mut payload);
            }

            payload.into()
        } else {
            Bytes::new()
        };

        let raw_frame = RawFrame {
            fin,
            opcode,
            payload,
        };

        Ok(raw_frame)
    }

    async fn encode(&mut self, raw_frame: RawFrame) -> Result<(), WebSocketError> {
        let fin = if raw_frame.fin { 1 } else { 0 };
        let opcode: u8 = raw_frame.opcode.into();
        let octet = (fin << 8) | opcode;
        self.stream.write_u8(octet).await?;

        let masked = match self.mask {
            Mask::Client => 1,
            Mask::Server => 0,
        };
        let mut octet = masked << 8;
        let payload_length = raw_frame.payload.len();
        octet |= match payload_length {
            (0..=125) => payload_length as u8,
            (126..=0xFFFF) => 126,
            _ => 127,
        };
        self.stream.write_u8(octet).await?;

        match payload_length {
            (0..=125) => (),
            (126..=0xFFFF) => self.stream.write_u16(payload_length as u16).await?,
            _ => self.stream.write_u64(payload_length as u64).await?,
        }

        let mut payload: BytesMut = raw_frame.payload.into();
        if let Mask::Client = self.mask {
            let masking_key = rand::random::<u32>();
            self.stream.write_u32(masking_key).await?;
            xor_payload(masking_key, &mut payload);
        }
        if payload_length > 0 {
            self.stream.write_all(&payload).await?;
        }

        Ok(())
    }

    fn start_manager(stream: TcpStream, mask: Mask) -> Controller {
        let (send_tx, send_rx) = flume::unbounded();
        let (receive_tx, receive_rx) = flume::unbounded();
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();

        let manager_handler = tokio::spawn(async move {
            let mut manager = Manager { stream, mask };

            loop {
                tokio::select! {
                    message = send_rx.recv_async() => {
                        let _ = message;

                        todo!()
                    },
                    raw_frame = manager.decode() => {
                        let _ = raw_frame;
                        let _ = receive_tx;

                        todo!()
                    },
                    _ = stop_rx => {
                        todo!()
                    },
                }
            }
        });

        Controller {
            send_tx,
            receive_rx,
            stop_tx,
            manager_handler,
        }
    }
}

pub struct Connection {
    controller: Controller,
}

impl Connection {
    pub(crate) fn new_client_connection(stream: TcpStream) -> Self {
        let controller = Manager::start_manager(stream, Mask::Client);

        Self { controller }
    }

    pub(crate) fn new_server_connection(stream: TcpStream) -> Self {
        let controller = Manager::start_manager(stream, Mask::Server);

        Self { controller }
    }

    pub async fn send(&self, message: Message) -> Result<(), WebSocketError> {
        self.controller.send(message).await
    }

    pub async fn receive(&self) -> Result<Message, WebSocketError> {
        self.controller.receive().await
    }

    pub async fn stop(self) {
        self.controller.stop().await;
    }
}
