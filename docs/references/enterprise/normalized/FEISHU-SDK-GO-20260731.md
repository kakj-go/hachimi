# Feishu long-connection transport mapping for Hachimi

Source: [ref:FEISHU-SDK-GO-20260731].

Hachimi's Rust transport implements the fixed public wire boundary without linking the Go SDK:

- bootstrap is limited to `/callback/ws/endpoint` and validates the returned WebSocket URL/config;
- only bounded binary protobuf `Frame` messages with known field numbers are accepted;
- fragments are reassembled by message ID and validated `sum`/`seq`; incomplete or oversized sets fail closed;
- event, ping and pong are distinguished through the fixed `type` header;
- ACK uses the original frame identity and a bounded JSON response payload with code `200`;
- heartbeat, reconnect backoff, cancellation and dedup are supervised by the local Gateway-owned runtime;
- credentials, Grants and Host tokens never enter frames or the enterprise ledger.

Acceptance is deterministic fixture coverage for protobuf decoding, fragmentation, ACK, heartbeat, disconnect/reconnect and duplicate suppression. Real tenant validation remains a separate external-environment status.
