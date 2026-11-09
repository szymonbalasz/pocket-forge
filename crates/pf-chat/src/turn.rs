//! One turn over a stream-json child (§10): send the stdin line, read stdout under
//! FR-PRM-36's budgets, stream the text, map the `result` (FR-PRM-37). Shared by the Claude
//! Code adapter and the fake, so both go through the same parser and error mapping.

use std::time::Duration;

use crate::backend::{CancelHandle, TurnEvent, TurnOutcome, Usage};
use crate::clock::{Clock, TurnBudget, TurnTimer};
use crate::error::{ChatError, ChatErrorKind};
use crate::process::ChildProcess;
use crate::stream::{LineOutcome, LineParser, StreamEvent, TurnResult};

/// How long a crashed child gets to report its exit code.
const EXIT_WAIT: Duration = Duration::from_secs(5);

/// What the turn saw besides text.
#[derive(Debug, Default)]
struct Signals {
    assistant_error: Option<String>,
    assistant_error_text: Option<String>,
    rate_limited: bool,
}

/// Runs one turn. Returns the outcome and whether the child is still usable for the next
/// turn (only after a `result`). `scrub` replaces the home path in stderr details.
pub fn run_turn(
    child: &mut dyn ChildProcess,
    stdin_line: &str,
    clock: &dyn Clock,
    budget: TurnBudget,
    cancel: &CancelHandle,
    on_event: &mut dyn FnMut(TurnEvent),
    scrub: &dyn Fn(&str) -> String,
) -> (TurnOutcome, bool) {
    cancel.arm(child.killer());
    let mut partial = String::new();
    if cancel.is_cancelled() {
        return (TurnOutcome::Canceled { partial }, false);
    }
    if let Err(e) = child.send_line(stdin_line) {
        if cancel.is_cancelled() {
            return (TurnOutcome::Canceled { partial }, false);
        }
        return (crashed(child, &partial, scrub, Some(e.to_string())), false);
    }
    let mut timer = TurnTimer::start(budget, clock.now());
    let mut parser = LineParser::new();
    let mut signals = Signals::default();
    let mut streaming = false;
    loop {
        if cancel.is_cancelled() {
            child.killer().kill();
            return (TurnOutcome::Canceled { partial }, false);
        }
        let Some(wait) = timer.remaining(clock.now()) else {
            child.killer().kill();
            let error = ChatError::new(ChatErrorKind::Timeout);
            return (TurnOutcome::Failed { error, partial }, false);
        };
        match child.next_line(wait) {
            crate::process::Next::Timeout => {}
            crate::process::Next::Eof => {
                if cancel.is_cancelled() {
                    return (TurnOutcome::Canceled { partial }, false);
                }
                return (crashed(child, &partial, scrub, None), false);
            }
            crate::process::Next::Line(line) => {
                timer.event(clock.now());
                match parser.push(&line) {
                    LineOutcome::Event(StreamEvent::AssistantText(t)) => {
                        if !streaming {
                            push_message(&t, &mut partial, on_event);
                        }
                    }
                    LineOutcome::Event(StreamEvent::TextDelta(t)) => {
                        // Only with --include-partial-messages; the whole messages that
                        // follow repeat the deltas and are skipped.
                        streaming = true;
                        if !t.is_empty() {
                            partial.push_str(&t);
                            on_event(TurnEvent::Delta(t));
                        }
                    }
                    LineOutcome::Event(StreamEvent::AssistantError { error, text }) => {
                        signals.assistant_error = Some(error);
                        signals.assistant_error_text = Some(text);
                    }
                    LineOutcome::Event(StreamEvent::RateLimit { status }) => {
                        if status.as_deref() == Some("rejected") {
                            signals.rate_limited = true;
                        }
                    }
                    LineOutcome::Event(StreamEvent::Result(r)) => {
                        return (finish(r, &signals, partial), true);
                    }
                    LineOutcome::Malformed => {
                        child.killer().kill();
                        let error = ChatError::new(ChatErrorKind::StreamMalformed)
                            .with_detail("three lines in a row weren't JSON");
                        return (TurnOutcome::Failed { error, partial }, false);
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Appends one whole assistant message, separated from the previous one by a blank line.
fn push_message(text: &str, partial: &mut String, on_event: &mut dyn FnMut(TurnEvent)) {
    if text.is_empty() {
        return;
    }
    let piece = if partial.is_empty() {
        text.to_owned()
    } else {
        format!("\n\n{text}")
    };
    partial.push_str(&piece);
    on_event(TurnEvent::Delta(piece));
}

fn crashed(
    child: &mut dyn ChildProcess,
    partial: &str,
    scrub: &dyn Fn(&str) -> String,
    io_error: Option<String>,
) -> TurnOutcome {
    let exit = child.wait(EXIT_WAIT);
    if exit.is_none() {
        child.killer().kill();
    }
    let mut detail = scrub(child.stderr_tail().trim());
    if let Some(e) = io_error {
        if !detail.is_empty() {
            detail.push('\n');
        }
        detail.push_str(&e);
    }
    let error = ChatError::new(ChatErrorKind::BackendCrashed {
        exit_code: exit.and_then(|e| e.code),
    })
    .with_detail(detail);
    TurnOutcome::Failed {
        error,
        partial: partial.to_owned(),
    }
}

/// Words that mean the network is down (the CLI's API errors).
const OFFLINE_HINTS: [&str; 9] = [
    "connection error",
    "unable to connect",
    "enotfound",
    "econnrefused",
    "etimedout",
    "eai_again",
    "network error",
    "fetch failed",
    "getaddrinfo",
];

fn finish(r: TurnResult, s: &Signals, partial: String) -> TurnOutcome {
    if !r.is_error {
        let text = if partial.is_empty() {
            r.result.clone().unwrap_or_default()
        } else {
            partial
        };
        let usage = match (r.input_tokens, r.output_tokens) {
            (Some(input_tokens), Some(output_tokens)) => Some(Usage {
                input_tokens,
                output_tokens,
                cost_usd: r.total_cost_usd,
            }),
            _ => None,
        };
        return TurnOutcome::Completed { text, usage };
    }
    let message = r
        .result
        .clone()
        .filter(|m| !m.trim().is_empty())
        .or_else(|| (!r.errors.is_empty()).then(|| r.errors.join("; ")))
        .or_else(|| s.assistant_error_text.clone())
        .unwrap_or_else(|| r.subtype.clone());
    let lower = message.to_lowercase();
    let error_kind = s.assistant_error.as_deref().unwrap_or_default();
    let error = if error_kind == "authentication_failed"
        || lower.contains("not logged in")
        || lower.contains("/login")
    {
        ChatError::new(ChatErrorKind::NotLoggedIn)
    } else if s.rate_limited
        || error_kind == "rate_limit"
        || lower.contains("hit your limit")
        || lower.contains("usage limit")
    {
        ChatError::usage_limit(Some(&message))
    } else if OFFLINE_HINTS.iter().any(|h| lower.contains(h)) {
        ChatError::new(ChatErrorKind::Offline).with_detail(message)
    } else {
        ChatError::backend_error(message)
    };
    TurnOutcome::Failed { error, partial }
}
