# Responses API behavior snapshot

- Reference ID: `OAI-PRODUCT-RESPONSES-20260730`
- Protocol: public OpenAI `/v1/responses` and `/v1/responses/compact` wire shapes from OpenAPI `2.3.0`.
- Hachimi mapping: a strict typed Item adapter maps text, function calls, usage, errors, cancellation, and only explicitly public reasoning summaries into provider-neutral events.
- Compaction boundary: opaque compacted continuation state is never interpreted as a human summary and never placed in ordinary transcript storage. Hachimi's semantic checkpoint remains the local fallback.
- Compatibility boundary: non-OpenAI dialects require a named, registered compatibility profile and a successful probe; malformed responses are rejected instead of repaired silently.
- Acceptance: JSON and SSE conformance, function-call arguments, usage, cancellation, error/incomplete status, capability drift, public summary filtering, and remote-compaction fallback tests pass.
