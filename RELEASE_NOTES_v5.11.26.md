# Inhouse Notes v5.11.26

## Responsive editing and safer scan cleanup

- Background Drive PDF preparation now waits for a longer idle window and aborts as soon as drawing or another interaction resumes, yielding the CPU and shared canvas work to the editor.
- Scanner cleanup learns a more conservative repeated dot pattern and removes isolated dots only when their local shape supports a printed-dot match.
- Local-contrast protection preserves continuous handwritten strokes and their antialiased edges, including faint marks that the illumination estimate cannot resolve.

## Verification

- Passed all 102 unit and release smoke tests and all 26 Playwright browser tests.
- Ran scanner visual validation against all four supplied photos; page, frame and footer-box detection succeeded on each.
- Visually compared the supplied page with cleanup enabled and disabled.

## Android

- The existing Android 1.0.10 shell loads this hosted web release automatically; no native APK change is required.
