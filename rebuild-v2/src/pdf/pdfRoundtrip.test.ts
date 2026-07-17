import { describe, expect, it } from 'vitest';
import { createDoc, createPage, makeStroke, packPoints } from '../core/model';
import { applyOp, removeStrokesOp } from '../core/ops';
import { exportDocPdf } from '../export/pdf';
import { importPdfBytes } from '../import/pdfImport';

describe('editable PDF Ink compatibility', () => {
  it('exports and reimports app strokes as removable PDF Ink annotations', async () => {
    const page = createPage({ width: 240, height: 320 });
    const stroke = makeStroke({
      tool: 'pen',
      color: '#123456',
      width: 3,
      points: packPoints([
        { x: 20, y: 30, p: 0.4 },
        { x: 40, y: 55, p: 0.6 },
        { x: 80, y: 75, p: 0.5 }
      ])
    });
    page.strokes.set(stroke.id, stroke);
    page.strokeOrder.push(stroke.id);
    const doc = createDoc({ pages: [page] });

    const blob = await exportDocPdf(doc);
    const bytes = new Uint8Array(await blob.arrayBuffer());
    const pdfjs = await import('pdfjs-dist/legacy/build/pdf.mjs');
    const pdf = await pdfjs.getDocument({ data: bytes.slice() }).promise;
    const annotations = await (await pdf.getPage(1)).getAnnotations({ intent: 'display' });
    expect(annotations.some((annotation) => annotation.subtype === 'Ink')).toBe(true);

    const imported = await importPdfBytes(bytes, 'roundtrip.pdf');
    expect(imported.pages).toHaveLength(1);
    expect(imported.pages[0]!.strokeOrder).toHaveLength(1);
    const importedStrokeId = imported.pages[0]!.strokeOrder[0]!;
    expect(imported.pages[0]!.strokes.get(importedStrokeId)!.color).toBe('#123456');

    const importedDoc = createDoc({ pages: imported.pages });
    applyOp(importedDoc, removeStrokesOp(importedDoc.pageOrder[0]!, [importedStrokeId]));
    expect(importedDoc.pages.get(importedDoc.pageOrder[0]!)!.strokes.has(importedStrokeId)).toBe(false);
  });
});
