importScripts(`rust-core-v5.js${self.location.search}`);
'use strict';
importScripts(`timeline-core-v5.js${self.location.search}`);
importScripts(`collaboration-core-v5.js${self.location.search}`);
let nextObject = 0;
const documents = new Map();
const objects = new Map();
const pngRasterCache = new Map();
let pngRasterCacheBytes = 0;
const PNG_RASTER_CACHE_LIMIT = 32 * 1024 * 1024;
async function preparedPng(bytes) {
    const buffer = bytes instanceof ArrayBuffer || ArrayBuffer.isView(bytes) ? bytes : null;
    const key = buffer && self.crypto?.subtle ? Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',buffer)),v=>v.toString(16).padStart(2,'0')).join('') : null;
    if (key && pngRasterCache.has(key)) {
        const raster=pngRasterCache.get(key);pngRasterCache.delete(key);pngRasterCache.set(key,raster);return raster;
    }
    const {image:png}=await PDFLib.PngEmbedder.for(bytes);
    const [colorBytes,alphaBytes]=await Promise.all([deflateBytes(png.rgbChannel),png.alphaChannel?deflateBytes(png.alphaChannel):null]);
    const raster={width:png.width,height:png.height,colorBytes,alphaBytes};
    const size=colorBytes.byteLength+(alphaBytes?.byteLength||0);
    if(key && size<=PNG_RASTER_CACHE_LIMIT) {
        while(pngRasterCacheBytes+size>PNG_RASTER_CACHE_LIMIT&&pngRasterCache.size) {
            const oldest=pngRasterCache.keys().next().value, value=pngRasterCache.get(oldest);
            pngRasterCacheBytes-=value.colorBytes.byteLength+(value.alphaBytes?.byteLength||0);pngRasterCache.delete(oldest);
        }
        pngRasterCache.set(key,raster);pngRasterCacheBytes+=size;
    }
    return raster;
}
function remember(value, owner) {
    const id = ++nextObject;
    objects.set(id, { value, owner });
    return id;
}
function objectFor(id, owner) {
    const object = objects.get(id);
    if (!object || object.owner !== owner) throw new Error('PDF object belongs to another or expired document');
    return object.value;
}
function disposeDocument(id) {
    documents.delete(id);
    for (const [key, object] of objects) if (object.owner === id) objects.delete(key);
}

// Same pen spline and highlighter blending as the interactive renderer.
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
    const end = points.length - 1;
    for (let i = 0; i < end; i++) {
        appendPenSplineSegment(ctx, points[Math.max(0, i - 1)], points[i], points[i + 1], points[Math.min(end, i + 2)]);
    }
    ctx.stroke();
}

// Identical to appendPenSplineSegment in app-v5.js: the centripetal
// Catmull-Rom spline through every measured pen sample.
function appendPenSplineSegment(target, p0, p1, p2, p3) {
    const ax = p2.x - p1.x, ay = p2.y - p1.y, length2 = ax * ax + ay * ay;
    // Below 2 px a segment is visually straight: skip the curve maths so
    // dense handwriting repaints as fast as a polyline.
    if (length2 < 4) { target.lineTo(p2.x, p2.y); return; }
    const bx = p1.x - p0.x, by = p1.y - p0.y, cx = p3.x - p2.x, cy = p3.y - p2.y;
    const d1 = Math.sqrt(Math.sqrt(bx * bx + by * by));
    const d2 = Math.sqrt(Math.sqrt(length2));
    const d3 = Math.sqrt(Math.sqrt(cx * cx + cy * cy));
    let c1x = p1.x, c1y = p1.y, c2x = p2.x, c2y = p2.y;
    if (d1 >= 1e-3) {
        const a = d1 * d1, b = d2 * d2, c = 2 * a + 3 * d1 * d2 + b, n = 3 * d1 * (d1 + d2);
        c1x = (a * p2.x - b * p0.x + c * p1.x) / n;
        c1y = (a * p2.y - b * p0.y + c * p1.y) / n;
    }
    if (d3 >= 1e-3) {
        const a = d3 * d3, b = d2 * d2, c = 2 * a + 3 * d3 * d2 + b, n = 3 * d3 * (d3 + d2);
        c2x = (a * p1.x - b * p3.x + c * p2.x) / n;
        c2y = (a * p1.y - b * p3.y + c * p2.y) / n;
    }
    target.bezierCurveTo(c1x, c1y, c2x, c2y, p2.x, p2.y);
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
    await self.InhouseRust?.ready;
    const rustSplit = self.InhouseRust?.splitRgba(rgba);
    const rgb = rustSplit?.rgb || new Uint8Array(width * height * 3);
    const alpha = rustSplit ? rustSplit.alpha : new Uint8Array(width * height);
    let transparent = rustSplit?.transparent || false;
    if (!rustSplit) for (let p = 0, c = 0; p < alpha.length; p++, c += 4) {
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

function embedPreparedVector(doc, vector) {
    const ref = doc.context.register(PDFLib.PDFRawStream.of(doc.context.obj({
        Type:'XObject', Subtype:'Form', FormType:1,
        BBox:[0,0,vector.width,vector.height], Resources:{}, Filter:'FlateDecode'
    }), vector.contentBytes));
    return {pdfVector:true, ref, width:vector.width, height:vector.height};
}

async function processTask(type, data) {
    if (type === 'packInkCheckpoint') {
        await InhouseRust.ready;
        const raw = new Uint8Array(data.bytes);
        const packed = InhouseRust.checkpointCodec(raw);
        return { inkCheckpoint: 1, codec: packed && packed.length < raw.length ? 'xor-v1' : 'raw-v1',
            bytes: packed && packed.length < raw.length ? packed : raw, rawLength: raw.length };
    }
    if (type === 'unpackInkCheckpoint') return InhouseRust.unpackCheckpoint(data);

    if (type === 'compress') return compress(data.json);
    if (type === 'prepareMetadata') {
        // Let native compression run while the worker computes the sync hash.
        // Both consume the same immutable payload; send the large JSON once.
        const [compressed, hashed] = await Promise.allSettled([
            compress(data.json),
            Promise.resolve().then(() => ihnCanonicalDocumentHash(
                JSON.parse(data.json), data.structure, data.calendar, data.name, data.fields
            ))
        ]);
        return {
            encoded: compressed.status === 'fulfilled' ? compressed.value : null,
            contentHash: hashed.status === 'fulfilled' ? hashed.value : null
        };
    }
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
        if (data.pdfRaster && data.vectorInk && !page.images?.length) {
            await InhouseRust.ready;
            const paths = InhouseRust.pdfInk(page.strokes, data.bounds, data.scale);
            if (paths) return {pdfVector:true, width:data.bounds.width, height:data.bounds.height,
                contentBytes:await deflateBytes(paths)};
        }
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
        if (!self.PDFLib) importScripts('vendor/pdf-lib/pdf-lib.min.js');
        if (type === 'pdfReady') return true;
        if (type === 'pdfLoad' || type === 'pdfCreate') {
            if (documents.size >= 32) throw new Error('Too many open processing documents; dispose completed PDFs');
            const doc = type === 'pdfLoad'
                ? await PDFLib.PDFDocument.load(data.bytes, data.options)
                : await PDFLib.PDFDocument.create();
            const id = ++nextObject;
            documents.set(id, { doc });
            return { id, pageCount: doc.getPageCount() };
        }
        if (type === 'pdfDispose') { disposeDocument(data.id); return true; }
        const doc = documents.get(data.id)?.doc;
        if (!doc) throw new Error('PDF processing document expired');
        const pageInfo = page => ({ id: remember(page, data.id), size: page.getSize() });
        if (type === 'pdfCopyPages') {
            const source = documents.get(data.source)?.doc;
            if (!source) throw new Error('Source PDF processing document expired');
            return (await doc.copyPages(source, data.indices)).map(pageInfo);
        }
        if (type === 'pdfAddPage') return pageInfo(doc.addPage(data.page ? objectFor(data.page, data.id) : data.size));
        if (type === 'pdfEmbed') {
            if (!['embedPng', 'embedJpg'].includes(data.kind)) throw new Error('Unsupported image format');
            if (data.bytes?.pdfVector) {
                const image = embedPreparedVector(doc, data.bytes);
                return {id:remember(image,data.id),ref:{objectNumber:image.ref.objectNumber}};
            }
            let raster = data.bytes?.pdfRaster ? data.bytes : null;
            if (!raster && data.kind === 'embedPng') {
                // Imported PNG backgrounds use exactly pdf-lib's decoder, but
                // native compression instead of recompressing them in JS during
                // final serialization. Preserve all decoded RGB/alpha samples.
                raster = await preparedPng(data.bytes);
            }
            const image = raster ? embedPreparedRaster(doc, raster) : await doc[data.kind](data.bytes);
            return { id: remember(image, data.id), ref: { objectNumber: image.ref.objectNumber } };
        }
        if (type === 'pdfDrawImage') {
            const page = objectFor(data.page, data.id);
            const image = objectFor(data.image, data.id);
            if (image.pdfVector) {
                const options = data.options || {};
                const name = page.node.newXObject('Ink', image.ref);
                page.pushOperators(...PDFLib.drawPage(name, {
                    x:options.x || 0, y:options.y || 0,
                    xScale:(options.width ?? image.width) / image.width,
                    yScale:(options.height ?? image.height) / image.height,
                    rotate:options.rotate || PDFLib.degrees(0),
                    xSkew:PDFLib.degrees(0), ySkew:PDFLib.degrees(0)
                }));
                return name.toString().replace(/^\//, '');
            }
            page.drawImage(image, data.options);
            const xobjects = page.node.lookup(PDFLib.PDFName.of('Resources')).lookup(PDFLib.PDFName.of('XObject'));
            for (const [name, ref] of xobjects.entries()) {
                if (ref.objectNumber === image.ref.objectNumber) return name.toString().replace(/^\//, '');
            }
            return null;
        }
        if (type === 'pdfRectangle') { objectFor(data.page, data.id).drawRectangle(data.options); return true; }
        if (type === 'pdfMetadata') {
            if (!['setKeywords', 'setTitle', 'setSubject', 'setCreator'].includes(data.method)) throw new Error('Unsupported metadata field');
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
        let archiveJson = InhouseTimelineCore.encodeArchiveJson(history);
        while (archiveJson.length > data.budget && history.length > 1) {
            const index = history.findIndex((entry, i) => i < history.length - 1 && !entry.isMilestone);
            if (index < 0) break;
            history.splice(index, 1);
            archiveJson = InhouseTimelineCore.encodeArchiveJson(history);
        }
        const retained = history.map(entry => `${entry.id}:${entry.ts}:${entry.contentHash}`);
        let encoded = await compress(archiveJson);
        while (encoded.length > data.budget && history.length > 1) {
            let index = history.findIndex((entry, i) => i < history.length - 1 && !entry.isMilestone);
            if (index < 0) index = history.findIndex((entry, i) => i < history.length - 1 && entry.kind !== 'baseline');
            history.splice(index < 0 ? 0 : index, 1);
            encoded = await compress(InhouseTimelineCore.encodeArchiveJson(history));
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
        const transfers = value?.pdfVector ? [value.contentBytes.buffer]
            : value?.inkCheckpoint ? [value.bytes.buffer] : value instanceof Uint8Array ? [value.buffer]
            : value?.pdfRaster ? [value.colorBytes.buffer, ...(value.alphaBytes ? [value.alphaBytes.buffer] : [])] : [];
        self.postMessage({ id, value }, transfers);
    } catch (error) {
        self.postMessage({ id, error: error?.message || String(error), errorName: error?.name || 'Error' });
    }
};
