# v5.16.8

Faster Drive save preparation.

- Start PDF warmup after a short writing pause; repeated save requests no longer postpone it.
- Share one PDF build between background preparation and foreground saves, including when content changes during a build. Reuse the exact prepared PDF for an unchanged document session and version.
- Reuse template JPEGs, stroke lists and overlay bounds across exports without changing PDF resolution, editable strokes or clean originals.
- Compress stroke metadata while computing its sync identity in the processing worker. Encode timeline JSON without cloning every snapshot again; retain identical archive bytes and integrity hashes.
- Cancel obsolete preparation waits promptly. Show “Checking Drive” during remote verification rather than leaving it labelled “Preparing”; keep remote merge checks and durable local saves.
- Applies to the web app and Android's web assets. No new APK or updater catalog is required.
