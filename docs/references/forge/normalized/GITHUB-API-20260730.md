# GitHub PR normalized behavior snapshot

- A repository is bound by Forge kind, HTTPS API base, owner/repository, remote URL SHA-256, source/target refs, and commit OID.
- Create, query, title/body/target update, close, and merge are supported. The source ref is an immutable precondition after creation.
- Every update/close/merge first queries the PR and compares expected revision and head commit.
- Merge requires a fresh high-risk interactive approval and sends the expected head SHA to the merge endpoint.
- A transport failure after dispatch is `indeterminate`; Hachimi queries remote state before any user-authorized follow-up and never reports an unconfirmed mutation as complete.
