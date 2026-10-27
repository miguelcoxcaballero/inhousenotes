# v5.17.19

Less idle work, prompt ink synchronization.

- The durable ink journal sleeps until its next useful deadline when idle. In a 60-second fake-clock comparison, scheduling passes fell from 500 to 60 on Home and 72 in an empty document, with the same storage and network request counts.
- With 4,000 sealed sources, total source visits fell from 2,000,000 to 432,000, including the new planner. Pending edits from another tab remain discoverable within two seconds.
- New edits wake uploads promptly and incoming hints can advance reception independently. Earlier scheduled work is never postponed; 120 ms upload-start spacing, busy-lease probes, four read lanes, retry deadlines and stopped-session guards remain intact.
- Validation: 95 transport unit checks, seven browser synchronization/heartbeat cases, native-asset verification and release smoke. Deterministic tests cover stalled reads, concurrent writes, external-tab recovery, retry cooldowns and timer bounds.

Web and Android web assets updated; APK/frozen installer unchanged.
