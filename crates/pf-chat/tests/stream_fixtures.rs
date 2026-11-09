//! The stream parser and the scrubber over the chat fixtures (PRM §13): the hand-made lines
//! in `fixtures/chat/handmade/`, and any real recordings in `fixtures/chat/claude-code/`.
#![allow(clippy::unwrap_used)]

use std::path::{Path, PathBuf};

use pf_chat::scrub::Scrubber;
use pf_chat::stream::{InitInfo, LineOutcome, LineParser, StreamEvent, TurnResult};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/chat")
}

/// A Windows home path built at run time (never written in a tracked file).
fn fake_home() -> String {
    ["C:", "Users", "alice"].join("\\")
}

fn cwd() -> String {
    format!("{}\\AppData\\Roaming\\pocket-forge\\chat\\t-1", fake_home())
}

fn lines(name: &str) -> Vec<String> {
    let text = std::fs::read_to_string(fixtures().join("handmade").join(name)).unwrap();
    text.lines()
        .map(|l| l.replace("{CWD}", &cwd().replace('\\', "\\\\")))
        .collect()
}

#[derive(Debug, Default)]
struct Parsed {
    init: Option<InitInfo>,
    text: String,
    deltas: String,
    result: Option<TurnResult>,
    malformed: bool,
}

fn parse(lines: &[String]) -> Parsed {
    let mut p = LineParser::new();
    let mut out = Parsed::default();
    for l in lines {
        match p.push(l) {
            LineOutcome::Event(StreamEvent::Init(i)) => out.init = Some(*i),
            LineOutcome::Event(StreamEvent::AssistantText(t)) => out.text.push_str(&t),
            LineOutcome::Event(StreamEvent::TextDelta(t)) => out.deltas.push_str(&t),
            LineOutcome::Event(StreamEvent::Result(r)) => out.result = Some(r),
            LineOutcome::Malformed => out.malformed = true,
            _ => {}
        }
    }
    out
}

#[test]
fn fr_prm_37_success_with_usage() {
    let p = parse(&lines("success-image.jsonl"));
    let init = p.init.unwrap();
    assert_eq!(init.claude_code_version.as_deref(), Some("2.1.319"));
    assert!(init.tools.is_empty() && init.plugins.is_empty());
    assert_eq!(init.agents, ["general-purpose"]);
    assert!(p.text.contains("```prompt\n"));
    let r = p.result.unwrap();
    assert_eq!((r.subtype.as_str(), r.is_error), ("success", false));
    assert_eq!(r.total_cost_usd, Some(0.0123));
    assert_eq!((r.input_tokens, r.output_tokens), (Some(1650), Some(58)));
}

#[test]
fn fr_prm_37_error_results() {
    let p = parse(&lines("not-logged-in.jsonl"));
    let r = p.result.unwrap();
    assert!(r.is_error);
    assert!(r.result.unwrap().contains("Please run /login"));

    let p = parse(&lines("usage-limit.jsonl"));
    let r = p.result.unwrap();
    assert!(r.is_error);
    assert!(r.result.unwrap().contains("limit"));
}

#[test]
fn fr_prm_27_partial_deltas_without_result() {
    let p = parse(&lines("partial-cancel.jsonl"));
    assert_eq!(p.deltas, "1\n2\n3\n4\n");
    assert!(p.result.is_none());
    assert!(p.text.is_empty());
}

#[test]
fn fr_prm_37_malformed_after_three_non_json_lines() {
    let p = parse(&lines("malformed.jsonl"));
    assert!(p.init.is_some());
    assert!(p.malformed);
}

/// Scrubbing keeps every line parseable to the same events, minus the scrubbed fields.
#[test]
fn prm_s10_scrubbed_handmade_lines_parse_the_same_and_do_not_leak() {
    for name in [
        "success-image.jsonl",
        "two-proposals.jsonl",
        "not-logged-in.jsonl",
        "usage-limit.jsonl",
        "partial-cancel.jsonl",
    ] {
        let raw = lines(name);
        let mut s = Scrubber::new();
        s.path(&cwd(), "<data-dir>/chat/<thread>")
            .path(&fake_home(), "<home>")
            .word("alice", "<user>");
        let scrubbed: Vec<String> = raw.iter().map(|l| s.scrub_line(l)).collect();
        let joined = scrubbed.join("\n");
        assert!(
            s.leaks(&joined).is_empty(),
            "{name}: {:?}",
            s.leaks(&joined)
        );
        assert!(joined.contains(r#""cwd":"<data-dir>/chat/<thread>""#) || !joined.contains("cwd"));
        assert!(!joined.contains("msg_01HANDMADE"), "{name}");

        let (a, b) = (parse(&raw), parse(&scrubbed));
        assert_eq!(a.text, b.text, "{name}");
        assert_eq!(a.deltas, b.deltas, "{name}");
        assert_eq!(a.result, b.result, "{name}");
        assert_eq!(a.init.map(|i| i.model), b.init.map(|i| i.model), "{name}");
    }
}

/// Real recordings (once the owner has run the probe) parse and are clean.
#[test]
fn prm_s10_recordings_parse_and_are_scrubbed() {
    let dir = fixtures().join("claude-code");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let checker = Scrubber::new();
    for e in entries.flatten() {
        let path = e.path();
        if path.extension().is_none_or(|x| x != "jsonl") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            checker.leaks(&text).is_empty(),
            "{name}: {:?}",
            checker.leaks(&text)
        );
        let raw: Vec<String> = text.lines().map(str::to_owned).collect();
        let p = parse(&raw);
        if !name.contains(".failed.") && !name.starts_with("partial") {
            assert!(p.result.is_some(), "{name}: no result line");
        }
        if let Some(init) = p.init {
            for (list, names) in init.lists() {
                assert!(names.is_empty(), "{name}: init {list} not scrubbed");
            }
        }
    }
}
