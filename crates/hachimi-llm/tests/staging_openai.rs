use std::{env, fs};

use futures_util::StreamExt;
use hachimi_llm::{ApiKeyStore, OpenAiCompatibleRuntime, SystemApiKeyStore, test_connection};
use hachimi_model_runtime::{ModelRuntime, ModelRuntimeError};
use hachimi_protocol::{
    LlmSettings, ModelCompactionRequest, ModelEvent, ModelMessage, ModelRequest,
    ProviderEmbeddingRequest, ProviderProtocolKind, StructuredOutputMode, ToolDescriptor,
    ToolEffect,
};
use serde::Deserialize;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct StagingConfig {
    base_url: String,
    chat_model: String,
    responses_model: String,
    embedding_model: String,
    secret_refs: Vec<String>,
    require_reasoning_summary: bool,
    require_remote_compaction: bool,
    #[serde(default = "default_overflow_probe_chars")]
    overflow_probe_chars: usize,
}

const fn default_overflow_probe_chars() -> usize {
    1_200_000
}

fn settings(config: &StagingConfig, protocol: ProviderProtocolKind) -> LlmSettings {
    LlmSettings {
        base_url: config.base_url.trim_end_matches('/').to_owned(),
        model_name: match protocol {
            ProviderProtocolKind::ChatCompletions => config.chat_model.clone(),
            ProviderProtocolKind::Responses => config.responses_model.clone(),
            ProviderProtocolKind::Embeddings => unreachable!(),
        },
        protocol,
        compatibility_profile_id: "openai-strict".into(),
        provider_endpoint_id: None,
        provider_account_id: None,
        embedding_model_name: if protocol == ProviderProtocolKind::Responses {
            config.embedding_model.clone()
        } else {
            String::new()
        },
        reasoning_summary: protocol == ProviderProtocolKind::Responses
            && config.require_reasoning_summary,
        remote_compaction: protocol == ProviderProtocolKind::Responses
            && config.require_remote_compaction,
        max_input_tokens: 128_000,
        max_output_tokens: 512,
        structured_output_mode: StructuredOutputMode::Auto,
    }
}

fn tool_probe_request() -> ModelRequest {
    ModelRequest {
        messages: vec![ModelMessage::user(
            "Call the hachimi_release_probe tool exactly once with {\"ok\":true}.",
        )],
        tools: vec![ToolDescriptor {
            name: "hachimi_release_probe".into(),
            description: "Return the fixed staging conformance marker".into(),
            input_schema: json!({
                "type": "object",
                "properties": {"ok": {"type": "boolean", "const": true}},
                "required": ["ok"],
                "additionalProperties": false
            }),
            effect: ToolEffect::ReadOnly,
            parallel_safe: false,
            required_scopes: Vec::new(),
        }],
        parallel_tool_calls: false,
        max_output_tokens: Some(128),
    }
}

async fn assert_tool_usage_and_completion(runtime: &OpenAiCompatibleRuntime) {
    let mut stream = runtime.stream(tool_probe_request(), CancellationToken::new());
    let mut tool = false;
    let mut usage = false;
    let mut completed = false;
    while let Some(event) = stream.next().await {
        match event.expect("valid provider event") {
            ModelEvent::ToolCallCompleted { call } => {
                assert_eq!(call.name, "hachimi_release_probe");
                assert_eq!(
                    call.arguments
                        .get("ok")
                        .and_then(serde_json::Value::as_bool),
                    Some(true)
                );
                tool = true;
            }
            ModelEvent::Usage { usage: value } => usage |= value.input_tokens > 0,
            ModelEvent::Completed { .. } => completed = true,
            _ => {}
        }
    }
    assert!(
        tool && usage && completed,
        "tool, usage, and completion are required"
    );
}

async fn assert_real_cancellation(runtime: &OpenAiCompatibleRuntime) {
    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        trigger.cancel();
    });
    let mut stream = runtime.stream(
        ModelRequest {
            messages: vec![ModelMessage::user(
                "Generate at least 3000 tokens of numbered release-test filler without stopping early.",
            )],
            tools: Vec::new(),
            parallel_tool_calls: false,
            max_output_tokens: Some(4_096),
        },
        cancellation,
    );
    let mut cancelled = false;
    while let Some(event) = stream.next().await {
        match event {
            Err(ModelRuntimeError::Cancelled) => {
                cancelled = true;
                break;
            }
            Err(error) => panic!("unexpected cancellation result: {error}"),
            Ok(_) => {}
        }
    }
    assert!(
        cancelled,
        "the real streaming request must observe cancellation"
    );
}

async fn assert_provider_error(config: &StagingConfig, api_key: &str) {
    let mut invalid = settings(config, ProviderProtocolKind::Responses);
    invalid.model_name = "hachimi-release-gate-intentionally-invalid-model".into();
    let runtime = OpenAiCompatibleRuntime::tool_calling(invalid, Some(api_key.into()))
        .expect("invalid-model runtime");
    let mut stream = runtime.stream(
        ModelRequest {
            messages: vec![ModelMessage::user("HACHIMI_ERROR_PROBE")],
            tools: Vec::new(),
            parallel_tool_calls: false,
            max_output_tokens: Some(16),
        },
        CancellationToken::new(),
    );
    let mut rejected = false;
    while let Some(event) = stream.next().await {
        match event {
            Err(ModelRuntimeError::Provider(_)) => {
                rejected = true;
                break;
            }
            Err(error) => panic!("unexpected invalid-model result: {error}"),
            Ok(_) => {}
        }
    }
    assert!(rejected, "the Provider must reject the invalid model");
}

async fn assert_context_overflow(runtime: &OpenAiCompatibleRuntime, overflow_probe_chars: usize) {
    assert!((600_000..=4_000_000).contains(&overflow_probe_chars));
    let mut content = String::with_capacity(overflow_probe_chars);
    let mut index = 0_u64;
    while content.len() < overflow_probe_chars {
        content.push_str(&format!("{index:016x}-"));
        index += 1;
    }
    let mut stream = runtime.stream(
        ModelRequest {
            messages: vec![ModelMessage::user(content)],
            tools: Vec::new(),
            parallel_tool_calls: false,
            max_output_tokens: Some(16),
        },
        CancellationToken::new(),
    );
    let mut overflow = false;
    while let Some(event) = stream.next().await {
        match event {
            Err(ModelRuntimeError::ContextOverflow) => {
                overflow = true;
                break;
            }
            Err(error) => panic!("unexpected overflow result: {error}"),
            Ok(_) => {}
        }
    }
    assert!(overflow, "the Provider must reject the oversized context");
}

#[tokio::test]
#[ignore = "requires a protected real OpenAI staging credential and models"]
async fn openai_product_adapters_conform_against_staging() {
    assert_eq!(
        env::var("HACHIMI_STAGING_ACTIVE_GATE").as_deref(),
        Ok("openai")
    );
    let path = env::var("HACHIMI_STAGING_OPENAI_CONFIG").expect("staging config path");
    let config: StagingConfig =
        serde_json::from_slice(&fs::read(path).expect("read staging config"))
            .expect("parse staging config");
    assert_eq!(
        config.secret_refs,
        ["credential-manager:provider:default"],
        "the staging gate must use the product API-key entry",
    );
    assert!(config.require_reasoning_summary);
    assert!(config.require_remote_compaction);
    let api_key = SystemApiKeyStore
        .get()
        .expect("credential manager")
        .expect("OpenAI staging credential");

    let chat = settings(&config, ProviderProtocolKind::ChatCompletions);
    assert!(
        test_connection(&chat, Some(&api_key))
            .await
            .expect("chat conformance")
            .success
    );
    let chat_runtime =
        OpenAiCompatibleRuntime::tool_calling(chat, Some(api_key.clone())).expect("chat runtime");
    assert_tool_usage_and_completion(&chat_runtime).await;

    let responses = settings(&config, ProviderProtocolKind::Responses);
    assert!(
        test_connection(&responses, Some(&api_key))
            .await
            .expect("responses, embeddings, compaction, and summary conformance")
            .success
    );

    let runtime = OpenAiCompatibleRuntime::tool_calling(responses, Some(api_key.clone()))
        .expect("responses runtime");
    assert_tool_usage_and_completion(&runtime).await;
    let embedding = runtime
        .embed(
            ProviderEmbeddingRequest {
                model: config.embedding_model.clone(),
                input: vec!["HACHIMI_EMBEDDING_DIMENSION_PROBE".into()],
                dimensions: None,
            },
            CancellationToken::new(),
        )
        .await
        .expect("real embedding");
    assert_eq!(embedding.vectors.len(), 1);
    assert!(!embedding.vectors[0].values.is_empty());
    assert!(
        embedding.vectors[0]
            .values
            .iter()
            .all(|value| value.is_finite())
    );
    assert!(embedding.usage.input_tokens > 0);

    let compacted = runtime
        .compact(
            ModelCompactionRequest {
                messages: vec![ModelMessage::user(
                    "Current goal: verify the Hachimi release gate. Pending work: preserve RELEASE_MARKER_73.",
                )],
                max_output_tokens: 256,
            },
            CancellationToken::new(),
        )
        .await
        .expect("real remote compaction");
    assert_eq!(compacted.replacement_messages.len(), 1);
    assert!(!compacted.replacement_messages[0].content.trim().is_empty());
    assert!(
        compacted.replacement_messages[0]
            .content
            .contains("RELEASE_MARKER_73")
    );
    assert!(compacted.usage.input_tokens > 0);

    assert_real_cancellation(&runtime).await;
    assert_provider_error(&config, &api_key).await;
    assert_context_overflow(&runtime, config.overflow_probe_chars).await;
}
