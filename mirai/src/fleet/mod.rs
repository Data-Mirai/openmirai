//! Fleet SoT — SQLite (WAL) source of truth for the agent fleet.
//!
//! The center persists the last-known state of every fleet member (agents,
//! nodes, bridges across the Mac / VPS) in a single `fleet` table. Members
//! report status over the `back-way status` endpoint; consumers read the fleet
//! via `GET /agents` or subscribe to live changes over SSE.
//!
//! - [`store`] — [`FleetStore`], the SQLite-backed store + event broadcaster.
//! - [`types`] — the SoT record ([`FleetMember`]) and request/event types.
//! - [`subscriber`] — [`FleetSubscriber`], the filtering pub-sub consumer of the
//!   event stream (the reactive `objective_complete` notification).

pub mod store;
pub mod subscriber;
pub mod types;

pub use store::{FleetError, FleetStore};
pub use subscriber::{EventFilter, FleetSubscriber};
pub use types::{
    FleetEvent, FleetEventKind, FleetMember, FleetQuery, FleetStatus, ObjectiveProgress,
    StatusUpdate,
};
