//! ContextCompiler -- 5-phase context assembly for LLM nodes.
//!
//! Phases:
//! 1. Agent Identity (name, description, system_prompt) -- priority 1
//! 2. Playbook Rules -- priority 2
//! 3. Session Context (current conversation) -- priority 3
//! 4. Long-term Memory (relevant entries) -- priority 4
//! 5. Compression -- if total > max_context_length, truncate lowest priority

use serde::{Deserialize, Serialize};

use crate::core::AgentSpec;
use crate::memory::LongTermEntry;

/// Default maximum context length in characters.
const DEFAULT_MAX_CONTEXT_LENGTH: usize = 12_000;

/// A named section of compiled context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextSection {
    pub name: String,
    pub content: String,
    /// 1 = highest priority.
    pub priority: u8,
}

/// The assembled context ready to be sent to the LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompiledContext {
    pub system_prompt: String,
    pub context_sections: Vec<ContextSection>,
    pub total_chars: usize,
}

impl CompiledContext {
    /// Concatenate all sections into a single prompt string.
    pub fn to_prompt_string(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.system_prompt.is_empty() {
            parts.push(self.system_prompt.clone());
        }
        for section in &self.context_sections {
            if !section.content.is_empty() {
                parts.push(format!(
                    "[{}]\n{}\n[/{}]",
                    section.name.to_uppercase(),
                    section.content,
                    section.name.to_uppercase()
                ));
            }
        }
        parts.join("\n\n")
    }
}

/// Assembles optimized context for LLM calls in 5 phases.
pub struct ContextCompiler {
    max_context_length: usize,
}

impl ContextCompiler {
    pub fn new() -> Self {
        Self {
            max_context_length: DEFAULT_MAX_CONTEXT_LENGTH,
        }
    }

    pub fn with_max_length(max_context_length: usize) -> Self {
        Self {
            max_context_length,
        }
    }

    /// Compile context from all available sources.
    ///
    /// # Phases
    /// 1. Agent Identity -- name, description, system_prompt (priority 1)
    /// 2. Playbook Rules -- injected rules (priority 2)
    /// 3. Session Context -- current conversation data (priority 3)
    /// 4. Long-term Memory -- relevant past entries (priority 4)
    /// 5. Compression -- truncate lowest-priority sections if over budget
    pub fn compile(
        &self,
        agent_spec: &AgentSpec,
        session_context: &str,
        memory_entries: &[LongTermEntry],
        playbook_rules: &[String],
    ) -> CompiledContext {
        let mut sections = Vec::new();

        // Phase 1: Agent Identity
        let mut identity_parts = Vec::new();
        identity_parts.push(format!("Agent: {}", agent_spec.name));
        if !agent_spec.description.is_empty() {
            identity_parts.push(agent_spec.description.clone());
        }
        let identity_content = identity_parts.join("\n");
        if !identity_content.is_empty() {
            sections.push(ContextSection {
                name: "Agent Identity".to_string(),
                content: identity_content,
                priority: 1,
            });
        }

        // Phase 2: Playbook Rules
        if !playbook_rules.is_empty() {
            let rules_text: String = playbook_rules
                .iter()
                .enumerate()
                .map(|(i, r)| format!("{}. {}", i + 1, r))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(ContextSection {
                name: "Playbook Rules".to_string(),
                content: rules_text,
                priority: 2,
            });
        }

        // Phase 3: Session Context (ALWAYS included -- REGLA-56)
        sections.push(ContextSection {
            name: "Session Context".to_string(),
            content: session_context.to_string(),
            priority: 3,
        });

        // Phase 4: Long-term Memory
        if !memory_entries.is_empty() {
            let mem_text: String = memory_entries
                .iter()
                .map(|e| format!("- [{}] {}", e.entry_type, e.content))
                .collect::<Vec<_>>()
                .join("\n");
            sections.push(ContextSection {
                name: "Memory".to_string(),
                content: mem_text,
                priority: 4,
            });
        }

        // System prompt
        let system_prompt = agent_spec
            .system_prompt
            .clone()
            .unwrap_or_default();

        // Phase 5: Compression
        let total_chars: usize = system_prompt.len()
            + sections.iter().map(|s| s.content.len()).sum::<usize>();

        if total_chars > self.max_context_length {
            self.compress(&mut sections, total_chars);
        }

        let final_chars = system_prompt.len()
            + sections.iter().map(|s| s.content.len()).sum::<usize>();

        CompiledContext {
            system_prompt,
            context_sections: sections,
            total_chars: final_chars,
        }
    }

    /// Compress sections by truncating lowest priority first (highest number).
    fn compress(&self, sections: &mut Vec<ContextSection>, total_chars: usize) {
        let excess = total_chars.saturating_sub(self.max_context_length);
        if excess == 0 {
            return;
        }

        // Sort by priority descending (lowest priority = highest number = first to truncate)
        let mut indices_by_priority: Vec<usize> = (0..sections.len()).collect();
        indices_by_priority.sort_by(|&a, &b| sections[b].priority.cmp(&sections[a].priority));

        let mut remaining_excess = excess;
        for &idx in &indices_by_priority {
            if remaining_excess == 0 {
                break;
            }
            let section = &mut sections[idx];
            let section_len = section.content.len();
            if section_len == 0 {
                continue;
            }

            let to_cut = remaining_excess.min(section_len);
            if to_cut >= section_len {
                // Remove entire section
                section.content = "[compressed]".to_string();
                remaining_excess = remaining_excess.saturating_sub(section_len);
            } else {
                // Keep start 60%, end 20%, cut middle
                let keep_chars = section_len - to_cut;
                let keep_start = (keep_chars as f64 * 0.75) as usize;
                let keep_end = keep_chars.saturating_sub(keep_start);
                section.content = format!(
                    "{}\n\n[... context compressed ...]\n\n{}",
                    &section.content[..keep_start],
                    &section.content[section_len.saturating_sub(keep_end)..]
                );
                remaining_excess = 0;
            }
        }
    }
}

impl Default for ContextCompiler {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn make_agent_spec(name: &str, system_prompt: Option<&str>) -> AgentSpec {
        AgentSpec {
            name: name.to_string(),
            description: "Test agent".to_string(),
            version: "v1".to_string(),
            agent_type: crate::core::AgentType::Managed,
            system_prompt: system_prompt.map(|s| s.to_string()),
            soul: None,
            inputs: None,
            outputs: None,
            graph: Default::default(),
            triggers: Vec::new(),
            config: Default::default(),
            resources: Vec::new(),
            metadata: HashMap::new(),
        }
    }

    fn make_memory_entry(content: &str, entry_type: &str) -> LongTermEntry {
        LongTermEntry {
            id: "m1".to_string(),
            entry_type: entry_type.to_string(),
            content: content.to_string(),
            tags: Vec::new(),
            session_id: None,
            created_at: 0.0,
            metadata: HashMap::new(),
        }
    }

    #[test]
    fn compile_all_phases() {
        let compiler = ContextCompiler::new();
        let spec = make_agent_spec("TestBot", Some("You are helpful."));
        let memory = vec![make_memory_entry("always retry on 429", "learning")];
        let rules = vec!["Be concise".to_string(), "Use examples".to_string()];

        let ctx = compiler.compile(&spec, "User asked about X", &memory, &rules);

        assert_eq!(ctx.system_prompt, "You are helpful.");
        assert_eq!(ctx.context_sections.len(), 4); // identity, rules, session, memory
        assert_eq!(ctx.context_sections[0].name, "Agent Identity");
        assert_eq!(ctx.context_sections[0].priority, 1);
        assert_eq!(ctx.context_sections[1].name, "Playbook Rules");
        assert_eq!(ctx.context_sections[2].name, "Session Context");
        assert_eq!(ctx.context_sections[3].name, "Memory");
        assert_eq!(ctx.context_sections[3].priority, 4);
    }

    #[test]
    fn compile_no_optional_sections() {
        let compiler = ContextCompiler::new();
        let spec = make_agent_spec("Bot", None);

        let ctx = compiler.compile(&spec, "hello", &[], &[]);

        // Identity + Session (always present)
        assert_eq!(ctx.context_sections.len(), 2);
        assert_eq!(ctx.system_prompt, "");
    }

    #[test]
    fn compile_compression_triggers() {
        let compiler = ContextCompiler::with_max_length(100);
        let spec = make_agent_spec("Bot", Some("system"));
        let long_context = "x".repeat(200);

        let ctx = compiler.compile(&spec, &long_context, &[], &[]);

        // Total should be reduced
        assert!(ctx.total_chars <= 200); // Some compression happened
    }

    #[test]
    fn compress_removes_lowest_priority_first() {
        let compiler = ContextCompiler::with_max_length(50);
        let spec = make_agent_spec("B", None);
        let memory = vec![make_memory_entry(&"m".repeat(100), "learning")];

        let ctx = compiler.compile(&spec, "session", &memory, &[]);

        // Memory (priority 4) should be compressed before identity (priority 1)
        let mem_section = ctx
            .context_sections
            .iter()
            .find(|s| s.name == "Memory");
        if let Some(ms) = mem_section {
            // Either compressed or fully removed
            assert!(ms.content.len() < 100 || ms.content.contains("compressed"));
        }
    }

    #[test]
    fn to_prompt_string() {
        let ctx = CompiledContext {
            system_prompt: "You are helpful.".to_string(),
            context_sections: vec![
                ContextSection {
                    name: "Session Context".to_string(),
                    content: "User asked about X".to_string(),
                    priority: 3,
                },
            ],
            total_chars: 50,
        };
        let prompt = ctx.to_prompt_string();
        assert!(prompt.contains("You are helpful."));
        assert!(prompt.contains("[SESSION CONTEXT]"));
        assert!(prompt.contains("User asked about X"));
    }

    #[test]
    fn default_max_length() {
        let c = ContextCompiler::default();
        assert_eq!(c.max_context_length, DEFAULT_MAX_CONTEXT_LENGTH);
    }
}
