# v5.17.5

Smaller live updates; exact final previews.

- Remove finished paths from transient Drive frames only after their exact stroke revision has been acknowledged in the durable journal. Subsequent pen samples no longer repeatedly transfer already-saved strokes.
- Preserve unfinished paths and newer revisions when an older receipt arrives. Confirmed ink remains in the journal and document; removing a preview never deletes the actual stroke.
- A final path replaces its earlier preview geometry, including shortened paths and multi-packet final batches. Receivers redraw the corrected preview without stale endpoints or premature save acknowledgements.
- Unit tests cover exact-revision retirement and final path replacement. Two isolated browser sessions verify live points, final geometry, durable delivery, and preview retirement with P2P disabled.

Applies to web and Android web assets; APK and frozen installer unchanged. Drive response times still determine end-to-end latency.
