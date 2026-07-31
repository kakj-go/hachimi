# Gitea and Forgejo Pull Requests API contract snapshot

Retrieved: 2026-07-30T20:10:00+08:00

Official documentation and public Forgejo Swagger checked:

- Gitea API 1.22: https://docs.gitea.com/api/1.22/
- Forgejo API usage: https://forgejo.org/docs/latest/user/api-usage/
- Forgejo-compatible Swagger: https://codeberg.org/swagger.v1.json

Wire contract fixed for this implementation:

- `POST /repos/{owner}/{repo}/pulls` creates a PR with title/body and head/base refs.
- `GET /repos/{owner}/{repo}/pulls/{index}` returns PR refs, head SHA, state, URL, and update fields.
- `PATCH /repos/{owner}/{repo}/pulls/{index}` uses `EditPullRequestOption`; it updates title/body/base/state but has no source-ref replacement field.
- `POST /repos/{owner}/{repo}/pulls/{index}/merge` uses `MergePullRequestForm` with `Do=merge`, optional `MergeTitleField`/`MergeMessageField`, and `head_commit_id`.
- Requests use token authentication and a caller-supplied HTTPS API v1 base URL, allowing self-hosted Gitea or Forgejo.

Reviews, labels, assignments, update-branch, branch deletion, and instance administration are outside this snapshot.
