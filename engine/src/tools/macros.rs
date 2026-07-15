//! Centralized macros for tool creation.

/// The core macro that defines a tool struct, a factory struct,
/// and implements the ToolFactory and Default traits.
#[macro_export]
macro_rules! define_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        category = $cat:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        pub struct $tool;

        pub struct $factory {
            spec: $crate::tools::base::ToolSpec,
        }

        impl $factory {
            pub fn new() -> Self {
                Self {
                    spec: $crate::tools::base::ToolSpec {
                        tool_type: $tool_type.into(),
                        name: $name.into(),
                        description: $desc.into(),
                        version: "1.0.0".into(),
                        category: $cat.into(),
                        inputs: vec![$($input),*],
                        outputs: vec![$($output),*],
                        config_fields: vec![$($cfg),*],
                    },
                }
            }
        }

        impl Default for $factory {
            fn default() -> Self {
                Self::new()
            }
        }

        impl $crate::tools::registry::ToolFactory for $factory {
            fn create(&self) -> ::std::sync::Arc<dyn $crate::tools::registry::Tool> {
                ::std::sync::Arc::new($tool)
            }
            fn spec(&self) -> &$crate::tools::base::ToolSpec {
                &self.spec
            }
        }
    };
}

#[macro_export]
macro_rules! logic_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "logic",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! ai_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "ai",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! data_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "data",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! fs_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        category = $cat:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = $cat,
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! git_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "git",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! mcp_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "mcp",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! output_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "output",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! system_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        category = $cat:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = $cat,
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! trigger_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "trigger",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}

#[macro_export]
macro_rules! agent_tool {
    (
        struct $tool:ident, factory $factory:ident;
        tool_type = $tool_type:expr,
        name = $name:expr,
        description = $desc:expr,
        inputs = [ $($input:expr),* $(,)? ],
        outputs = [ $($output:expr),* $(,)? ],
        config_fields = [ $($cfg:expr),* $(,)? ]
    ) => {
        $crate::define_tool! {
            struct $tool, factory $factory;
            tool_type = $tool_type,
            name = $name,
            description = $desc,
            category = "agent",
            inputs = [ $($input),* ],
            outputs = [ $($output),* ],
            config_fields = [ $($cfg),* ]
        }
    };
}
