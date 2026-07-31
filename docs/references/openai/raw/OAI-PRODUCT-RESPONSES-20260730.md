# OpenAI Responses API snapshot

- Retrieved: 2026-07-30T16:20:00+08:00
- Canonical documentation: https://developers.openai.com/api/reference/resources/responses/methods/create
- Canonical API: `POST https://api.openai.com/v1/responses`
- OpenAPI document version: `2.3.0`
- OpenAPI license: MIT

The official API schema identifies `CreateResponse` as the required request body and returns either a JSON `Response` or a `text/event-stream` `ResponseStreamEvent`. Responses use typed input/output Items. A message, function call, function-call output, and reasoning item are distinct Items. Function tools are internally tagged with `type: "function"`; returned calls use `type: "function_call"`, `call_id`, `name`, and JSON-string `arguments`.

The official migration guide recommends Responses for new integrations while Chat Completions remains supported. It documents `store: false` for stateless operation, `input` for message/Item input, `output` for typed result Items, and streamed lifecycle events such as text deltas, function-call argument deltas, completion, failure, and incomplete outcomes.

Reasoning summaries are requested through the public `reasoning.summary` field and returned as summary content on a public reasoning Item. Raw or encrypted reasoning content is not a displayable summary.

The official standalone compaction API is `POST /v1/responses/compact`. Its output contains opaque continuation state intended to be passed to a later Responses request. Official guidance says not to edit, inspect, or treat this opaque result as a human summary.

This repository snapshot intentionally records only the public wire-shape and behavioral boundary needed by Hachimi. It does not copy private Codex protocols, prompts, or hidden reasoning.
