# Inhouse Notes v5.1.0

This release makes multi-device editing faster, clearer, and more reliable.

- Edits are broadcast immediately instead of waiting for a later edit to wake a pending update.
- Content-based revision identities prevent devices from repeatedly uploading the same logical document back to each other.
- Pure remote changes no longer get mistaken for unsaved local edits, eliminating Drive update loops.
- Concurrent local and remote strokes, images, page additions, removals, reordering, calendar panels, document names, and Timeline state continue to merge through the existing document model.
- The connected-devices popup now identifies the main device and shows whether each device is connected peer-to-peer, over the local network, or through the Drive fallback.
- The save-dot popup now gives a simple per-device activity summary and states what revision Google Drive has saved.
- Returning to Home is now local-first and appears in under a second while the final Drive PDF upload safely continues in the background.
- Opening another document waits for the previous background save, so fast navigation cannot replace unsaved document state.
