# Inhouse Notes v5.2.1

This patch fixes Editor links and makes their behavior explicit and consistent.

- Copy link now preserves the selected role instead of silently changing every link to Viewer.
- Editor links carry their intended role and open the real Inhouse Notes editor after Google Drive authorization.
- Signed-in recipients use the same canvas, toolbars, page management, timeline, autosave, and collaboration UI as the document owner.
- Recipients who open an Editor link while signed out see the normal document UI with a focused “Sign in to edit” action.
- Viewer links remain available without sign-in and keep the anonymous public-view behavior introduced in v5.2.0.
- Google Drive writes still use OAuth; no access token or owner credential is placed in a public sharing URL.
