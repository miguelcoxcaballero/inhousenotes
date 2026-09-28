# Inhouse Notes v5.11.39

- Dense-page erasing now restores only the damaged canvas region during the gesture instead of waiting for a full staged repaint to commit after pen-up. Both whole-stroke and area erasers preserve the underlying template, images and surviving ink. The final full-page repaint remains intact.
- Remote eraser previews use the same regional repaint path before the gesture finishes or a document snapshot arrives. Stale staged renders are cancelled so they cannot paint deleted ink back over the result.
- Opening a Drive document prefetches signalling-key and recent-comment reads while the PDF is downloading/decoding. Prefetch is read-only and scoped to the file and document session; it cannot announce an empty document, create a key or apply edits before the normal collaboration startup.
- Cold discovery fetches the key and comments concurrently instead of serially. Prefetched results are consumed once, expire after 15 seconds and fall back to fresh reads on failure; an absent prefetched key is rechecked before creation.
- Regression tests check visible erased pixels while a pen remains captured and moving, area-eraser preservation of surviving fragments, remote non-final eraser previews, read-only prefetch reuse/session isolation, and parallel cold discovery. Real WebRTC three-context tests also passed three repeated runs with simulated Drive latency.

The Android shell is unchanged. Network and Drive signalling latency still affect P2P establishment; this release removes avoidable waits but does not guarantee a universal sub-second connection.
