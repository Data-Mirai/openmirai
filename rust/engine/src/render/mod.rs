//! Render subsystem -- Markdown to self-contained HTML with themes and charts.

pub mod charts;
pub mod engine;
pub mod themes;

pub use charts::{bar_chart, line_chart, pie_chart};
pub use engine::RenderEngine;
pub use themes::{dark_theme, light_theme, Theme};
