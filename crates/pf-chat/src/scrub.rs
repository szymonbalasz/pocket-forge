//! The recordings scrubber (§10, "Recordings scrub"; AGENTS §3.1).
//!
//! - `session_id`, `uuid` and message / request / tool ids → `<session-N>`, `<uuid-N>`,
//!   `<msg-N>`, `<req-N>`, `<toolu-N>` (the same raw id always gets the same placeholder).
//! - Registered absolute paths (cwd, data dir, home) → their placeholders; any other
//!   `<drive>:\Users\<name>`, `/Users/<name>`, `/home/<name>` → `<home>`.
//! - Claude Code project slugs (`C--Users-<name>-…`, a path with every separator turned
//!   into `-`) → `<project-slug>`; `init.memory_paths` values → `<memory-path>`; any other
//!   string value that is an absolute path (e.g. `powershell_path`) → `<path>`.
//! - E-mail addresses → `<email>`; organisation and account ids → `<org-id>` / `<account-id>`.
//! - `system/init`: `tools`, `mcp_servers`, `plugins`, `slash_commands`, `agents` and
//!   `skills` → `[]`; `apiKeySource` → `<redacted>`.
//! - Image data → `{"fixture": "<name>.jpg"}`.
//!
//! Non-JSON lines get the text rules. [`Scrubber::leaks`] re-checks the output; the probe
//! refuses to write a recording that still leaks.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

/// Keys emptied in `system/init` (§10).
pub const INIT_LISTS: [&str; 6] = [
    "tools",
    "mcp_servers",
    "plugins",
    "slash_commands",
    "agents",
    "skills",
];

const ID_PREFIXES: [(&str, &str); 3] = [("msg_", "msg"), ("req_", "req"), ("toolu_", "toolu")];

#[derive(Debug, Default)]
struct IdMap {
    map: HashMap<String, String>,
    counters: HashMap<&'static str, u32>,
}

impl IdMap {
    fn get(&mut self, ns: &'static str, raw: &str) -> String {
        if let Some(p) = self.map.get(raw) {
            return p.clone();
        }
        let n = self.counters.entry(ns).or_insert(0);
        *n += 1;
        let p = format!("<{ns}-{n}>");
        self.map.insert(raw.to_owned(), p.clone());
        p
    }
}

/// See the module docs.
#[derive(Debug, Default)]
pub struct Scrubber {
    /// (literal, placeholder), longest literal first; matched ASCII-case-insensitively.
    literals: Vec<(String, String)>,
    /// Whole words (e.g. the user name), matched case-sensitively.
    words: Vec<(String, String)>,
    ids: IdMap,
    image_names: Vec<String>,
    images_seen: usize,
}

impl Scrubber {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces `path` (with either slash style, and with doubled backslashes) by
    /// `placeholder`. Register the longest, most specific paths too; order doesn't matter.
    pub fn path(&mut self, path: &str, placeholder: &str) -> &mut Self {
        let path = path.trim_end_matches(['\\', '/']);
        if path.len() < 3 {
            return self;
        }
        let variants = [
            path.to_owned(),
            path.replace('\\', "/"),
            path.replace('/', "\\"),
            path.replace('/', "\\").replace('\\', "\\\\"),
        ];
        for v in variants {
            if !self.literals.iter().any(|(l, _)| *l == v) {
                self.literals.push((v, placeholder.to_owned()));
            }
        }
        self.literals
            .sort_by_key(|(l, _)| std::cmp::Reverse(l.len()));
        self
    }

    /// Replaces `word` as a whole word (e.g. the user name; ignored under 3 characters).
    pub fn word(&mut self, word: &str, placeholder: &str) -> &mut Self {
        if word.len() >= 3 {
            self.words.push((word.to_owned(), placeholder.to_owned()));
        }
        self
    }

    /// Fixture names for image blocks, in order (`image-N.jpg` after the list runs out).
    pub fn image_names(&mut self, names: &[&str]) -> &mut Self {
        self.image_names = names.iter().map(|s| (*s).to_owned()).collect();
        self
    }

    /// Scrubs one recorded stdout line.
    pub fn scrub_line(&mut self, line: &str) -> String {
        match serde_json::from_str::<Value>(line.trim()) {
            Ok(mut v) if v.is_object() => {
                self.scrub_value(&mut v, "");
                v.to_string()
            }
            _ => self.scrub_text(line),
        }
    }

    /// Scrubs a JSON value in place. `key` is the key it sits under ("" at the top).
    pub fn scrub_value(&mut self, v: &mut Value, key: &str) {
        match v {
            Value::Object(o) => self.scrub_object(o),
            Value::Array(items) => {
                for item in items {
                    self.scrub_value(item, key);
                }
            }
            Value::String(s) => *s = self.scrub_keyed_string(key, s),
            _ => {}
        }
    }

    fn scrub_object(&mut self, o: &mut Map<String, Value>) {
        let is_init = o.get("type").and_then(Value::as_str) == Some("system")
            && o.get("subtype").and_then(Value::as_str) == Some("init");
        if is_init {
            for k in INIT_LISTS {
                if o.contains_key(k) {
                    o.insert(k.to_owned(), json!([]));
                }
            }
        }
        if o.get("type").and_then(Value::as_str) == Some("image") && o.contains_key("source") {
            let name = self
                .image_names
                .get(self.images_seen)
                .cloned()
                .unwrap_or_else(|| format!("image-{}.jpg", self.images_seen + 1));
            self.images_seen += 1;
            o.insert("source".into(), json!({ "fixture": name }));
        }
        for (k, child) in o.iter_mut() {
            if k == "source" && child.get("fixture").is_some() {
                continue;
            }
            if k == "memory_paths" {
                redact_strings(child, "<memory-path>");
                continue;
            }
            self.scrub_value(child, k);
        }
    }

    fn scrub_keyed_string(&mut self, key: &str, s: &str) -> String {
        let k = key.to_ascii_lowercase().replace(['_', '-'], "");
        if k == "apikeysource" {
            return "<redacted>".into();
        }
        if k.contains("email") {
            return "<email>".into();
        }
        if k.starts_with("org") && (k.ends_with("id") || k.ends_with("uuid")) {
            return "<org-id>".into();
        }
        if k.starts_with("organization") && (k.ends_with("id") || k.ends_with("uuid")) {
            return "<org-id>".into();
        }
        if k.starts_with("account") && (k.ends_with("id") || k.ends_with("uuid")) {
            return "<account-id>".into();
        }
        if k == "sessionid" && !s.is_empty() {
            return self.ids.get("session", s);
        }
        if (k == "uuid" || k == "parentuuid") && !s.is_empty() {
            return self.ids.get("uuid", s);
        }
        let out = self.scrub_text(s);
        if is_absolute_path(&out) {
            return "<path>".into();
        }
        out
    }

    /// The text rules: ids, registered paths, other home paths, e-mails, words.
    pub fn scrub_text(&mut self, s: &str) -> String {
        let ids = &mut self.ids;
        let s = replace_spans(s, uuid_at, |m| ids.get("uuid", m));
        let s = replace_spans(&s, prefixed_id_at, |m| {
            let ns = ID_PREFIXES
                .iter()
                .find(|(p, _)| m.starts_with(p))
                .map_or("id", |(_, ns)| *ns);
            ids.get(ns, m)
        });
        let mut s = s;
        for (lit, placeholder) in &self.literals {
            s = replace_spans(
                &s,
                |b, i| literal_at(b, i, lit.as_bytes()),
                |_| placeholder.clone(),
            );
        }
        let s = replace_spans(&s, home_path_at, |_| "<home>".into());
        let s = replace_spans(&s, project_slug_at, |_| "<project-slug>".into());
        let mut s = replace_spans(&s, email_at, |_| "<email>".into());
        for (word, placeholder) in &self.words {
            s = replace_spans(
                &s,
                |b, i| word_at(b, i, word.as_bytes()),
                |_| placeholder.clone(),
            );
        }
        s
    }

    /// Why `text` still isn't safe to commit (empty when it is).
    pub fn leaks(&self, text: &str) -> Vec<&'static str> {
        let b = text.as_bytes();
        let mut out = Vec::new();
        let any =
            |f: &dyn Fn(&[u8], usize) -> Option<usize>| (0..b.len()).any(|i| f(b, i).is_some());
        if any(&home_path_at) {
            out.push("a home-folder path");
        }
        if any(&project_slug_at) {
            out.push("a Claude Code project slug");
        }
        if any(&drive_path_at) {
            out.push("an absolute path");
        }
        if any(&email_at) {
            out.push("an e-mail address");
        }
        if self
            .literals
            .iter()
            .any(|(l, _)| any(&|b, i| literal_at(b, i, l.as_bytes())))
        {
            out.push("a registered local path");
        }
        if self
            .words
            .iter()
            .any(|(w, _)| any(&|b, i| word_at(b, i, w.as_bytes())))
        {
            out.push("the user name");
        }
        if any(&uuid_at) {
            out.push("an unscrubbed id");
        }
        out
    }
}

/// Replaces every span `find` reports (start `i` → `Some(end)`) by `repl(span)`. `find`
/// must only match at ASCII bytes, so spans sit on char boundaries.
fn replace_spans(
    s: &str,
    mut find: impl FnMut(&[u8], usize) -> Option<usize>,
    mut repl: impl FnMut(&str) -> String,
) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let (mut i, mut last) = (0, 0);
    while i < b.len() {
        match find(b, i) {
            Some(end) if end > i && s.is_char_boundary(i) && s.is_char_boundary(end) => {
                out.push_str(&s[last..i]);
                out.push_str(&repl(&s[i..end]));
                i = end;
                last = end;
            }
            _ => i += 1,
        }
    }
    out.push_str(&s[last..]);
    out
}

fn is_word(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn boundary_before(b: &[u8], i: usize) -> bool {
    i == 0 || !is_word(b[i - 1])
}

fn literal_at(b: &[u8], i: usize, lit: &[u8]) -> Option<usize> {
    let end = i + lit.len();
    (end <= b.len() && b[i..end].eq_ignore_ascii_case(lit)).then_some(end)
}

fn word_at(b: &[u8], i: usize, word: &[u8]) -> Option<usize> {
    let end = i + word.len();
    (boundary_before(b, i)
        && end <= b.len()
        && &b[i..end] == word
        && b.get(end).is_none_or(|c| !is_word(*c)))
    .then_some(end)
}

/// 8-4-4-4-12 hex digits, not inside a longer word.
fn uuid_at(b: &[u8], i: usize) -> Option<usize> {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    if !boundary_before(b, i) {
        return None;
    }
    let mut p = i;
    for (n, len) in GROUPS.iter().enumerate() {
        if n > 0 {
            if b.get(p) != Some(&b'-') {
                return None;
            }
            p += 1;
        }
        for _ in 0..*len {
            if !b.get(p).is_some_and(u8::is_ascii_hexdigit) {
                return None;
            }
            p += 1;
        }
    }
    b.get(p).is_none_or(|c| !is_word(*c)).then_some(p)
}

/// Every string inside `v` → `placeholder`.
fn redact_strings(v: &mut Value, placeholder: &str) {
    match v {
        Value::String(s) => *s = placeholder.to_owned(),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|i| redact_strings(i, placeholder)),
        Value::Object(o) => o.values_mut().for_each(|i| redact_strings(i, placeholder)),
        _ => {}
    }
}

/// `X:\…`, `X:/…`, `\\…` (UNC or pipe) or `/…`: an absolute path as a whole value.
fn is_absolute_path(s: &str) -> bool {
    let b = s.as_bytes();
    let drive = b.len() >= 3
        && b[0].is_ascii_alphabetic()
        && b[1] == b':'
        && (b[2] == b'\\' || b[2] == b'/');
    drive || s.starts_with("\\\\") || (s.starts_with('/') && s.len() > 1)
}

/// `X:\` or `X:/` not inside a word (for [`Scrubber::leaks`]).
fn drive_path_at(b: &[u8], i: usize) -> Option<usize> {
    (b[i].is_ascii_alphabetic()
        && boundary_before(b, i)
        && b.get(i + 1) == Some(&b':')
        && matches!(b.get(i + 2), Some(b'\\' | b'/')))
    .then_some(i + 3)
}

/// A Claude Code project slug (a path with every separator turned into `-`): an upper-case
/// drive letter and `--` (Windows) or `-Users-` / `-home-` (Unix), then at least two more
/// slug segments. Prose like `a--b` doesn't match.
fn project_slug_at(b: &[u8], i: usize) -> Option<usize> {
    let slug = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.');
    let start = if b[i].is_ascii_uppercase()
        && boundary_before(b, i)
        && b[i..].get(1..3) == Some(&b"--"[..])
        && b.get(i + 3).is_some_and(u8::is_ascii_alphanumeric)
    {
        i + 3
    } else if b[i] == b'-'
        && (i == 0 || !slug(&b[i - 1]))
        && (b[i..].starts_with(b"-Users-") || b[i..].starts_with(b"-home-"))
    {
        i + 1
    } else {
        return None;
    };
    let mut p = start;
    while b.get(p).is_some_and(slug) {
        p += 1;
    }
    (b[start..p].contains(&b'-')).then_some(p)
}

/// `msg_…`, `req_…`, `toolu_…`: at least 16 alphanumerics with a digit (real ids are
/// ~24), so names like `msg_requeue_v1` stay.
fn prefixed_id_at(b: &[u8], i: usize) -> Option<usize> {
    if !boundary_before(b, i) {
        return None;
    }
    let (prefix, _) = ID_PREFIXES
        .iter()
        .find(|(p, _)| b[i..].starts_with(p.as_bytes()))?;
    let start = i + prefix.len();
    let mut p = start;
    while b.get(p).is_some_and(u8::is_ascii_alphanumeric) {
        p += 1;
    }
    let digit = b[start..p].iter().any(u8::is_ascii_digit);
    (p - start >= 16 && digit).then_some(p)
}

fn name_end(b: &[u8], mut p: usize) -> Option<usize> {
    let start = p;
    while b
        .get(p)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
    {
        p += 1;
    }
    (p > start).then_some(p)
}

fn skip_seps(b: &[u8], mut p: usize) -> Option<usize> {
    let start = p;
    while b.get(p).is_some_and(|c| *c == b'\\' || *c == b'/') {
        p += 1;
    }
    (p > start).then_some(p)
}

/// `<drive>:\Users\<name>` (any slashes), `/Users/<name>`, `/home/<name>`.
fn home_path_at(b: &[u8], i: usize) -> Option<usize> {
    if b[i].is_ascii_alphabetic() && b.get(i + 1) == Some(&b':') && boundary_before(b, i) {
        let p = skip_seps(b, i + 2)?;
        let p = literal_at(b, p, b"users")?;
        let p = skip_seps(b, p)?;
        return name_end(b, p);
    }
    if b[i] == b'/' && (i == 0 || !(is_word(b[i - 1]) || b[i - 1] == b':')) {
        for root in [&b"/Users/"[..], &b"/home/"[..]] {
            if b[i..].starts_with(root) {
                return name_end(b, i + root.len());
            }
        }
    }
    None
}

/// `<local>@<domain>.<tld>` (TLD ≥ 2 letters).
fn email_at(b: &[u8], i: usize) -> Option<usize> {
    let local = |c: &u8| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'%' | b'+' | b'-');
    if !local(&b[i]) || (i > 0 && local(&b[i - 1])) {
        return None;
    }
    let mut p = i;
    while b.get(p).is_some_and(local) {
        p += 1;
    }
    if b.get(p) != Some(&b'@') {
        return None;
    }
    p += 1;
    let dstart = p;
    while b
        .get(p)
        .is_some_and(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-'))
    {
        p += 1;
    }
    let mut end = p;
    while end > dstart && b[end - 1] == b'.' {
        end -= 1;
    }
    let domain = &b[dstart..end];
    let dot = domain.iter().rposition(|c| *c == b'.')?;
    let tld = &domain[dot + 1..];
    (dot > 0 && tld.len() >= 2 && tld.iter().all(u8::is_ascii_alphabetic)).then_some(end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Windows home path built at run time, so this file never holds one (hygiene).
    fn win_home(name: &str) -> String {
        ["C:", "Users", name].join("\\")
    }

    #[test]
    fn prm_s10_scrub_text_rules() {
        let home = win_home("alice");
        let mut s = Scrubber::new();
        s.path(&format!("{home}\\pf\\.pf-dev\\data"), "<data-dir>")
            .path(&home, "<home>")
            .word("alice", "<user>");
        let line = format!(
            "cwd {home}\\pf\\.pf-dev\\data\\chat\\t1 and {} mail alice@example.com by alice, aliceX; other {} id 0a1b2c3d-1111-2222-3333-444455556666 msg_01ABCDEFGHJKLMNPQR",
            home.replace('\\', "/"),
            win_home("bob"),
        );
        let out = s.scrub_text(&line);
        assert_eq!(
            out,
            "cwd <data-dir>\\chat\\t1 and <home> mail <email> by <user>, aliceX; other <home> id <uuid-1> <msg-1>"
        );
        assert!(s.leaks(&out).is_empty(), "{:?}", s.leaks(&out));
        assert!(!s.leaks(&line).is_empty());
    }

    #[test]
    fn prm_s10_scrub_unix_paths_and_lowercase_drive() {
        let mut s = Scrubber::new();
        // Built at run time, so this file never holds a home path (hygiene).
        let line = format!(
            "at /{}/carol/x and /{}/dave/y not /usr/home/z",
            "home", "Users"
        );
        let out = s.scrub_text(&line);
        assert_eq!(out, "at <home>/x and <home>/y not /usr/home/z");
        let lower = ["c:", "users", "erin", "x"].join("/");
        assert_eq!(s.scrub_text(&lower), "<home>/x");
    }

    #[test]
    fn prm_s10_scrub_init_and_ids_consistently() {
        let mut s = Scrubber::new();
        let init = r#"{"type":"system","subtype":"init","session_id":"0a1b2c3d-1111-2222-3333-444455556666","uuid":"9a1b2c3d-1111-2222-3333-444455556666","tools":["Read"],"mcp_servers":[{"name":"x","status":"ok"}],"plugins":[{"name":"p","path":"/opt/p"}],"slash_commands":["a"],"agents":["b"],"skills":["c"],"apiKeySource":"none","model":"claude-x"}"#;
        let out: Value = serde_json::from_str(&s.scrub_line(init)).unwrap();
        assert_eq!(out["session_id"], "<session-1>");
        assert_eq!(out["uuid"], "<uuid-1>");
        for k in INIT_LISTS {
            assert_eq!(out[k], json!([]), "{k}");
        }
        assert_eq!(out["apiKeySource"], "<redacted>");
        assert_eq!(out["model"], "claude-x");
        // The same session id in a later line maps to the same placeholder.
        let later = r#"{"type":"result","session_id":"0a1b2c3d-1111-2222-3333-444455556666","result":"see 0a1b2c3d-1111-2222-3333-444455556666"}"#;
        let out: Value = serde_json::from_str(&s.scrub_line(later)).unwrap();
        assert_eq!(out["session_id"], "<session-1>");
        assert_eq!(out["result"], "see <session-1>");
    }

    #[test]
    fn prm_s10_scrub_images_orgs_and_emails_by_key() {
        let mut s = Scrubber::new();
        s.image_names(&["overlay.jpg"]);
        let line = r#"{"type":"user","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/jpeg","data":"/9j/AAAA"}},{"type":"image","source":{"type":"base64","data":"xx"}},{"type":"text","text":"hi"}]},"organization_uuid":"o-1","accountUuid":"a-1","emailAddress":"x","orgId":"o-2"}"#;
        let out: Value = serde_json::from_str(&s.scrub_line(line)).unwrap();
        let c = &out["message"]["content"];
        assert_eq!(c[0]["source"], json!({"fixture": "overlay.jpg"}));
        assert_eq!(c[1]["source"], json!({"fixture": "image-2.jpg"}));
        assert_eq!(c[2]["text"], "hi");
        assert_eq!(out["organization_uuid"], "<org-id>");
        assert_eq!(out["orgId"], "<org-id>");
        assert_eq!(out["accountUuid"], "<account-id>");
        assert_eq!(out["emailAddress"], "<email>");
    }

    #[test]
    fn prm_s10_scrub_non_json_lines_and_multibyte_text() {
        let mut s = Scrubber::new();
        let out = s.scrub_line(&format!("Fehler – siehe /{}/carol/log · ok", "home"));
        assert_eq!(out, "Fehler – siehe <home>/log · ok");
        assert_eq!(s.scrub_line("short_id msg_abc"), "short_id msg_abc");
        assert_eq!(
            s.scrub_line("msg_interruptrequeue_v1 msg_01XFDUDYJgAACzvnptvVoYEL"),
            "msg_interruptrequeue_v1 <msg-1>"
        );
    }

    #[test]
    fn prm_s10_scrub_project_slugs_memory_paths_and_absolute_values() {
        let mut s = Scrubber::new();
        let slug = ["C-", "Users-erin-AppData-Roaming-pocket-forge"].join("-");
        let text = format!(
            "projects\\{slug}\\memory and -{}-erin-x but a--b ok",
            "home"
        );
        assert_eq!(
            s.scrub_text(&text),
            "projects\\<project-slug>\\memory and <project-slug> but a--b ok"
        );
        assert!(!s.leaks(&text).is_empty());
        let pipe = ["", "", ".", "pipe", "LOCAL", "cc-msg-1"].join("\\");
        let pwsh = ["C:", "Program Files", "PowerShell", "pwsh.exe"].join("\\");
        let init = json!({
            "type": "system", "subtype": "init",
            "memory_paths": {"auto": format!("<data-dir>\\projects\\{slug}")},
            "powershell_path": pwsh,
            "messaging_socket_path": pipe,
            "output_style": "default",
        })
        .to_string();
        let out: Value = serde_json::from_str(&s.scrub_line(&init)).unwrap();
        assert_eq!(out["memory_paths"]["auto"], "<memory-path>");
        assert_eq!(out["powershell_path"], "<path>");
        assert_eq!(out["messaging_socket_path"], "<path>");
        assert_eq!(out["output_style"], "default");
        let line = out.to_string();
        assert!(s.leaks(&line).is_empty(), "{:?}", s.leaks(&line));
    }
}
