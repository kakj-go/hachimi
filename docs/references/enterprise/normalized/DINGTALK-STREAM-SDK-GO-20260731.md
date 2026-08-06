# DingTalk Stream transport mapping for Hachimi

Source: [ref:DINGTALK-STREAM-SDK-GO-20260731].

Hachimi's Rust transport implements the fixed public wire boundary without linking the Go SDK:

- bootstrap subscriptions are limited to callback, disconnect and ping;
- only bounded WebSocket text frames matching the registered JSON shape are accepted;
- `headers.messageId` is the durable event/dedup identity and `headers.time` is replay-window input;
- ACK is JSON with code `200`, content type `application/json`, and the original message ID;
- ping/pong, reconnect backoff and cancellation are supervised by the local Gateway-owned runtime;
- duplicate events are ACKed but not delivered twice;
- credentials, Grants and Host tokens never enter frame metadata or the enterprise ledger.

Acceptance is deterministic fixture coverage for frame decoding, ACK, duplicate suppression, disconnect/reconnect and cross-connection dedup. Real tenant validation remains a separate external-environment status.
