//! The recorder (FR-PRV-27): wraps a transport, captures every exchange, and turns them into
//! an Appendix A recording with the auth stripped (it is never captured: the auth header is a
//! separate [`crate::Secret`] field), ids and signed URLs replaced by placeholders, and binary
//! bodies replaced by generated-fixture refs.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::replay::{
    Exchange, FORMAT_VERSION, RecordedRequest, RecordedResponse, Recording, RecordingSource,
    split_query,
};
use crate::transport::{
    BoxFuture, HttpRequest, HttpResponse, KEPT_HEADERS, Method, Transport, TransportError,
};

#[derive(Debug, Clone)]
struct Raw {
    method: Method,
    url: String,
    body: Option<Vec<u8>>,
    response: Result<HttpResponse, TransportError>,
}

/// Captures exchanges while forwarding them to the real transport.
#[derive(Debug)]
pub struct RecordingTransport {
    inner: Arc<dyn Transport>,
    log: Arc<Mutex<Vec<Raw>>>,
}

impl RecordingTransport {
    pub fn new(inner: Arc<dyn Transport>) -> Self {
        RecordingTransport {
            inner,
            log: Arc::default(),
        }
    }
}

impl Transport for RecordingTransport {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        let (method, url, body) = (request.method, request.url.clone(), request.body.clone());
        let log = self.log.clone();
        let fut = self.inner.send(request);
        Box::pin(async move {
            let response = fut.await;
            log.lock().unwrap_or_else(|e| e.into_inner()).push(Raw {
                method,
                url,
                body,
                response: response.clone(),
            });
            response
        })
    }
}

/// Names generated fixtures by content hash (e.g. the room image the recorder sent).
pub type FixtureNames = HashMap<String, String>;

/// The SHA-256 hex of some bytes (fixture lookup key).
pub fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Default)]
struct Redactor {
    ids: BTreeMap<String, String>,
    counters: HashMap<&'static str, usize>,
    urls: usize,
}

impl Redactor {
    fn id(&mut self, raw: &str, kind: &'static str) {
        if raw.is_empty() || self.ids.contains_key(raw) {
            return;
        }
        let placeholder = match kind {
            "team" => "<team-id>".to_owned(),
            "project" => "<project-id>".to_owned(),
            _ => {
                let n = self.counters.entry(kind).or_insert(0);
                *n += 1;
                format!("<{kind}-{n}>")
            }
        };
        self.ids.insert(raw.to_owned(), placeholder);
    }

    /// Collects ids from a response body by key.
    fn collect(&mut self, v: &Value, parent: &str) {
        match v {
            Value::Object(m) => {
                for (k, x) in m {
                    match (k.as_str(), x) {
                        ("assetId", Value::String(s)) => self.id(s, "asset"),
                        ("assetIds", Value::Array(a)) => a
                            .iter()
                            .filter_map(Value::as_str)
                            .for_each(|s| self.id(s, "asset")),
                        ("jobId", Value::String(s)) => self.id(s, "job"),
                        ("uploadId", Value::String(s)) => self.id(s, "upload"),
                        ("teamId", Value::String(s)) => self.id(s, "team"),
                        ("projectId", Value::String(s)) => self.id(s, "project"),
                        ("id", Value::String(s)) => match parent {
                            "asset" | "assets" => self.id(s, "asset"),
                            "job" => self.id(s, "job"),
                            "upload" => self.id(s, "upload"),
                            _ => {}
                        },
                        _ => {}
                    }
                    self.collect(x, k);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| self.collect(x, parent)),
            _ => {}
        }
    }

    fn text(&self, s: &str) -> String {
        let mut out = s.to_owned();
        for (raw, placeholder) in &self.ids {
            out = out.replace(raw, placeholder);
        }
        out
    }

    fn url(&mut self, s: &str, base: &str) -> Option<String> {
        let is_url = s.starts_with("https://") || s.starts_with("http://");
        (is_url && !s.starts_with(base)).then(|| {
            self.urls += 1;
            format!("fake://asset/{}", self.urls)
        })
    }

    fn value(
        &mut self,
        v: &Value,
        base: &str,
        fixtures: &FixtureNames,
        urls: &mut HashMap<String, String>,
    ) -> Value {
        match v {
            Value::String(s) if s.starts_with("data:") => {
                let data = s.split_once(',').map_or("", |(_, d)| d);
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .unwrap_or_default();
                json!({"fixture": fixtures.get(&content_hash(&bytes)).cloned().unwrap_or_else(|| "upload.png".into())})
            }
            Value::String(s) => {
                if let Some(u) = urls.get(s) {
                    return Value::String(u.clone());
                }
                if let Some(u) = self.url(s, base) {
                    urls.insert(s.clone(), u.clone());
                    return Value::String(u);
                }
                Value::String(self.text(s))
            }
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|x| self.value(x, base, fixtures, urls))
                    .collect(),
            ),
            Value::Object(m) => Value::Object(
                m.iter()
                    .map(|(k, x)| (k.clone(), self.value(x, base, fixtures, urls)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
}

fn binary_ref(bytes: &[u8], fixtures: &FixtureNames, fallback: String) -> Value {
    json!({"fixture": fixtures.get(&content_hash(bytes)).cloned().unwrap_or(fallback)})
}

impl RecordingTransport {
    /// The captured exchanges as a redacted recording. `extra_ids` are ids known up front
    /// (the team id, the project id) that must never appear.
    pub fn to_recording(
        &self,
        base: &str,
        model: &str,
        case: &str,
        recorded_on: &str,
        fixtures: &FixtureNames,
        extra_ids: &[(&str, &'static str)],
    ) -> Recording {
        let base = base.trim_end_matches('/');
        let log = self.log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut r = Redactor::default();
        for (id, kind) in extra_ids {
            r.id(id, kind);
        }
        for raw in &log {
            if let Ok(resp) = &raw.response
                && let Ok(v) = serde_json::from_slice::<Value>(&resp.body)
            {
                r.collect(&v, "");
            }
        }
        let mut urls: HashMap<String, String> = HashMap::new();
        let mut candidates = 0;
        let exchanges = log
            .iter()
            .map(|raw| {
                let rel = raw
                    .url
                    .strip_prefix(base)
                    .map(str::to_owned)
                    .or_else(|| urls.get(&raw.url).cloned())
                    .unwrap_or_else(|| r.url(&raw.url, base).unwrap_or_default());
                let (path, query) = split_query(&rel);
                let request = RecordedRequest {
                    method: raw.method,
                    path: r.text(&path),
                    query: query.into_iter().map(|(k, v)| (k, r.text(&v))).collect(),
                    body: raw
                        .body
                        .as_ref()
                        .map(|b| match serde_json::from_slice::<Value>(b) {
                            Ok(v) => r.value(&v, base, fixtures, &mut urls),
                            Err(_) => binary_ref(b, fixtures, "upload-part.png".into()),
                        }),
                };
                let response = match &raw.response {
                    Ok(resp) => RecordedResponse {
                        status: resp.status,
                        headers: resp
                            .headers
                            .iter()
                            .filter(|(k, _)| KEPT_HEADERS.contains(&k.as_str()))
                            .cloned()
                            .collect(),
                        body: match serde_json::from_slice::<Value>(&resp.body) {
                            Ok(v) => r.value(&v, base, fixtures, &mut urls),
                            Err(_) if resp.body.is_empty() => Value::Null,
                            Err(_) => {
                                candidates += 1;
                                binary_ref(
                                    &resp.body,
                                    fixtures,
                                    format!("candidate-{candidates}-512.png"),
                                )
                            }
                        },
                    },
                    Err(e) => RecordedResponse {
                        status: 0,
                        headers: BTreeMap::new(),
                        body: json!(if *e == TransportError::Timeout {
                            "timeout"
                        } else {
                            "offline"
                        }),
                    },
                };
                Exchange { request, response }
            })
            .collect();
        Recording {
            format_version: FORMAT_VERSION,
            provider: "scenario".into(),
            model: model.into(),
            case: case.into(),
            recorded_on: recorded_on.into(),
            source: RecordingSource::Recorded,
            exchanges,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::replay::{ReplayTransport, fixture_name};
    use crate::scenario::api::DEFAULT_BASE_URL;
    use crate::secret::Secret;
    use std::time::Duration;

    /// A scripted "live" server for the recorder test.
    #[derive(Debug)]
    struct Scripted(Mutex<Vec<HttpResponse>>);

    impl Transport for Scripted {
        fn send(&self, _r: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
            let next = self.0.lock().unwrap().remove(0);
            Box::pin(std::future::ready(Ok(next)))
        }
    }

    fn ok(body: Value) -> HttpResponse {
        HttpResponse {
            status: 200,
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("set-cookie".into(), "s=1".into()),
            ],
            body: body.to_string().into_bytes(),
        }
    }

    #[test]
    fn fr_prv_27_recorder_strips_auth_and_replaces_ids_urls_and_binaries() {
        let room = crate::fixtures::room_png(64);
        let fixtures: FixtureNames = [(content_hash(&room), "room-64.png".to_owned())].into();
        let live = Arc::new(Scripted(Mutex::new(vec![
            ok(
                json!({"asset": {"id": "asset_Zx81", "url": "https://cdn.example.com/a.png?X-Amz-Signature=sig"}}),
            ),
            ok(
                json!({"job": {"jobId": "job_9Qw", "status": "success", "metadata": {"assetIds": ["asset_Out1"]}}}),
            ),
            HttpResponse {
                status: 200,
                headers: vec![],
                body: vec![0x89, b'P', b'N', b'G', 1, 2, 3],
            },
        ])));
        let rec = RecordingTransport::new(live);
        let send = |method, url: &str, body: Option<Vec<u8>>| {
            crate::block_on(rec.send(HttpRequest {
                method,
                url: url.into(),
                headers: vec![],
                authorization: Some(Secret::new("Basic c2VjcmV0LWtleQ==")),
                body,
                timeout: Duration::from_secs(1),
            }))
            .unwrap()
        };
        use base64::Engine as _;
        let data_url = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&room)
        );
        send(
            Method::Post,
            &format!("{DEFAULT_BASE_URL}/assets?projectId=proj_secret1"),
            Some(
                json!({"image": data_url, "hide": true})
                    .to_string()
                    .into_bytes(),
            ),
        );
        send(
            Method::Get,
            &format!("{DEFAULT_BASE_URL}/jobs/job_9Qw"),
            None,
        );
        send(
            Method::Get,
            "https://cdn.example.com/a.png?X-Amz-Signature=sig",
            None,
        );

        let out = rec.to_recording(
            DEFAULT_BASE_URL,
            "m",
            "c",
            "2026-11-07",
            &fixtures,
            &[("proj_secret1", "project")],
        );
        let text = serde_json::to_string(&out).unwrap();
        for leak in [
            "c2VjcmV0",
            "Basic",
            "asset_Zx81",
            "job_9Qw",
            "asset_Out1",
            "cdn.example.com",
            "Signature",
            "proj_secret1",
            "set-cookie",
        ] {
            assert!(!text.contains(leak), "{leak} leaked: {text}");
        }
        assert_eq!(
            out.exchanges[0].request.body.as_ref().unwrap()["image"],
            json!({"fixture": "room-64.png"})
        );
        assert_eq!(out.exchanges[0].request.query["projectId"], "<project-id>");
        assert_eq!(out.exchanges[0].response.body["asset"]["id"], "<asset-1>");
        assert_eq!(
            out.exchanges[0].response.body["asset"]["url"],
            "fake://asset/1"
        );
        assert_eq!(out.exchanges[1].request.path, "/jobs/<job-1>");
        assert_eq!(out.exchanges[2].request.path, "fake://asset/1");
        assert_eq!(
            fixture_name(&out.exchanges[2].response.body),
            Some("candidate-1-512.png")
        );

        // A recording replays (round trip through the file format).
        let back = Recording::parse(&text).unwrap();
        let replay = ReplayTransport::new(DEFAULT_BASE_URL, &back);
        assert_eq!(replay.remaining(), 3);
    }
}
