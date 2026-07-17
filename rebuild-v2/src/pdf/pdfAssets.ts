import type { BinaryAsset } from '../persist/assets';
import type { Doc } from '../core/model';

interface PdfViewport {
  width: number;
  height: number;
}

interface PdfPage {
  getViewport(opts: { scale: number }): PdfViewport;
  render(opts: {
    canvasContext: CanvasRenderingContext2D;
    viewport: PdfViewport;
    annotationMode: number;
  }): { promise: Promise<void> };
}

interface PdfDocument {
  getPage(pageNumber: number): Promise<PdfPage>;
  destroy?(): Promise<void>;
}

interface PdfJsRuntime {
  getDocument(opts: { data: Uint8Array; disableWorker: boolean }): { promise: Promise<PdfDocument> };
}

const assets = new Map<string, BinaryAsset>();
const documents = new Map<string, Promise<PdfDocument>>();

export function registerPdfAssets(values: Iterable<BinaryAsset>): void {
  for (const asset of values) {
    assets.set(asset.id, asset);
    documents.delete(asset.id);
  }
}

export function unregisterPdfAssets(ids: Iterable<string>): void {
  for (const id of ids) {
    assets.delete(id);
    const doc = documents.get(id);
    documents.delete(id);
    void doc?.then((value) => value.destroy?.()).catch(() => undefined);
  }
}

export function getPdfAssetBytes(sourceId: string): Uint8Array | null {
  return assets.get(sourceId)?.bytes ?? null;
}

export function getPdfAsset(sourceId: string): BinaryAsset | null {
  return assets.get(sourceId) ?? null;
}

export function getPdfAssetsForDoc(doc: Doc): BinaryAsset[] {
  const result: BinaryAsset[] = [];
  const seen = new Set<string>();
  for (const pageId of doc.pageOrder) {
    const background = doc.pages.get(pageId)?.background;
    if (background?.kind !== 'pdf' || !background.sourceId || seen.has(background.sourceId)) continue;
    seen.add(background.sourceId);
    const asset = assets.get(background.sourceId);
    if (asset) result.push(asset);
  }
  return result;
}

export async function renderPdfBackground(
  sourceId: string,
  pageIndex: number,
  width: number,
  height: number
): Promise<HTMLCanvasElement | null> {
  const pdf = await loadDocument(sourceId);
  if (!pdf) return null;
  const page = await pdf.getPage(pageIndex + 1);
  const base = page.getViewport({ scale: 1 });
  const scale = Math.max(0.01, width / Math.max(1, base.width));
  const viewport = page.getViewport({ scale });
  const canvas = document.createElement('canvas');
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext('2d');
  if (!ctx) return null;
  ctx.save();
  ctx.setTransform(1, 0, 0, 1, 0, 0);
  ctx.fillStyle = '#fff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.restore();
  // Keep annotations out of the immutable background. Imported Ink
  // annotations live in the editable stroke layer instead.
  await page.render({ canvasContext: ctx, viewport, annotationMode: 0 }).promise;
  return canvas;
}

async function loadDocument(sourceId: string): Promise<PdfDocument | null> {
  const asset = assets.get(sourceId);
  if (!asset) return null;
  let pending = documents.get(sourceId);
  if (!pending) {
    pending = import('pdfjs-dist/legacy/build/pdf.mjs').then((module) => {
      const pdfjs = module as unknown as PdfJsRuntime;
      return pdfjs.getDocument({ data: asset.bytes.slice(), disableWorker: true }).promise;
    });
    documents.set(sourceId, pending);
  }
  return pending;
}
