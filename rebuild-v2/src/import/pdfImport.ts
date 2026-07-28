import { newId } from '../core/ids';
import { createPage, makeStroke, packPoints } from '../core/model';
import type { Page } from '../core/model';
import type { BinaryAsset } from '../persist/assets';
import { legacyPageToPage, legacyPayloadFromPdfKeywords } from '../persist/migrate';
import { loadPdfJsRuntime } from '../pdf/pdfJs';
import { extractFlattenedInk } from './flattenedInk';
import type { RecoveredInkStroke } from './flattenedInk';

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
  render(opts: {
    canvasContext: CanvasRenderingContext2D;
    viewport: PdfViewport;
    annotationMode: number;
  }): { promise: Promise<void> };
}

interface PdfJsDocument {
  numPages: number;
  getPage(pageNumber: number): Promise<PdfJsPage>;
  destroy?(): Promise<void>;
}

interface PdfJsRuntime {
  getDocument(opts: { data: Uint8Array }): { promise: Promise<PdfJsDocument> };
}

const MODEL_SCALE = 1;
const MAX_RECOVERY_RASTER_DIMENSION = 2200;
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
  const pdfjs = await loadPdfJsRuntime<PdfJsRuntime>();
  const pdf = await pdfjs.getDocument({ data: bytes.slice() }).promise;
  const sourceId = newId();
  const pages: Page[] = [];
  const assets: BinaryAsset[] = [];
  const legacyPayload = await readLegacyPayload(bytes);

  try {
    for (let pageNumber = 1; pageNumber <= pdf.numPages; pageNumber++) {
      const pdfPage = await pdf.getPage(pageNumber);
      const viewport = pdfPage.getViewport({ scale: MODEL_SCALE });
      const legacy = legacyPayload?.pages?.[pageNumber - 1];
      const page = legacy
        ? legacyPageToPage({
            ...legacy,
            backgroundSource: 'pdf',
            pdfPageIndex: pageNumber - 1,
            pageWidth: legacy.pageWidth ?? viewport.width,
            pageHeight: legacy.pageHeight ?? viewport.height
          })
        : createPage({ width: viewport.width, height: viewport.height });

      page.background = { kind: 'pdf', sourceId, pdfPageIndex: pageNumber - 1 };

      // Prefer real stroke data. Some legacy PDFs have metadata but no live
      // strokes, so standard PDF Ink still needs to be considered.
      const annotations = await pdfPage.getAnnotations({ intent: 'display' });
      const annotationCount = page.strokeOrder.length === 0
        ? appendInkAnnotations(page, annotations, viewport)
        : 0;

      // Samsung and some other note apps flatten handwriting into a raster.
      // Recover only pen-like coloured ink on a light neutral background.
      if (page.strokeOrder.length === 0 && page.imageOrder.length === 0 && annotationCount === 0) {
        const recovered = await recoverFlattenedInk(pdfPage, page, name, pageNumber);
        if (recovered) {
          page.background = { kind: 'pdf', sourceId: recovered.asset.id, pdfPageIndex: 0 };
          appendRecoveredStrokes(page, recovered.strokes);
          assets.push(recovered.asset);
        }
      }
      pages.push(page);
    }
  } finally {
    await pdf.destroy?.();
  }

  assets.unshift({
    id: sourceId,
    name,
    mimeType: 'application/pdf',
    bytes: bytes.slice(),
    createdAt: Date.now()
  });
  const referenced = new Set(pages.flatMap((page) =>
    page.background.kind === 'pdf' && page.background.sourceId ? [page.background.sourceId] : []
  ));
  return { pages, assets: assets.filter((asset) => referenced.has(asset.id)) };
}

function appendInkAnnotations(
  page: Page,
  annotations: PdfInkAnnotation[],
  viewport: PdfViewport
): number {
  let added = 0;
  for (const annotation of annotations) {
    if (annotation.annotationType !== PDF_ANNOTATION_INK && annotation.subtype !== 'Ink') continue;
    const color = annotationColor(annotation.color);
    const opacity = Number.isFinite(annotation.opacity) ? Number(annotation.opacity) : 1;
    const tool = opacity < 0.75 ? 'highlighter' : 'pen';
    const width = Math.max(0.7, Number(annotation.borderStyle?.width ?? 1) * MODEL_SCALE);

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
      added++;
    }
  }
  return added;
}

async function recoverFlattenedInk(
  pdfPage: PdfJsPage,
  page: Page,
  sourceName: string,
  pageNumber: number
): Promise<{ asset: BinaryAsset; strokes: RecoveredInkStroke[] } | null> {
  if (typeof document === 'undefined') return null;
  const base = pdfPage.getViewport({ scale: 1 });
  const recoveryScale = Math.max(1, Math.min(2, MAX_RECOVERY_RASTER_DIMENSION / Math.max(base.width, base.height)));
  const viewport = pdfPage.getViewport({ scale: recoveryScale });
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(viewport.width));
  canvas.height = Math.max(1, Math.round(viewport.height));
  const ctx = canvas.getContext('2d', { willReadFrequently: true });
  if (!ctx) return null;
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  await pdfPage.render({ canvasContext: ctx, viewport, annotationMode: 0 }).promise;
  const image = ctx.getImageData(0, 0, canvas.width, canvas.height);
  const extracted = extractFlattenedInk(image, page.width, page.height);
  if (!extracted) return null;

  const cleaned = ctx.createImageData(canvas.width, canvas.height);
  cleaned.data.set(extracted.cleaned);
  ctx.putImageData(cleaned, 0, 0);
  const pngBytes = await canvasPngBytes(canvas);
  const pdfBytes = await rasterBackgroundPdf(pngBytes, page.width, page.height);
  const id = newId();
  const stem = sourceName.replace(/\.pdf$/i, '') || 'document';
  return {
    asset: {
      id,
      name: `${stem}.editable-background-${pageNumber}.pdf`,
      mimeType: 'application/pdf',
      bytes: pdfBytes,
      createdAt: Date.now()
    },
    strokes: extracted.strokes
  };
}

function appendRecoveredStrokes(page: Page, recovered: RecoveredInkStroke[]): void {
  for (const candidate of recovered) {
    const stroke = makeStroke({
      tool: 'pen',
      color: candidate.color,
      width: candidate.width,
      points: candidate.points
    });
    page.strokes.set(stroke.id, stroke);
    page.strokeOrder.push(stroke.id);
  }
}

async function canvasPngBytes(canvas: HTMLCanvasElement): Promise<Uint8Array> {
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, 'image/png'));
  if (!blob) throw new Error('Could not encode recovered PDF background');
  return new Uint8Array(await blob.arrayBuffer());
}

async function rasterBackgroundPdf(
  pngBytes: Uint8Array,
  width: number,
  height: number
): Promise<Uint8Array> {
  const { PDFDocument } = await import('pdf-lib');
  const pdf = await PDFDocument.create();
  const page = pdf.addPage([width, height]);
  const image = await pdf.embedPng(pngBytes);
  page.drawImage(image, { x: 0, y: 0, width, height });
  return pdf.save();
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
