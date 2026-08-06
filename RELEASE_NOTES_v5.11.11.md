# Inhouse Notes v5.11.11

## Fresh GitHub Pages retries

- Retires an orphaned deployment for the same commit before a retry.
- Always uploads a fresh Pages artifact instead of attaching to a stale `deployment_in_progress` record.
- Keeps a single production queue and monitors the new deployment without the official action's ten-minute cancellation.
- Preserves the custom domain, its approved HTTPS certificate and the workflow-based publishing configuration.

## Scanner retained from v5.11.8

- Distinguishes the lower square box from the page's actual yellow frame.
- Keeps every detected frame segment bright yellow throughout the processing animation.
- Uses the four printed colour circles to calibrate page colour.
- Produces brush-matched ink tones while preserving a clean white sheet and the printed yellow.

The deployment monitor allows up to 35 minutes for a genuinely slow GitHub backend, but retries no longer wait on a known orphaned attempt.
