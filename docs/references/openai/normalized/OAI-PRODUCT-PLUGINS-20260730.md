# Plugins behavior snapshot

- Reference ID: `OAI-PRODUCT-PLUGINS-20260730`
- Product behavior: plugins are installable bundles that may provide skills, connectors, MCP servers, browser integration, scheduled-task templates, assets, or custom UI.
- Capability boundary: installation and enablement do not bypass the Codex host's sandbox/approval policy or an external service's own authentication and access controls.
- Hachimi mapping: manifest/content-hash validation, contribution revisions, permission-diff review, sandboxed runtime bindings, connector account authority, and no implicit grant from plugin metadata.
- Acceptance: tests must cover malformed bundles, revision drift, permission expansion review, connector/MCP identity pinning, disabled contributions, and uninstall/restart reconciliation.
