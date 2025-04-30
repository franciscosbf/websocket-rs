use std::ops::Deref;

use bytes::Bytes;

#[derive(Debug)]
pub struct Text(pub(crate) Bytes);

impl Text {
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.0) }
    }
}

impl TryFrom<&[u8]> for Text {
    type Error = std::str::Utf8Error;

    fn try_from(raw: &[u8]) -> Result<Self, Self::Error> {
        let checked_utf8 = std::str::from_utf8(raw)?.as_bytes();

        Ok(Text(Bytes::copy_from_slice(checked_utf8)))
    }
}

impl From<&str> for Text {
    fn from(raw: &str) -> Self {
        Text(Bytes::copy_from_slice(raw.as_bytes()))
    }
}

impl From<String> for Text {
    fn from(raw: String) -> Self {
        raw.as_str().into()
    }
}

impl Deref for Text {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

#[derive(Debug)]
pub struct Binary(pub(crate) Bytes);

impl Binary {
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl From<&[u8]> for Binary {
    fn from(raw: &[u8]) -> Self {
        Binary(Bytes::copy_from_slice(raw))
    }
}

impl From<Vec<u8>> for Binary {
    fn from(raw: Vec<u8>) -> Self {
        raw.as_slice().into()
    }
}

impl Deref for Binary {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        self.as_bytes()
    }
}

#[derive(Debug)]
pub enum Message {
    Text(Text),
    Binary(Binary),
}

impl Message {
    pub fn is_text(&self) -> bool {
        matches!(self, Self::Text(_))
    }

    pub fn is_binary(&self) -> bool {
        matches!(self, Self::Binary(_))
    }

    pub fn unwrap_text(self) -> Text {
        match self {
            Self::Text(text) => text,
            _ => panic!("called `Message::unwrap_text()` on a non `Text` value"),
        }
    }

    pub fn unwrap_binary(self) -> Binary {
        match self {
            Self::Binary(binary) => binary,
            _ => panic!("called `Message::unwrap_binary()` on a non `Binary` value"),
        }
    }
}

impl From<Text> for Message {
    fn from(text: Text) -> Self {
        Message::Text(text)
    }
}

impl From<Binary> for Message {
    fn from(binary: Binary) -> Self {
        Message::Binary(binary)
    }
}
