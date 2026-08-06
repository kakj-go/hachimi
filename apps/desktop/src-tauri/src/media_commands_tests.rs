use super::*;

fn provider_input(protocol: ProviderProtocolKind) -> LlmSettingsInput {
    LlmSettingsInput {
        base_url: "https://api.openai.com/v1".into(),
        model_name: "gpt-fixture".into(),
        protocol,
        compatibility_profile_id: "openai-strict".into(),
        provider_endpoint_id: None,
        provider_account_id: None,
        embedding_model_name: String::new(),
        reasoning_summary: false,
        remote_compaction: false,
        max_input_tokens: 0,
        max_output_tokens: 0,
        structured_output_mode: StructuredOutputMode::Auto,
        api_key: None,
        clear_api_key: false,
    }
}

#[test]
fn provider_feature_switches_keep_only_legacy_chat_and_gate_remote_context() {
    let mut features = hachimi_core::RuntimeFeatureSet::all_enabled();
    features.provider_extensions = false;
    assert!(
        validate_provider_feature_input(
            features,
            &provider_input(ProviderProtocolKind::ChatCompletions)
        )
        .is_ok()
    );
    let error =
        validate_provider_feature_input(features, &provider_input(ProviderProtocolKind::Responses))
            .expect_err("responses disabled");
    assert_eq!(error.code, "feature_disabled");
    assert_eq!(error.message, "provider_extensions");

    features.provider_extensions = true;
    features.provider_remote_context = false;
    let mut remote = provider_input(ProviderProtocolKind::Responses);
    remote.remote_compaction = true;
    let error =
        validate_provider_feature_input(features, &remote).expect_err("remote context disabled");
    assert_eq!(error.code, "feature_disabled");
    assert_eq!(error.message, "provider_remote_context");
}

#[test]
fn secret_user_input_disables_pet_speech_presentation() {
    let payload = ItemPayload::UserInputRequest {
        request_id: hachimi_protocol::UserInputRequestId::from("secret-input"),
        questions: vec![hachimi_protocol::UserInputQuestion {
            id: "secret".into(),
            header: "Secret".into(),
            prompt: "Token".into(),
            options: Vec::new(),
            secret: true,
            auto_resolution_ms: None,
            default_answer: None,
        }],
        display_answers: Vec::new(),
    };
    assert!(pet_payload_contains_secret_input(&payload));
    assert!(!pet_payload_contains_secret_input(
        &ItemPayload::Assistant {
            text: "safe".into(),
            phase: hachimi_protocol::AgentMessagePhase::FinalAnswer,
        }
    ));
}
