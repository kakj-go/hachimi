use hachimi_model_runtime::ModelRuntimeError;
use hachimi_protocol::{
    ProviderEmbeddingRequest, ProviderEmbeddingResult, ProviderEmbeddingVector, TokenUsage,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::{OpenAiCompatibleRuntime, is_context_overflow, provider_error_detail, request_error};

pub(super) async fn embed(
    runtime: &OpenAiCompatibleRuntime,
    request: ProviderEmbeddingRequest,
    cancellation: CancellationToken,
) -> Result<ProviderEmbeddingResult, ModelRuntimeError> {
    validate_request(&request)?;
    let endpoint = format!(
        "{}/embeddings",
        runtime.settings.base_url.trim_end_matches('/')
    );
    let mut body = json!({
        "model": request.model,
        "input": request.input,
        "encoding_format": "float",
    });
    if let Some(dimensions) = request.dimensions {
        body["dimensions"] = Value::from(dimensions);
    }
    let mut http_request = runtime.client.post(endpoint).json(&body);
    if let Some(secret) = runtime.api_key.as_deref().filter(|value| !value.is_empty()) {
        http_request = http_request.bearer_auth(secret);
    }
    let response = tokio::select! {
        () = cancellation.cancelled() => return Err(ModelRuntimeError::Cancelled),
        response = http_request.send() => response.map_err(|error| ModelRuntimeError::Provider(request_error(&error).to_string()))?,
    };
    let status = response.status();
    if !status.is_success() {
        let response_body = response.text().await.unwrap_or_default();
        let detail = provider_error_detail(&response_body)
            .unwrap_or_else(|| "provider rejected the embeddings request".into());
        if is_context_overflow(status, &detail) {
            return Err(ModelRuntimeError::ContextOverflow);
        }
        return Err(ModelRuntimeError::Provider(format!(
            "HTTP {status}: {detail}"
        )));
    }
    let response: Value = response.json().await.map_err(|_| {
        ModelRuntimeError::InvalidStream("embeddings endpoint returned invalid JSON".into())
    })?;
    parse_response(&response, request.input.len(), request.dimensions)
}

fn validate_request(request: &ProviderEmbeddingRequest) -> Result<(), ModelRuntimeError> {
    if request.model.trim().is_empty() || request.input.is_empty() || request.input.len() > 2_048 {
        return Err(ModelRuntimeError::Provider(
            "embedding model and between 1 and 2048 inputs are required".into(),
        ));
    }
    if request.input.iter().any(|input| input.is_empty()) {
        return Err(ModelRuntimeError::Provider(
            "embedding inputs cannot be empty".into(),
        ));
    }
    if request.dimensions == Some(0) {
        return Err(ModelRuntimeError::Provider(
            "embedding dimensions must be positive".into(),
        ));
    }
    Ok(())
}

fn parse_response(
    response: &Value,
    expected_count: usize,
    expected_dimensions: Option<u32>,
) -> Result<ProviderEmbeddingResult, ModelRuntimeError> {
    if response.get("object").and_then(Value::as_str) != Some("list") {
        return Err(ModelRuntimeError::InvalidStream(
            "embeddings response omitted object=list".into(),
        ));
    }
    let model = response
        .get("model")
        .and_then(Value::as_str)
        .filter(|model| !model.is_empty())
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("embeddings response omitted model".into())
        })?;
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("embeddings response omitted data".into())
        })?;
    if data.len() != expected_count {
        return Err(ModelRuntimeError::InvalidStream(
            "embeddings response count did not match input count".into(),
        ));
    }
    let mut vectors = Vec::with_capacity(data.len());
    let mut common_dimensions = None;
    for item in data {
        if item.get("object").and_then(Value::as_str) != Some("embedding") {
            return Err(ModelRuntimeError::InvalidStream(
                "embedding row omitted object=embedding".into(),
            ));
        }
        let index = item
            .get("index")
            .and_then(Value::as_u64)
            .and_then(|index| u32::try_from(index).ok())
            .ok_or_else(|| {
                ModelRuntimeError::InvalidStream("embedding row omitted index".into())
            })?;
        let embedding = item
            .get("embedding")
            .and_then(Value::as_array)
            .filter(|values| !values.is_empty())
            .ok_or_else(|| {
                ModelRuntimeError::InvalidStream("embedding row omitted vector".into())
            })?;
        let values = embedding
            .iter()
            .map(|value| {
                let value = value
                    .as_f64()
                    .filter(|value| value.is_finite())
                    .ok_or_else(|| {
                        ModelRuntimeError::InvalidStream(
                            "embedding vector contained a non-finite number".into(),
                        )
                    })?;
                let value = value as f32;
                value.is_finite().then_some(value).ok_or_else(|| {
                    ModelRuntimeError::InvalidStream(
                        "embedding vector exceeded finite f32 range".into(),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let dimensions = u32::try_from(values.len()).map_err(|_| {
            ModelRuntimeError::InvalidStream("embedding vector was too large".into())
        })?;
        if expected_dimensions.is_some_and(|expected| expected != dimensions)
            || common_dimensions.is_some_and(|expected| expected != dimensions)
        {
            return Err(ModelRuntimeError::InvalidStream(
                "embedding vector dimensions were inconsistent".into(),
            ));
        }
        common_dimensions = Some(dimensions);
        vectors.push(ProviderEmbeddingVector { index, values });
    }
    vectors.sort_by_key(|vector| vector.index);
    if vectors
        .iter()
        .enumerate()
        .any(|(expected, vector)| usize::try_from(vector.index).ok() != Some(expected))
    {
        return Err(ModelRuntimeError::InvalidStream(
            "embedding indices were missing or duplicated".into(),
        ));
    }
    let usage = response.get("usage").ok_or_else(|| {
        ModelRuntimeError::InvalidStream("embeddings response omitted usage".into())
    })?;
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("embeddings usage omitted prompt_tokens".into())
        })?;
    let total_tokens = usage
        .get("total_tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            ModelRuntimeError::InvalidStream("embeddings usage omitted total_tokens".into())
        })?;
    if total_tokens < input_tokens {
        return Err(ModelRuntimeError::InvalidStream(
            "embeddings usage total was smaller than prompt usage".into(),
        ));
    }
    Ok(ProviderEmbeddingResult {
        model: model.into(),
        vectors,
        usage: TokenUsage {
            input_tokens,
            output_tokens: 0,
        },
    })
}

#[cfg(test)]
mod tests {
    use hachimi_model_runtime::ModelRuntimeError;
    use serde_json::json;

    use super::parse_response;

    #[test]
    fn strict_embeddings_preserve_input_order() {
        let result = parse_response(
            &json!({
                "object": "list",
                "model": "embed",
                "data": [
                    { "object": "embedding", "index": 1, "embedding": [0.3, 0.4] },
                    { "object": "embedding", "index": 0, "embedding": [0.1, 0.2] }
                ],
                "usage": { "prompt_tokens": 4, "total_tokens": 4 }
            }),
            2,
            Some(2),
        )
        .expect("embedding result");
        assert_eq!(result.vectors[0].index, 0);
        assert_eq!(result.vectors[1].index, 1);
    }

    #[test]
    fn malformed_embedding_dimensions_fail_closed() {
        let error = parse_response(
            &json!({
                "object": "list",
                "model": "embed",
                "data": [{ "object": "embedding", "index": 0, "embedding": [0.1] }],
                "usage": { "prompt_tokens": 1, "total_tokens": 1 }
            }),
            1,
            Some(2),
        )
        .expect_err("dimension mismatch");
        assert!(matches!(error, ModelRuntimeError::InvalidStream(_)));
    }
}
