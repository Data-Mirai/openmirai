//! AgentMirai — the command-center layer over the OpenMirai engine.
//!
//! `openmirai-engine` is the pure agentic graph engine (graphs, agents, tools,
//! LLM adapters, workflow runs). This crate adds the **command-center** on top:
//!
//! - [`sessions`] — orchestrated Claude Code sessions over tmux (PRD-013).
//! - [`fleet`] — the fleet SoT (SQLite/WAL source of truth for every agent /
//!   node / bridge across the Mac & VPS).
//! - [`server`] — the composed HTTP API: the engine's core routes PLUS the
//!   `/api/v1/orchestrator/*`, `/api/v1/fleet/*` and `/ui` routes, under one
//!   shared auth / CORS / cross-site policy. The `mirai` binary serves this.
//!
//! The dependency direction is strict: `mirai → openmirai-engine`, never the
//! reverse. The engine knows nothing about sessions or the fleet.

pub mod fleet;
pub mod objectives;
pub mod server;
pub mod sessions;

// Re-export fleet SoT types.
pub use fleet::{
    EventFilter, FleetError, FleetEvent, FleetEventKind, FleetMember, FleetQuery, FleetStatus,
    FleetStore, FleetSubscriber, ObjectiveProgress, StatusUpdate,
};

// Re-export objective SoT types.
pub use objectives::{NewObjective, Objective, ObjectiveError, ObjectiveStatus, ObjectiveStore};

// Re-export session orchestration types.
pub use sessions::{
    detect_status, SessionBackend, SessionError, SessionManager, SessionRecord, SessionStatus,
    SpawnParams, TmuxBackend,
};

// Re-export the composed server surface.
pub use server::{create_router, serve, MiraiState};
