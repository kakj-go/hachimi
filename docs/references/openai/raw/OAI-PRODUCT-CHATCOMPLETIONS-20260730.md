# OpenAI Chat Completions API snapshot

- Retrieved: 2026-07-30T16:20:00+08:00
- Canonical documentation: https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create
- Canonical API: `POST https://api.openai.com/v1/chat/completions`
- OpenAPI document version: `2.3.0`
- OpenAPI license: MIT

The official API schema identifies `CreateChatCompletionRequest` as the request body and returns a JSON `CreateChatCompletionResponse` or a streamed sequence of `CreateChatCompletionStreamResponse` objects. Function tools are externally tagged under `type: "function"` and `function`; returned tool calls contain `id`, `type`, and `function.name` plus JSON-string `function.arguments`.

For streaming, chunks carry indexed `choices[].delta`, optional tool-call deltas, and a terminal `finish_reason`. Usage is returned in the response and can be requested in the stream. The protocol remains public and supported, but it does not imply Responses-only capabilities such as public reasoning summaries or standalone remote compaction.

This snapshot records the public wire boundary required for a strict Hachimi adapter; it does not authorize heuristic repair of malformed OpenAI-compatible responses.
