//! AgentMirai application state — the command-center layer over the engine.
//!
//! The engine keeps its own [`openmirai_engine::server::AppState`] (graphs,
//! agents, runner, workflows). [`MiraiState`] holds ONLY the AgentMirai fields:
//! orchestrated tmux sessions, the fleet SoT, the folder picker and the UI /
//! projects roots. Both states are merged into one axum router by
//! [`super::create_router`].

use std::path::PathBuf;
use std::sync::Arc;

use crate::fleet::FleetStore;
use crate::objectives::ObjectiveStore;
use crate::sessions::picker::FolderPicker;
use crate::sessions::{SessionManager, TmuxBackend};

/// Shared error body — re-exported from the engine so the wire shape is
/// identical across core and AgentMirai endpoints.
pub use openmirai_engine::server::ErrorResponse;

/// State passed to every AgentMirai handler via axum's `State`.
#[derive(Clone)]
pub struct MiraiState {
    /// Orchestrated Claude sessions over tmux (PRD-013).
    ///
    /// Created with the real [`TmuxBackend`] by default; [`super::serve`] loads
    /// and reconciles the persistent registry and starts polling. Tests replace
    /// this field with a manager over a fake backend before building the router.
    pub orchestrator: Arc<SessionManager>,
    /// Native host folder picker (PRD-013 M9). One dialog at a time.
    pub folder_picker: Arc<FolderPicker>,
    /// Roots scanned for first-level project directories (PRD-013 M7).
    pub projects_dirs: Vec<PathBuf>,
    /// Directory served as the static web UI under `/ui` (PRD-013 M6).
    /// `None` → `/ui` answers 404 with a clear message.
    pub ui_dir: Option<PathBuf>,
    /// Fleet SoT — SQLite (WAL) source of truth for the agent fleet.
    ///
    /// [`MiraiState::new`] defaults to an in-memory store; [`super::serve`]
    /// swaps in the file-backed one at `~/.openmirai/fleet.db` so the fleet
    /// survives restarts. Tests reach it directly for setup/asserts.
    pub fleet: Arc<FleetStore>,
    /// Objective SoT — SQLite (WAL) source of truth for fleet objectives and
    /// their agent links (the `objective_agents` bridge). Lives in the same
    /// `fleet.db` as [`MiraiState::fleet`].
    ///
    /// [`MiraiState::new`] defaults to an in-memory store; [`super::serve`] swaps
    /// in the file-backed one so objectives survive restarts. Tests reach it
    /// directly for setup/asserts.
    pub objectives: Arc<ObjectiveStore>,
}

impl MiraiState {
    /// Build the default AgentMirai state (real tmux backend, in-memory fleet).
    pub fn new() -> Self {
        Self {
            orchestrator: Arc::new(SessionManager::new(
                Arc::new(TmuxBackend::new()),
                SessionManager::default_registry_path(),
            )),
            folder_picker: Arc::new(FolderPicker::new()),
            projects_dirs: Vec::new(),
            ui_dir: None,
            fleet: Arc::new(FleetStore::in_memory().expect("in-memory fleet store must open")),
            objectives: Arc::new(
                ObjectiveStore::in_memory().expect("in-memory objective store must open"),
            ),
        }
    }
}

impl Default for MiraiState {
    fn default() -> Self {
        Self::new()
    }
}
