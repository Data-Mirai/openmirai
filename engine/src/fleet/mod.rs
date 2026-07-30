//! Fleet SoT — SQLite (WAL) source of truth for the agent fleet.
//!
//! The center persists the last-known state of every fleet member (agents,
//! nodes, bridges across the Mac / VPS) in a single `fleet` table. Members
//! report status over the `back-way status` endpoint; consumers read the fleet
//! via `GET /agents` or subscribe to live changes over SSE.
//!
//! - [`store`] — [`FleetStore`], the SQLite-backed store + event broadcaster.
//! - [`types`] — the SoT record ([`FleetMember`]) and request/event types.

pub mod store;
pub mod types;

pub use store::{FleetError, FleetStore};
pub use types::{
    FleetEvent, FleetEventKind, FleetMember, FleetQuery, FleetStatus, StatusUpdate,
};
