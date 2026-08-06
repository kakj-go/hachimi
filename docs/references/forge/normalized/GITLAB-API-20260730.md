# GitLab MR normalized behavior snapshot

- Project identity, remote URL hash, refs, exact source SHA, expected MR revision, approval, and idempotency key are fenced before mutation.
- Create, query, title/description/target update, close, and merge are supported. The source branch is an immutable precondition after creation.
- Merge passes the expected source SHA to GitLab to close the query-to-merge race.
- Mutation results are normalized to the same PR/MR record used by every Forge adapter.
- Unknown post-dispatch outcomes remain `indeterminate` until a remote query proves state.
