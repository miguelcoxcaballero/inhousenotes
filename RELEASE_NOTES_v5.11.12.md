# Inhouse Notes v5.11.12

## Lean and bounded GitHub Pages deployment

- Publishes only the production website instead of historical APKs, Android build sources, tests and prototypes.
- Reduces the Pages artifact from roughly 33.5 MB of repository content to the files the browser actually serves.
- Validates the entry page, CNAME, app runtime and scanner runtime before uploading.
- Retires an orphaned deployment for the same commit and creates a fresh attempt.
- Stops reporting a stalled attempt after five minutes instead of leaving a workflow apparently busy for tens of minutes.
- Keeps a single production queue and the approved HTTPS custom domain.

## Scanner retained from v5.11.8

- Distinguishes the lower square box from the page's actual yellow frame.
- Keeps every detected frame segment bright yellow throughout the processing animation.
- Uses the four printed colour circles to calibrate page colour.
- Produces brush-matched ink tones while preserving a clean white sheet and the printed yellow.
