// SPDX-License-Identifier: Apache-2.0

use serde::Deserialize;
use serde_json::{Value, json};

pub const BROADCAST_METHOD: &str = "blockchain.transaction.broadcast";

#[derive(Debug, PartialEq)]
pub enum ClientFrame {
    Broadcast { id: Value, raw_transaction: String },
    Passthrough { id: Option<Value> },
}

#[derive(Debug, PartialEq)]
pub struct ProtocolError {
    id: Value,
    code: i64,
    message: &'static str,
}

impl ProtocolError {
    #[must_use]
    pub fn response(&self) -> Vec<u8> {
        encode_response(&json!({
            "id": self.id,
            "error": {
                "code": self.code,
                "message": self.message,
            }
        }))
    }
}

#[derive(Deserialize)]
struct Request {
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Option<Value>,
}

/// Classifies one newline-delimited Electrum client request.
///
/// # Errors
///
/// Returns a protocol error when the request is malformed, batched, ambiguous,
/// or contains invalid transaction broadcast parameters.
pub fn classify(frame: &[u8]) -> Result<ClientFrame, ProtocolError> {
    let request = serde_json::from_slice::<Request>(frame).map_err(|_| invalid_request())?;
    let id = match request.id {
        Some(id @ (Value::Number(_) | Value::String(_))) => Some(id),
        Some(_) => return Err(invalid_request()),
        None => None,
    };
    if request.method != BROADCAST_METHOD {
        return Ok(ClientFrame::Passthrough { id });
    }

    let id = id.unwrap_or(Value::Null);
    let raw_transaction = match request.params {
        Some(Value::Array(values)) if values.len() == 1 => values
            .into_iter()
            .next()
            .and_then(|value| value.as_str().map(ToOwned::to_owned)),
        Some(Value::String(value)) => Some(value),
        _ => None,
    }
    .ok_or_else(|| invalid_params(id.clone()))?;

    if raw_transaction.is_empty()
        || raw_transaction.len() % 2 != 0
        || !raw_transaction.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(invalid_params(id));
    }

    Ok(ClientFrame::Broadcast {
        id,
        raw_transaction,
    })
}

#[must_use]
pub fn success_response(id: &Value, transaction_id: &str) -> Vec<u8> {
    encode_response(&json!({ "id": id, "result": transaction_id }))
}

#[must_use]
pub fn relay_error_response(id: &Value, relay_is_disabled: bool) -> Vec<u8> {
    let (code, message) = if relay_is_disabled {
        (-32_090, "private broadcasting is not configured")
    } else {
        (
            -32_091,
            "private broadcast failed; no public fallback was attempted",
        )
    };
    encode_response(&json!({
        "id": id,
        "error": { "code": code, "message": message }
    }))
}

#[must_use]
pub fn invalid_frame_response() -> Vec<u8> {
    invalid_request().response()
}

fn invalid_request() -> ProtocolError {
    ProtocolError {
        id: Value::Null,
        code: -32_600,
        message: "invalid Electrum request",
    }
}

fn invalid_params(id: Value) -> ProtocolError {
    ProtocolError {
        id,
        code: -32_602,
        message: "broadcast expects one hex-encoded transaction",
    }
}

fn encode_response(value: &Value) -> Vec<u8> {
    let mut response = serde_json::to_vec(&value).expect("JSON value serialization cannot fail");
    response.push(b'\n');
    response
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::{ClientFrame, classify};

    #[test]
    fn intercepts_array_and_legacy_string_broadcast_params() {
        let array = br#"{"id":7,"method":"blockchain.transaction.broadcast","params":["00aa"]}"#;
        assert_eq!(
            classify(array),
            Ok(ClientFrame::Broadcast {
                id: Value::from(7),
                raw_transaction: "00aa".into(),
            })
        );

        let string =
            br#"{"id":"wallet","method":"blockchain.transaction.broadcast","params":"00aa"}"#;
        assert_eq!(
            classify(string),
            Ok(ClientFrame::Broadcast {
                id: Value::from("wallet"),
                raw_transaction: "00aa".into(),
            })
        );
    }

    #[test]
    fn passes_through_non_broadcast_requests() {
        let request = br#"{"id":1,"method":"server.version","params":[]}"#;
        assert_eq!(
            classify(request),
            Ok(ClientFrame::Passthrough {
                id: Some(Value::from(1)),
            })
        );
    }

    #[test]
    fn rejects_batches_duplicate_methods_and_bad_hex() {
        assert!(classify(br"[]").is_err());
        assert!(
            classify(
                br#"{"id":1,"method":"server.version","method":"blockchain.transaction.broadcast","params":["00"]}"#
            )
            .is_err()
        );
        assert!(
            classify(
                br#"{"id":1,"method":"blockchain.transaction.broadcast","params":["not-hex"]}"#
            )
            .is_err()
        );
    }
}
