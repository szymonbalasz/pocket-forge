//! [`Core`]: the context plus in-memory state, and the command pattern every capability uses.
//!
//! A command is a type implementing [`Command`]: a name, a `Deserialize` input that is
//! validated at the boundary ([`Validate`]), a `Serialize` output, and a `run` that returns a
//! normalised [`CoreError`]. The Tauri layer and the future MCP endpoint (#36) only adapt
//! commands; [`Core::dispatch`] runs one by name from JSON.

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, MutexGuard};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::images::SchemeResponse;
use crate::project::OpenProject;
use crate::settings::{self, Settings, SettingsWarning};
use crate::thumb::Thumbnails;
use crate::tiles::{self, Served};

/// Input validation at the command boundary. Deserialisation already checked the shape.
pub trait Validate {
    fn validate(&self) -> CoreResult<()> {
        Ok(())
    }
}

/// A core command.
pub trait Command {
    /// The command name (`snake_case`, prefixed by its spec's area: `project_open`, …).
    const NAME: &'static str;
    type Input: DeserializeOwned + Validate + Send;
    type Output: Serialize + Send;
    fn run(core: &Core, input: Self::Input) -> CoreResult<Self::Output>;
}

/// In-memory state behind the core's lock.
#[derive(Debug)]
pub struct State {
    pub settings: Settings,
    pub settings_warnings: Vec<SettingsWarning>,
    pub project: Option<OpenProject>,
    /// Region edit's memory-only state (session spend, estimates).
    pub rgn: crate::region_edit::RgnState,
    /// Export's session settings and running export (EXP §8).
    pub exp: crate::export::ExpState,
    /// New image's session memory (the chat context reads it without a provider call).
    pub generate: crate::generate::GenState,
}

/// The core: seams plus state. One per app process.
#[derive(Debug)]
pub struct Core {
    /// Shared, so background work (an export, EXP) can commit through PRJ when it finishes.
    pub ctx: Arc<Context>,
    /// Shared with background work for the same reason; take it through [`Core::state`].
    state: Arc<Mutex<State>>,
    /// The `pf-tiles` registry, behind its own lock so serving never waits for a command
    /// (EDT). Lock order: `state` first, then `served`.
    pub(crate) served: Arc<Mutex<Served>>,
    /// The home-grid thumbnail worker (FR-PRJ-50).
    pub thumbs: Thumbnails,
    /// Set by `project_create_cancel`; read between files by a running `project_create`, which
    /// doesn't hold the state lock (FR-PRJ-11).
    pub(crate) create_cancel: AtomicBool,
    /// One creation at a time, so two batches never race for a name.
    pub(crate) create_lock: Mutex<()>,
    /// REF's session memory: header probes and prepared references (FR-REF-23). Has its own
    /// locks; never needs the state lock.
    pub refs: crate::references::RefSession,
    /// The prompt chat's memory (PRM slice 5): its own locks, never the state lock in a turn.
    pub chat: crate::prompt::chat::ChatRuntime,
}

impl Core {
    /// Loads settings (creating them on first run), prepares the library override and runs the
    /// startup sweep (FR-PRJ-09, R35).
    pub fn new(ctx: Context) -> CoreResult<Self> {
        let (settings, settings_warnings) = settings::load_or_init(&ctx)?;
        crate::library::prepare_override(&ctx)?;
        crate::library::sweep_startup(&ctx, &settings);
        crate::references::sweep_startup(&ctx, &settings);
        crate::prompt::chat::sweep_startup(&ctx);
        ctx.provider.set_account(settings.provider.account());
        Ok(Core {
            thumbs: Thumbnails::new(&ctx),
            ctx: Arc::new(ctx),
            state: Arc::new(Mutex::new(State {
                settings,
                settings_warnings,
                project: None,
                rgn: Default::default(),
                exp: Default::default(),
                generate: Default::default(),
            })),
            served: Arc::default(),
            create_cancel: AtomicBool::new(false),
            create_lock: Mutex::new(()),
            refs: Default::default(),
            chat: Default::default(),
        })
    }

    /// The state lock. Commands hold it for their whole run, so they're serialised.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The state lock's shared handle, for background work that outlives a command (EXP).
    pub(crate) fn state_handle(&self) -> Arc<Mutex<State>> {
        self.state.clone()
    }

    /// Validates the input and runs a command.
    pub fn run<C: Command>(&self, input: C::Input) -> CoreResult<C::Output> {
        let result = input.validate().and_then(|()| C::run(self, input));
        if let Err(e) = &result {
            tracing::info!(command = C::NAME, code = %e.code, message = %e.message, "command failed");
        }
        result
    }

    /// Runs a command by name from JSON input (tests, the future MCP endpoint).
    pub fn dispatch(&self, name: &str, input: Value) -> CoreResult<Value> {
        let entry = crate::commands::REGISTRY
            .iter()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| {
                CoreError::new(ErrorCode::NotFound, format!("unknown command {name:?}"))
            })?;
        (entry.1)(self, input)
    }

    /// Serves a `pf-tiles` request path. Never takes the state lock.
    pub fn serve_scheme(&self, path: &str) -> SchemeResponse {
        tiles::serve(&self.served, path)
    }

    /// The `pf-tiles` registry (see [`Core::serve_scheme`]).
    pub fn served(&self) -> MutexGuard<'_, Served> {
        self.served.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Adapter used by the command registry: JSON in, JSON out.
pub fn dispatch_json<C: Command>(core: &Core, input: Value) -> CoreResult<Value> {
    let input: C::Input = serde_json::from_value(input)
        .map_err(|e| CoreError::invalid("input", format!("invalid input: {e}")))?;
    let out = core.run::<C>(input)?;
    serde_json::to_value(out).map_err(|e| CoreError::internal(format!("serialising output: {e}")))
}
