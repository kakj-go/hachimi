# Embeddings API behavior snapshot

- Reference ID: `OAI-PRODUCT-EMBEDDINGS-20260730`
- Protocol: public OpenAI `/v1/embeddings` JSON wire shape from OpenAPI `2.3.0`.
- Hachimi mapping: provider-neutral request/result types preserve input ordering, dimensions, model identity, vectors, and usage.
- Boundary: embeddings are a Provider capability only; no Memory store, retrieval pipeline, or Memory migration is created.
- Acceptance: single/batch input, order/index validation, vector finiteness and dimensions, usage, cancellation, HTTP errors, and malformed-response rejection pass.
