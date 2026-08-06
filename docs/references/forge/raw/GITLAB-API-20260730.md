# GitLab Merge Requests API contract snapshot

Retrieved: 2026-07-30T20:10:00+08:00

Official documentation checked with HTTP 200:

- Merge requests API: https://docs.gitlab.com/api/merge_requests/

Wire contract fixed for this implementation:

- `POST /projects/:id/merge_requests` creates an MR with `source_branch`, `target_branch`, `title`, and `description`.
- `GET /projects/:id/merge_requests/:merge_request_iid` returns refs, SHA, state, web URL, and update fields used for reconciliation.
- `PUT /projects/:id/merge_requests/:merge_request_iid` updates `title`, `description`, `target_branch`, or closes with `state_event=close`. It does not replace `source_branch`.
- `PUT /projects/:id/merge_requests/:merge_request_iid/merge` accepts expected `sha` and an optional merge commit message.
- Requests use `PRIVATE-TOKEN` authentication and percent-encode the namespace/project ID.

Notes, approvals, reviewers, labels, assignments, pipelines, and source-branch deletion are outside this snapshot.
