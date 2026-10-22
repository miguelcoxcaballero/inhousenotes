# v5.16.44

Reduce pen stalls between handwritten strokes.

- Keep per-stroke journal writes immediate while postponing full-page checkpoints until 1.2 seconds of input inactivity. Explicit save-and-exit still bypasses the idle gate.
- Stop cancelled or obsolete page copies between small batches instead of finishing every dense-page clone after writing resumes.
- Apply the same writing idle window to offline document snapshots.
- Reject a delayed offline capture if a newer checkpoint from the same editor session is already durable.
- Android offline/native assets regenerated; installer unchanged.

A Chromium CPU profile with 3,000 existing strokes (100 points each), twelve pen contacts, 250 ms gaps and 4× CPU throttling showed 70 frame gaps above 50 ms before, versus one after; maximum gap fell from 683 ms to 67 ms. These are controlled desktop measurements, not a device-level frame-rate guarantee.

Regression checks cover immediate operation-journal writes without page compaction between letters, recovery of all new strokes after restart, and stopping a cancelled full-page copy at its first yield.
