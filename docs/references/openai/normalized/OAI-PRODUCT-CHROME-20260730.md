# Chrome extension behavior snapshot

- Reference ID: `OAI-PRODUCT-CHROME-20260730`
- Product behavior: the extension operates the user's existing signed-in Chrome profile, groups task tabs, and asks before interacting with a new website unless a site decision already exists.
- Capability boundary: Chrome extension permissions do not replace product confirmations, allowlists, blocklists, or scoped browser-history approval.
- Hachimi mapping: explicit native-host pairing, task-owned tabs/groups, exact site decisions, tab-scoped network rules, takeover cleanup, and separately granted history/CDP capabilities.
- Acceptance: extension tests must cover pairing nonce expiry, host identity, tab ownership, domain allow/block decisions, task-group cleanup, and denial of ungranted history or CDP access.
