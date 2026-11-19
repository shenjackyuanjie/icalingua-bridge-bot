//! 宿主与插件共用的 UTF-8 NDJSON 帧编解码。

use std::{io, str};

use serde::{Serialize, de::DeserializeOwned};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncWrite, AsyncWriteExt};

use crate::SdkError;

/// 单帧 JSON 正文的最大字节数，不含 LF 或 CRLF。
pub const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;

/// 接收不完整的帧时保留已消费的字节，允许在 select 中取消 read 后重新读取。
pub struct FrameReader<R> {
    reader: R,
    line: Vec<u8>,
}

impl<R: AsyncBufRead + Unpin> FrameReader<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader,
            line: Vec::new(),
        }
    }

    /// 返回 None 表示恰好在帧边界读到 EOF；遇到帧错误应关闭连接。
    pub async fn read<T: DeserializeOwned>(&mut self) -> Result<Option<T>, SdkError> {
        loop {
            let bytes = self.reader.fill_buf().await?;
            if bytes.is_empty() {
                return if self.line.is_empty() {
                    Ok(None)
                } else {
                    Err(SdkError::TruncatedFrame)
                };
            }
            let newline = bytes.iter().position(|&byte| byte == b'\n');
            let end = newline.unwrap_or(bytes.len());
            let max = MAX_LINE_BYTES
                + usize::from(
                    bytes[..end].last() == Some(&b'\r')
                        || (end == 0 && self.line.last() == Some(&b'\r')),
                );
            if self.line.len() + end > max {
                return Err(SdkError::LineTooLong);
            }
            self.line.extend_from_slice(&bytes[..end]);
            self.reader.consume(end + usize::from(newline.is_some()));
            if newline.is_some() {
                let line = self.line.strip_suffix(b"\r").unwrap_or(&self.line);
                let msg = serde_json::from_str(str::from_utf8(line)?)?;
                self.line.clear();
                return Ok(Some(msg));
            }
        }
    }
}

/// 一次性读帧。需要取消安全时使用持久的 FrameReader。
pub async fn read_frame<R: AsyncBufRead + Unpin, T: DeserializeOwned>(
    reader: &mut R,
) -> Result<Option<T>, SdkError> {
    FrameReader::new(reader).read().await
}

/// 序列化并限制分配规模；失败时不会把部分帧写入管道。
pub fn encode_frame<T: Serialize>(value: &T) -> Result<Vec<u8>, SdkError> {
    let mut buf = BoundedBuffer {
        bytes: Vec::new(),
        exceeded: false,
    };
    if let Err(err) = serde_json::to_writer(&mut buf, value) {
        return Err(if buf.exceeded {
            SdkError::LineTooLong
        } else {
            err.into()
        });
    }
    buf.bytes.push(b'\n');
    Ok(buf.bytes)
}

pub async fn write_frame<W: AsyncWrite + Unpin, T: Serialize>(
    writer: &mut W,
    value: &T,
) -> Result<(), SdkError> {
    let frame = encode_frame(value)?;
    writer.write_all(&frame).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests;

struct BoundedBuffer {
    bytes: Vec<u8>,
    exceeded: bool,
}

impl io::Write for BoundedBuffer {
    fn write(&mut self, chunk: &[u8]) -> io::Result<usize> {
        if chunk.len() > MAX_LINE_BYTES - self.bytes.len() {
            self.exceeded = true;
            return Err(io::Error::other("NDJSON 正文超过上限"));
        }
        self.bytes.extend_from_slice(chunk);
        Ok(chunk.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
