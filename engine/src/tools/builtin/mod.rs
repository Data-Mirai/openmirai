pub mod agent;
pub mod ai;
pub mod data;
pub mod filesystem;
pub mod git;
pub mod logic;
pub mod mcp;
pub mod output;
pub mod system;
pub mod trigger;

use crate::tools::registry::ToolRegistry;

/// Register all built-in tools into the given registry.
pub fn register_all_builtin_tools(registry: &mut ToolRegistry) {
    logic::register_logic_tools(registry);
    ai::register_ai_tools(registry);
    data::register_data_tools(registry);
    filesystem::register_filesystem_tools(registry);
    system::register_system_tools(registry);
    git::register_git_tools(registry);
    output::register_output_tools(registry);
    agent::register_agent_tools(registry);
    mcp::register_mcp_tools(registry);
    trigger::register_trigger_tools(registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_all_builtin_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_all_builtin_tools(&mut reg);
        // 7 logic + 3 ai + 10 data + 12 filesystem + 3 system + 4 git
        // + 1 output + 1 agent + 1 mcp + 5 trigger = 47
        assert_eq!(reg.list_tools().len(), 47);
    }
}
