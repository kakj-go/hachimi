// SPDX-License-Identifier: Apache-2.0
// Adapted from OpenAI Codex commit 4c43465133428898aa84f0bfc02c306ed65fb66a:
// codex-rs/codex-mcp/src/connection_manager/resources.rs and
// codex-rs/app-server-protocol/src/protocol/v2/mcp.rs.
// Hachimi uses bounded wire DTOs and keeps MCP data separate from authorization.

use std::collections::BTreeSet;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use hachimi_protocol::{
    McpPrompt, McpPromptArgument, McpPromptMessage, McpPromptResult, McpPromptRole, McpResource,
    McpResourceContent, McpResourceTemplate,
};
use serde_json::Value;

use crate::mcp::{MAX_SERVER_TEXT_CHARS, McpClientError, bounded_text, valid_tool_name};

pub(crate) const MAX_INVENTORY_PAGES: usize = 32;
pub(crate) const MAX_RESOURCE_COUNT: usize = 4_096;
pub(crate) const MAX_TEMPLATE_COUNT: usize = 4_096;
pub(crate) const MAX_PROMPT_COUNT: usize = 512;
pub(crate) const MAX_PROMPT_MESSAGES: usize = 256;
const MAX_URI_CHARS: usize = 4_096;
const MAX_CURSOR_CHARS: usize = 1_024;

#[derive(Debug, Clone, PartialEq)]
pub struct McpResourcePage {
    pub resources: Vec<McpResource>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpResourceTemplatePage {
    pub resource_templates: Vec<McpResourceTemplate>,
    pub next_cursor: Option<String>,
}

pub(crate) fn parse_resource(value: &Value) -> Result<McpResource, McpClientError> {
    let uri = required_bounded(value, "uri", MAX_URI_CHARS, "resource omitted valid uri")?;
    let name = required_bounded(value, "name", 256, "resource omitted valid name")?;
    Ok(McpResource {
        uri,
        name,
        title: optional_bounded(value, "title", 256)?,
        description: optional_bounded(value, "description", MAX_SERVER_TEXT_CHARS)?,
        mime_type: optional_bounded(value, "mimeType", 256)?,
        size: optional_u64(value, "size")?,
        annotations: optional_object(value, "annotations")?,
        meta: optional_object(value, "_meta")?,
    })
}

pub(crate) fn parse_resource_page(value: &Value) -> Result<McpResourcePage, McpClientError> {
    let resources =
        value
            .get("resources")
            .and_then(Value::as_array)
            .ok_or(McpClientError::InvalidResponse(
                "resources/list omitted resources",
            ))?;
    if resources.len() > MAX_RESOURCE_COUNT {
        return Err(McpClientError::InvalidInventory(
            "resource page exceeded the item limit".into(),
        ));
    }
    let resources = resources
        .iter()
        .map(parse_resource)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = next_cursor(value, &mut BTreeSet::new())?;
    Ok(McpResourcePage {
        resources,
        next_cursor,
    })
}

pub(crate) fn parse_resource_template(
    value: &Value,
) -> Result<McpResourceTemplate, McpClientError> {
    Ok(McpResourceTemplate {
        uri_template: required_bounded(
            value,
            "uriTemplate",
            MAX_URI_CHARS,
            "resource template omitted valid uriTemplate",
        )?,
        name: required_bounded(value, "name", 256, "resource template omitted valid name")?,
        title: optional_bounded(value, "title", 256)?,
        description: optional_bounded(value, "description", MAX_SERVER_TEXT_CHARS)?,
        mime_type: optional_bounded(value, "mimeType", 256)?,
        annotations: optional_object(value, "annotations")?,
        meta: optional_object(value, "_meta")?,
    })
}

pub(crate) fn parse_resource_template_page(
    value: &Value,
) -> Result<McpResourceTemplatePage, McpClientError> {
    let templates = value
        .get("resourceTemplates")
        .and_then(Value::as_array)
        .ok_or(McpClientError::InvalidResponse(
            "resources/templates/list omitted resourceTemplates",
        ))?;
    if templates.len() > MAX_TEMPLATE_COUNT {
        return Err(McpClientError::InvalidInventory(
            "resource template page exceeded the item limit".into(),
        ));
    }
    let resource_templates = templates
        .iter()
        .map(parse_resource_template)
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = next_cursor(value, &mut BTreeSet::new())?;
    Ok(McpResourceTemplatePage {
        resource_templates,
        next_cursor,
    })
}

pub(crate) fn parse_resource_contents(
    value: &Value,
    max_message_bytes: usize,
) -> Result<Vec<McpResourceContent>, McpClientError> {
    let values =
        value
            .get("contents")
            .and_then(Value::as_array)
            .ok_or(McpClientError::InvalidResponse(
                "resources/read omitted contents",
            ))?;
    if values.len() > 256 {
        return Err(McpClientError::InvalidInventory(
            "resource response exceeded the content item limit".into(),
        ));
    }
    values
        .iter()
        .map(|content| parse_resource_content(content, max_message_bytes))
        .collect()
}

pub(crate) fn parse_prompt(value: &Value) -> Result<McpPrompt, McpClientError> {
    let name = required_bounded(value, "name", 128, "prompt omitted valid name")?;
    if !valid_tool_name(&name) {
        return Err(McpClientError::InvalidPrompt(format!(
            "prompt name is invalid: {}",
            bounded_text(&name, 128)
        )));
    }
    let arguments = match value.get("arguments") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(arguments)) if arguments.len() <= 128 => arguments
            .iter()
            .map(|argument| {
                Ok(McpPromptArgument {
                    name: required_bounded(
                        argument,
                        "name",
                        128,
                        "prompt argument omitted valid name",
                    )?,
                    description: optional_bounded(argument, "description", MAX_SERVER_TEXT_CHARS)?,
                    required: argument.get("required").map_or(Ok(false), |value| {
                        value.as_bool().ok_or_else(|| {
                            McpClientError::InvalidPrompt(
                                "prompt argument required must be a boolean".into(),
                            )
                        })
                    })?,
                })
            })
            .collect::<Result<Vec<_>, McpClientError>>()?,
        Some(Value::Array(_)) => {
            return Err(McpClientError::InvalidPrompt(
                "prompt advertised too many arguments".into(),
            ));
        }
        Some(_) => {
            return Err(McpClientError::InvalidPrompt(
                "prompt arguments must be an array".into(),
            ));
        }
    };
    let unique = arguments
        .iter()
        .map(|argument| argument.name.as_str())
        .collect::<BTreeSet<_>>();
    if unique.len() != arguments.len() {
        return Err(McpClientError::InvalidPrompt(
            "prompt has duplicate argument names".into(),
        ));
    }
    Ok(McpPrompt {
        name,
        title: optional_bounded(value, "title", 256)?,
        description: optional_bounded(value, "description", MAX_SERVER_TEXT_CHARS)?,
        arguments,
    })
}

pub(crate) fn parse_prompt_result(value: &Value) -> Result<McpPromptResult, McpClientError> {
    let messages =
        value
            .get("messages")
            .and_then(Value::as_array)
            .ok_or(McpClientError::InvalidResponse(
                "prompts/get omitted messages",
            ))?;
    if messages.len() > MAX_PROMPT_MESSAGES {
        return Err(McpClientError::InvalidPrompt(
            "prompt result exceeded the message limit".into(),
        ));
    }
    let messages = messages
        .iter()
        .map(|message| {
            let role = match message.get("role").and_then(Value::as_str) {
                Some("user") => McpPromptRole::User,
                Some("assistant") => McpPromptRole::Assistant,
                _ => {
                    return Err(McpClientError::InvalidPrompt(
                        "prompt message has an invalid role".into(),
                    ));
                }
            };
            let content = message.get("content").cloned().ok_or_else(|| {
                McpClientError::InvalidPrompt("prompt message omitted content".into())
            })?;
            if !content.is_object() {
                return Err(McpClientError::InvalidPrompt(
                    "prompt message content must be an object".into(),
                ));
            }
            Ok(McpPromptMessage { role, content })
        })
        .collect::<Result<Vec<_>, McpClientError>>()?;
    Ok(McpPromptResult {
        description: optional_bounded(value, "description", MAX_SERVER_TEXT_CHARS)?,
        messages,
    })
}

pub(crate) fn next_cursor(
    value: &Value,
    seen: &mut BTreeSet<String>,
) -> Result<Option<String>, McpClientError> {
    let Some(cursor) = value.get("nextCursor") else {
        return Ok(None);
    };
    if cursor.is_null() {
        return Ok(None);
    }
    let cursor = cursor
        .as_str()
        .filter(|cursor| !cursor.is_empty() && cursor.chars().count() <= MAX_CURSOR_CHARS)
        .ok_or(McpClientError::InvalidResponse(
            "pagination returned an invalid cursor",
        ))?
        .to_owned();
    if !seen.insert(cursor.clone()) {
        return Err(McpClientError::InvalidResponse(
            "pagination returned a duplicate cursor",
        ));
    }
    Ok(Some(cursor))
}

pub(crate) fn validate_cursor(cursor: Option<&str>) -> Result<(), McpClientError> {
    if cursor.is_some_and(|cursor| {
        cursor.is_empty()
            || cursor.chars().count() > MAX_CURSOR_CHARS
            || cursor.chars().any(char::is_control)
    }) {
        return Err(McpClientError::InvalidInventory(
            "pagination cursor is invalid".into(),
        ));
    }
    Ok(())
}

fn parse_resource_content(
    value: &Value,
    max_message_bytes: usize,
) -> Result<McpResourceContent, McpClientError> {
    let uri = required_bounded(
        value,
        "uri",
        MAX_URI_CHARS,
        "resource content omitted valid uri",
    )?;
    let text = value.get("text");
    let blob = value.get("blob");
    if text.is_some() == blob.is_some() {
        return Err(McpClientError::InvalidInventory(
            "resource content must contain exactly one of text or blob".into(),
        ));
    }
    let budget = max_message_bytes.min(8 * 1024 * 1024);
    let text = match text {
        Some(value) => {
            let text = value.as_str().ok_or_else(|| {
                McpClientError::InvalidInventory("resource text must be a string".into())
            })?;
            if text.len() > budget {
                return Err(McpClientError::InvalidInventory(
                    "resource text exceeded the byte budget".into(),
                ));
            }
            Some(text.to_owned())
        }
        None => None,
    };
    let blob_base64 = match blob {
        Some(value) => {
            let blob = value.as_str().ok_or_else(|| {
                McpClientError::InvalidInventory("resource blob must be base64 text".into())
            })?;
            if blob.len() > budget.saturating_mul(4).div_ceil(3).saturating_add(4)
                || STANDARD.decode(blob).is_err()
            {
                return Err(McpClientError::InvalidInventory(
                    "resource blob is invalid or exceeded the byte budget".into(),
                ));
            }
            Some(blob.to_owned())
        }
        None => None,
    };
    Ok(McpResourceContent {
        uri,
        mime_type: optional_bounded(value, "mimeType", 256)?,
        text,
        blob_base64,
        content_reference: None,
    })
}

fn required_bounded(
    value: &Value,
    field: &str,
    max_chars: usize,
    error: &'static str,
) -> Result<String, McpClientError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.chars().count() <= max_chars
                && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .ok_or_else(|| McpClientError::InvalidInventory(error.into()))
}

fn optional_bounded(
    value: &Value,
    field: &str,
    max_chars: usize,
) -> Result<Option<String>, McpClientError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if value.chars().count() <= max_chars => Ok(Some(value.clone())),
        Some(_) => Err(McpClientError::InvalidInventory(format!(
            "{field} is invalid or exceeded its limit"
        ))),
    }
}

fn optional_u64(value: &Value, field: &str) -> Result<Option<u64>, McpClientError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| McpClientError::InvalidInventory(format!("{field} must be unsigned"))),
    }
}

fn optional_object(value: &Value, field: &str) -> Result<Option<Value>, McpClientError> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(value) if value.is_object() => Ok(Some(value.clone())),
        Some(_) => Err(McpClientError::InvalidInventory(format!(
            "{field} must be an object"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn duplicate_pagination_cursor_is_rejected() {
        let mut seen = BTreeSet::new();
        assert_eq!(
            next_cursor(&json!({ "nextCursor": "same" }), &mut seen).unwrap(),
            Some("same".into())
        );
        assert!(next_cursor(&json!({ "nextCursor": "same" }), &mut seen).is_err());
    }

    #[test]
    fn resource_content_requires_one_bounded_payload() {
        let parsed = parse_resource_contents(
            &json!({ "contents": [{ "uri": "memo://one", "text": "hello" }] }),
            4096,
        )
        .unwrap();
        assert_eq!(parsed[0].text.as_deref(), Some("hello"));
        assert!(
            parse_resource_contents(
                &json!({ "contents": [{ "uri": "memo://one", "text": "x", "blob": "eA==" }] }),
                4096,
            )
            .is_err()
        );
    }

    #[test]
    fn prompts_reject_duplicate_arguments_and_invalid_roles() {
        assert!(
            parse_prompt(&json!({
                "name": "brief",
                "arguments": [{ "name": "topic" }, { "name": "topic" }]
            }))
            .is_err()
        );
        assert!(
            parse_prompt_result(&json!({
                "messages": [{ "role": "system", "content": { "type": "text", "text": "x" } }]
            }))
            .is_err()
        );
    }
}
