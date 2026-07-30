# Inhouse Notes v5.3.0

- Peer-to-peer collaboration now continuously recovers from Drive fallback without reopening the document.
- Added connection timeouts, health checks, acknowledgements, bounded retry backoff and automatic recovery when the app or network returns.
- Healthy direct connections now stay active through temporary Drive-presence failures, and reconnect attempts continue automatically after a direct channel drops.
- Fixed encrypted signalling-key races between devices and prevented stale or undecryptable comments from causing repeated Drive requests.
- New peers always receive the current document immediately, even when nobody makes another edit.
- Fixed the first simultaneous edit sometimes remaining invisible until the next edit.
- Concurrent strokes, images, transforms and deletions now converge by stable IDs, causal stamps and durable tombstones.
- Undo and redo now create fresh causal writes, so restored or transformed content is not overwritten by an older deletion from another device.
- Page additions, deletions and reordering now use persistent causal metadata, so changes from several devices merge deterministically without update loops.
- Local and remote page-structure changes are serialized while their IndexedDB mapping is updated, preventing simultaneous moves, deletes and restores from crossing indices.
- Concurrent cross-page moves now resolve duplicate stroke or image locations deterministically and persist causal tombstones for every losing copy.
- Timeline restores causally revive deleted pages and no longer leak old document history or page data into a newly opened file.
- Concurrent document names, calendar settings, page backgrounds, templates, sizes and side panels now merge independently without devices echoing the same update back and forth.
- Legacy PDFs receive stable per-document page, stroke and image IDs, while simultaneous agenda-page additions are assigned distinct dates deterministically.
- Deferred Drive pulls survive overlapping downloads and must be merged before the next upload, preventing a delayed fallback sync from overwriting newer work.
- Collaboration metadata is stored in the PDF itself, preserving the same conflict handling through Drive fallback and after reopening.
