//! Unix socket 上的长度前缀帧编解码。
//!
//! 帧格式：`u32 LE 负载长度 + 负载（JSON）`。长度上限防止恶意或损坏的
//! 连接把内存打爆；发送方序列化失败会在写入前转成协议错误。

use std::io::{self, Read, Write};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// 单帧负载上限（16 MiB）。正常消息远小于此；超过即视为协议错误并断开。
pub const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024;

/// 从阻塞 reader 读一帧完整负载。
pub fn read_frame<R: Read>(reader: &mut R) -> io::Result<Vec<u8>> {
    let mut length_bytes = [0_u8; 4];
    reader.read_exact(&mut length_bytes)?;
    let length = u32::from_le_bytes(length_bytes) as u64;
    if length == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "zero-length IPC frame",
        ));
    }
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC frame exceeds {MAX_FRAME_BYTES} bytes"),
        ));
    }
    let mut payload = vec![0_u8; length as usize];
    reader.read_exact(&mut payload)?;
    Ok(payload)
}

/// 向阻塞 writer 写一帧负载。
pub fn write_frame<W: Write>(writer: &mut W, payload: &[u8]) -> io::Result<()> {
    if payload.is_empty() || payload.len() as u64 > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame payload out of range",
        ));
    }
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(payload)?;
    writer.flush()
}

/// 读一帧并反序列化。
pub fn read_message<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let payload = read_frame(reader)?;
    serde_json::from_slice(&payload).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC message decode failed: {error}"),
        )
    })
}

/// 序列化并写一帧。
pub fn write_message<W: Write, T: Serialize>(writer: &mut W, message: &T) -> io::Result<()> {
    let payload = serde_json::to_vec(message).map_err(|error| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("IPC message encode failed: {error}"),
        )
    })?;
    write_frame(writer, &payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
    struct Sample {
        id: u64,
        name: String,
    }

    #[test]
    fn frame_roundtrip_preserves_payload() {
        let message = Sample {
            id: 42,
            name: "verge".into(),
        };
        let mut buffer = Vec::new();
        write_message(&mut buffer, &message).unwrap();
        let mut cursor = std::io::Cursor::new(buffer);
        let decoded: Sample = read_message(&mut cursor).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn truncated_frame_is_an_error() {
        let mut cursor = std::io::Cursor::new(vec![0x05, 0x00, 0x00, 0x00, 0x01]);
        assert!(read_frame(&mut cursor).is_err());
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let mut cursor = std::io::Cursor::new(vec![0xff, 0xff, 0xff, 0x7f]);
        assert!(read_frame(&mut cursor).is_err());
    }

    #[test]
    fn zero_length_frame_is_rejected() {
        let mut cursor = std::io::Cursor::new(vec![0x00, 0x00, 0x00, 0x00]);
        assert!(read_frame(&mut cursor).is_err());
    }
}
