# Inhouse Notes v5.11.50 · A

Live camera document scanning.

Opening the scanner now opens a live rear-camera preview. The yellow overlay follows the detected page frame and observed bottom calibration boxes. A steady, sharp page triggers a three-frame burst; the clearest frame is straightened and enlarged for review. Choose another frame, scan again, or explicitly add the page to the document. Photo import remains available when a camera cannot be used.

Camera tracks stop on close, review and backgrounding, including when a permission prompt resolves after closing. Detection runs on reduced frames; document correction runs only after capture. The Notes Android builder now declares the camera permission, but previously installed APKs need a new signed APK before WebView live scanning is available. No Android binary or update channel catalog is changed by this web release.

Validation covers synthetic video capture, yellow-frame geometry, blur rejection, permission denial, late camera grants, retakes, confirmed durable insertion, and responsive layouts. Existing BroadcastChannel, cached document hashes and real WebRTC regression coverage remains in place. No editor or saving implementation was changed.
