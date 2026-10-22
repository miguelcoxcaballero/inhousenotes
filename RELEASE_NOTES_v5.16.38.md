# v5.16.38

Use the active Home session for background uploads.

- Home’s hidden PDF processor now obtains authentication from the visible app,
  including token renewal, instead of requiring a separate stored access token.
- Long-running uploads share the same refresh owner. A Drive 401 invalidates
  only the rejected token, preserving a newer token refreshed concurrently.
- Account identity is checked before and after authentication. Signing out or
  changing accounts stops the processor from obtaining another account’s token.
- Standalone Android background processing retains its persisted-auth path.

Validation: real-PDF browser regressions cover Home with no stored access token,
server-side token expiry and retry, concurrent-edit preservation, conditional
uploads, and an account change during authentication. Offline/native assets
regenerated for the existing Android APK; no installer changes.
