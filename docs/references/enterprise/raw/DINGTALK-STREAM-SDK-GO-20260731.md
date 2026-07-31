# DingTalk Stream SDK Go v0.9.1 bounded wire snapshot

- Repository: `https://github.com/open-dingtalk/dingtalk-stream-sdk-go`
- Tag/version: `v0.9.1`
- Commit: `d1cc841e6013c3f6513a5bb01dfe3219b9c37d17`
- Retrieved: `2026-07-31T02:26:41+08:00`
- License: MIT (`LICENSE` blob `09e3a2725bae3ec2b201fb3d08eb210c1ec5ade1`)
- Purpose: bounded protocol-source snapshot only; the Go SDK is not a Hachimi runtime dependency.

## Fixed source files

| File | Git blob SHA | Bounded facts retained |
|---|---|---|
| `payload/data_frame.go` | `e0ff0409012c963303edb50358205968ff3f6bb0` | JSON frame and response shape |
| `payload/utils.go` | `fd76374577ec4ffc3a6b624f78930ac5d7f7dc94` | header names, content types and response codes |
| `client/client.go` | `c544c4852b28bb6c913f114e0c1349a5c5191829` | WebSocket read/ACK, ping/pong and reconnect behavior |

## Exact bounded wire definitions

From `payload/data_frame.go`:

```go
type DataFrame struct {
    SpecVersion string          `json:"specVersion"`
    Type        string          `json:"type"`
    Time        int64           `json:"time"`
    Headers     DataFrameHeader `json:"headers"`
    Data        string          `json:"data"`
}

type DataFrameResponse struct {
    Code    int             `json:"code"`
    Headers DataFrameHeader `json:"headers"`
    Message string          `json:"message"`
    Data    string          `json:"data"`
}
```

From `payload/utils.go`:

```go
DataFrameHeaderKTopic       = "topic"
DataFrameHeaderKContentType = "contentType"
DataFrameHeaderKMessageId   = "messageId"
DataFrameHeaderKTime        = "time"
DataFrameContentTypeKJson   = "application/json"
DataFrameResponseStatusCodeKOK = 200
```

The fixed client reads WebSocket text messages, dispatches frames by type/topic, copies the input `messageId` into the response header when absent, sends the JSON response, emits WebSocket ping frames, requires pong within a bounded interval, and reconnects after disconnect when auto-reconnect is enabled. System subscriptions include `disconnect` and `ping`; the `ping` handler returns a success response carrying the original message ID and data.

Only the definitions and behavior above are retained. Application handlers, logging, proxy support and unrelated SDK code are intentionally omitted.
