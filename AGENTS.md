# Inhouse Notes release policy

This file is mandatory guidance for any AI or developer editing this repository.

## Version bump required for every upload

Every push or upload to GitHub `main` must increment the public app version,
including documentation-only or small UI changes. Do not publish two different
commits with the same visible version.

The single source of truth is:

```js
const APP_VERSION = 'x.y.z';
```

in the root `index.html`. The home and editor profile menus use
`[data-app-version]` and must display that value as `vx.y.z`. Do not hardcode
the release number separately in either profile menu.

Use semantic versioning:

- Patch (`x.y.Z`) by default for fixes, polish, documentation, and small features.
- Minor (`x.Y.0`) for a substantial group of new user-facing capabilities.
- Major (`X.0.0`) only for an intentionally incompatible release.

## Required release checks

Before pushing to `main`:

1. Increment `APP_VERSION` in the root `index.html`.
2. Confirm every `[data-app-version]` label resolves to the new number.
3. Regenerate any affected standalone or Android artifacts.
4. Run the relevant tests, type checks, and builds.
5. Commit the version bump together with the change it identifies.

After pushing:

1. Verify GitHub `main` points to the new commit.
2. Fetch `https://inhousenotes.com/` without cache and confirm it serves the new
   visible version.
3. Do not report the release as published until both checks pass.
