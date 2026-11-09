//! The stream-json stdout parser (§10, FR-PRM-37).
//!
//! Each stdout line is parsed as JSON. `system/init` → [`StreamEvent::Init`]; `assistant`
//! text blocks → [`StreamEvent::AssistantText`]; partial `stream_event` text deltas (only
//! with `--include-partial-messages`) → [`StreamEvent::TextDelta`]; `result` ends the turn.
//! Unknown types are ignored ([`StreamEvent::Other`]). [`MALFORMED_RUN`] non-JSON lines in a
//! row → [`LineOutcome::Malformed`] (`stream_malformed`).

use serde_json::Value;

/// Non-JSON lines in a row that make the stream malformed (§10).
pub const MALFORMED_RUN: u32 = 3;

/// What `system/init` says, as far as the chat needs it. The lists hold names only.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InitInfo {
    pub claude_code_version: Option<String>,
    pub model: Option<String>,
    pub api_key_source: Option<String>,
    pub tools: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub plugins: Vec<String>,
    pub slash_commands: Vec<String>,
    pub agents: Vec<String>,
    pub skills: Vec<String>,
}

impl InitInfo {
    /// The name lists, labelled, for isolation checks.
    pub fn lists(&self) -> [(&'static str, &[String]); 6] {
        [
            ("tools", &self.tools),
            ("mcp_servers", &self.mcp_servers),
            ("plugins", &self.plugins),
            ("slash_commands", &self.slash_commands),
            ("agents", &self.agents),
            ("skills", &self.skills),
        ]
    }
}

/// The `result` message that ends a turn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnResult {
    pub subtype: String,
    pub is_error: bool,
    pub result: Option<String>,
    pub total_cost_usd: Option<f64>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub errors: Vec<String>,
}

/// One parsed stdout line.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamEvent {
    Init(Box<InitInfo>),
    /// The text blocks of one `assistant` message, concatenated.
    AssistantText(String),
    /// One partial text delta (`stream_event` / `content_block_delta` / `text_delta`).
    TextDelta(String),
    Result(TurnResult),
    /// Any other type (`system` subtypes other than init, `user`, `rate_limit_event`, …).
    Other(String),
}

/// What [`LineParser::push`] made of a line.
#[derive(Debug, Clone, PartialEq)]
pub enum LineOutcome {
    Event(StreamEvent),
    /// An empty line.
    Blank,
    /// A non-JSON line, fewer than [`MALFORMED_RUN`] in a row.
    NonJson,
    /// The [`MALFORMED_RUN`]th non-JSON line in a row.
    Malformed,
}

/// Line-by-line parser with the non-JSON run counter.
#[derive(Debug, Default)]
pub struct LineParser {
    non_json_run: u32,
}

impl LineParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, line: &str) -> LineOutcome {
        let line = line.trim();
        if line.is_empty() {
            return LineOutcome::Blank;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v.is_object() => {
                self.non_json_run = 0;
                LineOutcome::Event(parse_event(&v))
            }
            _ => {
                self.non_json_run += 1;
                if self.non_json_run >= MALFORMED_RUN {
                    LineOutcome::Malformed
                } else {
                    LineOutcome::NonJson
                }
            }
        }
    }
}

/// Classifies one JSON object.
pub fn parse_event(v: &Value) -> StreamEvent {
    let ty = v["type"].as_str().unwrap_or_default();
    match ty {
        "system" if v["subtype"] == "init" => StreamEvent::Init(Box::new(parse_init(v))),
        "assistant" => StreamEvent::AssistantText(text_blocks(&v["message"]["content"])),
        "stream_event" => {
            let ev = &v["event"];
            match (ev["type"].as_str(), ev["delta"]["type"].as_str()) {
                (Some("content_block_delta"), Some("text_delta")) => StreamEvent::TextDelta(
                    ev["delta"]["text"].as_str().unwrap_or_default().to_owned(),
                ),
                _ => StreamEvent::Other(format!(
                    "stream_event/{}",
                    ev["type"].as_str().unwrap_or("?")
                )),
            }
        }
        "result" => StreamEvent::Result(parse_result(v)),
        other => StreamEvent::Other(other.to_owned()),
    }
}

fn text_blocks(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

fn names(v: &Value) -> Vec<String> {
    v.as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| match item {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(o) => o.get("name").and_then(Value::as_str).map(str::to_owned),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().map(str::to_owned)
}

fn parse_init(v: &Value) -> InitInfo {
    InitInfo {
        claude_code_version: opt_str(&v["claude_code_version"]),
        model: opt_str(&v["model"]),
        api_key_source: opt_str(&v["apiKeySource"]),
        tools: names(&v["tools"]),
        mcp_servers: names(&v["mcp_servers"]),
        plugins: names(&v["plugins"]),
        slash_commands: names(&v["slash_commands"]),
        agents: names(&v["agents"]),
        skills: names(&v["skills"]),
    }
}

fn parse_result(v: &Value) -> TurnResult {
    let errors = match &v["errors"] {
        Value::Array(items) => items
            .iter()
            .map(|e| match e {
                Value::String(s) => s.clone(),
                other => other["message"]
                    .as_str()
                    .map_or_else(|| other.to_string(), str::to_owned),
            })
            .collect(),
        _ => Vec::new(),
    };
    TurnResult {
        subtype: v["subtype"].as_str().unwrap_or_default().to_owned(),
        is_error: v["is_error"].as_bool().unwrap_or(false),
        result: opt_str(&v["result"]),
        total_cost_usd: v["total_cost_usd"].as_f64(),
        input_tokens: v["usage"]["input_tokens"].as_u64(),
        output_tokens: v["usage"]["output_tokens"].as_u64(),
        errors,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fr_prm_37_three_non_json_lines_in_a_row_are_malformed() {
        let mut p = LineParser::new();
        assert_eq!(p.push("oops"), LineOutcome::NonJson);
        assert_eq!(p.push(""), LineOutcome::Blank);
        assert_eq!(p.push("[1,2]"), LineOutcome::NonJson);
        assert!(matches!(
            p.push(r#"{"type":"user"}"#),
            LineOutcome::Event(StreamEvent::Other(_))
        ));
        assert_eq!(p.push("a"), LineOutcome::NonJson);
        assert_eq!(p.push("b"), LineOutcome::NonJson);
        assert_eq!(p.push("c"), LineOutcome::Malformed);
    }

    #[test]
    fn fr_prm_37_result_errors_accept_strings_and_objects() {
        let v: Value = serde_json::from_str(
            r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom",{"message":"bad"}]}"#,
        )
        .unwrap();
        let StreamEvent::Result(r) = parse_event(&v) else {
            panic!("result")
        };
        assert!(r.is_error);
        assert_eq!(r.subtype, "error_during_execution");
        assert_eq!(r.errors, ["boom", "bad"]);
        assert_eq!(r.total_cost_usd, None);
    }
}
