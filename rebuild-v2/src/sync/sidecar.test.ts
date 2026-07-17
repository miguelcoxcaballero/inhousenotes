import { afterEach, describe, expect, it } from 'vitest';
import { createDoc, createPage } from '../core/model';
import { registerPdfAssets, unregisterPdfAssets } from '../pdf/pdfAssets';
import { loadBundle, packDoc, serializeSidecar, unpackBundle } from './sidecar';

afterEach(() => unregisterPdfAssets(['pdf-source']));

describe('Drive sidecar', () => {
  it('round-trips a referenced source PDF once', () => {
    const first = createPage();
    const second = createPage();
    first.background = { kind: 'pdf', sourceId: 'pdf-source', pdfPageIndex: 0 };
    second.background = { kind: 'pdf', sourceId: 'pdf-source', pdfPageIndex: 1 };
    const doc = createDoc({ name: 'Samsung', pages: [first, second] });
    doc.pageTombstones.add('deleted-page');
    registerPdfAssets([{
      id: 'pdf-source',
      name: 'Samsung.pdf',
      mimeType: 'application/pdf',
      bytes: new Uint8Array([1, 2, 3, 4, 255]),
      createdAt: 42
    }]);

    const packed = packDoc(doc, 100, { writeId: 'write-2', parentWriteId: 'write-1' });
    const bundle = loadBundle(packed);

    expect(bundle).not.toBeNull();
    expect(bundle!.assets).toHaveLength(1);
    expect([...bundle!.assets[0]!.bytes]).toEqual([1, 2, 3, 4, 255]);
    expect(bundle!.doc.pages.get(first.id)!.background).toEqual(first.background);
    expect(bundle!.doc.pageTombstones.has('deleted-page')).toBe(true);
    expect(bundle!.writeId).toBe('write-2');
    expect(bundle!.parentWriteId).toBe('write-1');
  });

  it('still opens version 1 sidecars without assets', () => {
    const page = createPage();
    const doc = createDoc({ pages: [page] });
    const current = serializeSidecar(doc, 321) as unknown as Record<string, unknown>;
    current.version = 1;
    current.sidecarVersion = 1;
    delete current.assets;
    delete current.writeId;
    delete current.parentWriteId;

    const bundle = unpackBundle(JSON.stringify(current));
    expect(bundle).not.toBeNull();
    expect(bundle!.assets).toEqual([]);
    expect(bundle!.doc.id).toBe(doc.id);
    expect(bundle!.writeId).toContain(`legacy-${doc.id}-321`);
  });

  it('stores a Drive reference without repeating the PDF bytes', () => {
    const page = createPage();
    page.background = { kind: 'pdf', sourceId: 'pdf-source', pdfPageIndex: 0 };
    const doc = createDoc({ pages: [page] });
    const packed = packDoc(doc, 500, {
      inlineAssets: false,
      assetRefs: [{
        id: 'pdf-source',
        name: 'source.pdf',
        mimeType: 'application/pdf',
        createdAt: 44,
        fileId: 'drive-source-file'
      }]
    });

    const bundle = unpackBundle(packed)!;
    expect(bundle.assets).toEqual([]);
    expect(bundle.assetRefs).toEqual([{
      id: 'pdf-source',
      name: 'source.pdf',
      mimeType: 'application/pdf',
      createdAt: 44,
      fileId: 'drive-source-file'
    }]);
    expect(packed).not.toContain('AQIDBP8');
  });
});
