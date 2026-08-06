PRAGMA foreign_keys = ON;

CREATE TABLE provider_compatibility_profiles (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('openai_strict', 'registered_dialect')),
    protocols_json TEXT NOT NULL,
    profile_revision TEXT NOT NULL,
    builtin INTEGER NOT NULL CHECK (builtin IN (0, 1)),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

INSERT INTO provider_compatibility_profiles (
    id, display_name, kind, protocols_json, profile_revision, builtin, created_at_ms, updated_at_ms
) VALUES (
    'openai-strict', 'OpenAI strict', 'openai_strict',
    '["chat_completions","responses","embeddings"]',
    'openai-openapi-2.3.0', 1, 0, 0
);

CREATE TABLE provider_endpoints (
    id TEXT PRIMARY KEY NOT NULL,
    display_name TEXT NOT NULL,
    base_url TEXT NOT NULL,
    compatibility_profile_id TEXT NOT NULL REFERENCES provider_compatibility_profiles(id),
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    config_revision INTEGER NOT NULL CHECK (config_revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);

CREATE TABLE provider_accounts (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint_id TEXT NOT NULL REFERENCES provider_endpoints(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    secret_ref TEXT NOT NULL,
    enabled INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    config_revision INTEGER NOT NULL CHECK (config_revision > 0),
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    UNIQUE(endpoint_id, display_name)
);

INSERT INTO provider_endpoints (
    id, display_name, base_url, compatibility_profile_id, enabled,
    config_revision, created_at_ms, updated_at_ms
) VALUES (
    'default-openai', 'Default OpenAI-compatible endpoint',
    'http://localhost:11434/v1', 'openai-strict', 1, 1, 0, 0
);

INSERT INTO provider_accounts (
    id, endpoint_id, display_name, secret_ref, enabled,
    config_revision, created_at_ms, updated_at_ms
) VALUES (
    'default-openai', 'default-openai', 'Default account',
    'credential-manager:llm-api-key', 1, 1, 0, 0
);

CREATE TABLE provider_capability_probes (
    id TEXT PRIMARY KEY NOT NULL,
    endpoint_id TEXT NOT NULL REFERENCES provider_endpoints(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES provider_accounts(id) ON DELETE SET NULL,
    status TEXT NOT NULL CHECK (status IN ('succeeded', 'failed')),
    protocols_json TEXT NOT NULL,
    capabilities_json TEXT NOT NULL,
    capability_revision TEXT NOT NULL,
    stable_error_code TEXT,
    probed_at_ms INTEGER NOT NULL
);

CREATE INDEX provider_accounts_endpoint_idx
ON provider_accounts(endpoint_id, enabled, display_name);

CREATE INDEX provider_capability_probes_latest_idx
ON provider_capability_probes(endpoint_id, probed_at_ms DESC, id DESC);
