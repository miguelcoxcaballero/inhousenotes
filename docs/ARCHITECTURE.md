# Inhouse Notes web architecture

The root web application is served as static files and does not require an application server.

## Runtime boundaries

| File | Responsibility |
| --- | --- |
| `index.html` | Markup, styles and versioned dependency loading |
| `boot-v5.js` | Theme, OAuth callback and first-paint destination |
| `app-v5.js` | Editor orchestration, Drive/auth, persistence, PDF export and UI wiring |
| `runtime-core-v5.js` | Bounded requests, timeouts and retries |
| `security-core-v5.js` | Untrusted HTML sanitization and safe PDF loading policy |
| `timeline-core-v5.js` | Versioned checkpoints, deltas, validation and legacy Timeline decoding |
| `collaboration-core-v5.js` | Deterministic document/structure merge primitives |
| `live-collaboration-v5.js` | Browser, local-network, WebRTC and Drive rendezvous transports |

The pure core modules have no dependency on editor state. The orchestrator keeps the existing global editor API for compatibility while new domain logic should be added to the relevant core rather than returned to `index.html`.

## Timeline storage

Timeline schema v3 stores full snapshots at milestones and every eighth entry. Other entries contain changed page bodies, explicit removals, optional order changes and changed document fields. Each entry identifies its base and resulting snapshot hashes. A broken chain is rejected; legacy schema v2 arrays remain readable.

## Release verification

`npm run test:unit` runs deterministic merge, live transport, Timeline, timeout and static release checks. `npm run test:e2e` runs Chromium startup, persistence, security and multi-client chaos scenarios. Both suites are required by `.github/workflows/production-checks.yml` before a production change is considered healthy.
