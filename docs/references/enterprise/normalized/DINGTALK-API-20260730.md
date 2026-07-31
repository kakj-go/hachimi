# DingTalk normalized behavior snapshot

- Credentials are a tenant-bound `appKey`, `appSecret`, optional robot code and Stream client identity stored only in Credential Manager.
- Access tokens are cached only in memory and refreshed once after an authentication failure.
- Department/member reads enforce official cursor/page limits.
- Outbound text messages bind account, peer/thread, request identity and delivery ledger entry.
- Stream events are acknowledged only after durable ingress acceptance; duplicate event IDs replay the stored receipt.
- Disconnect, throttling and transient errors use persisted backoff and restart reconciliation.
- Event payloads cannot carry Grants or bypass the authenticated Scheduler EventSource ingress.
