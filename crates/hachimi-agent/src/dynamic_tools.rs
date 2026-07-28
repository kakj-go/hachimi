use std::collections::BTreeSet;

use hachimi_protocol::{
    CapabilityDegradation, DynamicToolRegistration, DynamicToolValidationError,
    ProviderCapabilities,
};

const RESERVED_NAMESPACES: &[&str] = &["hachimi", "system", "openai"];

#[derive(Debug, Clone, PartialEq)]
pub struct DynamicToolValidation {
    pub accepted: Vec<DynamicToolRegistration>,
    pub rejected: Vec<(DynamicToolRegistration, DynamicToolValidationError)>,
}

#[must_use]
pub fn validate_dynamic_tools(
    registrations: impl IntoIterator<Item = DynamicToolRegistration>,
    provider: ProviderCapabilities,
) -> DynamicToolValidation {
    let mut accepted = Vec::new();
    let mut rejected = Vec::new();
    let mut names = BTreeSet::new();
    for registration in registrations {
        match validate_registration(&registration, provider, &mut names) {
            Ok(()) => accepted.push(registration),
            Err(error) => rejected.push((registration, error)),
        }
    }
    DynamicToolValidation { accepted, rejected }
}

fn validate_registration(
    registration: &DynamicToolRegistration,
    provider: ProviderCapabilities,
    names: &mut BTreeSet<String>,
) -> Result<(), DynamicToolValidationError> {
    validate_identifier(&registration.namespace, "namespace", 64, true)?;
    validate_identifier(&registration.name, "name", 128, false)?;
    if RESERVED_NAMESPACES.contains(&registration.namespace.as_str()) {
        return Err(error(
            "reserved_namespace",
            "namespace",
            "the namespace is reserved by Hachimi",
        ));
    }
    if !registration.namespace.is_empty() && !provider.namespaced_tools {
        return Err(error(
            "namespace_unsupported",
            "namespace",
            "the selected provider does not support namespaced tools",
        ));
    }
    if registration.deferred && (registration.namespace.is_empty() || !provider.deferred_tools) {
        return Err(error(
            "deferred_tool_unsupported",
            "deferred",
            "deferred tools require a namespace and provider support",
        ));
    }
    if registration.requires_strict_schema && !provider.strict_json_schema {
        return Err(error(
            "strict_schema_unsupported",
            "inputSchema",
            "the selected provider does not support strict JSON Schema",
        ));
    }
    if !registration.input_schema.is_object()
        || registration
            .input_schema
            .get("type")
            .and_then(serde_json::Value::as_str)
            != Some("object")
    {
        return Err(error(
            "invalid_input_schema",
            "inputSchema",
            "tool input schema must be a JSON object schema",
        ));
    }
    for media in &registration.output_media {
        let supported = match media.as_str() {
            "text" => true,
            "image" => provider.image_input,
            "audio" => provider.audio_input,
            _ => false,
        };
        if !supported {
            return Err(error(
                "output_media_unsupported",
                "outputMedia",
                "tool output media is not supported by the selected provider",
            ));
        }
    }
    let qualified = if registration.namespace.is_empty() {
        registration.name.clone()
    } else {
        format!("{}.{}", registration.namespace, registration.name)
    };
    if !names.insert(qualified) {
        return Err(error(
            "duplicate_tool_name",
            "name",
            "a dynamic tool with the same qualified name is already registered",
        ));
    }
    Ok(())
}

fn validate_identifier(
    value: &str,
    field: &str,
    max_bytes: usize,
    allow_empty: bool,
) -> Result<(), DynamicToolValidationError> {
    if (!allow_empty && value.is_empty())
        || value.len() > max_bytes
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(error(
            "invalid_identifier",
            field,
            "identifier must be bounded ASCII letters, digits, underscore, or hyphen",
        ));
    }
    Ok(())
}

fn error(code: &str, field: &str, message: &str) -> DynamicToolValidationError {
    DynamicToolValidationError {
        code: code.into(),
        field: field.into(),
        message: message.into(),
    }
}

#[must_use]
pub fn negotiate_provider_capabilities(
    requested: ProviderCapabilities,
    actual: ProviderCapabilities,
) -> (ProviderCapabilities, Vec<CapabilityDegradation>) {
    let negotiated = ProviderCapabilities {
        tool_calls: requested.tool_calls && actual.tool_calls,
        parallel_tool_calls: requested.parallel_tool_calls && actual.parallel_tool_calls,
        namespaced_tools: requested.namespaced_tools && actual.namespaced_tools,
        deferred_tools: requested.deferred_tools && actual.deferred_tools,
        strict_json_schema: requested.strict_json_schema && actual.strict_json_schema,
        output_schema: requested.output_schema && actual.output_schema,
        text_input: requested.text_input && actual.text_input,
        image_input: requested.image_input && actual.image_input,
        audio_input: requested.audio_input && actual.audio_input,
        streaming_usage: requested.streaming_usage && actual.streaming_usage,
        reasoning_summary: requested.reasoning_summary && actual.reasoning_summary,
        realtime: requested.realtime && actual.realtime,
        http_transport: requested.http_transport && actual.http_transport,
        websocket_transport: requested.websocket_transport && actual.websocket_transport,
        remote_compaction: requested.remote_compaction && actual.remote_compaction,
        context_window: min_option(requested.context_window, actual.context_window),
        max_output_tokens: min_option(requested.max_output_tokens, actual.max_output_tokens),
    };
    let mut degradations = Vec::new();
    for (name, wanted, enabled) in [
        ("tool_calls", requested.tool_calls, negotiated.tool_calls),
        (
            "parallel_tool_calls",
            requested.parallel_tool_calls,
            negotiated.parallel_tool_calls,
        ),
        (
            "namespaced_tools",
            requested.namespaced_tools,
            negotiated.namespaced_tools,
        ),
        (
            "deferred_tools",
            requested.deferred_tools,
            negotiated.deferred_tools,
        ),
        (
            "strict_json_schema",
            requested.strict_json_schema,
            negotiated.strict_json_schema,
        ),
        (
            "output_schema",
            requested.output_schema,
            negotiated.output_schema,
        ),
        ("text_input", requested.text_input, negotiated.text_input),
        ("image_input", requested.image_input, negotiated.image_input),
        ("audio_input", requested.audio_input, negotiated.audio_input),
        (
            "streaming_usage",
            requested.streaming_usage,
            negotiated.streaming_usage,
        ),
        (
            "reasoning_summary",
            requested.reasoning_summary,
            negotiated.reasoning_summary,
        ),
        ("realtime", requested.realtime, negotiated.realtime),
        (
            "remote_compaction",
            requested.remote_compaction,
            negotiated.remote_compaction,
        ),
    ] {
        if wanted && !enabled {
            degradations.push(CapabilityDegradation {
                capability: name.into(),
                code: "provider_capability_unavailable".into(),
                message: format!("provider did not negotiate requested capability {name}"),
            });
        }
    }
    (negotiated, degradations)
}

fn min_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn registration(namespace: &str, name: &str) -> DynamicToolRegistration {
        DynamicToolRegistration {
            namespace: namespace.into(),
            name: name.into(),
            description: "test".into(),
            input_schema: json!({ "type": "object", "properties": {} }),
            deferred: false,
            requires_strict_schema: false,
            output_media: vec!["text".into()],
        }
    }

    #[test]
    fn rejects_namespace_features_the_provider_did_not_negotiate() {
        let result = validate_dynamic_tools(
            [registration("mcp", "read"), registration("", "plain")],
            ProviderCapabilities {
                tool_calls: true,
                text_input: true,
                ..ProviderCapabilities::default()
            },
        );
        assert_eq!(result.accepted.len(), 1);
        assert_eq!(result.rejected[0].1.code, "namespace_unsupported");
    }

    #[test]
    fn capability_negotiation_records_structured_degradation() {
        let requested = ProviderCapabilities {
            tool_calls: true,
            parallel_tool_calls: true,
            text_input: true,
            ..ProviderCapabilities::default()
        };
        let actual = ProviderCapabilities {
            text_input: true,
            ..ProviderCapabilities::default()
        };
        let (negotiated, degradation) = negotiate_provider_capabilities(requested, actual);
        assert!(!negotiated.tool_calls);
        assert_eq!(degradation.len(), 2);
    }

    #[test]
    fn rejects_invalid_duplicate_strict_and_unsupported_media_tools() {
        let mut duplicate = registration("mcp", "read");
        duplicate.description = "duplicate".into();
        let mut strict = registration("mcp", "strict");
        strict.requires_strict_schema = true;
        let mut image = registration("mcp", "image");
        image.output_media = vec!["image".into()];
        let result = validate_dynamic_tools(
            [
                registration("mcp", "read"),
                duplicate,
                registration("system", "reserved"),
                registration("mcp", "bad/name"),
                strict,
                image,
            ],
            ProviderCapabilities {
                tool_calls: true,
                namespaced_tools: true,
                text_input: true,
                ..ProviderCapabilities::default()
            },
        );
        assert_eq!(result.accepted.len(), 1);
        let codes = result
            .rejected
            .iter()
            .map(|(_, error)| error.code.as_str())
            .collect::<Vec<_>>();
        assert!(codes.contains(&"duplicate_tool_name"));
        assert!(codes.contains(&"reserved_namespace"));
        assert!(codes.contains(&"invalid_identifier"));
        assert!(codes.contains(&"strict_schema_unsupported"));
        assert!(codes.contains(&"output_media_unsupported"));
    }
}
