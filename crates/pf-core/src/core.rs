//! [`Core`]: the context plus in-memory state, and the command pattern every capability uses.
//!
//! A command is a type implementing [`Command`]: a name, a `Deserialize` input that is
//! validated at the boundary ([`Validate`]), a `Serialize` output, and a `run` that returns a
//! normalised [`CoreError`]. The Tauri layer and the future MCP endpoint (#36) only adapt
//! commands; [`Core::dispatch`] runs one by name from JSON.

use std::sync::{Mutex, MutexGuard};

use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::context::Context;
use crate::error::{CoreError, CoreResult, ErrorCode};
use crate::images::{self, ImageRegistry, SchemeResponse};
use crate::project::OpenProject;
use crate::settings::{self, Settings, SettingsWarning};

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
    pub images: ImageRegistry,
}

/// The core: seams plus state. One per app process.
#[derive(Debug)]
pub struct Core {
    pub ctx: Context,
    state: Mutex<State>,
}

impl Core {
    /// Loads settings (creating them on first run) and prepares the library override.
    pub fn new(ctx: Context) -> CoreResult<Self> {
        let (settings, settings_warnings) = settings::load_or_init(&ctx)?;
        crate::library::prepare_override(&ctx)?;
        Ok(Core {
            ctx,
            state: Mutex::new(State {
                settings,
                settings_warnings,
                project: None,
                images: ImageRegistry::default(),
            }),
        })
    }

    /// The state lock. Commands hold it for their whole run, so they're serialised.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
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

    /// Serves a `pf-tiles` request path.
    pub fn serve_scheme(&self, path: &str) -> SchemeResponse {
        images::serve(&self.state().images, path)
    }
}

/// Adapter used by the command registry: JSON in, JSON out.
pub fn dispatch_json<C: Command>(core: &Core, input: Value) -> CoreResult<Value> {
    let input: C::Input = serde_json::from_value(input)
        .map_err(|e| CoreError::invalid("input", format!("invalid input: {e}")))?;
    let out = core.run::<C>(input)?;
    serde_json::to_value(out).map_err(|e| CoreError::internal(format!("serialising output: {e}")))
}
