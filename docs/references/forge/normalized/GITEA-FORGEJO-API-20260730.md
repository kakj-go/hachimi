# Gitea/Forgejo PR normalized behavior snapshot

- One adapter targets the common v1 pull-request shape while retaining an explicit caller-supplied HTTPS API base for self-hosted instances.
- Create, query, title/body/target update, close, and merge are supported; the source ref is checked as immutable after creation.
- Merge sends the exact head commit ID in addition to Hachimi's local expected-revision check.
- API dialect drift fails closed as a protocol error rather than being silently repaired.
- Mutation uncertainty remains durable and must be reconciled by query before a follow-up action.
