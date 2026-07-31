# Feishu Go SDK v3.9.9 long-connection bounded wire snapshot

- Repository: `https://github.com/larksuite/oapi-sdk-go`
- Tag/version: `v3.9.9`
- Commit: `ff207b774541a195f0a98c5bfda1507905e45431`
- Retrieved: `2026-07-31T02:26:41+08:00`
- License: MIT (`LICENSE` blob `9d01995f1e1817ccbf3bf1fe135ed5bfc2c7b528`)
- Purpose: bounded protocol-source snapshot only; the Go SDK is not a Hachimi runtime dependency.

## Fixed source files

| File | Git blob SHA | Bounded facts retained |
|---|---|---|
| `ws/model.go` | `88fa5607e3185c3a55a0c67a22e95e4ee4dfaddf` | endpoint config, headers, ACK response and ping frame |
| `ws/const.go` | `6023df7485452031b07277c2b8496a70cfbb35e1` | bootstrap path, query keys, header keys and message kinds |
| `ws/client.go` | `154d2fc666c669604034c1a29b4ec3b0882b2ec6` | bootstrap, reconnect, heartbeat, fragmentation and response behavior |
| `ws/pbbp2.pb.go` | `a6416df7f299b3adb1f37df108cbeda623c298b8` | generated protobuf `Frame` field numbers and `Header` messages |

## Exact bounded wire definitions

From `ws/model.go` and `ws/const.go`:

```go
type ClientConfig struct {
    ReconnectCount    int `json:"ReconnectCount,omitempty"`
    ReconnectInterval int `json:"ReconnectInterval,omitempty"`
    ReconnectNonce    int `json:"ReconnectNonce,omitempty"`
    PingInterval      int `json:"PingInterval,omitempty"`
}

type Response struct {
    StatusCode int               `json:"code"`
    Headers    map[string]string `json:"headers"`
    Data       []byte            `json:"data"`
}

GenEndpointUri  = "/callback/ws/endpoint"
HeaderTimestamp = "timestamp"
HeaderType      = "type"
HeaderMessageID = "message_id"
HeaderSum       = "sum"
HeaderSeq       = "seq"
MessageTypeEvent = "event"
MessageTypePing  = "ping"
MessageTypePong  = "pong"
```

The generated protobuf source fixes `Frame` fields as `seq_id=1`, `log_id=2`, `service=3`, `method=4`, `headers=5`, `payload_encoding=6`, `payload_type=7`, `payload=8`, `log_id_new=9`; each `Header` carries `key=1` and `value=2`.

The fixed client obtains a WebSocket endpoint from the bootstrap path, applies server-provided reconnect and ping configuration, sends binary protobuf frames, reassembles data with `sum`/`seq` keyed by `message_id`, writes an ACK response in the received frame, and reconnects after a read failure when enabled.

Only the definitions and behavior above are retained. Event dispatcher internals, logging and unrelated SDK APIs are intentionally omitted.
