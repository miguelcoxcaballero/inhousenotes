# v5.16.55

Isolate live ink and defer bulk sync.

- Draw progressive pen segments on the existing bounded lasso layer, which is idle during pen contact. Keep the large page bitmap unchanged until lift; then commit original geometry once at page resolution and clear the temporary layer. Keep standard canvas contexts.
- Defer full collaboration snapshots across short pen lifts until 1.2 seconds without input. Immediate stroke packets and the durable stroke journal remain active.
- Recognize Android tablet user agents without Mobile as tablets, including Samsung SM-X700. Use touch canvas limits instead of the desktop budget.
- Regenerate web/offline and Android resources. No document-format migration or frozen installer changes.

Validation includes live-layer pixels, zero page-bitmap stroke paints during contact, one commit on lift, screenshot/reopen/persistence checks, tablet canvas budget, and collaboration snapshot deferral. Headless CPU-throttled profiling did not reproduce the reported 2 FPS; no 9 ms hardware-latency claim is made.
