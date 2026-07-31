# Computer Use behavior snapshot

- Reference ID: `OAI-PRODUCT-COMPUTER-20260730`
- Product behavior: Computer Use is opt-in, operates selected desktop applications, supports per-app controls, and keeps the user able to take over.
- Capability boundary: app access and consequential actions remain reviewable; protected, elevated, login, and secure desktop surfaces are not ordinary controllable targets.
- Hachimi mapping: app/window-scoped observe and act, frame/app/window fingerprint fencing, same-integrity `SendInput`, takeover, high-integrity/security-desktop rejection, and memory-only PNG frames.
- Acceptance: tests must cover app allowlists, stale frames, takeover, Hachimi/self-window rejection, elevated/security desktop denial, expiry, capacity limits, and zero new screenshot files.
