# OpenAI Embeddings API snapshot

- Retrieved: 2026-07-30T16:20:00+08:00
- Canonical documentation: https://developers.openai.com/api/reference/resources/embeddings/methods/create
- Canonical API: `POST https://api.openai.com/v1/embeddings`
- OpenAPI document version: `2.3.0`
- OpenAPI license: MIT

The official API schema identifies `CreateEmbeddingRequest` as the request body and `CreateEmbeddingResponse` as the JSON result. Requests contain `model`, string or string-array `input`, optional `dimensions`, and `encoding_format`. Hachimi uses only `encoding_format: "float"` in the first implementation.

Successful responses have `object: "list"`, indexed `data` rows with `object: "embedding"` and a numeric vector, a model identifier, and usage with prompt and total token counts. The first Hachimi implementation exposes this through a provider-neutral embeddings interface and does not connect it to Memory.
