//! Playbook -- rule-based prompt injection.
//!
//! Takes accumulated rules (from reflections or manual input) and injects
//! the most relevant ones into LLM prompts before execution.
//!
//! Rules can be auto-disabled when harmful feedback exceeds helpful (REGLA-47/48).

use serde::{Deserialize, Serialize};

/// Maximum rules to inject per prompt (REGLA-46).
const MAX_RULES_PER_PROMPT: usize = 10;

/// A single playbook rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaybookRule {
    pub id: String,
    /// The rule text to inject into prompts.
    pub rule: String,
    /// Whether the rule is currently active.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Optional context trigger -- when this substring appears, the rule applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger_condition: Option<String>,
    /// REGLA-47: auto-disable when harmful feedback exceeds helpful.
    #[serde(default = "default_true")]
    pub disable_on_harmful: bool,
}

fn default_true() -> bool {
    true
}

impl PlaybookRule {
    /// Create a simple rule with defaults.
    pub fn new(id: impl Into<String>, rule: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            rule: rule.into(),
            enabled: true,
            trigger_condition: None,
            disable_on_harmful: true,
        }
    }
}

/// Collection of playbook rules with injection and management methods.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Playbook {
    pub rules: Vec<PlaybookRule>,
}

impl Playbook {
    pub fn new(rules: Vec<PlaybookRule>) -> Self {
        Self { rules }
    }

    /// All currently enabled rules.
    pub fn get_active_rules(&self) -> Vec<&PlaybookRule> {
        self.rules.iter().filter(|r| r.enabled).collect()
    }

    /// Rules matching a given context string.
    ///
    /// Returns rules that either have no `trigger_condition` (always apply)
    /// or whose `trigger_condition` is a substring of `context`.
    /// Capped at `MAX_RULES_PER_PROMPT`.
    pub fn get_rules_for_context(&self, context: &str) -> Vec<&PlaybookRule> {
        let context_lower = context.to_lowercase();
        self.rules
            .iter()
            .filter(|r| {
                if !r.enabled {
                    return false;
                }
                match &r.trigger_condition {
                    None => true, // no trigger = always apply
                    Some(tc) => context_lower.contains(&tc.to_lowercase()),
                }
            })
            .take(MAX_RULES_PER_PROMPT)
            .collect()
    }

    /// Disable a rule by ID (REGLA-48).
    pub fn disable_rule(&mut self, id: &str) {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = false;
        }
    }

    /// Enable a rule by ID.
    pub fn enable_rule(&mut self, id: &str) {
        if let Some(rule) = self.rules.iter_mut().find(|r| r.id == id) {
            rule.enabled = true;
        }
    }

    /// Add a new rule.
    pub fn add_rule(&mut self, rule: PlaybookRule) {
        self.rules.push(rule);
    }

    /// Remove a rule by ID.
    pub fn remove_rule(&mut self, id: &str) {
        self.rules.retain(|r| r.id != id);
    }

    /// Concatenate all active rules into a single prompt injection block.
    pub fn to_prompt_injection(&self) -> String {
        let active = self.get_active_rules();
        if active.is_empty() {
            return String::new();
        }

        let mut lines = vec!["[PLAYBOOK RULES]".to_string()];
        for (i, rule) in active.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, rule.rule));
        }
        lines.push("[/PLAYBOOK RULES]".to_string());
        lines.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_rules() -> Vec<PlaybookRule> {
        vec![
            PlaybookRule {
                id: "r1".to_string(),
                rule: "Be concise in responses".to_string(),
                enabled: true,
                trigger_condition: None,
                disable_on_harmful: true,
            },
            PlaybookRule {
                id: "r2".to_string(),
                rule: "Use examples for technical topics".to_string(),
                enabled: true,
                trigger_condition: Some("technical".to_string()),
                disable_on_harmful: true,
            },
            PlaybookRule {
                id: "r3".to_string(),
                rule: "Disabled rule".to_string(),
                enabled: false,
                trigger_condition: None,
                disable_on_harmful: false,
            },
        ]
    }

    #[test]
    fn new_playbook() {
        let pb = Playbook::new(sample_rules());
        assert_eq!(pb.rules.len(), 3);
    }

    #[test]
    fn get_active_rules() {
        let pb = Playbook::new(sample_rules());
        let active = pb.get_active_rules();
        assert_eq!(active.len(), 2);
        assert!(active.iter().all(|r| r.enabled));
    }

    #[test]
    fn get_rules_for_context_no_trigger() {
        let pb = Playbook::new(sample_rules());
        let rules = pb.get_rules_for_context("random context");
        // r1 (no trigger) matches, r2 (trigger=technical) does NOT, r3 disabled
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].id, "r1");
    }

    #[test]
    fn get_rules_for_context_with_trigger() {
        let pb = Playbook::new(sample_rules());
        let rules = pb.get_rules_for_context("this is a technical question");
        // r1 (no trigger) + r2 (trigger matches)
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn get_rules_for_context_case_insensitive() {
        let pb = Playbook::new(sample_rules());
        let rules = pb.get_rules_for_context("TECHNICAL stuff");
        assert_eq!(rules.len(), 2);
    }

    #[test]
    fn disable_rule() {
        let mut pb = Playbook::new(sample_rules());
        pb.disable_rule("r1");
        let active = pb.get_active_rules();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "r2");
    }

    #[test]
    fn enable_rule() {
        let mut pb = Playbook::new(sample_rules());
        pb.enable_rule("r3");
        let active = pb.get_active_rules();
        assert_eq!(active.len(), 3);
    }

    #[test]
    fn add_rule() {
        let mut pb = Playbook::new(sample_rules());
        pb.add_rule(PlaybookRule::new("r4", "New rule"));
        assert_eq!(pb.rules.len(), 4);
        assert_eq!(pb.get_active_rules().len(), 3); // r1, r2, r4 (r3 still disabled)
    }

    #[test]
    fn remove_rule() {
        let mut pb = Playbook::new(sample_rules());
        pb.remove_rule("r2");
        assert_eq!(pb.rules.len(), 2);
        assert!(!pb.rules.iter().any(|r| r.id == "r2"));
    }

    #[test]
    fn to_prompt_injection_with_rules() {
        let pb = Playbook::new(sample_rules());
        let injection = pb.to_prompt_injection();
        assert!(injection.starts_with("[PLAYBOOK RULES]"));
        assert!(injection.ends_with("[/PLAYBOOK RULES]"));
        assert!(injection.contains("1. Be concise"));
        assert!(injection.contains("2. Use examples"));
        // Disabled rule not included
        assert!(!injection.contains("Disabled rule"));
    }

    #[test]
    fn to_prompt_injection_empty() {
        let pb = Playbook::new(vec![]);
        let injection = pb.to_prompt_injection();
        assert!(injection.is_empty());
    }

    #[test]
    fn to_prompt_injection_all_disabled() {
        let mut pb = Playbook::new(sample_rules());
        pb.disable_rule("r1");
        pb.disable_rule("r2");
        let injection = pb.to_prompt_injection();
        assert!(injection.is_empty());
    }

    #[test]
    fn playbook_rule_new() {
        let r = PlaybookRule::new("test-id", "test rule text");
        assert_eq!(r.id, "test-id");
        assert_eq!(r.rule, "test rule text");
        assert!(r.enabled);
        assert!(r.trigger_condition.is_none());
        assert!(r.disable_on_harmful);
    }

    #[test]
    fn playbook_rule_serde_roundtrip() {
        let r = PlaybookRule {
            id: "r1".to_string(),
            rule: "Be concise".to_string(),
            enabled: true,
            trigger_condition: Some("tech".to_string()),
            disable_on_harmful: false,
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: PlaybookRule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "r1");
        assert_eq!(back.trigger_condition, Some("tech".to_string()));
        assert!(!back.disable_on_harmful);
    }

    #[test]
    fn disable_nonexistent_rule_is_noop() {
        let mut pb = Playbook::new(sample_rules());
        pb.disable_rule("nonexistent");
        assert_eq!(pb.get_active_rules().len(), 2);
    }

    #[test]
    fn remove_nonexistent_rule_is_noop() {
        let mut pb = Playbook::new(sample_rules());
        pb.remove_rule("nonexistent");
        assert_eq!(pb.rules.len(), 3);
    }

    #[test]
    fn max_rules_cap() {
        let rules: Vec<PlaybookRule> = (0..20)
            .map(|i| PlaybookRule::new(format!("r{}", i), format!("rule {}", i)))
            .collect();
        let pb = Playbook::new(rules);
        let context_rules = pb.get_rules_for_context("anything");
        assert_eq!(context_rules.len(), MAX_RULES_PER_PROMPT);
    }
}
