# Inhouse Notes v5.11.2

## Save completion

- Removed the obsolete previous-save blocking screen from document opening.
- Home now absorbs any lifecycle save already started by page hiding or app
  backgrounding before it becomes visible.
- The editor remains present beneath the normal save transition until the exact
  current content version is durably checkpointed and confirmed in Drive.
- A recovery marker left by an actual browser or app termination retries
  silently when the same document is opened; it never blocks another document.

## Regression protection

- Release checks now reject the removed copy and any future attempt to await a
  previous exit-save promise inside the document-opening path.
- All unit, collaboration, persistence, and browser resilience tests pass.
