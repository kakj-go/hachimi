# GitHub Pull Requests REST API contract snapshot

Retrieved: 2026-07-30T20:10:00+08:00

Official documentation checked with HTTP 200:

- Pull requests: https://docs.github.com/en/rest/pulls/pulls?apiVersion=2022-11-28
- Merge a pull request: https://docs.github.com/en/rest/pulls/pulls?apiVersion=2022-11-28#merge-a-pull-request

Wire contract fixed for this implementation:

- `POST /repos/{owner}/{repo}/pulls` creates a pull request with `title`, `body`, `head`, and `base`.
- `GET /repos/{owner}/{repo}/pulls/{pull_number}` returns the current PR, head SHA, refs, state, and revision fields used for reconciliation.
- `PATCH /repos/{owner}/{repo}/pulls/{pull_number}` updates `title`, `body`, `base`, or `state`. The existing `head` ref is not replaceable by this endpoint.
- `PUT /repos/{owner}/{repo}/pulls/{pull_number}/merge` accepts the expected head `sha` plus optional commit title/message.
- Requests use `Accept: application/vnd.github+json`, `X-GitHub-Api-Version: 2022-11-28`, and bearer authentication.

Comments, reviews, labels, assignments, checks, and branch deletion are outside this snapshot.
