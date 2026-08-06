# Chat Completions behavior snapshot

- Reference ID: `OAI-PRODUCT-CHATCOMPLETIONS-20260730`
- Protocol: public OpenAI `/v1/chat/completions` JSON and SSE wire shapes from OpenAPI `2.3.0`.
- Hachimi mapping: the existing adapter remains available through an explicit protocol selection in the provider registry.
- Capability boundary: Chat Completions never advertises Responses-only remote compaction or reasoning-summary capabilities.
- Acceptance: request mapping, text and function streaming, usage, finish reason, cancellation, HTTP errors, context overflow, and strict malformed-response rejection pass.
