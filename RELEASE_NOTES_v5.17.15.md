# v5.17.15

Wake remote ink promptly; keep incomplete photos pending.

- New live-ink activity preempts an existing preview sleep instead of waiting up to 15 seconds. Static file listings no longer extend activity or reset retry deadlines.
- Recent per-device heartbeats already present in Drive metadata wake matching previews even without P2P. Repeated, stale, unrelated or implausibly future signals do not repeatedly wake a source; bounded clock skew remains supported.
- Hints arriving during a read produce one follow-up after that read. Concurrent read limits, network backoff and invalid-payload cooldown remain enforced.
- Home/exit checkpoints now share the normal save's photo/body consistency guard. An interrupted photo update cannot bypass it by leaving the editor; valid local repairs retain their ink and reopen offline.

- Full CI checks now finish their current run while newer releases queue, so frequent updates do not continually cancel verification.

- Validation: 75 transport unit tests, 10 local-save checks, native-asset and release smoke checks, and 26 browser cases including heartbeat wakeup, repeated/skewed signals, interrupted recovery, Home and offline reopening.

Web and Android web assets updated; APK/frozen installer unchanged.
