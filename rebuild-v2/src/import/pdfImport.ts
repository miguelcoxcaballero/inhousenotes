import { newId } from '../core/ids';
import { createPage, makeStroke, packPoints } from '../core/model';
import type { Page } from '../core/model';
import type { BinaryAsset } from '../persist/assets';
import { legacyPageToPage, legacyPayloadFromPdfKeywords } from '../persist/migrate';

export type ImportedPdfPage = Page;

export interface ImportedPdfDocument {
  pages: Page[];
  assets: BinaryAsset[];
}

interface PdfViewport {
  width: number;
  height: number;
  convertToViewportPoint(x: number, y: number): [number, number];
}

interface PdfInkAnnotation {
  id?: string;
  annotationType?: number;
  subtype?: string;
  inkLists?: ArrayLike<number>[];
  color?: ArrayLike<number>;
  opacity?: number;
  borderStyle?: { width?: number };
}

interface PdfJsPage {
  getViewport(opts: { scale: number }): PdfViewport;
  getAnnotations(opts: { intent: 'display' }): Promise<PdfInkAnnotation[]>;
}

interface PdfJsDocument {
  numPages: number;
  getPage(pageNumber: number): Promise<PdfJsPage>;
  destroy?(): Promise<void>;
}

interface PdfJsRuntime {
  getDocument(opts: { data: Uint8Array; disableWorker: boolean }): { promise: Promise<PdfJsDocument> };
}

const IMPORT_SCALE = 1.5;
const PDF_ANNOTATION_INK = 15;

export async function importPdfDocument(file: File): Promise<ImportedPdfDocument> {
  return importPdfBytes(new Uint8Array(await file.arrayBuffer()), file.name);
}

export async function importPdfUrl(url: string): Promise<ImportedPdfDocument> {
  const resp = await fetch(url);
  if (!resp.ok) throw new Error(`PDF download failed: ${resp.status}`);
  const name = filenameFromUrl(url);
  return importPdfBytes(new Uint8Array(await resp.arrayBuffer()), name);
}

export async function importPdfBytes(
  bytes: Uint8Array,
  name = 'document.pdf'
): Promise<ImportedPdfDocument> {
  const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs') as unknown as PdfJsRuntime;
  const pdf = await pdfjs.getDocument({ data: bytes.slice(), disableWorker: true }).promise;
  const sourceId = newId();
  const pages: Page[] = [];
  const legacyPayload = await readLegacyPayload(bytes);

  try {
    for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber++) {
      const pdfPage = await pdf.getPage(pageNumber);
      const viewport = pdfPage.getViewport({ scale: IMPORT_SCALE });
      const legacy = legacyPayload?.pages?.[pageNumber - 1];
      const page = legacy
        ? legacyPageToPage({
            ...legacy,
            backgroundSource: 'pdf',
            pdfPageIndex: pageNumber - 1,
            pageWidth: viewport.width,
            pageHeight: viewport.height
          })
        : createPage({ width: viewport.width, height: viewport.height });

      page.background = { kind: 'pdf', sourceId, pdfPageIndex: pageNumber - 1 };

      // Legacy Inhouse PDFs already carry their editable strokes in metadata.
      // Otherwise import standard PDF Ink annotations as normal app strokes.
      if (!legacy) {
        const annotations = await pdfPage.getAnnotations({ intent: 'display' });
        appendInkAnnotations(page, annotations, viewport);
      }
      pages.push(page);
    }
  } finally {
    await pdf.destroy?.();
  }

  return {
    pages,
    assets: [{
      id: sourceId,
      name,
      mimeType: 'application/pdf',
      bytes: bytes.slice(),
      createdAt: Date.now()
    }]
  };
}

function appendInkAnnotations(
  page: Page,
  annotations: PdfInkAnnotation[],
  viewport: PdfViewport
): void {
  for (const annotation of annotations) {
    if (annotation.annotationType !== PDF_ANNOTATION_INK && annotation.subtype !== 'Ink') continue;
    const color = annotationColor(annotation.color);
    const opacity = Number.isFinite(annotation.opacity) ? Number(annotation.opacity) : 1;
    const tool = opacity < 0.75 ? 'highlighter' : 'pen';
    const width = Math.max(0.7, Number(annotation.borderStyle?.width ?? 1) * IMPORT_SCALE);

    for (const list of annotation.inkLists ?? []) {
      const values = Array.from(list);
      const points: { x: number; y: number; p: number }[] = [];
      for (let i = 0; i + 1 < values.length; i += 2) {
        const [x, y] = viewport.convertToViewportPoint(values[i]!, values[i + 1]!);
        if (Number.isFinite(x) && Number.isFinite(y)) points.push({ x, y, p: 0.5 });
      }
      if (points.length === 0) continue;
      if (points.length === 1) points.push({ ...points[0]! });
      const stroke = makeStroke({ tool, color, width, points: packPoints(points) });
      page.strokes.set(stroke.id, stroke);
      page.strokeOrder.push(stroke.id);
    }
  }
}

function annotationColor(color: ArrayLike<number> | undefined): string {
  if (!color || color.length < 3) return '#000000';
  const parts = [Number(color[0]), Number(color[1]), Number(color[2])].map((value) => {
    const scaled = value <= 1 ? value * 255 : value;
    return Math.max(0, Math.min(255, Math.round(scaled))).toString(16).padStart(2, '0');
  });
  return `#${parts.join('')}`;
}

function filenameFromUrl(url: string): string {
  try {
    const name = new URL(url).pathname.split('/').pop();
    return name && /\.pdf$/i.test(name) ? decodeURIComponent(name) : 'document.pdf';
  } catch {
    return 'document.pdf';
  }
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
