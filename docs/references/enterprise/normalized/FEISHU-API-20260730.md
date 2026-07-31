# Feishu normalized behavior snapshot

- Credentials are a tenant-bound `appId`, `appSecret` and long-connection identity stored only in Credential Manager.
- Tenant tokens are cached only in memory and refreshed once after an authentication failure.
- Directory reads preserve official page token semantics and enforce local bounds.
- Outbound text messages bind receive ID type, peer/thread, idempotency identity and the durable delivery ledger.
- Long-connection events are acknowledged only after durable ingress acceptance and are deduplicated by tenant/account/event ID.
- Disconnect, throttling and transient errors use persisted backoff and restart reconciliation.
- Attachment bodies require an explicit bounded download and Artifact fencing before entering a Run.
