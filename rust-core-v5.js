/* Rust/WASM editor kernels and lossless ink codec. Persistence and network orchestration stay with the host. */
(function (root) {
    'use strict';
    const scriptUrl = typeof document !== 'undefined' && document.currentScript?.src;
    const base = scriptUrl || new URL('rust-core-v5.js', root.location.href).href;
    const url = new URL('wasm/inhouse_notes_core.wasm', base);
    url.search = new URL(scriptUrl || root.location.href).search;
    url.searchParams.delete('native');
    if(root.InhouseAssetUrl)url.search=new URL(root.InhouseAssetUrl(url.href)).search;
    let core = null, folderCore = false, folderScratch = null;
    const ready = (async () => {
        try {
            if (typeof WebAssembly === 'undefined') return false;
            const response = await fetch(url, { signal: AbortSignal.timeout(4000) });
            if (!response.ok) return false;
            const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
            if (instance.exports.core_abi_version() !== 1) return false;
            core = instance.exports;
            folderCore = core.folder_plan_version?.() === 1;
            return true;
        } catch (_) { return false; }
    })();
    function allocation(bytes) {
        if (!Number.isSafeInteger(bytes) || bytes < 1 || bytes > 128 * 1024 * 1024) throw new RangeError('Pixel allocation too large');
        const pointer = core.alloc(bytes);
        if (!pointer) throw new Error('Pixel allocation failed');
        return { pointer, bytes };
    }
    function view(block) { return new Uint8Array(core.memory.buffer, block.pointer, block.bytes); }
    function release(block) { if (block) core.free(block.pointer, block.bytes); }
    function splitRgba(rgba) {
        if (!core || !rgba?.length || rgba.length % 4) return null;
        let input, rgb, alpha;
        try {
            const pixels = rgba.length / 4;
            input = allocation(rgba.length); rgb = allocation(pixels * 3); alpha = allocation(pixels);
            view(input).set(rgba);
            const transparent = core.split_rgba(input.pointer, rgb.pointer, alpha.pointer, pixels) === 1;
            return { rgb: view(rgb).slice(), alpha: transparent ? view(alpha).slice() : null, transparent };
        } catch (_) { return null; }
        finally { release(input); release(rgb); release(alpha); }
    }
    // Geometry is planned once per navigation and handed to WAAPI. There is no
    // animation frame callback or DOM access on the WASM boundary.
    function folderBuffer() {
        if (!folderScratch) folderScratch = allocation(32 * 8);
        return folderScratch.pointer;
    }
    function folderMotion(bounds, rect, backwards = false) {
        if (!bounds || !rect) return null;
        const input = [bounds.left, bounds.top, bounds.width, bounds.height,
            rect.left, rect.top, rect.width, rect.height];
        if (!input.every(Number.isFinite) || bounds.width <= 0 || bounds.height <= 0 || rect.width <= 0 || rect.height <= 0) return null;
        let values, backend = 'javascript';
        if (folderCore && core.folder_motion) {
            try {
                const pointer = folderBuffer();
                if (!core.folder_motion(...input, +!!backwards, pointer)) return null;
                values = new Float64Array(core.memory.buffer, pointer, 29);
                backend = 'rust';
            } catch (_) { /* A cached or unavailable core must not block navigation. */ }
        }
        if (!values) {
            const scale = Math.max((bounds.width + 4) / rect.width, (bounds.height + 4) / (rect.height * .94));
            const width = rect.width * scale, height = rect.height * scale;
            const finalLeft = (bounds.width - width) / 2;
            const finalTop = (bounds.height - rect.height * .94 * scale) / 2 - rect.height * .06 * scale;
            const dx = finalLeft - (rect.left - bounds.left), dy = finalTop - (rect.top - bounds.top);
            values = [scale, finalLeft, finalTop, dx, dy, width, height, 1100 * scale,
                backwards ? 560 : 600, backwards ? -112 : 0, backwards ? 0 : -112,
                .55, 0, .15, 1, backwards ? .4 : .2, 0, .2, 1,
                backwards ? 0 : 1, 0, backwards ? .6 : .4, backwards ? 1 : 0,
                backwards ? 1 : 1 / scale, backwards ? 1 / scale : 1,
                backwards ? dx : 0, backwards ? dy : 0, backwards ? 0 : dx, backwards ? 0 : dy];
        }
        if (values[0] <= 0 || !values.every(Number.isFinite)) return null;
        const easing = index => `cubic-bezier(${values[index]}, ${values[index + 1]}, ${values[index + 2]}, ${values[index + 3]})`;
        return {
            scale: values[0], finalLeft: values[1], finalTop: values[2], dx: values[3], dy: values[4],
            width: values[5], height: values[6], perspective: values[7], duration: values[8],
            hingeFrom: values[9], hingeTo: values[10], easing: easing(11), hingeEasing: easing(15),
            labelFrames: [{ opacity: values[19] }, { opacity: values[20], offset: values[21] }, { opacity: values[22] }],
            zoomFrames: [
                { transform: `translate(${values[25]}px, ${values[26]}px) scale(${values[23]})` },
                { transform: `translate(${values[27]}px, ${values[28]}px) scale(${values[24]})` }
            ], backend
        };
    }
    function folderFade(elapsedFraction, backwards = false) {
        if (!Number.isFinite(elapsedFraction)) return null;
        let values, backend = 'javascript';
        if (folderCore && core.folder_fade) {
            try {
                const pointer = folderBuffer(), count = core.folder_fade(elapsedFraction, +!!backwards, pointer);
                if (count !== 3 && count !== 4) return null;
                values = new Float64Array(core.memory.buffer, pointer, count * 2);
                backend = 'rust';
            } catch (_) { /* Use the same plan while an older core is cached. */ }
        }
        if (!values) {
            const from = Math.min(.9, Math.max(0, Math.min(1, elapsedFraction)) + .03);
            const to = Math.max(.7, Math.min(1, from + .15));
            values = backwards ? [0, 1, .3, 1, 1, 0] : [0, 0, from, 0, to, 1, 1, 1];
        }
        const frames = [];
        for (let i = 0; i < values.length; i += 2) frames.push({ offset: values[i], opacity: values[i + 1] });
        Object.defineProperty(frames, 'backend', { value: backend });
        return frames;
    }
    function folderRows(heights, gap, columns, count) {
        if (!(heights instanceof Float64Array) || heights.length > 1000000 || !Number.isFinite(gap) || gap < 0
            || !Number.isSafeInteger(columns) || columns < 1 || columns > 0xffffffff
            || !Number.isSafeInteger(count) || count < 0 || count > 0xffffffff
            || Math.ceil(count / columns) > heights.length || !heights.every(height => Number.isFinite(height) && height > 0)) return null;
        const rowCount = Math.ceil(count / columns);
        const validRange = (top, bottom, overscan) => [top, bottom, overscan].every(Number.isFinite)
            && bottom >= top && overscan >= 0 && Number.isFinite(top - overscan) && Number.isFinite(bottom + overscan);
        const validSpan = (first, end) => Number.isSafeInteger(first) && Number.isSafeInteger(end)
            && first >= 0 && end >= first && end <= rowCount;
        if (folderCore && core.folder_rows_new && core.folder_rows_range && core.folder_rows_span && core.folder_rows_drop) {
            let input, output, context = 0;
            try {
                input = allocation(Math.max(8, heights.byteLength));
                view(input).set(new Uint8Array(heights.buffer, heights.byteOffset, heights.byteLength));
                context = core.folder_rows_new(input.pointer, heights.length, gap, columns, count);
                if (!context) return null;
                output = allocation(16);
                let disposed = false;
                return {
                    backend: 'rust',
                    range(top, bottom, overscan = 0) {
                        if (disposed || !validRange(top, bottom, overscan)) return null;
                        if (!core.folder_rows_range(context, top, bottom, overscan, output.pointer)) return null;
                        const result = new Float64Array(core.memory.buffer, output.pointer, 2);
                        return { start: result[0], end: result[1], backend: 'rust' };
                    },
                    span(first, end) {
                        if (disposed || !validSpan(first, end)) return null;
                        const height = core.folder_rows_span(context, first, end);
                        return Number.isFinite(height) ? height : null;
                    },
                    dispose() {
                        if (!disposed) { disposed = true; core.folder_rows_drop(context); release(output); }
                    }
                };
            } catch (_) { if (context) core.folder_rows_drop(context); release(output); }
            finally { release(input); }
        }
        // The prefix is built once, including when WASM has not finished
        // loading. Scrolling performs only two binary searches afterwards.
        const tops = new Float64Array(rowCount), bottoms = new Float64Array(rowCount);
        let top = 0, disposed = false;
        for (let row = 0; row < rowCount; row++) {
            const bottom = top + heights[row];
            if (!Number.isFinite(bottom) || !Number.isFinite(bottom + gap)) return null;
            tops[row] = top; bottoms[row] = bottom; top = bottom + gap;
        }
        const partition = (array, value, inclusive) => {
            let low = 0, high = array.length;
            while (low < high) {
                const middle = low + Math.floor((high - low) / 2);
                if (inclusive ? array[middle] <= value : array[middle] < value) low = middle + 1;
                else high = middle;
            }
            return low;
        };
        return {
            backend: 'javascript',
            range(top, bottom, overscan = 0) {
                if (disposed || !validRange(top, bottom, overscan)) return null;
                const first = partition(bottoms, top - overscan, false);
                const end = Math.max(first, partition(tops, bottom + overscan, true));
                return { start: Math.min(count, first * columns), end: Math.min(count, end * columns), backend: 'javascript' };
            },
            span(first, end) {
                if (disposed || !validSpan(first, end)) return null;
                return first === end ? 0 : bottoms[end - 1] - tops[first];
            },
            dispose() { disposed = true; }
        };
    }
    function checkpointCodec(bytes, decodedLength = null) {
        if (!core || core.ink_checkpoint_version?.() !== 1) return null;
        if (!(bytes instanceof Uint8Array) || !bytes.length) return null;
        const capacity = decodedLength === null ? Math.ceil(bytes.length / 8) * 9 : decodedLength;
        if (!Number.isSafeInteger(capacity) || capacity < 1 || capacity > 72 * 1024 * 1024) throw new RangeError('Ink checkpoint too large');
        let input, output;
        try {
            input = allocation(bytes.length); output = allocation(capacity);
            view(input).set(bytes);
            const length = decodedLength === null
                ? core.ink_checkpoint_pack(input.pointer, bytes.length, output.pointer, capacity)
                : core.ink_checkpoint_unpack(input.pointer, bytes.length, output.pointer, capacity);
            if (length < 0) throw new Error('Invalid ink checkpoint');
            return view(output).slice(0, length);
        } finally { release(input); release(output); }
    }
    async function unpackCheckpoint(data) {
        const { codec, rawLength } = data;
        const bytes = new Uint8Array(data.bytes);
        if (!Number.isSafeInteger(rawLength) || rawLength < 0 || rawLength > 64 * 1024 * 1024 || rawLength % 32) throw new Error('Invalid ink checkpoint length');
        if (codec === 'raw-v1') {
            if (bytes.length !== rawLength) throw new Error('Truncated ink checkpoint');
            return bytes;
        }
        if (codec !== 'xor-v1') throw new Error('Unknown ink checkpoint codec');
        await ready;
        const decoded = checkpointCodec(bytes, rawLength);
        if (decoded) return decoded;
        // Existing offline files remain readable even if WebAssembly is disabled.
        const output = new Uint8Array(rawLength), previous = new Uint8Array(32);
        let offset = 0;
        for (let word = 0; word < rawLength / 8; word++) {
            const length = bytes[offset++];
            if (length === undefined || length > 8 || offset + length > bytes.length) throw new Error('Truncated ink checkpoint');
            for (let byte = 0; byte < 8; byte++) {
                const lane = (word % 4) * 8 + byte;
                previous[lane] ^= byte < length ? bytes[offset + byte] : 0;
                output[word * 8 + byte] = previous[lane];
            }
            offset += length;
            if (word && word % 8192 === 0) await new Promise(resolve=>setTimeout(resolve,0));
        }
        if (offset !== bytes.length) throw new Error('Trailing ink checkpoint bytes');
        return output;
    }
    // Bounded, generation-scoped coordinates avoid re-packing long strokes on
    // every eraser sample. A document edit invalidates all cached geometry.
    const hitCache = new Map();
    let hitCacheBytes = 0, hitGeneration = null;
    function clearHitCache() {
        for (const entry of hitCache.values()) core.free(entry.pointer, entry.bytes);
        hitCache.clear(); hitCacheBytes = 0;
    }
    function strokeHitTest(points, x, y, radius, generation) {
        if (!core?.editor_stroke_hit || !Array.isArray(points) || points.length < 256
            || points.length > 262144 || ![x,y,radius].every(Number.isFinite)
            || radius < 0 || Math.max(Math.abs(x),Math.abs(y),radius) > 1e9) return null;
        if (generation !== hitGeneration) { clearHitCache(); hitGeneration = generation; }
        const bytes = points.length * 16;
        let entry = hitCache.get(points);
        if (entry && entry.bytes !== bytes) {
            core.free(entry.pointer,entry.bytes); hitCacheBytes -= entry.bytes;
            hitCache.delete(points); entry = null;
        }
        if (!entry) {
            while (hitCache.size >= 64 || hitCacheBytes + bytes > 4 * 1024 * 1024) {
                const [key, oldest] = hitCache.entries().next().value;
                core.free(oldest.pointer,oldest.bytes); hitCacheBytes -= oldest.bytes; hitCache.delete(key);
            }
            let pointer = 0;
            try {
                pointer = allocation(bytes).pointer;
                const coords = new Float64Array(core.memory.buffer, pointer, points.length * 2);
                for (let i=0;i<points.length;i++) {
                    const point=points[i];
                    if (!point || !Number.isFinite(point.x) || !Number.isFinite(point.y)
                        || Math.max(Math.abs(point.x),Math.abs(point.y)) > 1e9) { core.free(pointer,bytes); return null; }
                    coords[i*2]=point.x; coords[i*2+1]=point.y;
                }
                entry={pointer,bytes}; hitCacheBytes += bytes;
            } catch (_) { if(pointer)core.free(pointer,bytes); return null; }
        } else hitCache.delete(points);
        hitCache.set(points,entry);
        const result = core.editor_stroke_hit(entry.pointer,points.length,x,y,radius);
        return result < 0 ? null : !!result;
    }
    function documentWorkQueue() {
        if (core?.document_work_version?.() !== 1) return null;
        let handle = core.document_work_new(), output = null;
        if (!handle) return null;
        try { output = allocation(17 * 8); }
        catch (_) { core.document_work_drop(handle); return null; }
        return {
            add(id, session, quietMs) {
                return !!handle && Number.isInteger(id) && id > 0 && id <= 0xffffffff
                    && core.document_work_add(handle, id, session, quietMs) === 1;
            },
            cancel(id) { if (handle) core.document_work_cancel(handle, id); },
            poll(state) {
                if (!handle) throw new Error('Document work queue disposed');
                const count = core.document_work_poll(handle, state.now, state.lastActivity,
                    state.session, (state.busy ? 1 : 0) | (state.bypass ? 2 : 0), output.pointer);
                if (count < 0) throw new Error('Invalid document work state');
                // Recreate after every call: other kernels can grow WASM memory.
                const values = new Float64Array(core.memory.buffer, output.pointer, 17);
                const ready = [];
                for (let i = 0; i < count; i++) ready.push([values[1+i*2], values[2+i*2] === 1]);
                return {ready, delay:values[0]};
            },
            dispose() {
                if (!handle) return;
                core.document_work_drop(handle); handle = 0; release(output); output = null;
            }
        };
    }
    function pdfInk(strokes, bounds, rasterScale = 1) {
        if (!core?.pdf_ink_path || !Array.isArray(strokes) || !bounds) return null;
        let length = 0;
        for (const stroke of strokes) {
            if (stroke.tool === 'eraser-stroke') continue;
            if (stroke.tool !== 'pen' || !/^#[0-9a-f]{6}$/i.test(stroke.color)
                || !Array.isArray(stroke.points)) return null;
            if (stroke.points.length >= 2) length += 5 + stroke.points.length * 2;
        }
        // Dense/repeated ink is cheaper as a cached lossless raster. Keep the
        // vector fast path for ordinary sparse pen pages without dropping points.
        if (!length || length > 8192) return null;
        // A narrow line of tightly packed samples costs less to rasterize than
        // to encode as thousands of PDF operators. Compare actual crop pixels.
        const rasterPixels = bounds.width * bounds.height * rasterScale * rasterScale;
        if (!Number.isFinite(rasterPixels) || rasterPixels < length * 32) return null;
        let input, output;
        try {
            input = allocation(length * 8); output = allocation(length * 24 + 256);
            const values = new Float64Array(core.memory.buffer, input.pointer, length);
            let at = 0;
            for (const stroke of strokes) {
                if (stroke.tool === 'eraser-stroke' || stroke.points.length < 2) continue;
                values[at++] = stroke.width;
                for (let i=1;i<7;i+=2) values[at++] = parseInt(stroke.color.slice(i,i+2),16)/255;
                values[at++] = stroke.points.length;
                for (const point of stroke.points) { values[at++] = point.x; values[at++] = point.y; }
            }
            const used = core.pdf_ink_path(input.pointer, length, bounds.x, bounds.y + bounds.height, output.pointer, output.bytes);
            return used ? view(output).slice(0,used) : null;
        } catch (_) { return null; }
        finally { release(input); release(output); }
    }
    root.InhouseRust = Object.freeze({ ready, pdfInk, documentWorkQueue, strokeHitTest, checkpointCodec, unpackCheckpoint, splitRgba,
        folderMotion, folderFade, folderRows, available: () => !!core });
})(globalThis);
