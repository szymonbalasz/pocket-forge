//! The fake chat backend (FR-PRM-39): every case, through the same turn loop as the adapter.
#![allow(clippy::unwrap_used)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use pf_chat::backend::{
    BackendMode, CancelHandle, ChatBackend, ChatSession, SessionOptions, TurnEvent, TurnImage,
    TurnInput, TurnOutcome,
};
use pf_chat::clock::{Clock, FakeClock};
use pf_chat::error::ChatErrorKind;
use pf_chat::fake::{FakeBackend, FakeCase};
use pf_chat::system_prompt::Host;

fn input() -> TurnInput {
    TurnInput {
        host: Host::Rgn,
        message_text: "Context …\n\nmake this a kitchen".into(),
        user_text: "make this a kitchen".into(),
        model_id: Some("flux-fill-pro".into()),
        final_prompt: Some("a kitchen, no people or tokens".into()),
        images: vec![TurnImage {
            id: "rgn:overlay".into(),
            content_hash: "sha256:abcd".into(),
            width: 1024,
            height: 768,
            media_type: "image/jpeg".into(),
            bytes: vec![1; 3000],
        }],
    }
}

fn backend(case: FakeCase) -> (FakeBackend, FakeClock) {
    let clock = FakeClock::new();
    (FakeBackend::new(case, Arc::new(clock.clone())), clock)
}

fn session(b: &FakeBackend) -> Box<dyn ChatSession> {
    b.open_session(SessionOptions {
        first_host: Host::Rgn,
        cwd: PathBuf::from("unused"),
    })
    .unwrap()
}

fn turn(s: &mut dyn ChatSession) -> (TurnOutcome, Vec<String>) {
    let mut deltas = Vec::new();
    let out = s.run_turn(&input(), &CancelHandle::new(), &mut |e| {
        let TurnEvent::Delta(d) = e;
        deltas.push(d);
    });
    (out, deltas)
}

fn completed(case: FakeCase) -> String {
    let (b, _) = backend(case);
    let mut s = session(&b);
    let (out, deltas) = turn(s.as_mut());
    let TurnOutcome::Completed { text, usage } = out else {
        panic!("{case}: {out:?}")
    };
    assert_eq!(deltas.concat(), text);
    assert!(usage.unwrap().output_tokens > 0);
    assert!(s.is_alive());
    text
}

#[test]
fn fr_prm_39_success_quotes_the_user_and_lists_the_context() {
    let (b, _) = backend(FakeCase::Success);
    assert_eq!(b.mode(), BackendMode::Fake);
    assert!(b.detect().is_ok());
    let text = completed(FakeCase::Success);
    for want in [
        "\"make this a kitchen\"",
        "Host: Region edit",
        "rgn:overlay 1024×768, 3000 bytes, sha256:abcd",
        "Model: flux-fill-pro",
        "Final prompt: a kitchen, no people or tokens",
        "```prompt\nmake this a kitchen\n```",
    ] {
        assert!(text.contains(want), "missing {want:?} in {text}");
    }
    assert_eq!(text.matches("```prompt").count(), 1);
}

#[test]
fn fr_prm_39_proposal_counts() {
    assert_eq!(
        completed(FakeCase::TwoProposals)
            .matches("```prompt")
            .count(),
        2
    );
    assert_eq!(
        completed(FakeCase::NoProposal).matches("```prompt").count(),
        0
    );
}

#[test]
fn fr_prm_39_slow_streams_n_deltas_on_the_clock() {
    let (b, clock) = backend(FakeCase::Slow(3));
    let mut s = session(&b);
    let (out, deltas) = turn(s.as_mut());
    assert!(matches!(out, TurnOutcome::Completed { .. }));
    assert_eq!(deltas.len(), 3);
    assert_eq!(clock.now(), Duration::from_secs(3));

    // Stop during a slow reply keeps what arrived (FR-PRM-27).
    let (b, _) = backend(FakeCase::Slow(4));
    let mut s = session(&b);
    let cancel = CancelHandle::new();
    let stop = cancel.clone();
    let out = s.run_turn(&input(), &cancel, &mut |_| stop.cancel());
    let TurnOutcome::Canceled { partial } = out else {
        panic!("{out:?}")
    };
    assert!(!partial.is_empty() && !partial.contains("```"));
    assert!(!s.is_alive());
}

#[test]
fn fr_prm_39_detection_cases() {
    for (case, code) in [
        (FakeCase::NotInstalled, "not_installed"),
        (FakeCase::VersionOld, "version_unsupported"),
        (FakeCase::IsolationUnavailable, "isolation_unavailable"),
    ] {
        let (b, _) = backend(case);
        assert_eq!(b.detect().unwrap_err().code(), code);
        let e = b
            .open_session(SessionOptions {
                first_host: Host::Gen,
                cwd: PathBuf::from("unused"),
            })
            .err()
            .unwrap();
        assert_eq!(e.code(), code);
    }
    let (b, _) = backend(FakeCase::VersionOld);
    assert_eq!(
        b.detect().unwrap_err().message,
        "Claude Code 2.1.200 is too old (needs 2.1.319)."
    );
    // not_logged_in: detection says so, and a turn replays the owner's recording.
    let (b, _) = backend(FakeCase::NotLoggedIn);
    assert_eq!(b.detect().unwrap_err().code(), "not_logged_in");
    let mut s = session(&b);
    let (out, _) = turn(s.as_mut());
    assert!(matches!(&out, TurnOutcome::Failed { error, .. } if error.code() == "not_logged_in"));
    assert!(b.login_command().unwrap().is_none());
}

#[test]
fn fr_prm_39_turn_error_cases() {
    for (case, code, alive) in [
        (FakeCase::UsageLimit, "usage_limit", true),
        (FakeCase::Offline, "offline", true),
        (FakeCase::Malformed, "stream_malformed", false),
        (FakeCase::Crash, "backend_crashed", false),
        (FakeCase::Timeout, "timeout", false),
    ] {
        let (b, clock) = backend(case);
        let mut s = session(&b);
        let (out, _) = turn(s.as_mut());
        let TurnOutcome::Failed { error, partial } = out else {
            panic!("{case}: {out:?}")
        };
        assert_eq!(error.code(), code, "{case}");
        assert_eq!(s.is_alive(), alive, "{case}");
        match case {
            FakeCase::Crash => {
                assert_eq!(
                    error.kind,
                    ChatErrorKind::BackendCrashed { exit_code: Some(1) }
                );
                assert_eq!(partial, "Here is a first idea");
            }
            FakeCase::Timeout => {
                assert_eq!(partial, "Thinking about the room…");
                assert_eq!(clock.now(), Duration::from_secs(90));
            }
            FakeCase::UsageLimit => assert!(error.fix.contains("resets")),
            _ => {}
        }
    }
}

#[test]
fn fr_prm_39_case_changes_apply_to_the_next_turn() {
    let (b, _) = backend(FakeCase::Success);
    let mut s = session(&b);
    let (first, _) = turn(s.as_mut());
    assert!(matches!(first, TurnOutcome::Completed { .. }));
    b.set_case(FakeCase::TwoProposals);
    let (second, _) = turn(s.as_mut());
    let TurnOutcome::Completed { text, .. } = second else {
        panic!("{second:?}")
    };
    assert_eq!(text.matches("```prompt").count(), 2);
    assert_eq!(b.case(), FakeCase::TwoProposals);
}
