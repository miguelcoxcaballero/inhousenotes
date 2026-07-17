// Web Worker that builds PDF artifacts using pdf-lib.
// Renders each page with strokes on top of the original PDF (if any).
// Runs off the main thread to avoid blocking UI.

import type { JsonDoc, JsonPage } from '../core/serial';

interface PdfPageRuntime {
  getWidth(): number;
  getHeight(): number;
  drawRectangle(opts: object): void;
  drawLine(opts: object): void;
  drawImage(img: unknown, opts: object): void;
}

interface PdfDocumentRuntime {
  getPageCount(): number;
  getPage(index: number): PdfPageRuntime;
  addPage(size: [number, number]): PdfPageRuntime;
  embedFont(font: string): Promise<unknown>;
  embedJpg(bytes: Uint8Array): Promise<unknown>;
  embedPng(bytes: Uint8Array): Promise<unknown>;
  save(): Promise<Uint8Array>;
}

interface PdfLibRuntime {
  PDFDocument: {
    create(): Promise<PdfDocumentRuntime>;
    load(bytes: Uint8Array, opts?: object): Promise<PdfDocumentRuntime>;
  };
  StandardFonts: { Helvetica: string };
}

// ── Worker message types ───────────────────────────────────────────────────────

export interface BuildPdfMessage {
  type: 'build';
  doc: JsonDoc;
  originalPdfBase64?: string;
  quality: 'standard' | 'high';
}

export interface CancelMessage {
  type: 'cancel';
}

export type WorkerMessage = BuildPdfMessage | CancelMessage;

export interface PdfProgressResponse {
  type: 'progress';
  percent: number;
}

export interface PdfDoneResponse {
  type: 'done';
  blob: Blob;
}

export interface PdfErrorResponse {
  type: 'error';
  message: string;
}

export type WorkerResponse = PdfProgressResponse | PdfDoneResponse | PdfErrorResponse;

// ── PDF rendering constants ────────────────────────────────────────────────────

const DPI_STANDARD = 150;
const DPI_HIGH = 300;

const HIGHLIGHTER_ALPHA = 0.35;

// Page background colors
const BACKGROUND_COLORS: Record<string, string> = {
  default: '#FFFFFF',
  template: '#FFFFFF',
  agenda: '#FFFDE7',
  diary: '#FFF8E1'
};

// ── Worker entry point ─────────────────────────────────────────────────────────

let cancelled = false;

self.onmessage = async (event: MessageEvent<WorkerMessage>) => {
  const msg = event.data;

  if (msg.type === 'cancel') {
    cancelled = true;
    return;
  }

  if (msg.type !== 'build') return;

  cancelled = false;
  await buildPdf(msg.doc, msg.originalPdfBase64, msg.quality);
};

// ── PDF build logic ────────────────────────────────────────────────────────────

async function buildPdf(
  doc: JsonDoc,
  originalPdfBase64: string | undefined,
  quality: 'standard' | 'high'
): Promise<void> {
  try {
    const { PDFDocument, StandardFonts } = await loadPdfLib();

    const dpi = quality === 'high' ? DPI_HIGH : DPI_STANDARD;
    let pdf: PdfDocumentRuntime;

    // Load original PDF if provided
    if (originalPdfBase64) {
      const pdfBytes = base64ToBytes(originalPdfBase64);
      pdf = await PDFDocument.load(pdfBytes, { ignoreEncryption: true });
    } else {
      pdf = await PDFDocument.create();
    }

    const pageCount = doc.pages.length;
    await pdf.embedFont(StandardFonts.Helvetica);

    for (let i = 0; i < pageCount; i++) {
      if (cancelled) {
        postResponse({ type: 'error', message: 'Cancelled' });
        return;
      }

      const page = doc.pages[i]!;
      postResponse({ type: 'progress', percent: Math.round((i / pageCount) * 90) });

      // Get or create the page
      let pdfPage;
      if (originalPdfBase64 && i < pdf.getPageCount()) {
        // Use existing page from original PDF
        pdfPage = pdf.getPage(i);
      } else {
        // Create a new page
        const { width, height } = page;
        pdfPage = pdf.addPage([width, height]);
        fillPageBackground(pdfPage, page);
      }

      // Render strokes on top
      renderStrokesOnPage(pdfPage, page, dpi);

      // Render images
      await renderImagesOnPage(pdfPage, page, dpi, pdf);
    }

    postResponse({ type: 'progress', percent: 95 });

    // Save the PDF
    const pdfBytes = await pdf.save();
    const pdfBuffer = pdfBytes.buffer.slice(pdfBytes.byteOffset, pdfBytes.byteOffset + pdfBytes.byteLength) as ArrayBuffer;
    const blob = new Blob([pdfBuffer], { type: 'application/pdf' });

    postResponse({ type: 'progress', percent: 100 });
    postResponse({ type: 'done', blob });
  } catch (err) {
    postResponse({
      type: 'error',
      message: err instanceof Error ? err.message : String(err)
    });
  }
}

async function loadPdfLib(): Promise<PdfLibRuntime> {
  return await import('pdf-lib') as PdfLibRuntime;
}

function fillPageBackground(page: { getWidth: () => number; getHeight: () => number; drawRectangle: (opts: object) => void }, jsonPage: JsonPage): void {
  // Determine background color from template
  let bgColor = BACKGROUND_COLORS.default ?? '#FFFFFF';

  if (jsonPage.background && typeof jsonPage.background === 'object') {
    const bg = jsonPage.background as { kind?: string; template?: string };
    if (bg.kind === 'template' && bg.template && BACKGROUND_COLORS[bg.template]) {
      bgColor = BACKGROUND_COLORS[bg.template] ?? bgColor;
    }
  }

  // Draw background rectangle
  const color = hexToRgb(bgColor);

  page.drawRectangle({
    x: 0,
    y: 0,
    width: page.getWidth(),
    height: page.getHeight(),
    color
  });
}

function renderStrokesOnPage(
  page: { getHeight: () => number; drawLine: (opts: object) => void },
  jsonPage: JsonPage,
  dpi: number
): void {
  const scale = dpi / 96; // Base rendering at 96 DPI
  const pageHeight = page.getHeight();

  for (const stroke of jsonPage.strokes) {
    if (cancelled) return;
    if (stroke.points.length < 4) continue; // Need at least 2 points (6 values for x,y,p)

    const isHighlighter = stroke.tool === 'highlighter';
    const color = hexToRgb(stroke.color);
    const lineWidth = stroke.width * scale;

    // Draw the stroke as connected line segments
    for (let i = 0; i < stroke.points.length - 3; i += 3) {
      const x1 = stroke.points[i]! * scale;
      const y1 = pageHeight - stroke.points[i + 1]! * scale;
      const x2 = stroke.points[i + 3]! * scale;
      const y2 = pageHeight - stroke.points[i + 4]! * scale;

      page.drawLine({
        start: { x: x1, y: y1 },
        end: { x: x2, y: y2 },
        strokeColor: color,
        strokeWidth: lineWidth,
        opacity: isHighlighter ? HIGHLIGHTER_ALPHA : 1,
        lineCap: 1 // Round cap (pdf-lib uses numbers for line caps)
      });
    }
  }
}

async function renderImagesOnPage(
  page: { drawImage: (img: unknown, opts: object) => void; getHeight: () => number },
  jsonPage: JsonPage,
  dpi: number,
  pdf: { embedJpg: (bytes: Uint8Array) => Promise<unknown>; embedPng: (bytes: Uint8Array) => Promise<unknown> }
): Promise<void> {
  // Image rendering requires loading image bytes and embedding them
  for (const image of jsonPage.images) {
    if (cancelled) return;

    try {
      // Decode base64 image src
      const imageBytes = base64ToBytes(image.src);
      const isJpeg = image.src.startsWith('/9j/') || image.src.startsWith('data:image/jpeg');

      // Embed the image
      const embeddedImage = isJpeg
        ? await pdf.embedJpg(imageBytes)
        : await pdf.embedPng(imageBytes);

      const scale = dpi / 96;
      const pageHeight = page.getHeight();

      // Draw the image at the specified position
      page.drawImage(embeddedImage, {
        x: image.x * scale,
        y: pageHeight - image.y * scale - image.height * scale,
        width: image.width * scale,
        height: image.height * scale,
        rotate: image.rotation
          ? { type: 'degrees' as const, angle: image.rotation }
          : undefined
      });
    } catch {
      // Skip images that fail to render
    }
  }
}

// ── Utility functions ──────────────────────────────────────────────────────────

function hexToRgb(hex: string): { red: number; green: number; blue: number } {
  const clean = hex.replace('#', '');
  return {
    red: parseInt(clean.substring(0, 2), 16) / 255,
    green: parseInt(clean.substring(2, 4), 16) / 255,
    blue: parseInt(clean.substring(4, 6), 16) / 255
  };
}

function base64ToBytes(base64: string): Uint8Array {
  // Handle data URL prefix
  const cleanBase64 = base64.replace(/^data:image\/\w+;base64,/, '');
  const binary = atob(cleanBase64.replace(/-/g, '+').replace(/_/g, '/'));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) {
    bytes[i] = binary.charCodeAt(i);
  }
  return bytes;
}

function postResponse(response: WorkerResponse): void {
  self.postMessage(response);
}
