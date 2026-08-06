CREATE TABLE integration_probe_snapshots (
    account_id TEXT PRIMARY KEY NOT NULL REFERENCES integration_provider_accounts(id) ON DELETE CASCADE,
    credential_json TEXT NOT NULL,
    ingress_json TEXT NOT NULL,
    egress_json TEXT NOT NULL,
    api_json TEXT NOT NULL,
    probed_at_ms INTEGER NOT NULL
);
