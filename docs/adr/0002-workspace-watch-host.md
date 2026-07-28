# ADR-0002: Checkout-bound Workspace Watch Host

- Status: Accepted
- Date: 2026-07-25
- Owners: Harness Agent / Workbench

## Context

Existing Workspace operations intentionally launch short-lived workers. A recursive file watcher
is connection-scoped and long-running, but moving it into the Agent kernel or Tauri process would
break the filesystem boundary.

## Decision

1. Keep read/write/search/Exec as short-lived Worker requests.
2. Add a watch-server mode to the Workspace sidecar. It receives a checkout-bound, read-only token
   and exposes newline-delimited request/event envelopes over stdio.
3. The Desktop owns watch processes and binds each subscription to a Workbench window, Checkout,
   monotonically increasing generation, and cancellation token. Closing the window, changing the
   Checkout, unsubscribing, or cleaning the Checkout terminates the process.
4. Events contain relative paths and metadata only. They are coalesced, deduplicated, and tagged
   with the watch ID/generation. Queue overflow produces an `invalidate` event that forces an
   authoritative tree/Diff refresh.
5. Watch and fuzzy-search sessions are ephemeral. Run Diff manifests and baseline artifacts are
   persistent because completed Sessions must remain reviewable after restart.
6. Run Diff stores an immutable pre-write baseline per path and a persistent manifest. It computes
   Run scope independently from Checkout scope, and generation-fences recalculation triggered by
   Watch events.

## Consequences

- The Agent kernel never watches or enumerates the host filesystem directly.
- Watch crashes do not cancel an Agent Run; the UI reports degradation and can resubscribe.
- Search and Watch late results are rejected using generation fencing.
- The sidecar gains a long-lived mode, so cancellation, stdio framing, output bounds, and leak tests
  are mandatory.

## Source boundary

The lifecycle and invalidation boundaries were reviewed against the public Codex `file_watcher`,
`fuzzy_file_search`, and `turn_diff_tracker` implementations at fixed commit
`4c43465133428898aa84f0bfc02c306ed65fb66a`. Hachimi's Watch server, fuzzy-search session, and Run
Diff tracker are original implementations shaped around Hachimi's Workspace Host protocol and
SQLite manifests; no Codex implementation was copied or translated into those modules. A future
adaptation requires per-file provenance registration before editing.
