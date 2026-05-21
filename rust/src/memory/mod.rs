//! Memory subsystem -- short-term session traces and long-term persistent learning.

pub mod in_memory_backend;
pub mod long_term;
pub mod short_term;

pub use in_memory_backend::InMemoryBackend;
pub use long_term::{LongTermEntry, LongTermMemory, MemoryBackend};
pub use short_term::{ShortTermEntry, ShortTermMemory};
