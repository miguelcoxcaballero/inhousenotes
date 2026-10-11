# Inhouse Notes v5.11.51 · A

Live camera and simpler save status.

Android 1.0.14 (versionCode 15) includes the camera permission missing from previous APKs. Its package and signing key are unchanged. The live scanner tolerates the Android permission sheet hiding the WebView, waits for an advancing video feed, and offers retry if the camera is interrupted. It does not dismiss the scanner on a permission error. Install the new APK to enable the camera in the Android app; updating the hosted page alone cannot grant an absent native permission.

The save indicator is a borderless coloured dot with its accessible touch target preserved. The popup contains just the local save status and Google Drive status, an inline upload percentage, and one relevant action when needed. Error guidance appears only when action is needed; decorative symbols, repeated explanations and other-device details are removed. Hidden panels still avoid unnecessary rendering.

Validation covers live video, permission/background transitions, interruption and retry, confirmed page insertion, minimal save states and responsive layout. Existing BroadcastChannel, cached document hashes and real WebRTC coverage remains in place. The previously documented extreme/faded geometry limitation is unchanged.
