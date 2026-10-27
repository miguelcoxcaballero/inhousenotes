# v5.17.17

Sleep between useful live-sync polls.

- Quiet preview sources now sleep until their next discovery, freshness or retry deadline instead of running an empty scheduling pass every 100 ms.
- In a 15-second fake-clock comparison, idle scheduling fell from 151 passes to 7 while preserving the exact network request counts and timestamps. Inactive valid frames fell from 151 passes to 19 with the same requests.
- Fresh hints still preempt idle sleep immediately. Active or stalled requests retain bounded scheduling, four read lanes and existing retry deadlines; nonfinite or oversized timer delays cannot create a busy loop.
- Validation: 83 journal/live-preview unit checks, seven browser synchronization/heartbeat cases, native-asset verification and release smoke. No stroke, photo or durable save payload changes.

Web and Android web assets updated; APK/frozen installer unchanged.
