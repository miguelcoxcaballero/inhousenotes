# v5.17.2

- Drive now carries unfinished strokes through a separate transient preview channel. It prewarms a per-session live file in the same `inhouse notes` folder, then sends the newest samples while the pen is still down. Existing WebRTC remains the fastest route when available.
- Live writes coalesce at a 100 ms minimum interval; active direct-ID reads target 100 ms between completed reads. Frames contain complete recent paths, so missing a polling response cannot strand the next stroke segment behind a gap. Rendering appends new points on the next animation frame without an artificial playback delay.
- Durable stroke writes and active journal reads target 120 ms, reduced from 600/450 ms. Receiving/merging no longer holds the upload scheduler while a local pen contact pauses heavy work.
- Preview files never acknowledge a durable save. Confirmed journals clear temporary overlays; cancelled, stale and wrong-document previews are ignored. PDF compilation, local checkpoints and conditional merge protection remain separate.
- Live discovery hints defer PDF metadata changes during a save, preserving v5.17.1's fix for presence-related upload conflicts. The Android installer is unchanged.

Tests use simulated Drive requests and isolated browser contexts with P2P disabled to verify an unfinished, growing stroke is visible remotely before pen-up. Timing targets describe local scheduling, not guaranteed end-to-end latency: real latency and throttling still depend on Google Drive and the connection.
