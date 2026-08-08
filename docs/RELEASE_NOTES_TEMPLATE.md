These Windows binaries are currently unsigned.

## Verification Status

Local deterministic verification for this candidate includes Compaction retention and continuity, Connector action-effect authorization, App Server resume/rejoin, and low-resource Office Artifact E2E. The Desktop resume/rejoin matrix covers approval invalidation, user-input invalidation, read-only checkpoint resume, idempotent receipt reuse, and unknown side-effect recovery. Office E2E covers DOCX/XLSX/PPTX/PDF semantic assertions, malformed and traversal negatives, schema drift, interruption, restart idempotency, and artifact-reference-only model history.

The opt-in `test:desktop:stress` path uses a real managed Chromium through the BrowserHost API and a dedicated Win32 Computer fixture, with a 900-second maximum and a 2.5 GiB resource fuse. It is intended for low-resource short runs; the 600-second release soak is deliberately not part of this stage.

### External Verification Pending

| Status                | Scope                                          | Disclosure                                                                                                 |
| --------------------- | ---------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `Environment Blocked` | Real Provider and Forge staging                | No staging credentials, test repositories, or external accounts were available; no real requests were run. |
| `Environment Blocked` | Windows identity gates                         | Standard-user, elevated, security-desktop, and high-integrity environments were unavailable.               |
| `Environment Blocked` | Chrome/CEF and real Profile                    | No isolated real-user Chrome Profile or CEF/extension pairing was available.                               |
| `Environment Blocked` | Five-channel and enterprise organization gates | No real organizations, callback domains, credentials, or administrator grants were available.              |

`Environment Blocked` means not run because the required environment was unavailable; it is not a test failure.

Official packages include the default VRM whose license prohibits personal and corporate commercial use, so these packages are non-commercial distributions. The Hachimi source code remains Apache-2.0; removing or replacing that asset does not change the source-code license.

The attached `release-manifest.json` contains sanitized evidence for real OpenAI, five Forge environments, three external enterprise organizations, five Channel platforms, and both Windows release gates. It contains no API keys, tokens, raw platform messages, hidden reasoning, or attachment bodies.
