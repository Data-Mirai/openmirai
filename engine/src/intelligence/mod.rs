//! Intelligence module -- LLM-powered analysis, tracing, context assembly,
//! memory flushing, graph suggestions, and playbook rule injection.

pub mod context_compiler;
pub mod memory_flusher;
pub mod playbook;
pub mod reflector;
pub mod suggester;
pub mod tracer;

pub use context_compiler::{CompiledContext, ContextCompiler, ContextSection};
pub use memory_flusher::MemoryFlusher;
pub use playbook::{Playbook, PlaybookRule};
pub use reflector::{Reflection, Reflector};
pub use suggester::{Suggester, Suggestion, SuggestionType};
pub use tracer::{ExecutionTracer, TokenUsage, TraceRecord, TraceSummary};
