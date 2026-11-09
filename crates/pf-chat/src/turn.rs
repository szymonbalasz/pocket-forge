//! One turn over a stream-json child (§10): send the stdin line, read stdout under
//! FR-PRM-36's budgets, stream the text, map the `result` (FR-PRM-37). Shared by the Claude
//! Code adapter and the fake, so both go through the same parser and error mapping.

use std::time::Duration;

use crate::backend::{CancelHandle, TurnEvent, TurnOutcome, Usage};
use crate::clock::{Clock, TurnBudget, TurnTimer};
use crate::error::{ChatError, ChatErrorKind};
use crate::process::ChildProcess;
use crate::stream::{LineOutcome, LineParser, MAX_LINE, MAX_REPLY, StreamEvent, TurnResult};

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
                // An auth or limit message and then an exit without a `result` is still
                // that state, not a crash.
                if let Some(error) = signalled(&signals) {
                    let _ = child.wait(EXIT_WAIT);
                    return (TurnOutcome::Failed { error, partial }, false);
                }
                return (crashed(child, &partial, scrub, None), false);
            }
            crate::process::Next::Line(line) => {
                timer.event(clock.now());
                if line.len() > MAX_LINE {
                    child.killer().kill();
                    let error = ChatError::new(ChatErrorKind::StreamMalformed)
                        .with_detail("a stdout line was longer than 1 MiB");
                    return (TurnOutcome::Failed { error, partial }, false);
                }
                let fits = match parser.push(&line) {
                    LineOutcome::Event(StreamEvent::AssistantText(t)) => {
                        streaming || push_message(&t, &mut partial, on_event)
                    }
                    LineOutcome::Event(StreamEvent::TextDelta(t)) => {
                        // Only with --include-partial-messages; the whole messages that
                        // follow repeat the deltas and are skipped.
                        streaming = true;
                        append(&t, &mut partial, on_event)
                    }
                    LineOutcome::Event(StreamEvent::AssistantError { error, text }) => {
                        signals.assistant_error = Some(error);
                        signals.assistant_error_text = Some(text);
                        true
                    }
                    LineOutcome::Event(StreamEvent::RateLimit { status }) => {
                        if status.as_deref() == Some("rejected") {
                            signals.rate_limited = true;
                        }
                        true
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
                    _ => true,
                };
                if !fits {
                    child.killer().kill();
                    let error = ChatError::new(ChatErrorKind::StreamMalformed)
                        .with_detail("the reply was longer than 1 MiB");
                    return (TurnOutcome::Failed { error, partial }, false);
                }
            }
        }
    }
}

/// Appends one whole assistant message, separated from the previous one by a blank line.
/// `false` when the reply passed [`MAX_REPLY`] (what fits is kept).
fn push_message(text: &str, partial: &mut String, on_event: &mut dyn FnMut(TurnEvent)) -> bool {
    if text.is_empty() {
        true
    } else if partial.is_empty() {
        append(text, partial, on_event)
    } else {
        append(&format!("\n\n{text}"), partial, on_event)
    }
}

/// Appends `piece` to the reply and streams it, up to [`MAX_REPLY`] bytes in all (cut at a
/// character boundary); `false` when it didn't all fit (review-sec 7).
fn append(piece: &str, partial: &mut String, on_event: &mut dyn FnMut(TurnEvent)) -> bool {
    let room = MAX_REPLY.saturating_sub(partial.len());
    let kept = &piece[..piece.floor_char_boundary(room)];
    if !kept.is_empty() {
        partial.push_str(kept);
        on_event(TurnEvent::Delta(kept.to_owned()));
    }
    kept.len() == piece.len()
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
const OFFLINE_HINTS: [&str; 12] = [
    "connection error",
    "unable to connect",
    "enotfound",
    "econnrefused",
    "econnreset",
    "etimedout",
    "eai_again",
    "network error",
    "fetch failed",
    "getaddrinfo",
    "socket hang up",
    "network is unreachable",
];

/// Words that mean the login is missing, expired or revoked (FR-PRM-37 `not_logged_in`).
const AUTH_HINTS: [&str; 7] = [
    "not logged in",
    "/login",
    "authentication_error",
    "oauth token has expired",
    "token has been revoked",
    "invalid api key",
    "api error: 401",
];

/// Words that mean the plan's limit is reached (FR-PRM-37 `usage_limit`).
/// Not a bare "limit reached": "Context limit reached" is not the plan's limit.
const LIMIT_HINTS: [&str; 6] = [
    "hit your limit",
    "usage limit",
    "5-hour limit",
    "weekly limit",
    "session limit",
    "rate_limit_error",
];

/// Maps the CLI's error message and the turn's signals to an FR-PRM-37 error.
fn classify(message: &str, s: &Signals) -> ChatError {
    let lower = message.to_lowercase();
    let error_kind = s.assistant_error.as_deref().unwrap_or_default();
    if error_kind == "authentication_failed" || AUTH_HINTS.iter().any(|h| lower.contains(h)) {
        ChatError::new(ChatErrorKind::NotLoggedIn)
    } else if s.rate_limited
        || error_kind == "rate_limit"
        || LIMIT_HINTS.iter().any(|h| lower.contains(h))
    {
        ChatError::usage_limit(Some(message))
    } else if OFFLINE_HINTS.iter().any(|h| lower.contains(h)) {
        ChatError::new(ChatErrorKind::Offline).with_detail(message)
    } else {
        ChatError::backend_error(message)
    }
}

/// The state an assistant error or a rejected rate limit means when the stream ends
/// without a `result`; `None` when it would only be a `backend_error`.
fn signalled(s: &Signals) -> Option<ChatError> {
    if s.assistant_error.is_none() && !s.rate_limited {
        return None;
    }
    let e = classify(s.assistant_error_text.as_deref().unwrap_or_default(), s);
    (e.kind != ChatErrorKind::BackendError).then_some(e)
}

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
    let error = classify(&message, s);
    TurnOutcome::Failed { error, partial }
}
