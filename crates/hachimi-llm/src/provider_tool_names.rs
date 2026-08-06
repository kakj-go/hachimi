use std::collections::{BTreeMap, BTreeSet};

use hachimi_protocol::{ModelEvent, ModelMessage, ModelRequest};

const MAX_PROVIDER_TOOL_NAME_BYTES: usize = 64;

#[derive(Debug, Clone, Default)]
pub(super) struct ProviderToolNames {
    internal_to_provider: BTreeMap<String, String>,
    provider_to_internal: BTreeMap<String, String>,
}

impl ProviderToolNames {
    pub(super) fn for_request(request: &ModelRequest) -> Self {
        let mut names = request
            .tools
            .iter()
            .map(|tool| tool.name.clone())
            .collect::<BTreeSet<_>>();
        collect_message_names(&request.messages, &mut names);
        Self::from_names(names)
    }

    pub(super) fn for_messages(messages: &[ModelMessage]) -> Self {
        let mut names = BTreeSet::new();
        collect_message_names(messages, &mut names);
        Self::from_names(names)
    }

    fn from_names(names: BTreeSet<String>) -> Self {
        let mut internal_to_provider = BTreeMap::new();
        let mut provider_to_internal = BTreeMap::new();
        let mut used = BTreeSet::new();

        // Keep already-valid public names stable. Unsafe names are assigned only
        // after these reservations so sanitizing cannot shadow a real tool.
        for name in names.iter().filter(|name| is_provider_safe(name)) {
            internal_to_provider.insert(name.clone(), name.clone());
            provider_to_internal.insert(name.clone(), name.clone());
            used.insert(name.clone());
        }

        for name in names.iter().filter(|name| !is_provider_safe(name)) {
            let base = sanitized_base(name);
            let alias = unique_alias(&base, &used);
            used.insert(alias.clone());
            internal_to_provider.insert(name.clone(), alias.clone());
            provider_to_internal.insert(alias, name.clone());
        }

        Self {
            internal_to_provider,
            provider_to_internal,
        }
    }

    pub(super) fn encode<'a>(&'a self, name: &'a str) -> &'a str {
        self.internal_to_provider
            .get(name)
            .map_or(name, String::as_str)
    }

    fn decode<'a>(&'a self, name: &'a str) -> &'a str {
        self.provider_to_internal
            .get(name)
            .map_or(name, String::as_str)
    }

    fn is_complete_unambiguous_alias(&self, candidate: &str) -> bool {
        self.provider_to_internal.contains_key(candidate)
            && !self
                .provider_to_internal
                .keys()
                .any(|name| name != candidate && name.starts_with(candidate))
    }
}

#[derive(Debug, Clone)]
pub(super) struct ProviderToolEventDecoder {
    names: ProviderToolNames,
    pending_names: BTreeMap<u32, String>,
}

impl ProviderToolEventDecoder {
    pub(super) fn new(names: ProviderToolNames) -> Self {
        Self {
            names,
            pending_names: BTreeMap::new(),
        }
    }

    pub(super) fn decode_event(&mut self, event: ModelEvent) -> Vec<ModelEvent> {
        match event {
            ModelEvent::ToolCallDelta {
                index,
                id,
                name_delta,
                arguments_delta,
            } => {
                if !name_delta.is_empty() {
                    self.pending_names
                        .entry(index)
                        .or_default()
                        .push_str(&name_delta);
                }
                let should_flush = self.pending_names.get(&index).is_some_and(|name| {
                    !name.is_empty()
                        && (!arguments_delta.is_empty()
                            || self.names.is_complete_unambiguous_alias(name))
                });
                let decoded_name = if should_flush {
                    let encoded = self.pending_names.remove(&index).unwrap_or_default();
                    self.names.decode(&encoded).to_owned()
                } else {
                    String::new()
                };
                vec![ModelEvent::ToolCallDelta {
                    index,
                    id,
                    name_delta: decoded_name,
                    arguments_delta,
                }]
            }
            ModelEvent::ToolCallCompleted { mut call } => {
                call.name = self.names.decode(&call.name).to_owned();
                vec![ModelEvent::ToolCallCompleted { call }]
            }
            completed @ ModelEvent::Completed { .. } => {
                let mut events = std::mem::take(&mut self.pending_names)
                    .into_iter()
                    .map(|(index, encoded)| ModelEvent::ToolCallDelta {
                        index,
                        id: None,
                        name_delta: self.names.decode(&encoded).to_owned(),
                        arguments_delta: String::new(),
                    })
                    .collect::<Vec<_>>();
                events.push(completed);
                events
            }
            other => vec![other],
        }
    }
}

fn collect_message_names(messages: &[ModelMessage], names: &mut BTreeSet<String>) {
    for message in messages {
        if let Some(name) = &message.name {
            names.insert(name.clone());
        }
        names.extend(message.tool_calls.iter().map(|call| call.name.clone()));
    }
}

fn is_provider_safe(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_PROVIDER_TOOL_NAME_BYTES
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn sanitized_base(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len().min(MAX_PROVIDER_TOOL_NAME_BYTES));
    let mut last_was_separator = false;
    for character in name.chars() {
        let safe = character.is_ascii_alphanumeric() || matches!(character, '_' | '-');
        if safe {
            sanitized.push(character);
            last_was_separator = false;
        } else if !last_was_separator {
            sanitized.push('_');
            last_was_separator = true;
        }
        if sanitized.len() == MAX_PROVIDER_TOOL_NAME_BYTES {
            break;
        }
    }
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "tool".into()
    } else {
        sanitized.into()
    }
}

fn unique_alias(base: &str, used: &BTreeSet<String>) -> String {
    let candidate = base
        .get(..base.len().min(MAX_PROVIDER_TOOL_NAME_BYTES))
        .unwrap_or(base);
    if !used.contains(candidate) {
        return candidate.into();
    }
    for ordinal in 2_u64.. {
        let suffix = format!("__{ordinal}");
        let head_bytes = MAX_PROVIDER_TOOL_NAME_BYTES.saturating_sub(suffix.len());
        let head = base.get(..base.len().min(head_bytes)).unwrap_or(base);
        let candidate = format!("{head}{suffix}");
        if !used.contains(&candidate) {
            return candidate;
        }
    }
    unreachable!("u64 alias space exhausted")
}

#[cfg(test)]
mod tests {
    use hachimi_protocol::{
        ModelEvent, ModelMessage, ModelRequest, ModelToolCall, ToolCallId, ToolDescriptor,
        ToolEffect,
    };
    use serde_json::json;

    use super::*;

    fn descriptor(name: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: name.into(),
            description: String::new(),
            input_schema: json!({ "type": "object", "properties": {} }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: true,
            required_scopes: Vec::new(),
        }
    }

    #[test]
    fn aliases_unsafe_names_without_shadowing_valid_tools() {
        let long_name = "x".repeat(80);
        let request = ModelRequest {
            messages: vec![ModelMessage::assistant(
                "",
                vec![ModelToolCall {
                    id: ToolCallId::from("old"),
                    name: "历史/工具".into(),
                    arguments: json!({}),
                }],
            )],
            tools: vec![
                descriptor("agent.spawn"),
                descriptor("agent_spawn"),
                descriptor("agent/spawn"),
                descriptor(&long_name),
            ],
            parallel_tool_calls: true,
            max_output_tokens: None,
        };
        let names = ProviderToolNames::for_request(&request);
        let aliases = [
            "agent.spawn",
            "agent_spawn",
            "agent/spawn",
            &long_name,
            "历史/工具",
        ]
        .map(|name| names.encode(name).to_owned());

        assert_eq!(aliases[1], "agent_spawn");
        assert_eq!(aliases.iter().collect::<BTreeSet<_>>().len(), aliases.len());
        assert!(aliases.iter().all(|name| is_provider_safe(name)));
        assert_eq!(names.decode(&aliases[0]), "agent.spawn");
        assert_eq!(names.decode(&aliases[2]), "agent/spawn");
    }

    #[test]
    fn decodes_split_stream_names_before_dispatch() {
        let request = ModelRequest {
            messages: Vec::new(),
            tools: vec![descriptor("agent.spawn"), descriptor("agent_spawn")],
            parallel_tool_calls: true,
            max_output_tokens: None,
        };
        let names = ProviderToolNames::for_request(&request);
        let alias = names.encode("agent.spawn").to_owned();
        let split = alias.len() / 2;
        let mut decoder = ProviderToolEventDecoder::new(names);

        let first = decoder.decode_event(ModelEvent::ToolCallDelta {
            index: 0,
            id: Some(ToolCallId::from("call")),
            name_delta: alias[..split].into(),
            arguments_delta: String::new(),
        });
        let second = decoder.decode_event(ModelEvent::ToolCallDelta {
            index: 0,
            id: None,
            name_delta: alias[split..].into(),
            arguments_delta: "{}".into(),
        });

        assert!(
            matches!(&first[0], ModelEvent::ToolCallDelta { name_delta, .. } if name_delta.is_empty())
        );
        assert!(
            matches!(&second[0], ModelEvent::ToolCallDelta { name_delta, arguments_delta, .. } if name_delta == "agent.spawn" && arguments_delta == "{}")
        );
    }
}
