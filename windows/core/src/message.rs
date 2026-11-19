//! Typed view of what goes over the frames: the handshake we send, and the
//! ready / error / close / ping messages discord sends back.

use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;

use crate::frame::{self, Frame, FrameError, Opcode};

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    #[error("\"{0}\" is not a valid discord application id (expected 15 to 25 digits)")]
    BadClientId(String),
    #[error("malformed message from discord: {0}")]
    Malformed(String),
    #[error(transparent)]
    Frame(#[from] FrameError),
}

/// Discord application ids are snowflakes: 17 to 20 digits in practice. We
/// accept a slightly wider range, the schema in `shared/` uses the same one.
pub fn validate_client_id(id: &str) -> Result<(), ProtocolError> {
    let ok = (15..=25).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(ProtocolError::BadClientId(id.to_owned()))
    }
}

/// The bytes of the opening handshake frame (opcode 0).
pub fn handshake_frame(client_id: &str) -> Result<Vec<u8>, ProtocolError> {
    validate_client_id(client_id)?;
    let payload = json!({ "v": 1, "client_id": client_id }).to_string();
    Ok(frame::encode(Opcode::Handshake, &payload)?)
}

/// A pong that echoes the ping's payload.
pub fn pong_frame(ping_payload: &str) -> Result<Vec<u8>, ProtocolError> {
    Ok(frame::encode(Opcode::Pong, ping_payload)?)
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReadyInfo {
    /// The discord user the client is logged in as, when it tells us.
    pub username: Option<String>,
}

/// An `"evt": "ERROR"` dispatch or command response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub nonce: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseInfo {
    pub code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// The handshake worked, the connection is ready for commands.
    Ready(ReadyInfo),
    /// discord rejected something.
    Error(RpcError),
    /// A successful reply to a command we sent.
    Response { cmd: String, nonce: Option<String> },
    /// Any other event dispatch (we don't subscribe to any, but be tolerant).
    Event { evt: String },
    /// Discord is closing the connection.
    Close(CloseInfo),
    /// Must be answered with a pong carrying the same payload.
    Ping(String),
    Pong(String),
}

#[derive(Deserialize)]
struct RawMessage {
    cmd: Option<String>,
    evt: Option<String>,
    data: Option<Value>,
    nonce: Option<String>,
}

pub fn parse_incoming(frame: &Frame) -> Result<Incoming, ProtocolError> {
    match frame.opcode {
        Opcode::Ping => Ok(Incoming::Ping(frame.payload.clone())),
        Opcode::Pong => Ok(Incoming::Pong(frame.payload.clone())),
        Opcode::Handshake => Err(ProtocolError::Malformed(
            "discord sent a handshake frame".into(),
        )),
        Opcode::Close => {
            // Be lenient: a close with a missing or broken body is still a close.
            let v: Value = serde_json::from_str(&frame.payload).unwrap_or(Value::Null);
            Ok(Incoming::Close(CloseInfo {
                code: v.get("code").and_then(Value::as_i64).unwrap_or(0),
                message: v
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            }))
        }
        Opcode::Frame => {
            let raw: RawMessage = serde_json::from_str(&frame.payload)
                .map_err(|e| ProtocolError::Malformed(e.to_string()))?;
            let data = raw.data.unwrap_or(Value::Null);
            match raw.evt.as_deref() {
                Some("ERROR") => Ok(Incoming::Error(RpcError {
                    code: data.get("code").and_then(Value::as_i64).unwrap_or(0),
                    message: data
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown error")
                        .to_owned(),
                    nonce: raw.nonce,
                })),
                Some("READY") => Ok(Incoming::Ready(ReadyInfo {
                    username: data
                        .pointer("/user/username")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                })),
                Some(other) => Ok(Incoming::Event {
                    evt: other.to_owned(),
                }),
                None => Ok(Incoming::Response {
                    cmd: raw.cmd.unwrap_or_default(),
                    nonce: raw.nonce,
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FrameDecoder;

    fn frame(op: Opcode, payload: &str) -> Frame {
        Frame {
            opcode: op,
            payload: payload.to_owned(),
        }
    }

    #[test]
    fn client_id_validation() {
        assert!(validate_client_id("1402418491272986635").is_ok());
        assert!(validate_client_id("123456789012345").is_ok());
        for bad in [
            "",
            "1",
            "12345678901234",
            "abc",
            "14024184912729866x5",
            " 1402418491272986635",
            "1402418491272986635 ",
            "1234567890123456789012345 6",
            "١٤٠٢٤١٨٤٩١٢٧٢٩٨٦٦٣٥", // arabic-indic digits aren't ascii digits
        ] {
            assert_eq!(
                validate_client_id(bad),
                Err(ProtocolError::BadClientId(bad.into())),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn handshake_frame_contents() {
        let bytes = handshake_frame("1402418491272986635").unwrap();
        let mut d = FrameDecoder::new();
        d.push(&bytes);
        let f = d.next_frame().unwrap().unwrap();
        assert_eq!(f.opcode, Opcode::Handshake);
        let v: Value = serde_json::from_str(&f.payload).unwrap();
        assert_eq!(v, json!({ "v": 1, "client_id": "1402418491272986635" }));
    }

    #[test]
    fn handshake_rejects_bad_ids_before_building_anything() {
        assert!(matches!(
            handshake_frame("nope"),
            Err(ProtocolError::BadClientId(_))
        ));
    }

    #[test]
    fn pong_echoes_the_payload() {
        let bytes = pong_frame(r#"{"nonce":"x"}"#).unwrap();
        let mut d = FrameDecoder::new();
        d.push(&bytes);
        let f = d.next_frame().unwrap().unwrap();
        assert_eq!(f.opcode, Opcode::Pong);
        assert_eq!(f.payload, r#"{"nonce":"x"}"#);
    }

    #[test]
    fn parses_ready() {
        let f = frame(
            Opcode::Frame,
            r#"{"cmd":"DISPATCH","data":{"v":1,"config":{"cdn_host":"cdn.discordapp.com"},"user":{"id":"1","username":"jaymz"}},"evt":"READY","nonce":null}"#,
        );
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Ready(ReadyInfo {
                username: Some("jaymz".into())
            })
        );
    }

    #[test]
    fn ready_without_user_is_still_ready() {
        let f = frame(Opcode::Frame, r#"{"cmd":"DISPATCH","evt":"READY","data":{"v":1}}"#);
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Ready(ReadyInfo { username: None })
        );
    }

    #[test]
    fn parses_error_event() {
        let f = frame(
            Opcode::Frame,
            r#"{"cmd":"SET_ACTIVITY","evt":"ERROR","data":{"code":4000,"message":"bad payload"},"nonce":"abc"}"#,
        );
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Error(RpcError {
                code: 4000,
                message: "bad payload".into(),
                nonce: Some("abc".into())
            })
        );
    }

    #[test]
    fn error_with_missing_fields_still_parses() {
        let f = frame(Opcode::Frame, r#"{"evt":"ERROR"}"#);
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Error(RpcError {
                code: 0,
                message: "unknown error".into(),
                nonce: None
            })
        );
    }

    #[test]
    fn parses_successful_response() {
        let f = frame(
            Opcode::Frame,
            r#"{"cmd":"SET_ACTIVITY","data":{"name":"x"},"evt":null,"nonce":"n1"}"#,
        );
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Response {
                cmd: "SET_ACTIVITY".into(),
                nonce: Some("n1".into())
            }
        );
    }

    #[test]
    fn parses_other_events() {
        let f = frame(Opcode::Frame, r#"{"cmd":"DISPATCH","evt":"ACTIVITY_JOIN","data":{}}"#);
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Event {
                evt: "ACTIVITY_JOIN".into()
            }
        );
    }

    #[test]
    fn parses_close() {
        let f = frame(Opcode::Close, r#"{"code":4000,"message":"Invalid Client ID"}"#);
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Close(CloseInfo {
                code: 4000,
                message: "Invalid Client ID".into()
            })
        );
    }

    #[test]
    fn close_with_garbage_body_is_still_a_close() {
        let f = frame(Opcode::Close, "not json");
        assert_eq!(
            parse_incoming(&f).unwrap(),
            Incoming::Close(CloseInfo {
                code: 0,
                message: String::new()
            })
        );
    }

    #[test]
    fn ping_and_pong_keep_their_payload() {
        assert_eq!(
            parse_incoming(&frame(Opcode::Ping, r#"{"a":1}"#)).unwrap(),
            Incoming::Ping(r#"{"a":1}"#.into())
        );
        assert_eq!(
            parse_incoming(&frame(Opcode::Pong, "{}")).unwrap(),
            Incoming::Pong("{}".into())
        );
    }

    #[test]
    fn invalid_json_in_a_frame_is_malformed() {
        let f = frame(Opcode::Frame, "{nope");
        assert!(matches!(
            parse_incoming(&f),
            Err(ProtocolError::Malformed(_))
        ));
    }

    #[test]
    fn an_incoming_handshake_is_malformed() {
        let f = frame(Opcode::Handshake, "{}");
        assert!(matches!(
            parse_incoming(&f),
            Err(ProtocolError::Malformed(_))
        ));
    }
}
