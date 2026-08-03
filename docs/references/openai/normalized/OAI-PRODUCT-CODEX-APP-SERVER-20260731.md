# OpenAI Codex App Server behavior snapshot

Canonical source: https://developers.openai.com/codex/codex-manual.md

Retrieved: 2026-07-31T14:08:06+08:00

Hachimi Workbench uses the following documented behavior from this fixed snapshot:

- A thread contains turns, and turns contain typed items.
- Turn lifecycle notifications include `turn/started`, `turn/completed`, `turn/diff/updated`, and `turn/plan/updated`.
- Every item has authoritative `item/started` and `item/completed` lifecycle notifications.
- Streamed deltas are item-specific, including agent text, public reasoning summaries, plan text, and command output.
- Common item kinds include user and agent messages, plan, reasoning, command execution, file change, MCP calls, dynamic tool calls, and collaboration tool calls.
- `thread/status/changed` is the source for thread activity displayed outside the currently open thread.
- `tool/requestUserInput` asks one to three short questions and may offer free-form choices or an automatic resolution timeout.
- Completed items and completed turns are authoritative over temporary streamed state.

Hachimi maps Session to thread, Run to turn, and TranscriptItem to ThreadItem. It preserves Hachimi's permission, sandbox, persistence, audit, and secret-handling boundaries rather than copying private Codex implementation details.
