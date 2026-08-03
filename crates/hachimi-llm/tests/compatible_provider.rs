use std::{collections::BTreeMap, env};

use futures_util::StreamExt;
use hachimi_llm::{ApiKeyStore, OpenAiCompatibleRuntime, SystemApiKeyStore};
use hachimi_model_runtime::ModelRuntime;
use hachimi_protocol::{
    LlmSettings, ModelEvent, ModelMessage, ModelRequest, ProviderProtocolKind,
    StructuredOutputMode, ToolDescriptor, ToolEffect,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[ignore = "requires an explicitly configured OpenAI-compatible provider"]
async fn compatible_provider_accepts_aliased_tool_names() {
    let base_url = env::var("HACHIMI_COMPAT_BASE_URL").expect("compatible provider base URL");
    let model_name = env::var("HACHIMI_COMPAT_MODEL").expect("compatible provider model");
    let api_key = SystemApiKeyStore
        .get()
        .expect("credential manager")
        .expect("configured provider credential");
    let runtime = OpenAiCompatibleRuntime::tool_calling(
        LlmSettings {
            base_url,
            model_name,
            protocol: ProviderProtocolKind::Responses,
            structured_output_mode: StructuredOutputMode::Disabled,
            max_output_tokens: 256,
            ..LlmSettings::default()
        },
        Some(api_key),
    )
    .expect("compatible runtime");
    let mut stream = runtime.stream(
        ModelRequest {
            messages: vec![ModelMessage::user("你好，请简短回复。不要调用工具。")],
            tools: vec![ToolDescriptor {
                name: "agent.spawn".into(),
                description: "Return a fixed compatibility probe; do not answer with text.".into(),
                input_schema: json!({
                    "type": "object",
                    "properties": { "task": { "type": "string" } },
                    "required": ["task"],
                    "additionalProperties": false
                }),
                effect: ToolEffect::ReadOnly,
                parallel_safe: false,
                required_scopes: Vec::new(),
            }],
            parallel_tool_calls: false,
            max_output_tokens: Some(128),
        },
        CancellationToken::new(),
    );

    let mut names = BTreeMap::<u32, String>::new();
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("valid compatible provider event") {
            ModelEvent::ToolCallDelta {
                index, name_delta, ..
            } => names.entry(index).or_default().push_str(&name_delta),
            ModelEvent::ToolCallCompleted { call } => {
                names.insert(u32::try_from(names.len()).unwrap_or(u32::MAX), call.name);
            }
            ModelEvent::Completed { .. } => completed = true,
            ModelEvent::AgentMessageStarted { .. }
            | ModelEvent::AgentMessageDelta { .. }
            | ModelEvent::AgentMessageCompleted { .. }
            | ModelEvent::TextDelta { .. }
            | ModelEvent::ReasoningDelta { .. }
            | ModelEvent::Usage { .. } => {}
        }
    }

    assert!(completed, "compatible provider stream must complete");
    assert!(
        names.values().all(|name| name == "agent.spawn"),
        "any provider tool alias must be restored to the internal tool identity"
    );
}
