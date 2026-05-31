//! Universe — Multi-agent routing system.
//!
//! A Universe is a container of Souls (agents with personality) that receives
//! messages and routes them to the most appropriate agent based on configurable
//! strategies.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::soul::Soul;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Router strategy for selecting which agent handles a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum RouterStrategy {
    /// Use an LLM to classify the message and select the best agent.
    LlmClassify,
    /// Match keywords in the message to agent capabilities.
    #[default]
    KeywordMatch,
    /// Rotate agents in order.
    RoundRobin,
    /// User explicitly mentions agent name (e.g., "@analyst").
    Explicit,
}

/// Universe configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniverseConfig {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub router_strategy: RouterStrategy,
    #[serde(default)]
    pub router_prompt: Option<String>,
    #[serde(default)]
    pub default_response: String,
}

/// A registered agent within the Universe.
#[derive(Debug, Clone)]
pub struct UniverseAgent {
    pub soul: Soul,
    pub agent_id: String,
}

/// Routing decision result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub agent_name: String,
    pub agent_id: String,
    pub confidence: f64,
    pub strategy_used: String,
    pub reason: Option<String>,
}

/// The Universe runtime.
pub struct Universe {
    pub config: UniverseConfig,
    pub agents: Vec<UniverseAgent>,
    round_robin_idx: std::sync::atomic::AtomicUsize,
}

impl Universe {
    pub fn new(config: UniverseConfig) -> Self {
        Self {
            config,
            agents: Vec::new(),
            round_robin_idx: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub fn add_agent(&mut self, soul: Soul, agent_id: impl Into<String>) {
        self.agents.push(UniverseAgent {
            soul,
            agent_id: agent_id.into(),
        });
    }

    /// Route a message to the best agent.
    pub fn route(&self, message: &str) -> RoutingDecision {
        match self.config.router_strategy {
            RouterStrategy::Explicit => self.route_explicit(message),
            RouterStrategy::KeywordMatch => self.route_keyword(message),
            RouterStrategy::RoundRobin => self.route_round_robin(),
            RouterStrategy::LlmClassify => {
                // LLM-based routing requires async — fall back to keyword for sync calls.
                // In production, use route_async which calls the LLM.
                self.route_keyword(message)
            }
        }
    }

    /// Route with @mention syntax: "@analyst what are the sales?"
    fn route_explicit(&self, message: &str) -> RoutingDecision {
        let msg_lower = message.to_lowercase();
        for agent in &self.agents {
            let mention = format!("@{}", agent.soul.name.to_lowercase());
            if msg_lower.contains(&mention) {
                return RoutingDecision {
                    agent_name: agent.soul.name.clone(),
                    agent_id: agent.agent_id.clone(),
                    confidence: 1.0,
                    strategy_used: "explicit".into(),
                    reason: Some(format!("Mentioned @{}", agent.soul.name)),
                };
            }
        }
        // Fall back to keyword matching if no explicit mention.
        self.route_keyword(message)
    }

    /// Route by matching message keywords to agent capabilities.
    fn route_keyword(&self, message: &str) -> RoutingDecision {
        let msg_lower = message.to_lowercase();
        let mut best_score = 0usize;
        let mut best_agent: Option<&UniverseAgent> = None;

        for agent in &self.agents {
            let score: usize = agent
                .soul
                .capabilities
                .iter()
                .filter(|cap| {
                    let cap_lower = cap.to_lowercase().replace('_', " ");
                    // Check if any word from the capability appears in the message.
                    cap_lower
                        .split_whitespace()
                        .any(|word| msg_lower.contains(word))
                })
                .count();

            if score > best_score {
                best_score = score;
                best_agent = Some(agent);
            }
        }

        match best_agent {
            Some(agent) => RoutingDecision {
                agent_name: agent.soul.name.clone(),
                agent_id: agent.agent_id.clone(),
                confidence: (best_score as f64 / agent.soul.capabilities.len().max(1) as f64)
                    .min(1.0),
                strategy_used: "keyword_match".into(),
                reason: Some(format!("{} capability matches", best_score)),
            },
            None => self.default_routing(),
        }
    }

    /// Round-robin: rotate through agents.
    fn route_round_robin(&self) -> RoutingDecision {
        if self.agents.is_empty() {
            return self.default_routing();
        }
        let idx = self
            .round_robin_idx
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.agents.len();
        let agent = &self.agents[idx];
        RoutingDecision {
            agent_name: agent.soul.name.clone(),
            agent_id: agent.agent_id.clone(),
            confidence: 1.0,
            strategy_used: "round_robin".into(),
            reason: Some(format!("Index {idx}")),
        }
    }

    fn default_routing(&self) -> RoutingDecision {
        // If there's at least one agent, use the first as default.
        if let Some(agent) = self.agents.first() {
            return RoutingDecision {
                agent_name: agent.soul.name.clone(),
                agent_id: agent.agent_id.clone(),
                confidence: 0.1,
                strategy_used: "default".into(),
                reason: Some("No match — using default agent".into()),
            };
        }
        RoutingDecision {
            agent_name: "none".into(),
            agent_id: "none".into(),
            confidence: 0.0,
            strategy_used: "none".into(),
            reason: Some("No agents registered in universe".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// A2A Protocol — Agent-to-Agent messaging
// ---------------------------------------------------------------------------

/// A2A message types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum A2AMessageType {
    Request,
    Response,
    Broadcast,
    Delegate,
}

/// Agent-to-Agent message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AMessage {
    pub id: String,
    pub from_agent: String,
    pub to_agent: String,
    pub message_type: A2AMessageType,
    pub payload: Value,
    pub correlation_id: Option<String>,
    pub timestamp: u64,
}

/// GroupChat session for multi-agent debate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatConfig {
    pub topic: String,
    pub participants: Vec<String>,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default)]
    pub moderator_strategy: ModeratorStrategy,
    #[serde(default)]
    pub consensus_required: bool,
}

fn default_max_rounds() -> usize {
    5
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModeratorStrategy {
    #[default]
    RoundRobin,
    TopicBased,
}

/// A single turn in a group chat.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatTurn {
    pub round: usize,
    pub agent_name: String,
    pub content: String,
}

/// Group chat result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupChatResult {
    pub topic: String,
    pub rounds_completed: usize,
    pub transcript: Vec<GroupChatTurn>,
    pub consensus_reached: bool,
    pub summary: Option<String>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_soul(name: &str, caps: &[&str]) -> Soul {
        Soul {
            name: name.into(),
            identity: format!("I am {name}"),
            personality: "Helpful".into(),
            capabilities: caps.iter().map(|c| c.to_string()).collect(),
            constraints: vec![],
            workflows: vec![],
            knowledge_refs: vec![],
            context: String::new(),
        }
    }

    #[test]
    fn keyword_routing() {
        let config = UniverseConfig {
            name: "test".into(),
            description: "test universe".into(),
            router_strategy: RouterStrategy::KeywordMatch,
            router_prompt: None,
            default_response: "I can't help".into(),
        };
        let mut uni = Universe::new(config);
        uni.add_agent(
            make_soul("analyst", &["analysis", "reporting", "data"]),
            "a1",
        );
        uni.add_agent(make_soul("support", &["support", "help", "tickets"]), "a2");

        let decision = uni.route("Can you analyze the sales data?");
        assert_eq!(decision.agent_name, "analyst");
        assert!(decision.confidence > 0.0);

        let decision = uni.route("I need help with my ticket");
        assert_eq!(decision.agent_name, "support");
    }

    #[test]
    fn explicit_routing() {
        let config = UniverseConfig {
            name: "test".into(),
            description: "".into(),
            router_strategy: RouterStrategy::Explicit,
            router_prompt: None,
            default_response: "".into(),
        };
        let mut uni = Universe::new(config);
        uni.add_agent(make_soul("analyst", &["analysis"]), "a1");
        uni.add_agent(make_soul("support", &["support"]), "a2");

        let decision = uni.route("@analyst what were yesterday's sales?");
        assert_eq!(decision.agent_name, "analyst");
        assert_eq!(decision.confidence, 1.0);
        assert_eq!(decision.strategy_used, "explicit");
    }

    #[test]
    fn round_robin_routing() {
        let config = UniverseConfig {
            name: "test".into(),
            description: "".into(),
            router_strategy: RouterStrategy::RoundRobin,
            router_prompt: None,
            default_response: "".into(),
        };
        let mut uni = Universe::new(config);
        uni.add_agent(make_soul("a", &[]), "a1");
        uni.add_agent(make_soul("b", &[]), "a2");
        uni.add_agent(make_soul("c", &[]), "a3");

        assert_eq!(uni.route("hello").agent_name, "a");
        assert_eq!(uni.route("hello").agent_name, "b");
        assert_eq!(uni.route("hello").agent_name, "c");
        assert_eq!(uni.route("hello").agent_name, "a"); // wraps around
    }

    #[test]
    fn empty_universe_returns_none() {
        let config = UniverseConfig {
            name: "empty".into(),
            description: "".into(),
            router_strategy: RouterStrategy::KeywordMatch,
            router_prompt: None,
            default_response: "".into(),
        };
        let uni = Universe::new(config);
        let decision = uni.route("hello");
        assert_eq!(decision.agent_name, "none");
        assert_eq!(decision.confidence, 0.0);
    }

    #[test]
    fn a2a_message_serde() {
        let msg = A2AMessage {
            id: "m1".into(),
            from_agent: "analyst".into(),
            to_agent: "support".into(),
            message_type: A2AMessageType::Request,
            payload: serde_json::json!({"query": "customer count"}),
            correlation_id: Some("corr-1".into()),
            timestamp: 1234567890,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: A2AMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(back.from_agent, "analyst");
        assert_eq!(back.message_type, A2AMessageType::Request);
    }

    #[test]
    fn groupchat_config_defaults() {
        let config: GroupChatConfig =
            serde_json::from_str(r#"{"topic": "scaling", "participants": ["a", "b"]}"#).unwrap();
        assert_eq!(config.max_rounds, 5);
        assert_eq!(config.moderator_strategy, ModeratorStrategy::RoundRobin);
        assert!(!config.consensus_required);
    }
}
