/* Rust/WASM numeric kernels and lossless ink codec. Persistence and network orchestration stay with the host. */
(function (root) {
    'use strict';
    const scriptUrl = typeof document !== 'undefined' && document.currentScript?.src;
    const base = scriptUrl || new URL(root.location.pathname.includes('/scanner/') ? '../../rust-core-v5.js' : 'rust-core-v5.js', root.location.href).href;
    const url = new URL('wasm/inhouse_notes_core.wasm', base);
    url.search = new URL(scriptUrl || root.location.href).search;
    url.searchParams.delete('native');
    if(root.InhouseAssetUrl)url.search=new URL(root.InhouseAssetUrl(url.href)).search;
    let core = null, folderCore = false, folderScratch = null;
    const ready = (async () => {
        try {
            if (typeof WebAssembly === 'undefined') return false;
            const response = await fetch(url, { signal: AbortSignal.timeout(20000) });
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
    function warp(input, width, height) {
        if (!core || !input?.data || input.width < 2 || input.height < 2) return null;
        let source, output;
        try {
            source = allocation(input.data.length);
            output = allocation(width * height * 4);
            view(source).set(input.data);
        } catch (_) { release(source); release(output); return null; }
        let disposed = false;
        return {
            cell(left, right, top, bottom, tl, tr, bl, br) {
                if (disposed) throw new Error('Pixel buffer released');
                core.warp_cell(source.pointer, input.width, input.height, output.pointer, width, height,
                    left, right, top, bottom, tl.x, tl.y, tr.x, tr.y, bl.x, bl.y, br.x, br.y);
            },
            copyTo(destination) { if (!disposed) destination.set(view(output)); },
            dispose() { if (!disposed) { disposed = true; release(source); release(output); } }
        };
    }
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
    function scanColors(rgba, original, width, height, field, mask, fw, fh, parameters, coverage = null, neutralSupport = null) {
        // An older cached core cannot produce the optional geometry plane.
        // Use the JS fallback instead of returning an all-zero ink mask.
        if (!core?.correct_scan_rows || (coverage && core.correct_scan_rows.length < 12) || (neutralSupport && core.correct_scan_rows.length < 13)
            || ![width,height,fw,fh].every(value => Number.isSafeInteger(value) && value > 0)
            || width > 16384 || height > 16384 || rgba.length !== width * height * 4 || original.length !== rgba.length || field.length !== fw * fh
            || parameters.length !== 61 || (mask && mask.length !== fw * fh) || (coverage && coverage.length !== width * height)
            || (neutralSupport && (!(neutralSupport instanceof Float32Array) || neutralSupport.length !== fw*fh))) return null;
        const blocks = [];
        const copy = array => { const block = allocation(array.byteLength); blocks.push(block);
            view(block).set(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)); return block; };
        try {
            const data = copy(rgba), raw = copy(original), light = copy(field), drawing = mask ? copy(mask) : null, params = copy(parameters);
            const geometry = coverage ? copy(coverage) : null, support = neutralSupport ? copy(neutralSupport) : null;
            let disposed = false;
            return {
                rows(from, to) { if (!disposed) core.correct_scan_rows(data.pointer, raw.pointer, width, height,
                    light.pointer, drawing?.pointer || 0, fw, fh, params.pointer, from, to, geometry?.pointer || 0, support?.pointer || 0); },
                copyTo(destination) { if (!disposed) { destination.set(view(data)); if (geometry) coverage.set(view(geometry)); } },
                dispose() { if (!disposed) { disposed = true; blocks.forEach(release); } }
            };
        } catch (_) { blocks.forEach(release); return null; }
    }
    function balancePaper(rgba, width, height, white, fw, fh, neutral = null) {
        if (!core?.balance_paper_rows || (neutral && core.balance_paper_rows.length < 9) || ![width,height,fw,fh].every(v => Number.isSafeInteger(v) && v > 0)
            || width > 16384 || height > 16384 || rgba.length !== width*height*4
            || !(white instanceof Float32Array) || white.length !== fw*fh*2
            || (neutral && (!(neutral instanceof Float32Array) || neutral.length !== fw*fh*4))) return null;
        let data, field, grey;
        try {
            data = allocation(rgba.byteLength); field = allocation(white.byteLength);
            view(data).set(rgba); view(field).set(new Uint8Array(white.buffer,white.byteOffset,white.byteLength));
            if (neutral) { grey=allocation(neutral.byteLength); view(grey).set(new Uint8Array(neutral.buffer,neutral.byteOffset,neutral.byteLength)); }
            let disposed = false;
            return {
                rows(from,to) { if (!disposed) core.balance_paper_rows(data.pointer,width,height,field.pointer,fw,fh,from,to,grey?.pointer || 0); },
                copyTo(destination) { if (!disposed) destination.set(view(data)); },
                dispose() { if (!disposed) { disposed=true; release(data); release(field); release(grey); } }
            };
        } catch (_) { release(data); release(field); release(grey); return null; }
    }
    function separateInk(rgba) {
        if (!core?.separate_ink || !rgba?.length || rgba.length % 4) return false;
        let pixels;
        try { pixels = allocation(rgba.length); view(pixels).set(rgba);
            core.separate_ink(pixels.pointer, rgba.length / 4); rgba.set(view(pixels)); return true;
        } catch (_) { return false; }
        finally { release(pixels); }
    }
    function surfaceFit(parameters, points, width, height, frame, guides = [], preserveEnds = false) {
        if (!core?.surface_fit_new || parameters.length < 10 || parameters.length > 32 || (parameters.length - 8) % 2
            || points.length > 4500 || guides.length > 160 || !Number.isFinite(width) || !Number.isFinite(height)
            || width <= 0 || height <= 0) return null;
        const paths = [], offsets = [0];
        if (frame) for (const name of ['top','right','bottom','left']) {
            const path = frame.paths?.[name];
            if (!path || path.length < 2 || path.length > 1024) return null;
            path.forEach(point => paths.push(point.x, point.y)); offsets.push(paths.length / 2);
        }
        else offsets.push(0,0,0,0);
        const inputs = [new Float64Array(parameters), new Float64Array(points.flatMap(point => [point.u,point.v,point.x,point.y])),
            new Float64Array(paths), new Uint32Array(offsets),
            new Float64Array(guides.flatMap(guide => [guide.u,guide.v,guide.x,guide.y,guide.du,guide.dv,guide.tx,guide.ty]))];
        if (inputs.some(array => !array.every(Number.isFinite))) return null;
        const blocks = []; let context = 0;
        try {
            for (const input of inputs) {
                const block = allocation(Math.max(8,input.byteLength)); blocks.push(block);
                view(block).set(new Uint8Array(input.buffer,input.byteOffset,input.byteLength));
            }
            const info = allocation(16); blocks.push(info);
            context = core.surface_fit_new(blocks[0].pointer,parameters.length,blocks[1].pointer,points.length,
                blocks[2].pointer,paths.length/2,blocks[3].pointer,blocks[4].pointer,guides.length,width,height,+preserveEnds);
            if (!context) throw new Error('Surface input rejected');
            let disposed = false;
            return {
                step() { return !disposed && core.surface_fit_step(context,1) === 1; },
                result() {
                    if (disposed) return null;
                    core.surface_fit_result(context,blocks[0].pointer,info.pointer);
                    const values = new Float64Array(core.memory.buffer,info.pointer,2);
                    return {p:Array.from(new Float64Array(core.memory.buffer,blocks[0].pointer,parameters.length)),cost:values[0],iterations:values[1],backend:'rust'};
                },
                dispose() { if (!disposed) { disposed=true; core.surface_fit_drop(context); blocks.forEach(release); } }
            };
        } catch (_) { if (context) core.surface_fit_drop(context); blocks.forEach(release); return null; }
    }
    function inkAxes(rgba, width, height, coverage = null) {
        if (!core?.ink_axes || (coverage && (core.ink_axes.length < 6 || core.ink_geometry_version?.() !== 2))
            || !Number.isSafeInteger(width) || !Number.isSafeInteger(height)
            || width < 2 || height < 2 || width > 4096 || height > 4096 || width*height > 12000000
            || rgba?.length !== width*height*4 || (coverage && coverage.length !== width*height)) return null;
        let pixels, mask, radii, geometry;
        try {
            pixels=allocation(rgba.length); mask=allocation(width*height); radii=allocation(width*height*2);
            view(pixels).set(rgba);
            if (coverage) { geometry=allocation(coverage.length); view(geometry).set(coverage); }
            const status=core.ink_axes(pixels.pointer,width,height,mask.pointer,radii.pointer,geometry?.pointer || 0);
            if (status===2) return {rejected:true};
            if (status!==1) return null;
            return {mask:view(mask).slice(), radii:new Uint16Array(core.memory.buffer,radii.pointer,width*height).slice(), backend:'rust'};
        } catch (_) { return null; }
        finally { release(pixels); release(mask); release(radii); release(geometry); }
    }
    function inkRepair(rendered, radii, coverage, width, height, capacity = 12000) {
        if (!core?.ink_repair_discs || !Number.isSafeInteger(width) || !Number.isSafeInteger(height)
            || width < 2 || height < 2 || width > 4096 || height > 4096 || width*height > 12000000
            || !Number.isSafeInteger(capacity) || capacity < 0 || capacity > 12000
            || rendered?.length !== width*height*4 || rendered.byteLength !== width*height*4 || !(radii instanceof Uint16Array) || radii.length !== width*height
            || !(coverage instanceof Uint8Array) || coverage.length !== width*height) return null;
        if (!capacity) return {discs: new Float64Array(0), backend:'rust'};
        let pixels, distance, measured, output;
        try {
            pixels=allocation(rendered.byteLength); distance=allocation(radii.byteLength);
            measured=allocation(coverage.byteLength); output=allocation(capacity*3*8);
            view(pixels).set(rendered);
            view(distance).set(new Uint8Array(radii.buffer,radii.byteOffset,radii.byteLength));
            view(measured).set(coverage);
            const count=core.ink_repair_discs(pixels.pointer,distance.pointer,measured.pointer,width,height,output.pointer,capacity);
            if (count<0 || count>capacity) return null;
            return {discs:new Float64Array(core.memory.buffer,output.pointer,count*3).slice(),backend:'rust'};
        } catch (_) { return null; }
        finally { release(pixels); release(distance); release(measured); release(output); }
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
    // Generic bridge to the ported logic modules (rust-core/src/bridge.rs).
    let textEncoder = null, textDecoder = null;
    const codecs = () => { textEncoder ||= new TextEncoder(); textDecoder ||= new TextDecoder(); };
    function callBytes(name, bytes = null) {
        if (!core?.bridge_call || typeof name !== 'string') return null;
        codecs();
        const nameBytes = textEncoder.encode(name), data = bytes || new Uint8Array(0);
        let nameBlock = null, inputBlock = null, lengthBlock = null, pointer = 0, total = 0;
        try {
            nameBlock = allocation(nameBytes.length); view(nameBlock).set(nameBytes);
            if (data.length) { inputBlock = allocation(data.length); view(inputBlock).set(data); }
            lengthBlock = allocation(8);
            pointer = core.bridge_call(nameBlock.pointer, nameBytes.length, inputBlock ? inputBlock.pointer : 0, data.length, lengthBlock.pointer);
            if (!pointer) throw new Error(`Rust call ${name} was rejected`);
            total = new Uint32Array(core.memory.buffer, lengthBlock.pointer, 1)[0];
            const out = new Uint8Array(core.memory.buffer, pointer, total);
            const status = out[0], body = out.slice(1);
            if (status !== 0) throw new Error(textDecoder.decode(body));
            return body;
        } finally { release(nameBlock); release(inputBlock); release(lengthBlock); if (pointer) core.free(pointer, total); }
    }
    function callJson(name, value) {
        codecs();
        const body = callBytes(name, textEncoder.encode(JSON.stringify(value === undefined ? null : value)));
        return body ? JSON.parse(textDecoder.decode(body)) : null;
    }
    // Typed-pointer access for hot kernels. The memory view must be re-read after every call.
    const runtime = Object.freeze({ allocation, view, release, exports: () => core });
    const api = { ready, strokeHitTest, checkpointCodec, unpackCheckpoint, warp, splitRgba, scanColors, balancePaper, separateInk, surfaceFit, inkAxes, inkRepair,
        folderMotion, folderFade, folderRows, callBytes, callJson, runtime, available: () => !!core };
    // >>> stream:scan_light (only this area edits between its markers)
    // <<< stream:scan_light
    // >>> stream:scan_color (only this area edits between its markers)
    // <<< stream:scan_color
    // >>> stream:scan_geometry (only this area edits between its markers)
    // <<< stream:scan_geometry
    // >>> stream:scan_live (only this area edits between its markers)
    // <<< stream:scan_live
    // >>> stream:editor_input (only this area edits between its markers)
    // <<< stream:editor_input
    // >>> stream:editor_render (only this area edits between its markers)
    // <<< stream:editor_render
    // >>> stream:editor_view (only this area edits between its markers)
    // <<< stream:editor_view
    // >>> stream:editor_history (only this area edits between its markers)
    // <<< stream:editor_history
    // >>> stream:collab (only this area edits between its markers)
    // <<< stream:collab
    // >>> stream:offline (only this area edits between its markers)
    // <<< stream:offline
    // >>> stream:docio (only this area edits between its markers)
    // PDF export bookkeeping computed in Rust (rust-core/src/docio). Every call throws when the core is missing:
    // callers `await InhouseRust.ready` first and must not continue with a JS substitute.
    api.docio = (() => {
        const need = () => {
            if (!core?.bridge_call) throw new Error('The Rust core (WASM) is required for PDF export but is not available');
            codecs();
        };
        const frame = (...parts) => {
            let size = 4 * parts.length;
            for (const part of parts) size += part.length;
            const out = new Uint8Array(size), view = new DataView(out.buffer);
            let at = 0;
            for (const part of parts) { view.setUint32(at, part.length, true); at += 4; out.set(part, at); at += part.length; }
            return out;
        };
        const encode = value => textEncoder.encode(typeof value === 'string' ? value : JSON.stringify(value));
        const NONE = new Uint8Array(0);
        let scratch = 0;
        const scratchPointer = () => {
            if (!scratch) scratch = core.alloc(128);
            if (!scratch) throw new Error('Rust scratch allocation failed');
            return scratch;
        };
        const number = value => typeof value === 'number' ? value : NaN;
        const readBounds = (view, at) => view.getUint8(at) === 1 ? {
            x: view.getFloat64(at + 1, true), y: view.getFloat64(at + 9, true),
            width: view.getFloat64(at + 17, true), height: view.getFloat64(at + 25, true) } : null;
        const stampSource = stamps => stamps instanceof Map ? [...stamps].map(([id, stamp]) => ({ id, stamp })) : stamps;
        // Strokes cross the boundary as a JSON "shell" (everything except the point coordinates) plus typed arrays
        // holding every point, because stringifying and re-parsing thousands of doubles dominates the cost otherwise.
        // Packing needs every stroke to be an object with an id and every point an object with numeric x/y and
        // numeric-or-absent pressure; anything else travels as plain JSON, which Rust handles identically.
        let packHolder = null;
        const packPlaceholder = function (key, value) {
            return key === 'points' && this === packHolder && Array.isArray(value) ? { '\u0001packedPoints': value.length } : value;
        };
        const packStrokes = (strokes, allowPacking = true) => {
            const plain = () => ({ json: JSON.stringify(strokes), coords: NONE, flags: NONE });
            if (!allowPacking || !strokes.length) return plain();
            let total = 0;
            for (let i = 0; i < strokes.length; i++) {
                const stroke = strokes[i];
                if (!stroke || typeof stroke !== 'object' || Array.isArray(stroke) || !stroke.id) return plain();
                if (Array.isArray(stroke.points)) total += stroke.points.length;
            }
            if (!total) return plain();
            const numbers = new Float64Array(total * 3), flags = new Uint8Array(total);
            let at = 0;
            for (let i = 0; i < strokes.length; i++) {
                const points = strokes[i].points;
                if (!Array.isArray(points)) continue;
                for (let k = 0; k < points.length; k++, at++) {
                    const point = points[k];
                    if (point === null || typeof point !== 'object') return plain();
                    const x = point.x, y = point.y, pressure = point.p;
                    if (typeof x !== 'number' || typeof y !== 'number') return plain();
                    numbers[at * 3] = x; numbers[at * 3 + 1] = y;
                    if (typeof pressure === 'number') { numbers[at * 3 + 2] = pressure; flags[at] = 1; }
                    else if (pressure !== undefined && pressure !== null) return plain();
                }
            }
            const shells = new Array(strokes.length);
            try {
                for (let i = 0; i < strokes.length; i++) { packHolder = strokes[i]; shells[i] = JSON.stringify(packHolder, packPlaceholder); }
            } finally { packHolder = null; }
            return { json: `[${shells.join(',')}]`, coords: new Uint8Array(numbers.buffer), flags };
        };
        const strokeList = value => Array.isArray(value) ? value : [];
        return Object.freeze({
            /** True when `strokes` can cross the boundary as typed arrays (exposed for tests and diagnostics). */
            packable(strokes) { return packStrokes(strokeList(strokes)).coords.length > 0; },
            /**
             * Duplicate removal, point compaction and the page metadata entry.
             * `job` holds the scalar page fields; `strokes` is the page's stroke array (null when reusing `cachedCompact`
             * and no bounds are needed). `options.pack = false` forces the plain-JSON hand-off (used by tests).
             * Returns { keep: indices of the surviving strokes, bounds, pdfPageIndex (as written to the entry),
             * compact: Uint8Array, meta: JSON text }.
             */
            pageExport(job, strokes = null, cachedCompact = NONE, options = {}) {
                need();
                const header = { ...job };
                if (header.deletedStrokeStamps) header.deletedStrokeStamps = stampSource(header.deletedStrokeStamps);
                const packed = strokes ? packStrokes(strokeList(strokes), options.pack !== false) : { json: '', coords: NONE, flags: NONE };
                const out = callBytes('docio.page_export', frame(encode(header), encode(packed.json), cachedCompact || NONE, packed.coords, packed.flags));
                const view = new DataView(out.buffer, out.byteOffset, out.byteLength);
                const count = view.getUint32(0, true), keep = new Array(count);
                for (let i = 0; i < count; i++) keep[i] = view.getUint32(4 + i * 4, true);
                const at = 4 + count * 4, bounds = readBounds(view, at);
                const pdfPageIndex = view.getFloat64(at + 33, true);
                const compactOffset = view.getUint32(at + 41, true), compactLength = view.getUint32(at + 45, true), start = at + 49;
                // `compact` is a view into the entry bytes: no copy, and it is what a later unchanged save passes back.
                return { keep, bounds, pdfPageIndex: Number.isNaN(pdfPageIndex) ? null : pdfPageIndex,
                    compact: out.subarray(start + compactOffset, start + compactOffset + compactLength),
                    meta: textDecoder.decode(out.subarray(start)) };
            },
            /** getPdfOverlayContentBounds: { x, y, width, height } or null. */
            overlayBounds(strokes, images, width, height, scale, options = {}) {
                need();
                const packed = packStrokes(strokeList(strokes), options.pack !== false);
                const out = callBytes('docio.overlay_bounds', frame(encode({ images, width, height, scale }), encode(packed.json), packed.coords, packed.flags));
                return readBounds(new DataView(out.buffer, out.byteOffset, out.byteLength), 0);
            },
            /** computePageHash. */
            pageHash(page, a4Width, a4Height, options = {}) {
                need();
                const background = page.backgroundImage;
                const strokes = Array.isArray(page.strokes);
                const header = { images: page.images, deletedStrokeIds: page.deletedStrokeIds,
                    deletedStrokeStamps: stampSource(page.deletedStrokeStamps), bgSource: page.backgroundSource,
                    pdfIndex: page.pdfPageIndex, bgLen: background ? background.length : 0,
                    bgHead: background ? background.slice(0, 160) : '', pageWidth: page.pageWidth, pageHeight: page.pageHeight,
                    a4Width, a4Height, strokes: strokes ? undefined : page.strokes };
                const packed = strokes ? packStrokes(page.strokes, options.pack !== false) : { json: '', coords: NONE, flags: NONE };
                return textDecoder.decode(callBytes('docio.page_hash', frame(encode(header), encode(packed.json), packed.coords, packed.flags)));
            },
            /** Export profile, scale/quality and (pdf-lib) the copyPages plan. */
            exportPlan(options) { need(); return callJson('docio.export_plan', options); },
            /** Stored page size in px and mm: { width, height, mmWidth, mmHeight, landscape }. */
            pageGeometry(width, height, a4Width, a4Height) {
                need();
                const pointer = scratchPointer();
                if (!core.docio_page_geometry(pointer, number(width), number(height), a4Width, a4Height)) throw new Error('Rust page geometry rejected its input');
                const v = new Float64Array(core.memory.buffer, pointer, 5);
                return { width: v[0], height: v[1], mmWidth: v[2], mmHeight: v[3], landscape: v[4] === 1 };
            },
            /** pdf-lib page size; copiedWidthPt/copiedHeightPt only for pages copied from the source PDF. */
            pdfLibGeometry(width, height, a4Width, a4Height, copiedWidthPt = NaN, copiedHeightPt = NaN) {
                need();
                const pointer = scratchPointer();
                if (!core.docio_pdflib_geometry(pointer, number(width), number(height), a4Width, a4Height, copiedWidthPt, copiedHeightPt)) throw new Error('Rust page geometry rejected its input');
                const v = new Float64Array(core.memory.buffer, pointer, 6);
                return { logicalWidth: v[0], logicalHeight: v[1], widthPx: v[2], heightPx: v[3], widthPt: v[4], heightPt: v[5] };
            },
            /** Placement of the cropped overlay PNG in PDF points. */
            overlayPlacement(bounds, widthPt, heightPt, logicalWidth, logicalHeight) {
                need();
                const pointer = scratchPointer();
                if (!core.docio_overlay_placement(pointer, bounds.x, bounds.y, bounds.width, bounds.height, widthPt, heightPt, logicalWidth, logicalHeight)) throw new Error('Rust overlay placement rejected its input');
                const v = new Float64Array(core.memory.buffer, pointer, 4);
                return { x: v[0], y: v[1], width: v[2], height: v[3] };
            }
        });
    })();
    // <<< stream:docio
    // >>> stream:storage (only this area edits between its markers)
    // <<< stream:storage
    // >>> stream:drive_home (only this area edits between its markers)
    // <<< stream:drive_home
    // >>> stream:calendar (only this area edits between its markers)
    // <<< stream:calendar
    // >>> stream:ts_engine (only this area edits between its markers)
    // <<< stream:ts_engine
    root.InhouseRust = Object.freeze(api);
})(globalThis);
