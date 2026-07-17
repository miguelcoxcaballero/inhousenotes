import { createPage } from '../core/model';
import type { Page } from '../core/model';
import { legacyPageToPage, legacyPayloadFromPdfKeywords } from '../persist/migrate';

export type ImportedPdfPage = Page;

interface PdfJsPage {
  getViewport(opts: { scale: number }): { width: number; height: number };
  render(opts: { canvasContext: CanvasRenderingContext2D; viewport: { width: number; height: number } }): { promise: Promise<void> };
}

interface PdfJsDocument {
  numPages: number;
  getPage(pageNumber: number): Promise<PdfJsPage>;
}

interface PdfJsRuntime {
  getDocument(opts: { data: Uint8Array; disableWorker: boolean }): { promise: Promise<PdfJsDocument> };
}

export async function importPdfPages(file: File): Promise<Page[]> {
  const bytes = new Uint8Array(await file.arrayBuffer());
  return importPdfBytes(bytes);
}

export async function importPdfUrl(url: string): Promise<Page[]> {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`PDF download failed: ${resp.status}`);
  return importPdfBytes(new Uint8Array(await resp.arrayBuffer()));
}

async function importPdfBytes(bytes: Uint8Array): Promise<Page[]> {
  const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs') as PdfJsRuntime;
  const pdf = await pdfjs.getDocument({ data: bytes, disableWorker: true }).promise;
  const pages: Page[] = [];
  const legacyPayload = await readLegacyPayload(bytes);

  for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber++) {
    const pdfPage = await pdf.getPage(pageNumber);
    const viewport = pdfPage.getViewport({ scale: 1.5 });
    const canvas = document.createElement('canvas');
    canvas.width = Math.max(1, Math.round(viewport.width));
    canvas.height = Math.max(1, Math.round(viewport.height));
    const ctx = canvas.getContext('2d');
    if (!ctx) continue;
    await pdfPage.render({ canvasContext: ctx, viewport }).promise;
    const rendered = createPage({
      width: viewport.width,
      height: viewport.height,
      background: { kind: 'custom', src: canvas.toDataURL('image/png') }
    });
    const legacy = legacyPayload?.pages?.[pageNumber - 1];
    if (legacy) {
      pages.push(legacyPageToPage({
        ...legacy,
        backgroundSource: 'custom',
        backgroundImage: rendered.background.kind === 'custom' ? rendered.background.src : undefined,
        pageWidth: viewport.width,
        pageHeight: viewport.height
      }));
    } else {
      pages.push(rendered);
    }
  }

  return pages;
}

async function readLegacyPayload(bytes: Uint8Array) {
  try {
    const { PDFDocument } = await import('pdf-lib');
    const copy = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
    const doc = await PDFDocument.load(copy, { ignoreEncryption: true });
    const rawKeywords = doc.getKeywords() as unknown;
    const keywords = Array.isArray(rawKeywords) ? rawKeywords.join(' ') : String(rawKeywords ?? '');
    return legacyPayloadFromPdfKeywords(keywords);
  } catch {
    return null;
  }
}
