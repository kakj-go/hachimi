# File organization validation

- Confirm every source and destination remains within authorized roots.
- Recheck source hashes or etags immediately before mutation.
- Reject duplicate destinations, reserved names, unsafe links, and silent overwrite.
- After each batch, reconcile source count, destination count, hashes, failures, and rollback entries.
- Store only a controlled relative-path manifest; do not expose secret content or unrelated host paths.
