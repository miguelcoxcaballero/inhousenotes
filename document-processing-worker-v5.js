'use strict';
importScripts(`timeline-core-v5.js${self.location.search}`);
importScripts(`collaboration-core-v5.js${self.location.search}`);
let nextObject = 0;
const documents = new Map();
const objects = new Map();
function remember(value, owner) {
    const id = ++nextObject;
    objects.set(id, { value, owner });
    return id;
}
function disposeDocument(id) {
    documents.delete(id);
    for (const [key, object] of objects) if (object.owner === id) objects.delete(key);
}

// Same quadratic paths and highlighter blending as the interactive renderer.
function drawStroke(ctx, stroke) {
    const points = stroke.points || [];
    if (!points.length || stroke.tool === 'eraser-stroke') return;
    if (stroke.tool === 'highlighter') {
        const width = Math.max(4, stroke.width || 12);
        ctx.save();
        ctx.globalCompositeOperation = 'multiply';
        ctx.strokeStyle = ctx.fillStyle = stroke.color || '#ffde00';
        ctx.lineCap = ctx.lineJoin = 'round';
        if (points.length === 1) {
            ctx.globalAlpha = 0.24;
            ctx.beginPath(); ctx.arc(points[0].x, points[0].y, width * 0.52, 0, Math.PI * 2); ctx.fill();
        } else {
            for (const [alpha, lineWidth] of [[0.22, width], [0.1, Math.max(2, width * 0.62)]]) {
                ctx.globalAlpha = alpha; ctx.lineWidth = lineWidth;
                ctx.beginPath(); ctx.moveTo(points[0].x, points[0].y);
                for (let i = 1; i < points.length - 1; i++) {
                    ctx.quadraticCurveTo(points[i].x, points[i].y,
                        (points[i].x + points[i + 1].x) / 2, (points[i].y + points[i + 1].y) / 2);
                }
                ctx.lineTo(points[points.length - 1].x, points[points.length - 1].y); ctx.stroke();
            }
        }
        ctx.restore(); return;
    }
    if (points.length < 2) return;
    ctx.globalAlpha = 1;
    ctx.globalCompositeOperation = stroke.tool === 'eraser-area' ? 'destination-out' : 'source-over';
    ctx.strokeStyle = stroke.tool === 'eraser-area' ? 'rgba(0,0,0,1)' : stroke.color;
    ctx.lineWidth = stroke.width; ctx.lineCap = ctx.lineJoin = 'round';
    ctx.beginPath(); ctx.moveTo(points[0].x, points[0].y);
    if (points.length > 2) {
        ctx.lineTo((points[0].x + points[1].x) / 2, (points[0].y + points[1].y) / 2);
        for (let i = 1; i < points.length - 1; i++) {
            ctx.quadraticCurveTo(points[i].x, points[i].y,
                (points[i].x + points[i + 1].x) / 2, (points[i].y + points[i + 1].y) / 2);
        }
    }
    ctx.lineTo(points[points.length - 1].x, points[points.length - 1].y); ctx.stroke();
}

async function compress(value) {
    const stream = new Blob([value]).stream().pipeThrough(new CompressionStream('deflate'));
    const bytes = new Uint8Array(await new Response(stream).arrayBuffer());
    let binary = '';
    for (let offset = 0; offset < bytes.length; offset += 16384) {
        binary += String.fromCharCode(...bytes.subarray(offset, offset + 16384));
    }
    return btoa(binary);
}

async function deflateBytes(bytes) {
    return new Uint8Array(await new Response(
        new Blob([bytes]).stream().pipeThrough(new CompressionStream('deflate'))
    ).arrayBuffer());
}

async function preparePdfRaster(canvas) {
    const { width, height } = canvas;
    const rgba = canvas.getContext('2d').getImageData(0, 0, width, height).data;
    const rgb = new Uint8Array(width * height * 3);
    const alpha = new Uint8Array(width * height);
    let transparent = false;
    for (let p = 0, c = 0; p < alpha.length; p++, c += 4) {
        rgb[p * 3] = rgba[c]; rgb[p * 3 + 1] = rgba[c + 1]; rgb[p * 3 + 2] = rgba[c + 2];
        alpha[p] = rgba[c + 3];
        transparent ||= alpha[p] !== 255;
    }
    const [colorBytes, alphaBytes] = await Promise.all([
        deflateBytes(rgb), transparent ? deflateBytes(alpha) : null
    ]);
    return { pdfRaster: true, width, height, colorBytes, alphaBytes };
}

function embedPreparedRaster(doc, raster) {
    // Match pdf-lib's PNG image/soft-mask dictionaries, but feed them already
    // compressed lossless samples. Avoid PNG encode -> PNG decode -> JS deflate.
    const { width, height, colorBytes, alphaBytes } = raster;
    const common = { Type: 'XObject', Subtype: 'Image', Width: width, Height: height,
        BitsPerComponent: 8, Filter: 'FlateDecode' };
    const mask = alphaBytes ? doc.context.register(PDFLib.PDFRawStream.of(
        doc.context.obj({ ...common, ColorSpace: 'DeviceGray', Decode: [0, 1] }), alphaBytes
    )) : undefined;
    const ref = doc.context.register(PDFLib.PDFRawStream.of(
        doc.context.obj({ ...common, ColorSpace: 'DeviceRGB', SMask: mask }), colorBytes
    ));
    const embedder = new PDFLib.PngEmbedder({ width, height, bitsPerComponent: 8 });
    return PDFLib.PDFImage.of(ref, doc, embedder);
}

async function processTask(type, data) {
    if (type === 'compress') return compress(data.json);
    if (type === 'timelineHash') {
        let hash = 0;
        for (let i = 0; i < data.json.length; i++) hash = ((hash << 5) - hash + data.json.charCodeAt(i)) | 0;
        return `${Math.abs(hash).toString(36).slice(0, 8)}:${data.json.length.toString(36)}`;
    }
    if (type === 'decodeTimeline') {
        if (data.payload.length > 48 * 1024 * 1024) throw new Error('Timeline exceeds safe input size');
        const bytes = Uint8Array.from(atob(data.payload), c => c.charCodeAt(0));
        const reader = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('deflate')).getReader();
        const chunks = [];
        let size = 0;
        for (;;) {
            const { value, done } = await reader.read();
            if (done) break;
            size += value.byteLength;
            if (size > 96 * 1024 * 1024) { await reader.cancel(); throw new Error('Timeline expands beyond safe size'); }
            chunks.push(value);
        }
        const json = await new Blob(chunks).text();
        return JSON.stringify(InhouseTimelineCore.materializeArchive(JSON.parse(json)));
    }
    if (type === 'renderOverlay') {
        const page = JSON.parse(data.json);
        const canvas = new OffscreenCanvas(Math.max(1, Math.round(data.bounds.width * data.scale)),
            Math.max(1, Math.round(data.bounds.height * data.scale)));
        const ctx = canvas.getContext('2d');
        ctx.setTransform(data.scale, 0, 0, data.scale, -data.bounds.x * data.scale, -data.bounds.y * data.scale);
        for (const image of page.images || []) {
            if (!image?.src) continue;
            const response = await fetch(image.src);
            if (!response.ok) throw new Error('Could not load overlay image');
            const bitmap = await createImageBitmap(await response.blob());
            ctx.save(); ctx.translate(image.x, image.y); ctx.rotate(image.rotation || 0);
            ctx.drawImage(bitmap, -image.width / 2, -image.height / 2, image.width, image.height);
            ctx.restore(); bitmap.close();
        }
        for (const stroke of page.strokes || []) drawStroke(ctx, stroke);
        if (data.pdfRaster) {
            try { return await preparePdfRaster(canvas); }
            finally { canvas.width = canvas.height = 0; }
        }
        const blob = await canvas.convertToBlob({ type: 'image/png' });
        canvas.width = canvas.height = 0;
        return new Uint8Array(await blob.arrayBuffer());
    }
    if (type === 'contentHash') {
        return ihnCanonicalDocumentHash(JSON.parse(data.json), data.structure, data.calendar, data.name, data.fields);
    }
    if (type.startsWith('pdf')) {
        if (!self.PDFLib) importScripts('https://cdnjs.cloudflare.com/ajax/libs/pdf-lib/1.17.1/pdf-lib.min.js');
        if (type === 'pdfReady') return true;
        if (type === 'pdfLoad' || type === 'pdfCreate') {
            if (type === 'pdfLoad') {
                for (const [id, item] of documents) if (item.source) disposeDocument(id);
            }
            const doc = type === 'pdfLoad'
                ? await PDFLib.PDFDocument.load(data.bytes, data.options)
                : await PDFLib.PDFDocument.create();
            const id = ++nextObject;
            documents.set(id, { doc, source: type === 'pdfLoad' });
            return { id, pageCount: doc.getPageCount() };
        }
        if (type === 'pdfDispose') { disposeDocument(data.id); return true; }
        const doc = documents.get(data.id)?.doc;
        if (!doc) throw new Error('PDF processing document expired');
        const pageInfo = page => ({ id: remember(page, data.id), size: page.getSize() });
        if (type === 'pdfCopyPages') return (await doc.copyPages(documents.get(data.source).doc, data.indices)).map(pageInfo);
        if (type === 'pdfAddPage') return pageInfo(doc.addPage(data.page ? objects.get(data.page).value : data.size));
        if (type === 'pdfEmbed') {
            let raster = data.bytes?.pdfRaster ? data.bytes : null;
            if (!raster && data.kind === 'embedPng') {
                // Imported PNG backgrounds use exactly pdf-lib's decoder, but
                // native compression instead of recompressing them in JS during
                // final serialization. Preserve all decoded RGB/alpha samples.
                const { image: png } = await PDFLib.PngEmbedder.for(data.bytes);
                const [colorBytes, alphaBytes] = await Promise.all([
                    deflateBytes(png.rgbChannel), png.alphaChannel ? deflateBytes(png.alphaChannel) : null
                ]);
                raster = { width: png.width, height: png.height, colorBytes, alphaBytes };
            }
            const image = raster ? embedPreparedRaster(doc, raster) : await doc[data.kind](data.bytes);
            return { id: remember(image, data.id), ref: { objectNumber: image.ref.objectNumber } };
        }
        if (type === 'pdfDrawImage') {
            const page = objects.get(data.page).value;
            const image = objects.get(data.image).value;
            page.drawImage(image, data.options);
            const xobjects = page.node.lookup(PDFLib.PDFName.of('Resources')).lookup(PDFLib.PDFName.of('XObject'));
            for (const [name, ref] of xobjects.entries()) {
                if (ref.objectNumber === image.ref.objectNumber) return name.toString().replace(/^\//, '');
            }
            return null;
        }
        if (type === 'pdfRectangle') { objects.get(data.page).value.drawRectangle(data.options); return true; }
        if (type === 'pdfMetadata') {
            const keywords = data.method === 'setKeywords' && Array.isArray(data.value) ? data.value.join(' ') : null;
            if (keywords !== null && /^[A-Za-z0-9+/=;:_,.\- ]*$/.test(keywords)) {
                // Our compressed/base64 metadata is ASCII. pdf-lib's public
                // setter expands it to UTF-16 hex (4x), then compresses it all
                // again at save. A standard literal PDF string round-trips the
                // same Keywords without that costly expansion. Other text uses
                // the original Unicode-safe setter.
                doc.getInfoDict().set(PDFLib.PDFName.of('Keywords'), PDFLib.PDFString.of(keywords));
            } else doc[data.method](data.value);
            return true;
        }
        if (type === 'pdfAttach') { await doc.attach(data.bytes, data.name, data.options); return true; }
        if (type === 'pdfSave') return await doc.save(data.options);
    }
    if (type === 'timeline') {
        const history = JSON.parse(data.json);
        let archiveJson = JSON.stringify(InhouseTimelineCore.createArchive(history));
        while (archiveJson.length > data.budget && history.length > 1) {
            const index = history.findIndex((entry, i) => i < history.length - 1 && !entry.isMilestone);
            if (index < 0) break;
            history.splice(index, 1);
            archiveJson = JSON.stringify(InhouseTimelineCore.createArchive(history));
        }
        const retained = history.map(entry => `${entry.id}:${entry.ts}:${entry.contentHash}`);
        let encoded = await compress(archiveJson);
        while (encoded.length > data.budget && history.length > 1) {
            let index = history.findIndex((entry, i) => i < history.length - 1 && !entry.isMilestone);
            if (index < 0) index = history.findIndex((entry, i) => i < history.length - 1 && entry.kind !== 'baseline');
            history.splice(index < 0 ? 0 : index, 1);
            encoded = await compress(JSON.stringify(InhouseTimelineCore.createArchive(history)));
        }
        return { archiveJson, retained, encoded: encoded.length <= data.budget ? encoded : '' };
    }
    if (type === 'materializeTimeline') {
        return JSON.stringify(InhouseTimelineCore.materializeArchive(JSON.parse(data.json)));
    }
    throw new Error(`Unknown document processing task: ${type}`);
}

self.onmessage = async ({ data: { id, type, data } }) => {
    try {
        const value = await processTask(type, data);
        const transfers = value instanceof Uint8Array ? [value.buffer]
            : value?.pdfRaster ? [value.colorBytes.buffer, ...(value.alphaBytes ? [value.alphaBytes.buffer] : [])] : [];
        self.postMessage({ id, value }, transfers);
    } catch (error) {
        self.postMessage({ id, error: error?.message || String(error) });
    }
};
