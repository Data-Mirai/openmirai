//! Agent Templates — pre-built agent configurations for quick start.
//!
//! Each template provides a complete AgentSpec (graph + config) that can be
//! customized and deployed immediately. Solves the "blank canvas" problem.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Category for organizing templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateCategory {
    Assistant,
    Automation,
    Analysis,
    Integration,
    Devops,
}

/// A pre-built agent template.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTemplate {
    pub id: String,
    pub name: String,
    pub category: TemplateCategory,
    pub description: String,
    pub required_providers: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Complete agent spec as JSON Value.
    pub spec: Value,
}

// ---------------------------------------------------------------------------
// Built-in templates
// ---------------------------------------------------------------------------

/// Get all built-in templates.
pub fn builtin_templates() -> Vec<AgentTemplate> {
    vec![
        qa_assistant(),
        data_analyzer(),
        code_reviewer(),
        web_scraper(),
        email_summarizer(),
        multi_step_researcher(),
        automated_report(),
        guardian(),
        translator(),
        meeting_notes(),
    ]
}

/// Get a template by ID.
pub fn get_template(id: &str) -> Option<AgentTemplate> {
    builtin_templates().into_iter().find(|t| t.id == id)
}

/// List templates, optionally filtered by category.
pub fn list_templates(category: Option<&TemplateCategory>) -> Vec<AgentTemplate> {
    let all = builtin_templates();
    match category {
        Some(cat) => all.into_iter().filter(|t| t.category == *cat).collect(),
        None => all,
    }
}

// ---------------------------------------------------------------------------
// Template definitions
// ---------------------------------------------------------------------------

fn qa_assistant() -> AgentTemplate {
    AgentTemplate {
        id: "qa-assistant".into(),
        name: "Q&A Assistant".into(),
        category: TemplateCategory::Assistant,
        description: "Simple chatbot that answers questions using an LLM".into(),
        required_providers: vec!["any".into()],
        tags: vec!["chat".into(), "qa".into(), "beginner".into()],
        spec: json!({
            "name": "qa-assistant",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "llm", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Answer the user's question concisely and accurately.",
                        "temperature": 0.7,
                        "max_tokens": 1024
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "llm"},
                    {"source": "llm", "target": "out"}
                ]
            }
        }),
    }
}

fn data_analyzer() -> AgentTemplate {
    AgentTemplate {
        id: "data-analyzer".into(),
        name: "Data Analyzer".into(),
        category: TemplateCategory::Analysis,
        description: "Reads a file and generates insights using an LLM".into(),
        required_providers: vec!["any".into()],
        tags: vec!["data".into(), "analysis".into(), "csv".into()],
        spec: json!({
            "name": "data-analyzer",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "read", "tool_type": "filesystem/read_file"},
                    {"id": "analyze", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Analyze the following data and provide key insights, patterns, and recommendations.",
                        "max_tokens": 2048
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "read"},
                    {"source": "read", "target": "analyze"},
                    {"source": "analyze", "target": "out"}
                ]
            }
        }),
    }
}

fn code_reviewer() -> AgentTemplate {
    AgentTemplate {
        id: "code-reviewer".into(),
        name: "Code Reviewer".into(),
        category: TemplateCategory::Devops,
        description: "Reviews git diff and suggests improvements".into(),
        required_providers: vec!["any".into()],
        tags: vec!["code".into(), "review".into(), "git".into()],
        spec: json!({
            "name": "code-reviewer",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "diff", "tool_type": "git/diff"},
                    {"id": "review", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Review this code diff. Identify bugs, security issues, and suggest improvements. Be specific.",
                        "max_tokens": 2048
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "diff"},
                    {"source": "diff", "target": "review"},
                    {"source": "review", "target": "out"}
                ]
            }
        }),
    }
}

fn web_scraper() -> AgentTemplate {
    AgentTemplate {
        id: "web-scraper".into(),
        name: "Web Scraper + Summarizer".into(),
        category: TemplateCategory::Automation,
        description: "Scrapes a web page and summarizes content with LLM".into(),
        required_providers: vec!["any".into()],
        tags: vec!["web".into(), "scraping".into(), "summary".into()],
        spec: json!({
            "name": "web-scraper",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "scrape", "tool_type": "data/web_scrape"},
                    {"id": "clean", "tool_type": "data/html_to_markdown"},
                    {"id": "summarize", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Summarize the following web page content in 3-5 bullet points.",
                        "max_tokens": 1024
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "scrape"},
                    {"source": "scrape", "target": "clean"},
                    {"source": "clean", "target": "summarize"},
                    {"source": "summarize", "target": "out"}
                ]
            }
        }),
    }
}

fn email_summarizer() -> AgentTemplate {
    AgentTemplate {
        id: "email-summarizer".into(),
        name: "Email/Document Summarizer".into(),
        category: TemplateCategory::Assistant,
        description: "Reads a document and generates a concise summary".into(),
        required_providers: vec!["any".into()],
        tags: vec!["email".into(), "summary".into(), "document".into()],
        spec: json!({
            "name": "email-summarizer",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "read", "tool_type": "filesystem/read_file"},
                    {"id": "summarize", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Summarize this document. Include: key points, action items, and decisions made.",
                        "max_tokens": 1024
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "read"},
                    {"source": "read", "target": "summarize"},
                    {"source": "summarize", "target": "out"}
                ]
            }
        }),
    }
}

fn multi_step_researcher() -> AgentTemplate {
    AgentTemplate {
        id: "multi-step-researcher".into(),
        name: "Multi-Step Researcher".into(),
        category: TemplateCategory::Analysis,
        description: "Plans research, scrapes multiple sources in parallel (fan-out), then synthesizes".into(),
        required_providers: vec!["any".into()],
        tags: vec!["research".into(), "fanout".into(), "synthesis".into()],
        spec: json!({
            "name": "multi-step-researcher",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "plan", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Given the research topic, generate 3 specific search queries.",
                        "output_schema": {"type":"object","properties":{"queries":{"type":"array"}},"required":["queries"]},
                        "max_tokens": 512
                    }},
                    {"id": "search1", "tool_type": "data/web_scrape", "config": {"url_field": "queries[0]"}},
                    {"id": "search2", "tool_type": "data/web_scrape", "config": {"url_field": "queries[1]"}},
                    {"id": "search3", "tool_type": "data/web_scrape", "config": {"url_field": "queries[2]"}},
                    {"id": "synthesize", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Synthesize the research results into a comprehensive analysis.",
                        "max_tokens": 2048
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "plan"},
                    {"source": "plan", "target": "search1"},
                    {"source": "plan", "target": "search2"},
                    {"source": "plan", "target": "search3"},
                    {"source": "search1", "target": "synthesize"},
                    {"source": "search2", "target": "synthesize"},
                    {"source": "search3", "target": "synthesize"},
                    {"source": "synthesize", "target": "out"}
                ]
            }
        }),
    }
}

fn automated_report() -> AgentTemplate {
    AgentTemplate {
        id: "automated-report".into(),
        name: "Automated Report Generator".into(),
        category: TemplateCategory::Automation,
        description: "Reads from database on schedule and generates formatted reports".into(),
        required_providers: vec!["any".into()],
        tags: vec!["report".into(), "database".into(), "schedule".into()],
        spec: json!({
            "name": "automated-report",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/schedule", "config": {"interval": "daily"}},
                    {"id": "query", "tool_type": "data/db_read", "config": {
                        "query": "SELECT * FROM metrics WHERE date = CURRENT_DATE"
                    }},
                    {"id": "format", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Format this data into a professional daily report with sections: Overview, Key Metrics, Trends, and Recommendations.",
                        "max_tokens": 2048
                    }},
                    {"id": "save", "tool_type": "data/storage_write"},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "query"},
                    {"source": "query", "target": "format"},
                    {"source": "format", "target": "save"},
                    {"source": "save", "target": "out"}
                ]
            }
        }),
    }
}

fn guardian() -> AgentTemplate {
    AgentTemplate {
        id: "guardian".into(),
        name: "System Guardian".into(),
        category: TemplateCategory::Devops,
        description: "Monitors system health and analyzes anomalies with LLM".into(),
        required_providers: vec!["any".into()],
        tags: vec!["monitoring".into(), "health".into(), "alerting".into()],
        spec: json!({
            "name": "guardian",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/heartbeat", "config": {"interval_seconds": 300}},
                    {"id": "check", "tool_type": "system/bash", "config": {
                        "command": "echo '{\"cpu\": '$(top -l 1 | grep 'CPU usage' | awk '{print $3}')'}'"
                    }},
                    {"id": "analyze", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Analyze system metrics. Report any anomalies or concerns.",
                        "max_tokens": 512
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "check"},
                    {"source": "check", "target": "analyze"},
                    {"source": "analyze", "target": "out"}
                ]
            }
        }),
    }
}

fn translator() -> AgentTemplate {
    AgentTemplate {
        id: "translator".into(),
        name: "Multi-Language Translator".into(),
        category: TemplateCategory::Assistant,
        description: "Detects source language and translates to target".into(),
        required_providers: vec!["any".into()],
        tags: vec!["translation".into(), "language".into(), "i18n".into()],
        spec: json!({
            "name": "translator",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "detect", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Detect the language of the input text. Respond with JSON.",
                        "output_schema": {"type":"object","properties":{"language":{"type":"string"},"confidence":{"type":"number"}},"required":["language"]},
                        "max_tokens": 128
                    }},
                    {"id": "translate", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Translate the text to the target language. Maintain tone and meaning.",
                        "max_tokens": 2048
                    }},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "detect"},
                    {"source": "detect", "target": "translate"},
                    {"source": "translate", "target": "out"}
                ]
            }
        }),
    }
}

fn meeting_notes() -> AgentTemplate {
    AgentTemplate {
        id: "meeting-notes".into(),
        name: "Meeting Notes Generator".into(),
        category: TemplateCategory::Assistant,
        description: "Transcribes audio and generates structured meeting notes".into(),
        required_providers: vec!["any".into()],
        tags: vec!["meeting".into(), "transcription".into(), "notes".into()],
        spec: json!({
            "name": "meeting-notes",
            "version": "v1",
            "graph": {
                "nodes": [
                    {"id": "trigger", "tool_type": "trigger/manual"},
                    {"id": "transcribe", "tool_type": "ai/transcribe"},
                    {"id": "notes", "tool_type": "ai/llm_call", "config": {
                        "prompt": "Generate structured meeting notes from this transcript. Include: Attendees, Agenda, Key Decisions, Action Items (with assignees), and Next Steps.",
                        "max_tokens": 2048
                    }},
                    {"id": "save", "tool_type": "data/vault_write"},
                    {"id": "out", "tool_type": "output/response"}
                ],
                "edges": [
                    {"source": "trigger", "target": "transcribe"},
                    {"source": "transcribe", "target": "notes"},
                    {"source": "notes", "target": "save"},
                    {"source": "save", "target": "out"}
                ]
            }
        }),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_templates_has_10() {
        let templates = builtin_templates();
        assert_eq!(templates.len(), 10);
    }

    #[test]
    fn get_template_by_id() {
        let template = get_template("qa-assistant");
        assert!(template.is_some());
        assert_eq!(template.unwrap().name, "Q&A Assistant");
    }

    #[test]
    fn get_nonexistent_template() {
        assert!(get_template("does-not-exist").is_none());
    }

    #[test]
    fn filter_by_category() {
        let assistants = list_templates(Some(&TemplateCategory::Assistant));
        assert!(assistants.len() >= 3); // qa, email-summarizer, translator, meeting-notes
        assert!(assistants.iter().all(|t| t.category == TemplateCategory::Assistant));
    }

    #[test]
    fn all_templates_have_valid_spec() {
        for template in builtin_templates() {
            assert!(!template.id.is_empty());
            assert!(!template.name.is_empty());
            assert!(!template.description.is_empty());
            // Spec should have graph with nodes
            assert!(template.spec["graph"]["nodes"].is_array());
        }
    }

    #[test]
    fn template_serde_roundtrip() {
        let template = get_template("data-analyzer").unwrap();
        let json = serde_json::to_string(&template).unwrap();
        let back: AgentTemplate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "data-analyzer");
    }
}
