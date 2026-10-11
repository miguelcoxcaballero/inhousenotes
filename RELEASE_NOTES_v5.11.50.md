# Inhouse Notes v5.11.50 · A

Live scanning and clearer saving.

Opening the scanner now opens a live rear-camera preview. The yellow overlay follows the detected page frame and observed bottom calibration boxes. A steady, sharp page triggers a three-frame burst; the clearest frame is straightened and enlarged for review. Choose another frame, scan again, or explicitly add the page to the document. Photo import remains available when a camera cannot be used.

Camera tracks stop on close, review and backgrounding, including when a permission prompt resolves after closing. Detection runs on reduced frames; document correction runs only after capture. The Notes Android builder now declares the camera permission, but previously installed APKs need a new signed APK before WebView live scanning is available. No Android binary or update channel catalog is changed by this web release.

Validation covers synthetic video capture, yellow-frame geometry, blur rejection, permission denial, late camera grants, retakes, confirmed durable insertion, and responsive layouts. Existing BroadcastChannel, cached document hashes and real WebRTC regression coverage remains in place. The scanner work is merged with the independent editor and save-status improvements.

Known existing limitation: the legacy full-resolution geometry test exceeds its tolerances for the nearly grey `faded-yellow` and strongly curled `strong-right-curl-rose` fixtures. Both errors reproduce with identical values on baseline `1738a84`; the correction algorithm and those assertions are unchanged. Review the captured page before adding it, especially with faded or curled paper.

Save details distinguish the local copy from Google Drive, show upload progress and explain offline, reconnect and failure states. The status button is separate from the read/edit toggle, with keyboard opening, Escape dismissal and focus restoration.

Local save failures remain visible until a successful local write. Full checkpoints retry up to three times, including pending edits in read mode; retries and late results stay tied to their document session. Confirmed local timestamps no longer borrow a Drive save time.

Closed save details and hidden page thumbnails avoid unnecessary rendering, and reopening Manage Pages no longer adds duplicate drag handlers.
