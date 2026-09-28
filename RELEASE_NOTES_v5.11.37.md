# Inhouse Notes v5.11.37

- Drive upload response-body reads retain their timeout and cancellation. Receiving HTTP headers alone no longer disables the watchdog while completion JSON is still pending. A timeout queries the resumable session to recover confirmed progress instead of blindly uploading again.
- Upload chunks are 1 MiB rather than 4 MiB, shortening the interval between server-confirmed progress updates and limiting retransmission on unstable connections. This does not increase the network's bandwidth.
- Expired sessions are discarded. Repeated acknowledgements without progress fail explicitly after four attempts, retaining the unsaved document rather than claiming success or looping.
- Browser regressions cover a stalled completion body, dropped chunks, and repeated zero-progress responses.
- The hosted runtime updates without a new Android APK. Existing lossless PDF preparation and P2P behaviour are unchanged.
