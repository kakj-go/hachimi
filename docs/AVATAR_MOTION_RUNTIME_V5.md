# Avatar Motion Runtime V5

## Architecture

Runtime V5 is a breaking replacement for V4. It does not read the V4 motion catalog or preserve
the old fixed fade fields. The frame path is fixed and tested:

`product event -> BehaviorScheduler -> MotionIntent -> AnimationGraph -> TransitionPlanner -> pose composition -> full-pose inertialization -> interaction feedback -> foot contact/IK -> face/gaze/lip sync -> SpringBone`

- `@hachimi/pet` owns product priority, input coalescing, interruption and deadlines.
- `@hachimi/avatar-motion-runtime` owns feature sampling, slot arbitration, transition planning,
  composition and continuity math.
- `hachimi-motion` owns the V5 catalog, import validation, immutable built-ins and user metadata.
- Motion Lab owns transition inspection and asset diagnostics.

The runtime uses one winner for each `base`, `locomotion`, `speech` and `action` slot. Direct pet
code does not call `AnimationAction`; all VRMA data is sampled into semantic poses before layers
are composed.

## V4 Baseline

The breaking rewrite is measured against the last V4 source revision, not against inferred product
claims. V4 scheduled touch, pat and generic click actions with a fixed `130 ms` delay, blocked a
pending interaction while any foreground motion was active, then imposed a further `350 ms` idle
return settle. Every catalog motion used fixed `220 ms` transition-in and `260 ms` transition-out
envelopes, and continuity correction only covered rotation with a fixed `75 ms` half-life. Drag,
speech and locomotion did not share a transition-entry search or a per-frame continuity gate.

Because V4 did not record rendered bone/root/LookAt frames, historical single-frame jump values
cannot be reconstructed honestly after its removal. V5 therefore establishes the first replayable
numeric baseline: direct local feedback `<=80 ms`, full-body entry `<=120 ms`, ordinary bone step
`<12 degrees`, root step `<0.5%` avatar height, contact-foot drift `<1.5%` avatar height and LookAt
step `<4 degrees`. Desktop E2E records these channels from the production frame pipeline.

## Transition Model

Each retargeted motion builds a 60 Hz feature index keyed by skeleton signature, asset SHA-256 and
feature algorithm version. Frames contain pose, angular/root/expression/LookAt velocity, loop
phase, contact state and entry/exit eligibility. The planner searches the first 120 ms or declared
entry windows and minimizes pose `0.45`, velocity `0.25`, foot contact `0.20` and root `0.10`.
Indexes are serialized through the desktop motion service into `motions-v5/features/` with atomic,
size-bounded writes. Invalid or stale cache records are ignored and rebuilt asynchronously; cache
failure never blocks the render frame.

The selected target is dead-blended from the last rendered pose and velocity. Half-lives are root
100 ms, body 80 ms, arms 65 ms, LookAt 60 ms and expressions 50 ms. Runtime output additionally
limits ordinary bone changes to 12 degrees/frame, root changes to 0.5% avatar height/frame and
LookAt changes to 4 degrees/frame. IK runs after this step and therefore cannot be overwritten by
the transition solver.

## Interaction Policy

Priorities are drag 100, touch/pat 90, user locomotion 80, speech 70, autonomous action 40 and
idle 10. Continuous pat and drag updates mutate one feedback state and never restart a VRMA.
Local pose/expression/gaze feedback is applied on the next rendered frame; a cached full-body
reaction selects a safe target phase immediately and may never wait beyond 120 ms. Speech uses
the speech slot and cannot displace drag or touch feedback.

The built-in bundle does not expose locomotion clips. `action_recover_to_idle` is the only derived
recovery entry and samples the first 260 ms of the pinned generic `waiting` motion. The locomotion
slot and stage controller remain runtime extension points for future user or plugin motion sets;
missing locomotion leaves the avatar stationary.

Motion Lab exposes source and target feature frames, foot contact, pose coverage, transition cost,
root trajectory, loop seam and per-frame bone/root/LookAt peaks. Its bounded core transition matrix
uses the same automated admission thresholds as runtime verification; it is a diagnostic surface,
not a separate playback implementation.

## Failure Rules

- Generic `waiting` is the permanent base layer. On first load the hidden model prepares
  `waiting`, then `appearing`; the first visible frame already contains both sampled layers.
  `appearing` and every later one-shot action fade away to the continuously running base, so no
  Rest/T-pose frame is exposed between motions.
- User VRMA is inspected before catalog commit. Finite accessors, monotonic key times, humanoid
  tracks and core skeleton coverage are validated. Successful imports enter the `unknown` family
  with `analysisStatus=ready` and the conservative profile. Failed analysis remains visible as a
  diagnostic entry with `analysisStatus=failed`, is automatically disabled, and cannot be bound,
  enabled or resolved as a runtime asset.
- A missing locomotion motion leaves the avatar stationary and does not substitute idle sliding.
- Model switches clear compiled poses, feature indexes, intent state, feedback and constraints.

## Sources And Status

Pinned sources, versions, licenses and implementation status are maintained in
`docs/references/avatar-motion/registry.json` and included in release source summaries. Full Motion
Matching is explicitly out of scope; the implemented feature search only chooses transition entry
frames.

## Verification

Unit and integration tests cover shortest-path quaternion deltas, pose/velocity feature indexes,
persistent cache restoration, contact matching, profile half-life overrides, safe-exit deadlines,
slot arbitration, derived source-range sampling, interaction input merging and failed user
admission. Desktop E2E drives the production event boundary for appearing/waiting startup, the full
core transition matrix, action return to waiting, touch/pat/drag, speech, a ready user VRMA, a
missing user VRMA fallback and VRM model replacement/restoration while recording continuity
metrics from rendered frames.
