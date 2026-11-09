//! What a turn says to the backend (§10 "Stdin message"): the host note (FR-PRM-44), the
//! context block built from the [`ChatContext`] (FR-PRM-41), then the user's text. Also the
//! strip's one-line summary (FR-PRM-26) and proposal extraction (FR-PRM-28).

use crate::manifest::Kind;
use crate::prompt::context::{ChatContext, ChatHost, ChatImage};

/// FR-PRM-28: proposals per turn at most.
pub const MAX_PROPOSALS: usize = 3;

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Map => "battlemap",
        Kind::Token => "token",
        Kind::Item => "item art",
    }
}

pub fn host_label(host: ChatHost) -> &'static str {
    match host {
        ChatHost::Rgn => "Region edit",
        ChatHost::Gen => "New image",
    }
}

pub fn pf_host(host: ChatHost) -> pf_chat::system_prompt::Host {
    match host {
        ChatHost::Rgn => pf_chat::system_prompt::Host::Rgn,
        ChatHost::Gen => pf_chat::system_prompt::Host::Gen,
    }
}

/// The context block: what the model, settings and prompt are, and which images are attached.
pub fn context_block(
    ctx: &ChatContext,
    sent: &[ChatImage],
    dropped: &[ChatImage],
    already: &[ChatImage],
) -> String {
    let mut s = String::from("[Context]\n");
    s.push_str(&format!(
        "Asset: {} ({})\n",
        kind_name(ctx.kind),
        host_label(ctx.host)
    ));
    if ctx.model.id.is_empty() {
        s.push_str("Model: none chosen yet\n");
    } else if ctx.model.name == ctx.model.id {
        s.push_str(&format!("Model: {}\n", ctx.model.id));
    } else {
        s.push_str(&format!("Model: {} ({})\n", ctx.model.name, ctx.model.id));
    }
    if !ctx.controls.is_empty() {
        s.push_str("Settings:\n");
        for c in &ctx.controls {
            let mut line = format!("- {} = {}", c.name, c.value);
            match (c.min, c.max) {
                (Some(lo), Some(hi)) => line.push_str(&format!(" (range {lo}–{hi})")),
                (Some(lo), None) => line.push_str(&format!(" (min {lo})")),
                (None, Some(hi)) => line.push_str(&format!(" (max {hi})")),
                (None, None) => {}
            }
            if let Some(allowed) = &c.allowed {
                let vals: Vec<String> = allowed.iter().map(ToString::to_string).collect();
                line.push_str(&format!(" (one of {})", vals.join(", ")));
            }
            s.push_str(&line);
            s.push('\n');
        }
    }
    s.push_str(&format!(
        "Typed prompt: {}\n",
        if ctx.typed_prompt.trim().is_empty() {
            "(empty)"
        } else {
            ctx.typed_prompt.trim()
        }
    ));
    if !ctx.helpers.is_empty() {
        s.push_str("Helper texts (appended automatically):\n");
        for h in &ctx.helpers {
            s.push_str(&format!("- {}: {}\n", h.label, h.text));
        }
    }
    if let Some(b) = &ctx.blend_instruction {
        if b.on {
            s.push_str(&format!(
                "Blend instruction (appended automatically): {}\n",
                b.text
            ));
        } else {
            s.push_str("Blend instruction: off\n");
        }
    }
    s.push_str(&format!(
        "Final prompt as it would be sent: {}\n",
        ctx.final_prompt
    ));
    for f in &ctx.facts {
        s.push_str(&format!("Note: {f}\n"));
    }
    if !sent.is_empty() {
        s.push_str("Attached images, in order:\n");
        for (i, img) in sent.iter().enumerate() {
            s.push_str(&format!("{}. {} ({}×{})\n", i + 1, img.label, img.w, img.h));
        }
    }
    if !already.is_empty() {
        let names: Vec<&str> = already.iter().map(|i| i.label.as_str()).collect();
        s.push_str(&format!(
            "Unchanged since an earlier message: {}\n",
            names.join(", ")
        ));
    }
    if !dropped.is_empty() {
        let names: Vec<&str> = dropped.iter().map(|i| i.label.as_str()).collect();
        s.push_str(&format!(
            "Not attached (turn limit): {}\n",
            names.join(", ")
        ));
    }
    s.push_str("[/Context]");
    s
}

/// The whole user message: host note (when the host differs from the session's first),
/// the context block, then the user's text.
pub fn compose(note: Option<String>, block: &str, user_text: &str) -> String {
    let mut out = String::new();
    if let Some(n) = note {
        out.push_str(&n);
        out.push_str("\n\n");
    }
    out.push_str(block);
    out.push_str("\n\n");
    out.push_str(user_text.trim());
    out
}

/// FR-PRM-26: "crop 1024×768 + mask · prompt · Ideogram 4.5 · 4 controls". The image count is
/// what the turn sends; images the session already received are named apart ("3 images + 6
/// already sent", R103), or "(already sent)" when none is new; images the turn limit drops are
/// named on their own (FR-PRM-25), not counted (M4).
pub fn summary(
    ctx: &ChatContext,
    sent: &[ChatImage],
    dropped: &[ChatImage],
    include_images: bool,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if !include_images {
        parts.push("text only".into());
    } else if ctx.images.is_empty() {
        parts.push("no image".into());
    } else {
        let first = &ctx.images[0];
        let already = ctx.images.len().saturating_sub(sent.len() + dropped.len());
        let images = |n: usize| format!("{n} image{}", if n == 1 { "" } else { "s" });
        parts.push(match first.role {
            crate::prompt::context::ChatImageRole::RegionOverlay => {
                let what = format!("crop {}×{} + mask", first.w, first.h);
                if sent.is_empty() {
                    format!("{what} (already sent)")
                } else {
                    what
                }
            }
            _ if sent.is_empty() => format!("{} (already sent)", images(already)),
            _ if already > 0 => format!("{} + {already} already sent", images(sent.len())),
            _ => images(sent.len()),
        });
    }
    if !ctx.typed_prompt.trim().is_empty() {
        parts.push("prompt".into());
    }
    if !ctx.model.name.is_empty() {
        parts.push(ctx.model.name.clone());
    }
    if !ctx.controls.is_empty() {
        let n = ctx.controls.len();
        parts.push(format!("{n} control{}", if n == 1 { "" } else { "s" }));
    }
    parts.join(" · ")
}

/// FR-PRM-28: the bodies of fenced blocks tagged `prompt`, at most three, trimmed; empty
/// blocks are skipped.
pub fn proposals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let t = line.trim();
        let Some(tag) = t.strip_prefix("```") else {
            continue;
        };
        let is_prompt = tag.trim().eq_ignore_ascii_case("prompt");
        let mut body = Vec::new();
        for l in lines.by_ref() {
            if l.trim_start().starts_with("```") {
                break;
            }
            body.push(l);
        }
        let body = body.join("\n").trim().to_owned();
        if is_prompt && !body.is_empty() && out.len() < MAX_PROPOSALS {
            out.push(body);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::context::{ChatImageRole, ChatModel};

    fn gen_ctx(n: usize) -> ChatContext {
        ChatContext {
            host: ChatHost::Gen,
            kind: Kind::Map,
            images: (0..n)
                .map(|i| ChatImage {
                    id: format!("ref:{i}"),
                    role: ChatImageRole::Reference,
                    label: format!("ref {i}"),
                    w: 64,
                    h: 64,
                    content_hash: format!("h{i}"),
                    jpeg: Vec::new(),
                })
                .collect(),
            typed_prompt: String::new(),
            helpers: Vec::new(),
            blend_instruction: None,
            final_prompt: String::new(),
            model: ChatModel {
                id: "fake".into(),
                name: String::new(),
            },
            controls: Vec::new(),
            facts: Vec::new(),
        }
    }

    #[test]
    fn fr_prm_26_r103_the_summary_counts_only_images_that_will_be_sent() {
        let ctx = gen_ctx(9);
        // walk-p4r L3: 6 received earlier in the session, 3 new ones go out.
        let sent = &ctx.images[6..];
        assert_eq!(summary(&ctx, sent, &[], true), "3 images + 6 already sent");
        assert_eq!(summary(&ctx, &[], &[], true), "9 images (already sent)");
        // Nothing received yet: 6 go out, the turn limit drops 3 (named apart, not counted).
        let (sent, dropped) = ctx.images.split_at(6);
        assert_eq!(summary(&ctx, sent, dropped, true), "6 images");
        assert_eq!(
            summary(&ctx, &ctx.images[..1], &[], true),
            "1 image + 8 already sent"
        );
    }

    #[test]
    fn fr_prm_28_prompt_fences_max_three() {
        let text = "Ideas:\n\n```prompt\none\n```\n\n```text\nnot this\n```\n```prompt\n  two\nlines \n```\n```prompt\n```\n```Prompt\nthree\n```\n```prompt\nfour\n```";
        assert_eq!(proposals(text), ["one", "two\nlines", "three"]);
        assert!(proposals("no fences at all").is_empty());
        // An unclosed fence still yields its body.
        assert_eq!(proposals("```prompt\nopen end"), ["open end"]);
    }
}
