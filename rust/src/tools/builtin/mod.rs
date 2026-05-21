pub mod ai;
pub mod data;
pub mod filesystem;
pub mod git;
pub mod logic;
pub mod system;

use crate::tools::registry::ToolRegistry;

/// Register all built-in tools (logic, ai, data, filesystem, system, git) into the given registry.
pub fn register_all_builtin_tools(registry: &mut ToolRegistry) {
    logic::register_logic_tools(registry);
    ai::register_ai_tools(registry);
    data::register_data_tools(registry);
    filesystem::register_filesystem_tools(registry);
    system::register_system_tools(registry);
    git::register_git_tools(registry);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_all_builtin_tools_adds_all() {
        let mut reg = ToolRegistry::new();
        register_all_builtin_tools(&mut reg);
        // 4 logic + 2 ai + 4 data + 6 filesystem + 1 system + 4 git = 21
        assert_eq!(reg.list_tools().len(), 21);
    }
}
