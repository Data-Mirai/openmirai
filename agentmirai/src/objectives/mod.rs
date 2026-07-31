//! Objective SoT — SQLite (WAL) source of truth for fleet objectives.
//!
//! An objective is a user-stated goal the wizard turns into agents. The center
//! persists every objective in the `objectives` table and wires it to the fleet
//! members working it through the `objective_agents` bridge table (living in the
//! same `fleet.db`, right next to the flota).
//!
//! - [`store`] — [`ObjectiveStore`], the SQLite-backed store.
//! - [`types`] — the SoT record ([`Objective`]) and request types.

pub mod store;
pub mod types;

pub use store::{ObjectiveError, ObjectiveStore};
pub use types::{NewObjective, Objective, ObjectiveStatus};
