# Inhouse Notes v5.11.36

- PDF preparation now embeds losslessly compressed RGB and alpha samples directly from the background worker. The overlay no longer needs PNG encoding followed by PNG decoding and JavaScript recompression. Unchanged overlays keep using the existing revision-safe cache.
- Imported PNGs retain pdf-lib's decoded samples, with native compression before serialization. No resolution or quality reduction is introduced.
- Compressed ASCII metadata is written as a standard PDF literal string rather than being expanded into UTF-16 hexadecimal text. Other metadata retains the Unicode-safe path. Stroke restoration, Timeline and collaboration metadata remain embedded in the PDF.
- Peer discovery uses response-paced polling: 300ms between checks during initial negotiation and 700ms otherwise, rather than a fixed 2.5-second interval. Existing connections continue discovering a third device. There is no overlapping scheduled poll and failures back off up to 15 seconds.
- Fresh encrypted rendezvous signals trigger negotiation immediately instead of waiting for the supervisor tick. Network changes reopen the fast discovery window. Healthy channels and the existing Drive fallback remain intact.
- Concurrent duplicate SDP answers are applied only once. Crossed offers keep a deterministic initiator until the channel opens, and a newer offer can replace an obsolete answer session instead of waiting for its timeout. Speculative ICE candidate pools are no longer allocated for each short-lived negotiation.
- Regression tests verify pixel-identical rendered PDFs and metadata readback, duplicate-poll suppression, error backoff, and real WebRTC handshakes among three isolated browser contexts/accounts using mocked Drive signalling.

## Measurements and limits

The 2,400-stroke / 240,000-point desktop Chromium fixture took approximately 2.0 seconds to prepare, versus approximately 3.4 seconds in v5.11.35. PDF size fell from roughly 1.84 MB to 1.22 MB. This excludes Drive upload time and does not guarantee the same timings on mobile devices.

Peer discovery/negotiation starts automatically; connection establishment still depends on Drive signalling latency, ICE and network restrictions. Universal sub-second connectivity cannot be guaranteed without changing the infrastructure.

The native Android wrapper is unchanged and loads the hosted runtime; no new APK is required.
