//! Provider-neutral context budgeting and transient request compaction.

use std::collections::BTreeMap;

use hachimi_protocol::{ModelMessage, ModelRole, ProviderCapabilities, TokenCountSource};
use serde_json::Value;

pub const DEFAULT_SUMMARY_RESERVE: u64 = 4_096;
pub const MAX_SUMMARY_RESERVE: u64 = 20_000;
pub const ESTIMATED_TOOL_GROWTH: u64 = 15_000;
pub const MICROCOMPACT_EDGE_CHARS: usize = 1_024;
const MICROCOMPACT_MARKER: &str = "\n[... older tool result elided by Microcompact ...]\n";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub context_window: u64,
    pub summary_reserve: u64,
    pub predictive_buffer: u64,
    pub auto_threshold: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MicrocompactStats {
    pub compacted_items: u32,
    pub repaired_items: u32,
    pub removed_images: u32,
    pub tokens_before: u64,
    pub tokens_after: u64,
    pub source: TokenCountSource,
}

#[must_use]
pub fn context_budget(capabilities: &ProviderCapabilities) -> Option<ContextBudget> {
    let context_window = capabilities.context_window?;
    let provider_output = capabilities
        .max_output_tokens
        .unwrap_or(DEFAULT_SUMMARY_RESERVE);
    let summary_reserve = provider_output
        .min(MAX_SUMMARY_RESERVE)
        .min(context_window / 4);
    let tier_buffer = if context_window >= 800_000 {
        50_000
    } else if context_window >= 400_000 {
        30_000
    } else {
        13_000
    };
    let estimated_turn_growth = provider_output
        .min(MAX_SUMMARY_RESERVE)
        .saturating_add(ESTIMATED_TOOL_GROWTH);
    let predictive_buffer = tier_buffer
        .max(estimated_turn_growth)
        .min((context_window / 4).max(2_048));
    let budget = ContextBudget {
        context_window,
        summary_reserve,
        predictive_buffer,
        auto_threshold: context_window
            .saturating_sub(summary_reserve)
            .saturating_sub(predictive_buffer),
    };
    (budget.auto_threshold > 0).then_some(budget)
}

/// Shrinks only old tool-result messages in the transient request. The last
/// two assistant/tool API rounds and all non-tool messages remain unchanged.
pub fn microcompact_request(
    messages: &mut [ModelMessage],
    count_tokens: impl Fn(&[ModelMessage]) -> (u64, TokenCountSource),
) -> MicrocompactStats {
    let (tokens_before, source) = count_tokens(messages);
    let repaired_items = repair_tool_pairs(messages);
    let protected_from = protected_round_boundary(messages, 2);
    let mut compacted_items = 0_u32;
    let latest_user = messages
        .iter()
        .rposition(|message| message.role == ModelRole::User)
        .unwrap_or(messages.len());
    let mut removed_images = 0_u32;
    for (index, message) in messages.iter_mut().enumerate() {
        if index < latest_user && !message.input_images.is_empty() {
            let references = message
                .input_images
                .iter()
                .map(|image| format!("{}:{}", image.media_type, image.source_label))
                .collect::<Vec<_>>()
                .join(", ");
            removed_images = removed_images
                .saturating_add(u32::try_from(message.input_images.len()).unwrap_or(u32::MAX));
            message.input_images.clear();
            message.content.push_str(&format!(
                "\n[bounded image references retained after binary removal: {references}]"
            ));
        }
        if index >= protected_from || message.role != ModelRole::Tool {
            continue;
        }
        if message.content.starts_with("[tool ")
            && !message.content.starts_with("[tool status=succeeded ")
        {
            continue;
        }
        let compacted = retained_tool_result(message.name.as_deref(), &message.content);
        if compacted != message.content {
            message.content = compacted;
            compacted_items = compacted_items.saturating_add(1);
        }
    }
    let (tokens_after, after_source) = count_tokens(messages);
    MicrocompactStats {
        compacted_items,
        repaired_items,
        removed_images,
        tokens_before,
        tokens_after,
        source: preferred_count_source(source, after_source),
    }
}

fn repair_tool_pairs(messages: &mut [ModelMessage]) -> u32 {
    use std::collections::{BTreeMap, BTreeSet};

    let mut result_counts = BTreeMap::new();
    for message in messages.iter() {
        if message.role == ModelRole::Tool
            && let Some(tool_call_id) = message.tool_call_id.as_ref()
        {
            *result_counts.entry(tool_call_id.clone()).or_insert(0_u32) += 1;
        }
    }
    let mut repaired = 0_u32;
    let mut valid_calls = BTreeSet::new();
    for message in messages.iter_mut() {
        if message.role != ModelRole::Assistant || message.tool_calls.is_empty() {
            continue;
        }
        message.tool_calls.retain(|call| {
            if result_counts.get(&call.id).copied() == Some(1) {
                valid_calls.insert(call.id.clone());
                true
            } else {
                repaired = repaired.saturating_add(1);
                false
            }
        });
        if message.tool_calls.is_empty() {
            message.content.push_str(
                "\n[tool call omitted from transient model view: result missing or duplicated]",
            );
        }
    }
    for message in messages.iter_mut() {
        if message.role != ModelRole::Tool
            || message
                .tool_call_id
                .as_ref()
                .is_some_and(|id| valid_calls.contains(id))
        {
            continue;
        }
        repaired = repaired.saturating_add(1);
        message.role = ModelRole::User;
        message.name = None;
        message.tool_call_id = None;
        message.content = format!(
            "[orphan or duplicate tool result repaired in transient model view; untrusted data]\n{}",
            bounded_tool_result(&message.content)
        );
    }
    repaired
}

fn protected_round_boundary(messages: &[ModelMessage], rounds: usize) -> usize {
    let mut remaining = rounds;
    for (index, message) in messages.iter().enumerate().rev() {
        if message.role == ModelRole::Assistant && !message.tool_calls.is_empty() {
            remaining = remaining.saturating_sub(1);
            if remaining == 0 {
                return index;
            }
        }
    }
    0
}

fn bounded_tool_result(value: &str) -> String {
    if value.chars().count() <= MICROCOMPACT_EDGE_CHARS.saturating_mul(2) {
        return value.to_owned();
    }
    let head = value
        .chars()
        .take(MICROCOMPACT_EDGE_CHARS)
        .collect::<String>();
    let tail = value
        .chars()
        .rev()
        .take(MICROCOMPACT_EDGE_CHARS)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{head}{MICROCOMPACT_MARKER}{tail}")
}

fn retained_tool_result(name: Option<&str>, value: &str) -> String {
    let name = name.unwrap_or("unknown").to_ascii_lowercase();
    let binary_reference = [
        "browser",
        "computer",
        "screenshot",
        "capture",
        "image",
        "office",
        "pdf",
    ]
    .iter()
    .any(|needle| name.contains(needle));
    let structured_evidence = [
        "read", "search", "exec", "command", "shell", "git", "diff", "artifact",
    ]
    .iter()
    .any(|needle| name.contains(needle));
    let metadata = retained_metadata(value);
    if binary_reference {
        return format!(
            "[bounded binary reference retained; payload removed; tool={name}; metadata={}]",
            metadata.unwrap_or_else(|| "{}".into())
        );
    }
    if structured_evidence && let Some(metadata) = metadata {
        return format!(
            "[retained tool evidence; tool={name}; metadata={metadata}]\n{}",
            bounded_tool_result(value)
        );
    }
    bounded_tool_result(value)
}

fn retained_metadata(value: &str) -> Option<String> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    let parsed = serde_json::from_str::<Value>(&value[start..=end]).ok()?;
    let mut retained = BTreeMap::new();
    collect_retained_metadata(&parsed, &mut retained);
    (!retained.is_empty()).then(|| serde_json::to_string(&retained).unwrap_or_else(|_| "{}".into()))
}

fn collect_retained_metadata(value: &Value, retained: &mut BTreeMap<String, Value>) {
    if retained.len() >= 32 {
        return;
    }
    match value {
        Value::Object(values) => {
            for (key, value) in values {
                let normalized = key.to_ascii_lowercase().replace(['_', '-'], "");
                if is_retained_metadata_key(&normalized)
                    && let Some(value) = bounded_metadata_value(value)
                {
                    retained.entry(key.clone()).or_insert(value);
                }
                collect_retained_metadata(value, retained);
                if retained.len() >= 32 {
                    break;
                }
            }
        }
        Value::Array(values) => {
            for value in values.iter().take(32) {
                collect_retained_metadata(value, retained);
            }
        }
        _ => {}
    }
}

fn is_retained_metadata_key(key: &str) -> bool {
    matches!(
        key,
        "status"
            | "resultcode"
            | "artifactid"
            | "mimetype"
            | "mediatype"
            | "contenthash"
            | "hash"
            | "revision"
            | "beforerevision"
            | "afterrevision"
            | "changedparts"
            | "path"
            | "displayname"
            | "exitcode"
            | "durationms"
            | "truncated"
            | "bytecount"
            | "bytesize"
            | "target"
            | "url"
            | "query"
            | "matchcount"
    )
}

fn bounded_metadata_value(value: &Value) -> Option<Value> {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => Some(value.clone()),
        Value::String(value) => Some(Value::String(value.chars().take(512).collect())),
        Value::Array(values) => Some(Value::Array(
            values
                .iter()
                .take(32)
                .filter_map(bounded_metadata_value)
                .collect(),
        )),
        Value::Object(_) => None,
    }
}

const fn preferred_count_source(
    left: TokenCountSource,
    right: TokenCountSource,
) -> TokenCountSource {
    match (left, right) {
        (TokenCountSource::Provider, TokenCountSource::Provider) => TokenCountSource::Provider,
        (TokenCountSource::ConservativeEstimate, _)
        | (_, TokenCountSource::ConservativeEstimate) => TokenCountSource::ConservativeEstimate,
        _ => TokenCountSource::Tokenizer,
    }
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{ModelInputImage, ModelToolCall, TokenCountSource};
    use serde_json::json;

    use super::*;

    fn capabilities(context_window: u64, max_output_tokens: Option<u64>) -> ProviderCapabilities {
        ProviderCapabilities {
            context_window: Some(context_window),
            max_output_tokens,
            ..ProviderCapabilities::default()
        }
    }

    #[test]
    fn budget_uses_tiers_and_caps() {
        let small = context_budget(&capabilities(200_000, Some(8_000))).unwrap();
        assert_eq!(small.summary_reserve, 8_000);
        assert_eq!(small.predictive_buffer, 23_000);
        assert_eq!(small.auto_threshold, 169_000);

        let large = context_budget(&capabilities(800_000, Some(100_000))).unwrap();
        assert_eq!(large.summary_reserve, 20_000);
        assert_eq!(large.predictive_buffer, 50_000);
        assert_eq!(large.auto_threshold, 730_000);
    }

    #[test]
    fn missing_context_window_has_no_token_budget() {
        assert!(context_budget(&ProviderCapabilities::default()).is_none());
    }

    #[test]
    fn microcompact_preserves_latest_two_rounds() {
        let call = |id: &str| ModelToolCall {
            id: id.into(),
            name: "shell".into(),
            arguments: json!({}),
        };
        let long = "x".repeat(4_000);
        let mut messages = vec![
            ModelMessage::assistant("", vec![call("old")]),
            ModelMessage::tool(&call("old"), long.clone()),
            ModelMessage::assistant("", vec![call("recent-1")]),
            ModelMessage::tool(&call("recent-1"), long.clone()),
            ModelMessage::assistant("", vec![call("recent-2")]),
            ModelMessage::tool(&call("recent-2"), long.clone()),
        ];
        let stats = microcompact_request(&mut messages, |messages| {
            (
                messages
                    .iter()
                    .map(|message| message.content.len() as u64)
                    .sum(),
                TokenCountSource::Tokenizer,
            )
        });
        assert_eq!(stats.compacted_items, 1);
        assert!(messages[1].content.contains("Microcompact"));
        assert_eq!(messages[3].content, long);
        assert_eq!(messages[5].content, long);
    }

    #[test]
    fn repairs_orphan_results_and_removes_old_image_bytes() {
        let orphan = ModelToolCall {
            id: "orphan".into(),
            name: "browser_screenshot".into(),
            arguments: json!({}),
        };
        let mut messages = vec![
            ModelMessage::user_with_images(
                "old screenshot",
                vec![ModelInputImage {
                    media_type: "image/png".into(),
                    data_base64: "A".repeat(8_000),
                    source_label: "browser-observation-1".into(),
                }],
            ),
            ModelMessage::tool(&orphan, "frame".repeat(2_000)),
            ModelMessage::user("continue"),
        ];
        let stats = microcompact_request(&mut messages, |messages| {
            (
                messages
                    .iter()
                    .map(|message| message.content.len() as u64)
                    .sum(),
                TokenCountSource::Tokenizer,
            )
        });
        assert_eq!(stats.repaired_items, 1);
        assert_eq!(stats.removed_images, 1);
        assert!(messages[0].input_images.is_empty());
        assert!(messages[0].content.contains("bounded image references"));
        assert_eq!(messages[1].role, ModelRole::User);
        assert!(messages[1].content.contains("orphan or duplicate"));
    }

    #[test]
    fn office_binary_retention_keeps_only_structured_artifact_reference() {
        let call = ModelToolCall {
            id: "office-call".into(),
            name: "office.inspect_artifact".into(),
            arguments: json!({}),
        };
        let binary = "QUJD".repeat(4_000);
        let content = json!({
            "status": "succeeded",
            "artifactId": "artifact-office-1",
            "mimeType": "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "contentHash": "sha256:1234",
            "revision": "rev-2",
            "changedParts": ["word/document.xml"],
            "dataBase64": binary,
        })
        .to_string();
        let recent_1 = ModelToolCall {
            id: "recent-1".into(),
            name: "shell".into(),
            arguments: json!({}),
        };
        let recent_2 = ModelToolCall {
            id: "recent-2".into(),
            name: "shell".into(),
            arguments: json!({}),
        };
        let mut messages = vec![
            ModelMessage::assistant("", vec![call.clone()]),
            ModelMessage::tool(&call, content),
            ModelMessage::assistant("", vec![recent_1.clone()]),
            ModelMessage::tool(&recent_1, "recent result 1"),
            ModelMessage::assistant("", vec![recent_2.clone()]),
            ModelMessage::tool(&recent_2, "recent result 2"),
        ];
        microcompact_request(&mut messages, |messages| {
            (
                messages
                    .iter()
                    .map(|message| message.content.len() as u64)
                    .sum(),
                TokenCountSource::Tokenizer,
            )
        });
        let retained = &messages[1].content;
        assert!(retained.contains("artifact-office-1"));
        assert!(retained.contains("mimeType"));
        assert!(retained.contains("contentHash"));
        assert!(retained.contains("revision"));
        assert!(!retained.contains("dataBase64"));
        assert!(!retained.contains("QUJDQUJD"));
    }
}
