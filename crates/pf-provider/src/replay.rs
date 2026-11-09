//! The recording format (Appendix A) and [`ReplayTransport`], which plays a recording back in
//! order and checks every request against it: method, path, query, body shape, and that no
//! signed URL got an auth header (FR-PRV-09, FR-PRV-27). Recordings are `ScenarioProvider`
//! replay only (R17).

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::transport::{BoxFuture, HttpRequest, HttpResponse, Method, Transport, TransportError};

pub const FORMAT_VERSION: u32 = 1;

/// Appendix A: one recording file, `fixtures/provider/<model>/<case>.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recording {
    pub format_version: u32,
    pub provider: String,
    pub model: String,
    pub case: String,
    pub recorded_on: String,
    pub source: RecordingSource,
    pub exchanges: Vec<Exchange>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingSource {
    Recorded,
    Synthetic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exchange {
    pub request: RecordedRequest,
    pub response: RecordedResponse,
}

/// `path` is relative to the API base (`/models/x`) or an absolute non-API URL
/// (`fake://asset/1`). `body` is JSON, `{"fixture": "<name>.png"}` for binary, or absent
/// (not checked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedRequest {
    pub method: Method,
    pub path: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub query: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

/// `body`: JSON, `{"fixture": "<name>.png"}` for image bytes, or a string for raw text.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecordedResponse {
    pub status: u16,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub body: Value,
}

impl Recording {
    pub fn parse(json: &str) -> Result<Self, String> {
        let r: Recording = serde_json::from_str(json).map_err(|e| e.to_string())?;
        if r.format_version != FORMAT_VERSION {
            return Err(format!("unknown format_version {}", r.format_version));
        }
        Ok(r)
    }
}

/// The fixture a JSON value names, if it is `{"fixture": "<name>"}`.
pub fn fixture_name(v: &Value) -> Option<&str> {
    match v {
        Value::Object(m) if m.len() == 1 => m.get("fixture").and_then(Value::as_str),
        _ => None,
    }
}

/// Splits `?a=b&c=d` off a path. Values are percent-decoded.
pub fn split_query(path_and_query: &str) -> (String, BTreeMap<String, String>) {
    let (path, q) = path_and_query
        .split_once('?')
        .unwrap_or((path_and_query, ""));
    let query = q
        .split('&')
        .filter(|s| !s.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (percent_decode(k), percent_decode(v))
        })
        .collect();
    (percent_decode(path), query)
}

/// Minimal percent-decoding (paths and query values the adapter itself encoded).
pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        if bytes[i] == b'%'
            && let Some(b) = hex
        {
            out.push(b);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Does `actual` have the shape of `recorded`? Objects: same keys, recursively. Arrays: same
/// length, element-wise. A `{"fixture"}` matches any string (base64 data). Scalars: same type.
pub fn same_shape(recorded: &Value, actual: &Value) -> Result<(), String> {
    fn walk(r: &Value, a: &Value, at: &str) -> Result<(), String> {
        if fixture_name(r).is_some() {
            return if a.is_string() {
                Ok(())
            } else {
                Err(format!("{at}: expected file data, got {a}"))
            };
        }
        match (r, a) {
            (Value::Object(rm), Value::Object(am)) => {
                let rk: Vec<_> = rm.keys().collect();
                let ak: Vec<_> = am.keys().collect();
                if rk != ak {
                    return Err(format!("{at}: keys {ak:?}, expected {rk:?}"));
                }
                rm.iter()
                    .try_for_each(|(k, rv)| walk(rv, &am[k], &format!("{at}.{k}")))
            }
            (Value::Array(ra), Value::Array(aa)) => {
                if ra.len() != aa.len() {
                    return Err(format!("{at}: {} items, expected {}", aa.len(), ra.len()));
                }
                ra.iter()
                    .zip(aa)
                    .enumerate()
                    .try_for_each(|(i, (rv, av))| walk(rv, av, &format!("{at}[{i}]")))
            }
            (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Null, Value::Null) => Ok(()),
            _ => Err(format!("{at}: {a} doesn't match {r}")),
        }
    }
    walk(recorded, actual, "body")
}

/// What a replayed request looked like (for assertions).
#[derive(Debug, Clone, PartialEq)]
pub struct SeenRequest {
    pub method: Method,
    /// Relative to the API base, or the absolute URL.
    pub path: String,
    pub authorized: bool,
    /// The body, if it was JSON.
    pub body: Option<Value>,
}

/// Plays a recording back in order. A request that doesn't match the next exchange, or one
/// past the end, panics with what differed: replay tests fail loudly. A recorded status of 0
/// simulates a transport failure (body `"offline"` or `"timeout"`).
#[derive(Debug)]
pub struct ReplayTransport {
    base: String,
    pending: Mutex<VecDeque<Exchange>>,
    seen: Mutex<Vec<SeenRequest>>,
    /// `<model>/<case>`, for failure messages.
    pub name: String,
}

impl ReplayTransport {
    pub fn new(base: &str, recording: &Recording) -> Self {
        ReplayTransport {
            base: base.trim_end_matches('/').to_owned(),
            pending: Mutex::new(recording.exchanges.iter().cloned().collect()),
            seen: Mutex::new(vec![]),
            name: format!("{}/{}", recording.model, recording.case),
        }
    }

    pub fn from_exchanges(base: &str, exchanges: Vec<Exchange>) -> Self {
        ReplayTransport {
            base: base.trim_end_matches('/').to_owned(),
            pending: Mutex::new(exchanges.into()),
            seen: Mutex::new(vec![]),
            name: "inline".into(),
        }
    }

    /// Requests replayed so far.
    pub fn seen(&self) -> Vec<SeenRequest> {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Exchanges not yet replayed.
    pub fn remaining(&self) -> usize {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    fn relative(&self, url: &str) -> String {
        url.strip_prefix(&self.base)
            .map(str::to_owned)
            .unwrap_or_else(|| url.to_owned())
    }

    fn check(&self, ex: &Exchange, req: &HttpRequest) -> Result<(), String> {
        let (path, query) = split_query(&self.relative(&req.url));
        let r = &ex.request;
        if r.method != req.method || r.path != path {
            return Err(format!(
                "expected {} {}, got {} {path}",
                r.method.as_str(),
                r.path,
                req.method.as_str()
            ));
        }
        if r.query != query {
            return Err(format!("{path}: query {query:?}, expected {:?}", r.query));
        }
        match (&r.body, &req.body) {
            // No recorded body: not checked (hand-built exchanges in unit tests).
            (None, _) => Ok(()),
            (Some(rb), Some(ab)) if fixture_name(rb).is_some() => {
                if ab.is_empty() {
                    Err(format!("{path}: empty binary body"))
                } else {
                    Ok(())
                }
            }
            (Some(rb), Some(ab)) => {
                let actual: Value = serde_json::from_slice(ab)
                    .map_err(|e| format!("{path}: body is not JSON ({e})"))?;
                same_shape(rb, &actual).map_err(|e| format!("{path}: {e}"))
            }
            (r, a) => Err(format!(
                "{path}: body present {} vs recorded {}",
                a.is_some(),
                r.is_some()
            )),
        }
    }
}

impl Transport for ReplayTransport {
    fn send(&self, req: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        let ex = self
            .pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pop_front()
            .unwrap_or_else(|| panic!("replay {}: unexpected extra request {req:?}", self.name));
        if let Err(why) = self.check(&ex, &req) {
            panic!("replay {}: request mismatch: {why}", self.name);
        }
        let path = self.relative(&req.url);
        let is_api = req.url.starts_with(&self.base);
        assert!(
            is_api || req.authorization.is_none(),
            "replay {}: auth header sent to a non-API URL {path}",
            self.name
        );
        self.seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(SeenRequest {
                method: req.method,
                path,
                authorized: req.authorization.is_some(),
                body: req
                    .body
                    .as_deref()
                    .and_then(|b| serde_json::from_slice(b).ok()),
            });
        let resp = &ex.response;
        if resp.status == 0 {
            let err = match resp.body.as_str() {
                Some("timeout") => TransportError::Timeout,
                Some("interrupted") => TransportError::Interrupted,
                _ => TransportError::Offline("replayed".into()),
            };
            return Box::pin(std::future::ready(Err(err)));
        }
        let body = match (&resp.body, fixture_name(&resp.body)) {
            (_, Some(name)) => {
                crate::fixtures::by_name(name).unwrap_or_else(|| b"this is not an image".to_vec())
            }
            (Value::Null, _) => vec![],
            (Value::String(s), _) => s.clone().into_bytes(),
            (v, _) => v.to_string().into_bytes(),
        };
        Box::pin(std::future::ready(Ok(HttpResponse {
            status: resp.status,
            headers: resp
                .headers
                .iter()
                .map(|(k, v)| (k.to_ascii_lowercase(), v.clone()))
                .collect(),
            body,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn fr_prv_27_shape_matching() {
        assert!(
            same_shape(
                &json!({"a": 1, "b": {"fixture": "x.png"}}),
                &json!({"a": 2, "b": "data:..."})
            )
            .is_ok()
        );
        assert!(same_shape(&json!({"a": 1}), &json!({"a": 1, "b": 2})).is_err());
        assert!(same_shape(&json!({"a": [1]}), &json!({"a": ["x"]})).is_err());
    }

    #[test]
    fn query_split_and_decode() {
        let (p, q) = split_query("/generate/custom/model%20x?dryRun=true&a=b%2Fc");
        assert_eq!(p, "/generate/custom/model x");
        assert_eq!(q.get("dryRun").map(String::as_str), Some("true"));
        assert_eq!(q.get("a").map(String::as_str), Some("b/c"));
    }
}
