//! Eval Framework — automated quality evaluation of agent outputs.
//!
//! Evaluates execution results on multiple dimensions:
//! - Format compliance (programmatic — does output match schema?)
//! - Latency (programmatic — within acceptable thresholds?)
//! - Relevance, Faithfulness, Completeness (LLM-as-judge)

use serde::{Deserialize, Serialize};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Evaluation type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalType {
    /// Does the output match the expected format/schema?
    FormatCompliance,
    /// Was the response generated within acceptable latency?
    Latency,
    /// Is the response relevant to the input question?
    Relevance,
    /// Is the response grounded in provided data (no hallucination)?
    Faithfulness,
    /// Does the response cover all required points?
    Completeness,
}

/// Result of a single evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub eval_type: EvalType,
    /// Score from 0.0 (worst) to 1.0 (best).
    pub score: f64,
    pub details: Option<String>,
    pub judge_model: Option<String>,
}

/// Evaluation configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalConfig {
    pub eval_types: Vec<EvalType>,
    /// Maximum acceptable latency in ms (for latency eval).
    #[serde(default = "default_max_latency")]
    pub max_latency_ms: u64,
    /// Expected JSON schema (for format_compliance eval).
    pub expected_schema: Option<Value>,
}

fn default_max_latency() -> u64 {
    5000
}

// ---------------------------------------------------------------------------
// Programmatic evaluators (no LLM needed)
// ---------------------------------------------------------------------------

/// Evaluate format compliance: does the response parse as valid JSON matching
/// the expected schema?
pub fn eval_format_compliance(response: &str, schema: Option<&Value>) -> EvalResult {
    // Try to parse as JSON.
    let parsed: Result<Value, _> = serde_json::from_str(response);

    match parsed {
        Err(_) => EvalResult {
            eval_type: EvalType::FormatCompliance,
            score: 0.0,
            details: Some("Response is not valid JSON".into()),
            judge_model: None,
        },
        Ok(value) => {
            if let Some(schema) = schema {
                // Check required fields.
                let mut score = 1.0;
                let mut issues = Vec::new();

                if let (Some(_props), Some(obj)) = (
                    schema.get("properties").and_then(|p| p.as_object()),
                    value.as_object(),
                ) {
                    let required: Vec<&str> = schema
                        .get("required")
                        .and_then(|r| r.as_array())
                        .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect())
                        .unwrap_or_default();

                    for field in &required {
                        if !obj.contains_key(*field) {
                            score -= 1.0 / required.len() as f64;
                            issues.push(format!("Missing required field: {field}"));
                        }
                    }
                }

                EvalResult {
                    eval_type: EvalType::FormatCompliance,
                    score: score.max(0.0),
                    details: if issues.is_empty() {
                        Some("All fields present and valid".into())
                    } else {
                        Some(issues.join("; "))
                    },
                    judge_model: None,
                }
            } else {
                // No schema — just being valid JSON is enough.
                EvalResult {
                    eval_type: EvalType::FormatCompliance,
                    score: 1.0,
                    details: Some("Valid JSON".into()),
                    judge_model: None,
                }
            }
        }
    }
}

/// Evaluate latency: was the response within acceptable time?
pub fn eval_latency(duration_ms: u64, max_ms: u64) -> EvalResult {
    let score = if duration_ms <= max_ms {
        1.0 - (duration_ms as f64 / max_ms as f64 * 0.5) // Linear decay, but stays above 0.5 if within limit
    } else {
        (max_ms as f64 / duration_ms as f64).max(0.0) // Below threshold
    };

    EvalResult {
        eval_type: EvalType::Latency,
        score: score.clamp(0.0, 1.0),
        details: Some(format!("{}ms / {}ms max", duration_ms, max_ms)),
        judge_model: None,
    }
}

/// Build an LLM prompt for relevance/faithfulness/completeness evaluation.
/// The caller invokes the LLM and parses the score.
pub fn build_judge_prompt(
    eval_type: &EvalType,
    input: &str,
    output: &str,
    context: Option<&str>,
) -> String {
    let criteria = match eval_type {
        EvalType::Relevance => "Is the response relevant to the input question? Score 0-10.",
        EvalType::Faithfulness => {
            "Is the response grounded in the provided context? Does it hallucinate? Score 0-10."
        }
        EvalType::Completeness => {
            "Does the response cover all key points from the input? Score 0-10."
        }
        _ => return String::new(),
    };

    let ctx_section = context
        .map(|c| format!("\n\n## Context Provided\n{c}"))
        .unwrap_or_default();

    format!(
        "You are an evaluation judge. Rate the following response.\n\n\
         ## Criteria\n{criteria}\n\n\
         ## Input\n{input}{ctx_section}\n\n\
         ## Response\n{output}\n\n\
         Respond with ONLY a JSON object: {{\"score\": <0-10>, \"reason\": \"<brief explanation>\"}}"
    )
}

/// Parse a judge response (expects `{"score": N, "reason": "..."}`)
pub fn parse_judge_response(response: &str) -> Option<EvalResult> {
    // Try direct parse first, then extract from markdown code blocks.
    let json_str = serde_json::from_str::<Value>(response)
        .ok()
        .map(|_| response.to_string())
        .or_else(|| {
            // Extract from ```json ... ``` blocks
            let re = regex::Regex::new(r"```(?:json)?\s*\n?([\s\S]*?)\n?\s*```").ok()?;
            re.captures(response).map(|c| c[1].trim().to_string())
        })
        .or_else(|| {
            // Find first { ... } block
            let start = response.find('{')?;
            let mut depth = 0i32;
            for (i, c) in response[start..].char_indices() {
                if c == '{' {
                    depth += 1;
                }
                if c == '}' {
                    depth -= 1;
                }
                if depth == 0 {
                    return Some(response[start..start + i + 1].to_string());
                }
            }
            None
        })?;
    let parsed: Value = serde_json::from_str(&json_str).ok()?;
    let raw_score = parsed.get("score")?.as_f64()?;
    let reason = parsed
        .get("reason")
        .and_then(|r| r.as_str())
        .map(String::from);

    Some(EvalResult {
        eval_type: EvalType::Relevance, // Caller sets the correct type
        score: (raw_score / 10.0).clamp(0.0, 1.0),
        details: reason,
        judge_model: None,
    })
}

// ---------------------------------------------------------------------------
// Integrated eval — calls REAL LLM for judge evals
// ---------------------------------------------------------------------------

/// The text under evaluation: the agent's input, its output, and optional context.
#[derive(Clone, Copy)]
pub struct EvalInput<'a> {
    pub input: &'a str,
    pub output: &'a str,
    pub context: Option<&'a str>,
}

/// Execute a full eval run against a completed session.
///
/// - Programmatic types (format_compliance, latency) run without LLM.
/// - LLM-as-judge types (relevance, faithfulness, completeness) call the real LLM.
pub async fn execute_eval(
    eval_types: &[EvalType],
    io: EvalInput<'_>,
    duration_ms: u64,
    output_schema: Option<&Value>,
    llm: &dyn crate::core::context::LLMResource,
    judge_model: &str,
) -> Vec<EvalResult> {
    let mut results = Vec::new();

    for eval_type in eval_types {
        match eval_type {
            EvalType::FormatCompliance => {
                results.push(eval_format_compliance(io.output, output_schema));
            }
            EvalType::Latency => {
                results.push(eval_latency(duration_ms, 5000));
            }
            EvalType::Relevance | EvalType::Faithfulness | EvalType::Completeness => {
                let prompt = build_judge_prompt(eval_type, io.input, io.output, io.context);
                if prompt.is_empty() {
                    continue;
                }

                // Call REAL LLM for evaluation.
                match llm.call(judge_model, &prompt, &[], 0.1, 256).await {
                    Ok(response) => {
                        if let Some(mut eval_result) = parse_judge_response(&response.response) {
                            eval_result.eval_type = eval_type.clone();
                            eval_result.judge_model = Some(response.model);
                            results.push(eval_result);
                        } else {
                            results.push(EvalResult {
                                eval_type: eval_type.clone(),
                                score: 0.5,
                                details: Some(format!(
                                    "LLM responded but could not parse score: {}",
                                    &response.response[..100.min(response.response.len())]
                                )),
                                judge_model: Some(response.model),
                            });
                        }
                    }
                    Err(e) => {
                        results.push(EvalResult {
                            eval_type: eval_type.clone(),
                            score: 0.0,
                            details: Some(format!("LLM judge error: {e}")),
                            judge_model: None,
                        });
                    }
                }
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_compliance_valid_json() {
        let result = eval_format_compliance(r#"{"answer": "42"}"#, None);
        assert_eq!(result.score, 1.0);
    }

    #[test]
    fn format_compliance_invalid_json() {
        let result = eval_format_compliance("not json", None);
        assert_eq!(result.score, 0.0);
    }

    #[test]
    fn format_compliance_with_schema() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["name", "score"],
            "properties": {
                "name": {"type": "string"},
                "score": {"type": "number"}
            }
        });
        // Valid
        let result = eval_format_compliance(r#"{"name": "test", "score": 42}"#, Some(&schema));
        assert_eq!(result.score, 1.0);

        // Missing one required field
        let result = eval_format_compliance(r#"{"name": "test"}"#, Some(&schema));
        assert!(result.score < 1.0);
        assert!(result.score > 0.0);
    }

    #[test]
    fn latency_within_threshold() {
        let result = eval_latency(1000, 5000);
        assert!(result.score > 0.5);
    }

    #[test]
    fn latency_exceeds_threshold() {
        let result = eval_latency(10000, 5000);
        assert!(result.score < 1.0);
        assert!(result.score > 0.0);
    }

    #[test]
    fn latency_zero() {
        let result = eval_latency(0, 5000);
        assert_eq!(result.score, 1.0);
    }

    #[test]
    fn judge_prompt_generation() {
        let prompt = build_judge_prompt(
            &EvalType::Relevance,
            "What is the capital of France?",
            "Paris is the capital of France.",
            None,
        );
        assert!(prompt.contains("evaluation judge"));
        assert!(prompt.contains("relevant"));
        assert!(prompt.contains("capital of France"));
    }

    #[test]
    fn parse_judge_response_valid() {
        let response = r#"{"score": 8, "reason": "Mostly relevant"}"#;
        let result = parse_judge_response(response).unwrap();
        assert!((result.score - 0.8).abs() < 0.01);
    }

    #[test]
    fn parse_judge_response_invalid() {
        assert!(parse_judge_response("not json").is_none());
        assert!(parse_judge_response("{}").is_none());
    }
}
