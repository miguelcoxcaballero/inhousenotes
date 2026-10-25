# v5.16.49

Preserve uploaded photo pages when saving.

- Decode local PNG/JPEG data URLs directly instead of fetching them through connect-src. The security policy permits displaying local images but intentionally blocks data-URL network requests; failed fetches previously produced successful PDFs with white pages.
- Reject an unreadable photo background instead of silently saving a blank PDF. The fallback renderer must also decode the image successfully.
- Do not reuse the live ink-only canvas for raster exports when its photo/PDF/template background lives in CSS; compose the background into the exported pixels.
- Regenerate web/offline/Android resources. The frozen installer is unchanged.

Validation: reproduce the white-page failure before the fix; check image pixels after local reload, PDF export, reopening and a second save, with and without an original PDF. Test raster fallback and rejection of unreadable image bytes.
