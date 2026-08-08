//! Wire codecs shared by adapters.
//!
//! - [`jsonrpc`] — JSON-RPC 2.0 framing used by the ACP adapter (over stdio).
//! - [`msgpack`] — MessagePack helpers used by the managed-provider adapter (over WebSocket).

/// JSON-RPC 2.0 message model. Line-delimited JSON on the wire.
pub mod jsonrpc {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    /// A request/response id: number or string (JSON-RPC allows both).
    #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum RpcId {
        Num(i64),
        Str(String),
    }

    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct RpcError {
        pub code: i64,
        pub message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub data: Option<Value>,
    }

    /// A permissive JSON-RPC frame. We classify it after parsing because an ACP
    /// peer multiplexes requests, responses, and notifications on one stream.
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    pub struct RpcMessage {
        pub jsonrpc: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub id: Option<RpcId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub method: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub params: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub result: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pub error: Option<RpcError>,
    }

    /// What a parsed frame actually is.
    #[derive(Clone, Debug, PartialEq)]
    pub enum RpcKind {
        /// Peer asks us to do something and expects a response.
        Request {
            id: RpcId,
            method: String,
            params: Value,
        },
        /// Peer informs us; no response expected.
        Notification { method: String, params: Value },
        /// Peer answers one of our requests.
        Response {
            id: RpcId,
            result: Result<Value, RpcError>,
        },
    }

    impl RpcMessage {
        pub fn request(id: RpcId, method: impl Into<String>, params: Value) -> Self {
            Self {
                jsonrpc: "2.0".into(),
                id: Some(id),
                method: Some(method.into()),
                params: Some(params),
                result: None,
                error: None,
            }
        }

        pub fn notification(method: impl Into<String>, params: Value) -> Self {
            Self {
                jsonrpc: "2.0".into(),
                id: None,
                method: Some(method.into()),
                params: Some(params),
                result: None,
                error: None,
            }
        }

        pub fn response_ok(id: RpcId, result: Value) -> Self {
            Self {
                jsonrpc: "2.0".into(),
                id: Some(id),
                method: None,
                params: None,
                result: Some(result),
                error: None,
            }
        }

        pub fn response_err(id: RpcId, error: RpcError) -> Self {
            Self {
                jsonrpc: "2.0".into(),
                id: Some(id),
                method: None,
                params: None,
                result: None,
                error: Some(error),
            }
        }

        /// Classify the frame into a [`RpcKind`].
        pub fn classify(&self) -> RpcKind {
            let params = self.params.clone().unwrap_or(Value::Null);
            match (&self.method, &self.id) {
                (Some(method), Some(id)) => RpcKind::Request {
                    id: id.clone(),
                    method: method.clone(),
                    params,
                },
                (Some(method), None) => RpcKind::Notification {
                    method: method.clone(),
                    params,
                },
                (None, Some(id)) => RpcKind::Response {
                    id: id.clone(),
                    result: match &self.error {
                        Some(e) => Err(e.clone()),
                        None => Ok(self.result.clone().unwrap_or(Value::Null)),
                    },
                },
                (None, None) => RpcKind::Notification {
                    method: String::new(),
                    params,
                },
            }
        }

        /// Encode as a single newline-terminated JSON line (stdio framing).
        pub fn to_line(&self) -> Result<String, serde_json::Error> {
            let mut s = serde_json::to_string(self)?;
            s.push('\n');
            Ok(s)
        }

        pub fn from_line(line: &str) -> Result<Self, serde_json::Error> {
            serde_json::from_str(line.trim())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde_json::json;

        #[test]
        fn classifies_request_notification_and_response() {
            let req = RpcMessage::request(RpcId::Num(1), "session/new", json!({"cwd":"/tmp"}));
            assert!(matches!(req.classify(), RpcKind::Request { .. }));

            let note = RpcMessage::notification("session/update", json!({"x":1}));
            assert!(matches!(note.classify(), RpcKind::Notification { .. }));

            let ok = RpcMessage::response_ok(RpcId::Num(1), json!({"ok":true}));
            assert!(matches!(
                ok.classify(),
                RpcKind::Response { result: Ok(_), .. }
            ));

            let err = RpcMessage::response_err(
                RpcId::Str("a".into()),
                RpcError {
                    code: -32601,
                    message: "no".into(),
                    data: None,
                },
            );
            assert!(matches!(
                err.classify(),
                RpcKind::Response { result: Err(_), .. }
            ));
        }

        #[test]
        fn line_round_trip() {
            let m = RpcMessage::request(RpcId::Num(7), "initialize", json!({"v":1}));
            let line = m.to_line().unwrap();
            assert!(line.ends_with('\n'));
            let back = RpcMessage::from_line(&line).unwrap();
            assert_eq!(m, back);
        }

        #[test]
        fn parses_real_acp_shaped_frame() {
            let line = r#"{"jsonrpc":"2.0","id":1,"method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"t"}}}"#;
            let m = RpcMessage::from_line(line).unwrap();
            match m.classify() {
                RpcKind::Request { method, .. } => assert_eq!(method, "session/request_permission"),
                other => panic!("expected request, got {other:?}"),
            }
        }
    }
}

/// MessagePack helpers (managed-provider transport). Works on native and wasm.
pub mod msgpack {
    use crate::error::{Error, Result};
    use serde::{de::DeserializeOwned, Serialize};

    pub fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>> {
        rmp_serde::to_vec_named(value).map_err(|e| Error::Codec(e.to_string()))
    }

    pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
        rmp_serde::from_slice(bytes).map_err(|e| Error::Codec(e.to_string()))
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use serde::{Deserialize, Serialize};

        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Envelope {
            schema_version: u32,
            kind: String,
        }

        #[test]
        fn msgpack_round_trip() {
            let e = Envelope {
                schema_version: 1,
                kind: "timeline".into(),
            };
            let bytes = encode(&e).unwrap();
            let back: Envelope = decode(&bytes).unwrap();
            assert_eq!(e, back);
        }
    }
}
