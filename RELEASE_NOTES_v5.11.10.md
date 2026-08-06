# Inhouse Notes v5.11.10

## Reliable GitHub Pages publishing

- Replaces the deploy action's hard-coded ten-minute cancellation window with a durable Pages API monitor.
- Allows a slow GitHub Pages backend up to 35 minutes to finish the same deployment.
- Leaves valid server-side work active if monitoring times out instead of cancelling it.
- Resumes an existing healthy deployment when a workflow run is retried, avoiding competing uploads.
- Keeps a single production queue so releases cannot overtake one another.

## Scanner retained from v5.11.8

- Distinguishes the lower square box from the page's actual yellow frame.
- Keeps every detected frame segment bright yellow throughout the processing animation.
- Uses the four printed colour circles to calibrate page colour.
- Produces brush-matched ink tones while preserving a clean white sheet and the printed yellow.
