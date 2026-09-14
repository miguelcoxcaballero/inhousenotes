import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';
import test from 'node:test';

// Regression guard for the PDF-erase-leaves-a-trace bug: a PDF whose clean
// original bytes couldn't be captured at import time permanently loses the
// ability to erase strokes without a trace (see app-v5.js buildPdfBlob). That
// only happened for PDFs between 60MB and the app's real 64MB upload ceiling
// because the byte-capture cap was silently stricter than the accepted
// upload size. This pins the two constants together so they can't drift
// apart again without a human noticing.

const here = path.dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(path.join(here, '..', 'app-v5.js'), 'utf8');
const securitySource = readFileSync(path.join(here, '..', 'security-core-v5.js'), 'utf8');

test('security-core still gates PDF uploads at 64MB', () => {
  assert.match(securitySource, /MAX_PDF_BYTES\s*=\s*64\s*\*\s*1024\s*\*\s*1024/);
});

test('the clean-original capture cap is tied to securityCore.MAX_PDF_BYTES, not a smaller hardcoded value', () => {
  assert.match(appSource, /ORIGINAL_BYTES_MAX\s*=\s*securityCore\.MAX_PDF_BYTES\s*;/);
});
