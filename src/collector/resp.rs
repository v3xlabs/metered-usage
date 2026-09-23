//! The part of RESP2 that CLIProxyAPI serves on its proxy port: commands go out as arrays
//! of bulk strings, and replies come back as any of the five RESP2 types.

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufStream};
use tokio::net::TcpStream;

/// A usage record is a few kilobytes. A length far above this is a stream out of step.
const MAX_BULK_LENGTH: usize = 16 * 1024 * 1024;
const MAX_ARRAY_LENGTH: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Simple(String),
    Error(String),
    Integer(i64),
    Bulk(Option<Vec<u8>>),
    Array(Option<Vec<Reply>>),
}

impl Reply {
    /// Names the reply without quoting it: a reply can be a record that holds a key.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Simple(_) => "a simple string",
            Self::Error(_) => "an error",
            Self::Integer(_) => "an integer",
            Self::Bulk(None) => "a null bulk string",
            Self::Bulk(Some(_)) => "a bulk string",
            Self::Array(None) => "a null array",
            Self::Array(Some(_)) => "an array",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RespError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the server closed the connection")]
    Closed,
    #[error("protocol: {0}")]
    Protocol(String),
}

pub struct Connection {
    stream: BufStream<TcpStream>,
}

/// An array header comes before its elements, so `read` collects them without recursion.
enum Frame {
    Reply(Reply),
    ArrayOf(usize),
}

impl Connection {
    pub async fn connect(host: &str, port: u16) -> Result<Self, RespError> {
        let stream = TcpStream::connect((host, port)).await?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream: BufStream::new(stream),
        })
    }

    pub async fn send(&mut self, arguments: &[&[u8]]) -> Result<(), RespError> {
        self.stream
            .write_all(format!("*{}\r\n", arguments.len()).as_bytes())
            .await?;
        for argument in arguments {
            self.stream
                .write_all(format!("${}\r\n", argument.len()).as_bytes())
                .await?;
            self.stream.write_all(argument).await?;
            self.stream.write_all(b"\r\n").await?;
        }
        self.stream.flush().await?;
        Ok(())
    }

    pub async fn command(&mut self, arguments: &[&[u8]]) -> Result<Reply, RespError> {
        self.send(arguments).await?;
        self.read().await
    }

    /// Not cancel safe: a reply abandoned halfway leaves the stream mid-frame.
    pub async fn read(&mut self) -> Result<Reply, RespError> {
        let mut open: Vec<(Vec<Reply>, usize)> = Vec::new();
        loop {
            let mut reply = match self.read_frame().await? {
                Frame::Reply(reply) => reply,
                Frame::ArrayOf(length) => {
                    open.push((Vec::with_capacity(length), length));
                    continue;
                }
            };
            loop {
                let Some((items, length)) = open.last_mut() else {
                    return Ok(reply);
                };
                items.push(reply);
                if items.len() < *length {
                    break;
                }
                let (items, _) = open.pop().unwrap_or_default();
                reply = Reply::Array(Some(items));
            }
        }
    }

    async fn read_frame(&mut self) -> Result<Frame, RespError> {
        let line = self.read_line().await?;
        let (&kind, rest) = line
            .split_first()
            .ok_or_else(|| RespError::Protocol("empty line".to_owned()))?;
        let text = std::str::from_utf8(rest)
            .map_err(|_| RespError::Protocol("header is not UTF-8".to_owned()))?;
        let reply = match kind {
            b'+' => Reply::Simple(text.to_owned()),
            b'-' => Reply::Error(text.to_owned()),
            b':' => Reply::Integer(parse_integer(text)?),
            b'$' => match parse_length(text, MAX_BULK_LENGTH)? {
                None => Reply::Bulk(None),
                Some(length) => {
                    let mut body = vec![0; length + 2];
                    self.stream.read_exact(&mut body).await?;
                    if !body.ends_with(b"\r\n") {
                        return Err(RespError::Protocol(
                            "bulk string is not terminated".to_owned(),
                        ));
                    }
                    body.truncate(length);
                    Reply::Bulk(Some(body))
                }
            },
            b'*' => match parse_length(text, MAX_ARRAY_LENGTH)? {
                None => Reply::Array(None),
                Some(0) => Reply::Array(Some(Vec::new())),
                Some(length) => return Ok(Frame::ArrayOf(length)),
            },
            other => {
                return Err(RespError::Protocol(format!(
                    "unknown type byte {:?}",
                    char::from(other)
                )));
            }
        };
        Ok(Frame::Reply(reply))
    }

    async fn read_line(&mut self) -> Result<Vec<u8>, RespError> {
        let mut line = Vec::new();
        if self.stream.read_until(b'\n', &mut line).await? == 0 {
            return Err(RespError::Closed);
        }
        if !line.ends_with(b"\r\n") {
            return Err(RespError::Protocol("line is not terminated".to_owned()));
        }
        line.truncate(line.len() - 2);
        Ok(line)
    }
}

fn parse_integer(text: &str) -> Result<i64, RespError> {
    text.parse()
        .map_err(|_| RespError::Protocol(format!("{text:?} is not an integer")))
}

/// `-1` is the null bulk string or array.
fn parse_length(text: &str, limit: usize) -> Result<Option<usize>, RespError> {
    match parse_integer(text)? {
        -1 => Ok(None),
        length => usize::try_from(length)
            .ok()
            .filter(|&length| length <= limit)
            .map(Some)
            .ok_or_else(|| RespError::Protocol(format!("length {length} is out of range"))),
    }
}
