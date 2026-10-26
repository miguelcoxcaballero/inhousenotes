# v5.16.58

Draw final pen segments during contact.

- Remove the lower-resolution live pen surface and lift-time full-stroke redraw. Append measured pen segments directly to the definitive page bitmap each frame; lift only adds any missing release samples and persists the stroke.
- Draw the first pen dot on that same bitmap. Remove the unused provisional pen-tail renderer. Keep frame batching and incremental segment work; no full-page replay while writing.
- Set logical-to-bitmap coordinates explicitly in both axes during drawing and repaint, including fractional-size backing buffers. Preserve raw captured points and rounded joins without curve smoothing.
- Keep highlighter behavior and standard canvas contexts; regenerate web/offline and Android resources. Frozen installer unchanged.

Validation covers identical page pixels before/after lift with no new sample, definitive ink during contact, coordinate alignment at 1.25/2.25/3.5 scales, sharp corners, persistence/reopen, zoom and contact recovery. Actual tablet frame rate is not established by these tests.
