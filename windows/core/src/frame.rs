//! Discord ipc framing.
//!
//! ```text
//! | opcode: u32 LE | length: u32 LE | payload: `length` bytes of utf-8 json |
//! ```
//!
//! [`FrameDecoder`] is a pure, sans-io buffer: push bytes in, pull frames out.
//! It copes with frames split across reads and several frames in one read.
//! [`FrameReader`] wraps it around any `AsyncRead`.

use std::io;

use thiserror::Error;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Size of the opcode + length header.
pub const HEADER_LEN: usize = 8;

/// Largest payload we will encode or accept. Discord's own frames are tiny, so
/// anything bigger than this is a bug or garbage and not worth allocating for.
pub const MAX_PAYLOAD_LEN: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum Opcode {
    Handshake = 0,
    Frame = 1,
    Close = 2,
    Ping = 3,
    Pong = 4,
}

impl Opcode {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Handshake),
            1 => Some(Self::Frame),
            2 => Some(Self::Close),
            3 => Some(Self::Ping),
            4 => Some(Self::Pong),
            _ => None,
        }
    }
}

/// One decoded frame. The payload is always valid utf-8.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub opcode: Opcode,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FrameError {
    #[error("unknown opcode {0}")]
    UnknownOpcode(u32),
    #[error("frame payload of {len} bytes is over the {max} byte limit")]
    TooLarge { len: usize, max: usize },
    #[error("frame payload is not valid utf-8")]
    InvalidUtf8,
}

/// Build the bytes for one frame.
pub fn encode(opcode: Opcode, payload: &str) -> Result<Vec<u8>, FrameError> {
    let len = payload.len();
    if len > MAX_PAYLOAD_LEN {
        return Err(FrameError::TooLarge {
            len,
            max: MAX_PAYLOAD_LEN,
        });
    }
    let mut out = Vec::with_capacity(HEADER_LEN + len);
    out.extend_from_slice(&(opcode as u32).to_le_bytes());
    out.extend_from_slice(&(len as u32).to_le_bytes());
    out.extend_from_slice(payload.as_bytes());
    Ok(out)
}

/// Incremental frame decoder.
///
/// Once it returns an error the byte stream can't be resynchronised, so the
/// error is sticky: every later call returns it again. Drop the connection.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    buf: Vec<u8>,
    failed: Option<FrameError>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        if self.failed.is_none() {
            self.buf.extend_from_slice(bytes);
        }
    }

    /// Bytes received but not yet turned into a frame.
    pub fn buffered(&self) -> usize {
        self.buf.len()
    }

    /// Try to pull the next complete frame. `Ok(None)` means "need more bytes".
    pub fn next_frame(&mut self) -> Result<Option<Frame>, FrameError> {
        if let Some(err) = &self.failed {
            return Err(err.clone());
        }
        match self.try_next() {
            Ok(frame) => Ok(frame),
            Err(err) => {
                self.buf.clear();
                self.failed = Some(err.clone());
                Err(err)
            }
        }
    }

    fn try_next(&mut self) -> Result<Option<Frame>, FrameError> {
        if self.buf.len() < HEADER_LEN {
            return Ok(None);
        }
        let raw_opcode = u32::from_le_bytes(self.buf[0..4].try_into().expect("4 bytes"));
        let len = u32::from_le_bytes(self.buf[4..8].try_into().expect("4 bytes")) as usize;

        // Validate the header as soon as we have it, so garbage fails fast
        // instead of waiting for a payload that is never going to arrive.
        let opcode = Opcode::from_u32(raw_opcode).ok_or(FrameError::UnknownOpcode(raw_opcode))?;
        if len > MAX_PAYLOAD_LEN {
            return Err(FrameError::TooLarge {
                len,
                max: MAX_PAYLOAD_LEN,
            });
        }
        if self.buf.len() < HEADER_LEN + len {
            return Ok(None);
        }

        let payload = std::str::from_utf8(&self.buf[HEADER_LEN..HEADER_LEN + len])
            .map_err(|_| FrameError::InvalidUtf8)?
            .to_owned();
        self.buf.drain(..HEADER_LEN + len);
        Ok(Some(Frame { opcode, payload }))
    }
}

#[derive(Debug, Error)]
pub enum ReadError {
    #[error("connection closed")]
    Eof,
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("bad frame: {0}")]
    Frame(#[from] FrameError),
}

/// Reads whole frames off an `AsyncRead`.
///
/// [`read_frame`](Self::read_frame) is cancel safe, so it can be used as a
/// `select!` branch: partial data stays buffered in the decoder.
pub struct FrameReader<R> {
    inner: R,
    decoder: FrameDecoder,
    scratch: Box<[u8; 4096]>,
}

impl<R: AsyncRead + Unpin> FrameReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            decoder: FrameDecoder::new(),
            scratch: Box::new([0; 4096]),
        }
    }

    pub async fn read_frame(&mut self) -> Result<Frame, ReadError> {
        loop {
            if let Some(frame) = self.decoder.next_frame()? {
                return Ok(frame);
            }
            let n = self.inner.read(&mut self.scratch[..]).await?;
            if n == 0 {
                return Err(ReadError::Eof);
            }
            self.decoder.push(&self.scratch[..n]);
        }
    }
}

#[derive(Debug, Error)]
pub enum WriteError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("bad frame: {0}")]
    Frame(#[from] FrameError),
}

/// Encode and write one frame, then flush.
pub async fn write_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    opcode: Opcode,
    payload: &str,
) -> Result<(), WriteError> {
    let bytes = encode(opcode, payload)?;
    writer.write_all(&bytes).await?;
    writer.flush().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_all(bytes: &[u8]) -> Vec<Frame> {
        let mut d = FrameDecoder::new();
        d.push(bytes);
        let mut out = Vec::new();
        while let Some(f) = d.next_frame().unwrap() {
            out.push(f);
        }
        out
    }

    #[test]
    fn encode_layout_matches_the_spec_vector() {
        let bytes = encode(Opcode::Handshake, r#"{"v":1,"client_id":"1"}"#).unwrap();
        assert_eq!(&bytes[..4], &[0, 0, 0, 0]);
        assert_eq!(&bytes[4..8], &[0x17, 0, 0, 0]);
        assert_eq!(&bytes[8..], br#"{"v":1,"client_id":"1"}"#);
    }

    #[test]
    fn header_is_little_endian_for_every_opcode() {
        for (op, n) in [
            (Opcode::Handshake, 0u32),
            (Opcode::Frame, 1),
            (Opcode::Close, 2),
            (Opcode::Ping, 3),
            (Opcode::Pong, 4),
        ] {
            let bytes = encode(op, "{}").unwrap();
            assert_eq!(bytes[..4], n.to_le_bytes());
            assert_eq!(bytes[4..8], 2u32.to_le_bytes());
        }
    }

    #[test]
    fn round_trip_every_opcode() {
        for op in [
            Opcode::Handshake,
            Opcode::Frame,
            Opcode::Close,
            Opcode::Ping,
            Opcode::Pong,
        ] {
            let bytes = encode(op, r#"{"hello":"wörld ✓"}"#).unwrap();
            let frames = decode_all(&bytes);
            assert_eq!(
                frames,
                vec![Frame {
                    opcode: op,
                    payload: r#"{"hello":"wörld ✓"}"#.into()
                }]
            );
        }
    }

    #[test]
    fn empty_payload_round_trips() {
        let bytes = encode(Opcode::Ping, "").unwrap();
        assert_eq!(bytes.len(), HEADER_LEN);
        assert_eq!(decode_all(&bytes)[0].payload, "");
    }

    #[test]
    fn partial_reads_byte_by_byte() {
        let bytes = encode(Opcode::Frame, r#"{"cmd":"SET_ACTIVITY"}"#).unwrap();
        let mut d = FrameDecoder::new();
        for (i, b) in bytes.iter().enumerate() {
            assert_eq!(d.next_frame().unwrap(), None, "frame appeared early at {i}");
            d.push(&[*b]);
        }
        let frame = d.next_frame().unwrap().expect("complete frame");
        assert_eq!(frame.payload, r#"{"cmd":"SET_ACTIVITY"}"#);
        assert_eq!(d.buffered(), 0);
    }

    #[test]
    fn header_split_across_pushes() {
        let bytes = encode(Opcode::Frame, "{}").unwrap();
        let mut d = FrameDecoder::new();
        d.push(&bytes[..3]);
        assert_eq!(d.next_frame().unwrap(), None);
        d.push(&bytes[3..9]);
        assert_eq!(d.next_frame().unwrap(), None);
        d.push(&bytes[9..]);
        assert!(d.next_frame().unwrap().is_some());
    }

    #[test]
    fn several_frames_in_one_read() {
        let mut bytes = encode(Opcode::Frame, "{\"a\":1}").unwrap();
        bytes.extend(encode(Opcode::Ping, "{\"b\":2}").unwrap());
        bytes.extend(encode(Opcode::Close, "{\"c\":3}").unwrap());
        let frames = decode_all(&bytes);
        assert_eq!(frames.len(), 3);
        assert_eq!(frames[1].opcode, Opcode::Ping);
        assert_eq!(frames[2].payload, "{\"c\":3}");
    }

    #[test]
    fn leftover_bytes_stay_buffered() {
        let first = encode(Opcode::Frame, "{}").unwrap();
        let second = encode(Opcode::Frame, "{\"x\":true}").unwrap();
        let mut d = FrameDecoder::new();
        d.push(&first);
        d.push(&second[..5]);
        assert!(d.next_frame().unwrap().is_some());
        assert_eq!(d.next_frame().unwrap(), None);
        assert_eq!(d.buffered(), 5);
        d.push(&second[5..]);
        assert_eq!(d.next_frame().unwrap().unwrap().payload, "{\"x\":true}");
    }

    #[test]
    fn unknown_opcode_fails_fast_without_waiting_for_payload() {
        let mut d = FrameDecoder::new();
        // header only, claims 1000 bytes that will never come
        let mut header = 9u32.to_le_bytes().to_vec();
        header.extend(1000u32.to_le_bytes());
        d.push(&header);
        assert_eq!(d.next_frame(), Err(FrameError::UnknownOpcode(9)));
    }

    #[test]
    fn oversized_frame_is_rejected_from_the_header() {
        let mut d = FrameDecoder::new();
        let mut header = 1u32.to_le_bytes().to_vec();
        header.extend(u32::MAX.to_le_bytes());
        d.push(&header);
        assert!(matches!(
            d.next_frame(),
            Err(FrameError::TooLarge { len, .. }) if len == u32::MAX as usize
        ));
    }

    #[test]
    fn exactly_the_limit_is_fine() {
        let payload = "a".repeat(MAX_PAYLOAD_LEN);
        let bytes = encode(Opcode::Frame, &payload).unwrap();
        assert_eq!(decode_all(&bytes)[0].payload.len(), MAX_PAYLOAD_LEN);
    }

    #[test]
    fn encode_rejects_oversized_payloads() {
        let payload = "a".repeat(MAX_PAYLOAD_LEN + 1);
        assert!(matches!(
            encode(Opcode::Frame, &payload),
            Err(FrameError::TooLarge { .. })
        ));
    }

    #[test]
    fn invalid_utf8_is_an_error() {
        let mut bytes = 1u32.to_le_bytes().to_vec();
        bytes.extend(2u32.to_le_bytes());
        bytes.extend([0xff, 0xfe]);
        let mut d = FrameDecoder::new();
        d.push(&bytes);
        assert_eq!(d.next_frame(), Err(FrameError::InvalidUtf8));
    }

    #[test]
    fn errors_are_sticky_and_drop_buffered_data() {
        let mut d = FrameDecoder::new();
        d.push(&[0xde, 0xad, 0xbe, 0xef, 0, 0, 0, 0]);
        let err = d.next_frame().unwrap_err();
        d.push(&encode(Opcode::Frame, "{}").unwrap());
        assert_eq!(d.next_frame().unwrap_err(), err);
        assert_eq!(d.buffered(), 0);
    }

    #[test]
    fn pure_garbage_does_not_panic() {
        // deterministic pseudo random bytes
        let mut x: u32 = 0x1234_5678;
        for _ in 0..200 {
            let mut chunk = Vec::new();
            for _ in 0..64 {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                chunk.push((x >> 24) as u8);
            }
            let mut d = FrameDecoder::new();
            d.push(&chunk);
            let _ = d.next_frame();
        }
    }

    #[tokio::test]
    async fn reader_reassembles_frames_from_tiny_chunks() {
        let (mut tx, rx) = tokio::io::duplex(8);
        let mut bytes = encode(Opcode::Frame, r#"{"n":1}"#).unwrap();
        bytes.extend(encode(Opcode::Pong, r#"{"n":2}"#).unwrap());
        let writer = tokio::spawn(async move {
            for chunk in bytes.chunks(3) {
                tx.write_all(chunk).await.unwrap();
            }
        });
        let mut reader = FrameReader::new(rx);
        assert_eq!(reader.read_frame().await.unwrap().payload, r#"{"n":1}"#);
        let second = reader.read_frame().await.unwrap();
        assert_eq!(second.opcode, Opcode::Pong);
        writer.await.unwrap();
        assert!(matches!(reader.read_frame().await, Err(ReadError::Eof)));
    }

    #[tokio::test]
    async fn reader_reports_eof_in_the_middle_of_a_frame() {
        let (mut tx, rx) = tokio::io::duplex(64);
        let bytes = encode(Opcode::Frame, r#"{"n":1}"#).unwrap();
        tx.write_all(&bytes[..bytes.len() - 2]).await.unwrap();
        drop(tx);
        let mut reader = FrameReader::new(rx);
        assert!(matches!(reader.read_frame().await, Err(ReadError::Eof)));
    }

    #[tokio::test]
    async fn reader_surfaces_garbage_as_a_frame_error() {
        let (mut tx, rx) = tokio::io::duplex(64);
        tx.write_all(&[7, 0, 0, 0, 0, 0, 0, 0]).await.unwrap();
        let mut reader = FrameReader::new(rx);
        assert!(matches!(
            reader.read_frame().await,
            Err(ReadError::Frame(FrameError::UnknownOpcode(7)))
        ));
    }

    #[tokio::test]
    async fn write_frame_then_read_frame() {
        let (mut tx, rx) = tokio::io::duplex(256);
        write_frame(&mut tx, Opcode::Ping, r#"{"t":1}"#).await.unwrap();
        let mut reader = FrameReader::new(rx);
        let frame = reader.read_frame().await.unwrap();
        assert_eq!(frame.opcode, Opcode::Ping);
    }
}
