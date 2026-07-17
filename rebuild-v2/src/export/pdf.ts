import { PDFDocument, PDFString, StandardFonts, degrees, rgb } from 'pdf-lib';
import type { Doc, Img, Page, Stroke } from '../core/model';
import { getPdfAssetBytes } from '../pdf/pdfAssets';

export async function exportDocPdf(doc: Doc): Promise<Blob> {
  const pdf = await PDFDocument.create();
  const font = await pdf.embedFont(StandardFonts.Helvetica);

  for (const pageId of doc.pageOrder) {
    const modelPage = doc.pages.get(pageId);
    if (!modelPage) continue;
    const page = pdf.addPage([modelPage.width, modelPage.height]);
    page.drawRectangle({
      x: 0,
      y: 0,
      width: modelPage.width,
      height: modelPage.height,
      color: rgb(1, 1, 1)
    });

    await drawBackground(pdf, page, modelPage, font);
    for (const id of modelPage.imageOrder) {
      const image = modelPage.images.get(id);
      if (image) await drawImage(pdf, page, modelPage, image);
    }
    for (const id of modelPage.strokeOrder) {
      const stroke = modelPage.strokes.get(id);
      if (stroke) addInkAnnotation(pdf, page, modelPage, stroke);
    }
  }

  const bytes = await pdf.save();
  const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
  return new Blob([buffer], { type: 'application/pdf' });
}

export function downloadBlob(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

async function drawBackground(
  pdf: PDFDocument,
  page: ReturnType<PDFDocument['addPage']>,
  modelPage: Page,
  font: Awaited<ReturnType<PDFDocument['embedFont']>>
): Promise<void> {
  if (modelPage.background.kind === 'custom') {
    await drawDataImage(pdf, page, modelPage.background.src, 0, 0, modelPage.width, modelPage.height, 0);
  } else if (modelPage.background.kind === 'pdf' && modelPage.background.sourceId) {
    const bytes = getPdfAssetBytes(modelPage.background.sourceId);
    if (bytes) {
      const [embedded] = await pdf.embedPdf(bytes, [modelPage.background.pdfPageIndex]);
      if (embedded) {
        page.drawPage(embedded, {
          x: 0,
          y: 0,
          width: modelPage.width,
          height: modelPage.height
        });
      }
    }
  } else if (modelPage.background.kind === 'template') {
    const lineColor = rgb(0.88, 0.91, 0.95);
    const step = modelPage.background.template === 'diary' ? 28 : 32;
    for (let y = step; y < modelPage.height; y += step) {
      page.drawLine({
        start: { x: 0, y: modelPage.height - y },
        end: { x: modelPage.width, y: modelPage.height - y },
        thickness: 0.5,
        color: lineColor
      });
    }
  }

  if (modelPage.sidePanel) {
    const panelWidth = Math.min(220, modelPage.width * 0.28);
    page.drawRectangle({
      x: modelPage.width - panelWidth,
      y: 0,
      width: panelWidth,
      height: modelPage.height,
      color: rgb(0.97, 0.98, 0.99)
    });
    page.drawLine({
      start: { x: modelPage.width - panelWidth, y: 0 },
      end: { x: modelPage.width - panelWidth, y: modelPage.height },
      thickness: 0.8,
      color: rgb(0.70, 0.76, 0.84)
    });
    page.drawText(modelPage.sidePanel.mode === 'week' ? 'Week' : 'Day', {
      x: modelPage.width - panelWidth + 16,
      y: modelPage.height - 34,
      size: 14,
      font,
      color: rgb(0.06, 0.09, 0.16)
    });
    modelPage.sidePanel.dateKeys.slice(0, 7).forEach((key, index) => {
      page.drawText(key, {
        x: modelPage.width - panelWidth + 16,
        y: modelPage.height - 62 - index * 20,
        size: 11,
        font,
        color: rgb(0.29, 0.33, 0.41)
      });
    });
  }
}

async function drawImage(
  pdf: PDFDocument,
  page: ReturnType<PDFDocument['addPage']>,
  modelPage: Page,
  image: Img
): Promise<void> {
  await drawDataImage(
    pdf,
    page,
    image.src,
    image.x - image.width / 2,
    modelPage.height - image.y - image.height / 2,
    image.width,
    image.height,
    image.rotation
  );
}

async function drawDataImage(
  pdf: PDFDocument,
  page: ReturnType<PDFDocument['addPage']>,
  src: string,
  x: number,
  y: number,
  width: number,
  height: number,
  rotationRadians: number
): Promise<void> {
  const bytes = dataUrlToBytes(src);
  if (!bytes) return;
  const embedded = isJpeg(src) ? await pdf.embedJpg(bytes) : await pdf.embedPng(bytes);
  page.drawImage(embedded, {
    x,
    y,
    width,
    height,
    rotate: rotationRadians ? degrees((rotationRadians * 180) / Math.PI) : undefined
  });
}

function addInkAnnotation(
  pdf: PDFDocument,
  page: ReturnType<PDFDocument['addPage']>,
  modelPage: Page,
  stroke: Stroke
): void {
  if (stroke.points.length < 3) return;
  const points: number[] = [];
  for (let i = 0; i + 1 < stroke.points.length; i += 3) {
    points.push(stroke.points[i]!, modelPage.height - stroke.points[i + 1]!);
  }
  if (points.length === 2) points.push(points[0]!, points[1]!);

  const color = hexComponents(stroke.color);
  const padding = Math.max(1, stroke.width);
  const rect = [
    Math.max(0, stroke.bbox.x0 - padding),
    Math.max(0, modelPage.height - stroke.bbox.y1 - padding),
    Math.min(modelPage.width, stroke.bbox.x1 + padding),
    Math.min(modelPage.height, modelPage.height - stroke.bbox.y0 + padding)
  ];
  const annotation = pdf.context.obj({
    Type: 'Annot',
    Subtype: 'Ink',
    Rect: rect,
    InkList: [points],
    C: color,
    CA: stroke.tool === 'highlighter' ? 0.35 : 1,
    BS: { Type: 'Border', W: stroke.width, S: 'S' },
    F: 4,
    NM: PDFString.of(stroke.id),
    Contents: PDFString.of('Inhouse Notes ink')
  });
  page.node.addAnnot(pdf.context.register(annotation));
}

function dataUrlToBytes(src: string): Uint8Array | null {
  const comma = src.indexOf(',');
  if (comma < 0) return null;
  const binary = atob(src.slice(comma + 1));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes;
}

function isJpeg(src: string): boolean {
  return /^data:image\/jpe?g/i.test(src) || src.startsWith('/9j/');
}

function hexToRgb(hex: string): ReturnType<typeof rgb> {
  const [r, g, b] = hexComponents(hex);
  return rgb(r, g, b);
}

function hexComponents(hex: string): [number, number, number] {
  const clean = hex.replace('#', '').padEnd(6, '0');
  return [
    parseInt(clean.slice(0, 2), 16) / 255,
    parseInt(clean.slice(2, 4), 16) / 255,
    parseInt(clean.slice(4, 6), 16) / 255
  ];
}
