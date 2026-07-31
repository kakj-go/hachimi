# Browser behavior snapshot

- Reference ID: `OAI-PRODUCT-BROWSER-20260730`
- Product behavior: the built-in browser uses a profile separate from the user's regular browser, treats page content as untrusted, requires site permission by default, and separately confirms sensitive actions.
- Capability boundary: observe and act are distinct reviewable operations; full CDP access is an explicit developer-mode capability with an additional approval boundary.
- Hachimi mapping: managed Chromium uses an isolated profile, exact document/resource origins, capability grants, private-network policy, observation fencing, and explicit sensitive-action approval.
- Acceptance: Browser Host tests must cover isolated state, origin/capability denial, prompt-injection handling, stale observations, download/upload controls, and CDP being disabled unless explicitly granted.
