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
    const api = { ready, checkpointCodec, unpackCheckpoint, warp, splitRgba, scanColors, balancePaper, separateInk, surfaceFit, inkAxes, inkRepair,
        folderMotion, folderFade, folderRows, callBytes, callJson, runtime, available: () => !!core };
    // >>> stream:scan_light (only this area edits between its markers)
    const scanLight = {};
    // Stencil removal (rust-core/src/scan_light/stencil.rs). `geometric` and `photographed` are the dot observation
    // lists of the scanner; the pixels are edited by calling step() until it returns false and then copyTo().
    scanLight.stencil = function stencil(rgba, width, height, pxPerCm, targetPaper, geometric, photographed) {
        if (!core?.scan_stencil_new) return null;
        if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 1 || height < 1 || width > 16384 || height > 16384
            || !ArrayBuffer.isView(rgba) || rgba.BYTES_PER_ELEMENT !== 1 || rgba.length !== width * height * 4
            || !Number.isFinite(pxPerCm) || pxPerCm <= 0 || !Number.isFinite(targetPaper)) return null;
        const first = Array.isArray(geometric) ? geometric : [], second = Array.isArray(photographed) ? photographed : [];
        const count = first.length + second.length;
        // The algorithm ignores observation lists outside this size range, so nothing is marshalled for them.
        const used = count >= 600 && count <= 6000 ? count : 0;
        const records = new Float64Array(used * 5);
        for (let i = 0; i < used; i++) {
            const point = i < first.length ? first[i] : second[i - first.length];
            const column = point.col ?? point.column, row = point.row;
            records[i * 5] = typeof column === 'number' ? column : NaN;
            records[i * 5 + 1] = typeof row === 'number' ? row : NaN;
            records[i * 5 + 2] = typeof point.x === 'number' ? point.x : NaN;
            records[i * 5 + 3] = typeof point.y === 'number' ? point.y : NaN;
            records[i * 5 + 4] = point.centerRefined === true && Number.isFinite(point.dotContrast) && point.dotContrast > 0 ? 1 : 0;
        }
        let pixels = null, input = null, output = null, context = 0, disposed = false;
        const dispose = () => {
            if (disposed) return;
            disposed = true;
            if (context) core.scan_stencil_drop(context);
            release(pixels); release(input); release(output);
        };
        try {
            pixels = allocation(rgba.length); view(pixels).set(new Uint8Array(rgba.buffer, rgba.byteOffset, rgba.length));
            if (used) { input = allocation(records.byteLength); view(input).set(new Uint8Array(records.buffer)); }
            output = allocation(7 * 8);
            context = core.scan_stencil_new(pixels.pointer, rgba.length, width, height, pxPerCm, targetPaper,
                input ? input.pointer : 0, used, used ? first.length : 0);
            if (!context) throw new Error('Stencil input rejected');
        } catch (_) { dispose(); return null; }
        return {
            // True while more work remains. Each call is one bounded slice, so the caller can yield and cancel between calls.
            step() {
                if (disposed) throw new Error('Stencil buffer released');
                const status = core.scan_stencil_step(context);
                if (status < 0) throw new Error('Rust stencil removal failed');
                return status === 1;
            },
            copyTo(destination) { if (!disposed) destination.set(view(pixels)); },
            result() {
                if (disposed || core.scan_stencil_result(context, output.pointer) !== 0) throw new Error('Rust stencil result unavailable');
                const v = new Float64Array(core.memory.buffer, output.pointer, 7);
                return { method: v[0] === 1 ? 'observed-dots' : 'recovered-dot-phase', observations: v[1], photographedObservations: v[2],
                    confirmed: v[3], isolatedRemoved: v[4], isolatedRejected: v[5], protectedComponents: v[6] };
            },
            dispose
        };
    };
    scanLight.isYellow = (r, g, b) => !!core?.scan_is_yellow?.(r, g, b); // one call per colour: for tests, not for pixel loops
    api.scanLight = scanLight;
    // <<< stream:scan_light
    // >>> stream:scan_color (only this area edits between its markers)
    // Capture quality (scan_color/quality.rs): acutance, lattice preflight and readiness decision. Pure typed-array
    // kernels; the host reads pixels. A raster is {width,height,rgba} (luma is computed in Rust) or {width,height,gray}.
    const QUALITY_REASONS = ['sharp-page', 'focus-or-motion-blur', 'sharp-grid', 'page-clipped', 'page-texture', 'grid-not-found'];
    function qualityMeasure(mode, raster, detection) {
        if (!core?.scan_color_quality || !core.scan_color_luma) throw new Error('Rust core is unavailable: capture quality kernels are missing');
        const quad = detection?.quad;
        if (!Array.isArray(quad) || quad.length !== 4) {
            if (mode === 1) return null;
            throw new RangeError('Capture quality needs a four-corner page quad');
        }
        if (typeof raster === 'function') raster = raster();
        const { width, height } = raster, pixels = width * height;
        if (!Number.isSafeInteger(width) || !Number.isSafeInteger(height) || width < 2 || height < 2 || width > 16384 || height > 16384
            || (raster.rgba ? raster.rgba.length !== pixels * 4 : !(raster.gray instanceof Float32Array) || raster.gray.length !== pixels)) {
            throw new RangeError('Capture quality raster has invalid dimensions');
        }
        const coords = [], offsets = [0];
        for (const edge of detection.frameEdges || []) {
            if (mode === 1 && !(edge && typeof edge[Symbol.iterator] === 'function')) { offsets.push(coords.length / 2); continue; }
            for (const point of edge) coords.push(point.x, point.y);
            offsets.push(coords.length / 2);
        }
        const blocks = [];
        const take = bytes => { const block = allocation(bytes); blocks.push(block); return block; };
        try {
            const grayBlock = take(pixels * 4), rgbaBlock = raster.rgba ? take(pixels * 4) : null;
            const quadBlock = take(64), edgeBlock = take(Math.max(16, coords.length * 8)), offsetBlock = take((offsets.length) * 4 + 4), outBlock = take(160);
            // Every allocation is done: memory can no longer grow, so these views stay valid.
            if (rgbaBlock) view(rgbaBlock).set(new Uint8Array(raster.rgba.buffer, raster.rgba.byteOffset, raster.rgba.byteLength));
            else new Float32Array(core.memory.buffer, grayBlock.pointer, pixels).set(raster.gray);
            const quadView = new Float64Array(core.memory.buffer, quadBlock.pointer, 8);
            quad.forEach((point, index) => { quadView[index * 2] = point.x; quadView[index * 2 + 1] = point.y; });
            new Float64Array(core.memory.buffer, edgeBlock.pointer, Math.max(2, coords.length)).set(coords);
            new Uint32Array(core.memory.buffer, offsetBlock.pointer, offsets.length).set(offsets);
            if (rgbaBlock && core.scan_color_luma(rgbaBlock.pointer, grayBlock.pointer, pixels) !== 0) throw new Error('Capture quality luma kernel rejected its input');
            const status = core.scan_color_quality(mode, grayBlock.pointer, width, height, quadBlock.pointer, edgeBlock.pointer,
                offsetBlock.pointer, offsets.length - 1, coords.length / 2, outBlock.pointer);
            if (status === -2) throw new RangeError('Capture frame edge has a non-finite coordinate');
            if (status !== 0) throw new Error('Capture quality kernel rejected its input');
            return Array.from(new Float64Array(core.memory.buffer, outBlock.pointer, 20));
        } finally { blocks.forEach(release); }
    }
    const qualitySharpness = o => ({ valid: o[0] === 1, clarity: o[1], border: o[2], interior: Number.isNaN(o[3]) ? null : o[3], zones: o[4],
        edges: o[5], interiorMinimum: Number.isNaN(o[6]) ? null : o[6], reason: QUALITY_REASONS[o[19]] });
    function qualityGrid(o) {
        const empty = { valid: false, dots: 0, matched: 0, rows: 0, columns: 0, coverage: 0, confidence: 0 };
        switch (o[8]) {
            case 1: return empty;
            case 2: return { ...empty, reason: 'page-clipped' };
            case 3: return { ...empty, dots: o[10], reason: 'page-texture' };
            default: return { valid: o[9] === 1, dots: o[10], matched: o[11], cells: o[12], rows: o[13], columns: o[14],
                coverage: o[15], confidence: o[16], contrast: o[17] };
        }
    }
    api.captureQuality = {
        inspect: (raster, detection) => qualitySharpness(qualityMeasure(0, raster, detection)),
        inspectGrid: (raster, detection) => { const o = qualityMeasure(1, raster, detection); return o ? qualityGrid(o) : qualityGrid([0, 0, 0, 0, 0, 0, 0, 0, 1]); },
        inspectReadiness(raster, detection) {
            const o = qualityMeasure(2, raster, detection), sharpness = qualitySharpness(o);
            if (o[7] === 1) return { ...sharpness, valid: false, grid: null };
            const grid = qualityGrid(o);
            return { ...sharpness, valid: grid.valid, grid };
        }
    };
    // <<< stream:scan_color
    // >>> stream:scan_geometry (only this area edits between its markers)
    // <<< stream:scan_geometry
    // >>> stream:scan_live (only this area edits between its markers)
    // <<< stream:scan_live
    // >>> stream:editor_input (only this area edits between its markers)
    // Resident stroke store for the eraser: coordinates are uploaded once per stroke (and generation) and every
    // later hit test sends only scalars plus a slot list. All geometry runs in rust-core/src/editor_input.rs.
    {
        const wasm = () => {
            if (!core?.editor_input_version) throw new Error('Rust editor core is unavailable');
            return core;
        };
        const entries = new WeakMap();
        let epoch = 1, lastGeneration = null, generationSeq = 0, generationId = 0;
        let boundsOut = 0, block = null, blockPoints = 0;
        const registry = typeof FinalizationRegistry === 'function' ? new FinalizationRegistry(held => {
            if (core && held.epoch === epoch) core.editor_input_slot_free(held.slot);
        }) : null;
        function clearStore() { core.editor_input_store_clear(); epoch++; }
        function currentGeneration(text) {
            if (text !== lastGeneration) { lastGeneration = text; generationSeq = generationSeq % 0xfffffffe + 1; generationId = generationSeq; }
            return generationId;
        }
        function upload(points, count, slot, generation) {
            const pointer = core.editor_input_scratch(count);
            if (!pointer) throw new RangeError('Stroke has too many points');
            const target = new Float64Array(core.memory.buffer, pointer, count * 2);
            for (let i = 0; i < count; i++) {
                const point = points[i];
                if (point) { target[i * 2] = point.x; target[i * 2 + 1] = point.y; }
                else { target[i * 2] = NaN; target[i * 2 + 1] = NaN; }
            }
            return core.editor_input_commit(slot, count, generation) === 1;
        }
        // The entry mirrors the old bounds cache: same array, length and end points means the stored bounds hold.
        function entryFor(stroke) {
            const points = stroke.points, count = points.length, first = points[0], last = points[count - 1];
            let entry = entries.get(stroke);
            if (entry && entry.epoch === epoch && entry.points === points && entry.count === count
                && entry.fx === first.x && entry.fy === first.y && entry.lx === last.x && entry.ly === last.y) return entry;
            if (!entry || entry.epoch !== epoch) {
                const slot = core.editor_input_slot_alloc();
                entry = { slot, epoch, bounds: null };
                registry?.register(stroke, { slot, epoch });
                entries.set(stroke, entry);
            }
            if (!upload(points, count, entry.slot, 0)) throw new Error('Rust stroke store rejected the stroke');
            boundsOut ||= core.alloc(32);
            core.editor_input_bounds(entry.slot, boundsOut);
            const b = new Float64Array(core.memory.buffer, boundsOut, 4);
            entry.bounds = { minX: b[0], minY: b[1], maxX: b[2], maxY: b[3] };
            entry.points = points; entry.count = count; entry.fx = first.x; entry.fy = first.y; entry.lx = last.x; entry.ly = last.y;
            return entry;
        }
        const usable = stroke => !!stroke && Array.isArray(stroke.points) && stroke.points.length > 0;
        api.strokeBounds = stroke => { wasm(); return usable(stroke) ? entryFor(stroke).bounds : null; };
        api.strokeMayIntersect = (stroke, x, y, radius, extra = 0) => {
            const c = wasm();
            return usable(stroke) && c.editor_input_may_intersect(entryFor(stroke).slot, x, y, radius, extra, (stroke.width || 0) * 0.5) === 1;
        };
        api.segmentDistance = (px, py, x1, y1, x2, y2) => wasm().editor_input_segment_distance(px, py, x1, y1, x2, y2);
        // Per page: stroke objects, their point arrays and lengths, plus the slot/half-width lists the Rust query reads.
        // Within one document generation, identical stroke and point-array references mean nothing changed, so a
        // query validates with three comparisons per stroke instead of re-deriving each stroke's signature.
        const pages = new WeakMap();
        function pageList(strokes, n, id) {
            let page = pages.get(strokes);
            if (page && page.id === id && page.epoch === epoch && page.n === n) {
                const { refs, arrays, counts } = page;
                let i = 0;
                for (; i < n; i++) {
                    const stroke = strokes[i], points = stroke.points;
                    if (stroke !== refs[i] || points !== arrays[i] || points.length !== counts[i]) break;
                }
                if (i === n) return page;
            }
            page = { id, epoch, n, refs: new Array(n), arrays: new Array(n), counts: new Uint32Array(n),
                slots: new Uint32Array(n), widths: new Float64Array(n) };
            for (let i = 0; i < n; i++) {
                const stroke = strokes[i];
                page.refs[i] = stroke; page.arrays[i] = stroke.points; page.counts[i] = stroke.points?.length ?? 0;
                if (stroke.tool && stroke.tool.startsWith('eraser') || !usable(stroke)) continue;
                page.slots[i] = entryFor(stroke).slot;
                page.widths[i] = (stroke.width || 0) * 0.5;
            }
            pages.set(strokes, page);
            return page;
        }
        // Indices (highest first) of the non-eraser strokes whose polyline passes within `radius` of (x, y).
        // `generation` invalidates retained coordinates after any document edit.
        api.strokesHitCircle = (strokes, x, y, radius, generation) => {
            const c = wasm(), n = strokes.length;
            if (!n) return [];
            const id = currentGeneration(generation);
            // A stroke that cannot be retained (store full) is tested once from the scratch buffer instead.
            let cleared = false;
            for (let attempt = 0; attempt < 3; attempt++) {
                const extra = [], page = pageList(strokes, n, id);
                let restart = false;
                for (let pass = 0; pass < 2 && !restart; pass++) {
                    const bytes = n * 20 + 8;
                    if (!block || blockPoints < n) {
                        if (block) c.free(block.pointer, block.bytes);
                        block = allocation(bytes); blockPoints = n;
                    }
                    const base = block.pointer, slotsAt = base + n * 8, outAt = slotsAt + n * 4, countsAt = outAt + n * 8;
                    new Float64Array(c.memory.buffer, base, n).set(page.widths);
                    new Uint32Array(c.memory.buffer, slotsAt, n).set(page.slots);
                    if (!c.editor_input_circle_query(slotsAt, base, n, x, y, radius, 2, id, 1, outAt, countsAt)) throw new Error('Rust eraser query rejected its arguments');
                    const memory = c.memory.buffer, counts = new Uint32Array(memory, countsAt, 2);
                    const hits = Array.from(new Uint32Array(memory, outAt, counts[0]));
                    if (!counts[1]) return extra.length ? hits.concat(extra).sort((a, b) => b - a) : hits;
                    for (const index of Array.from(new Uint32Array(memory, outAt + n * 4, counts[1]))) {
                        const stroke = strokes[index], count = stroke.points.length;
                        if (upload(stroke.points, count, entries.get(stroke).slot, id)) continue;
                        if (!cleared) { clearStore(); cleared = true; restart = true; break; }
                        // upload() left the points in the scratch buffer; test them without retaining.
                        if (c.editor_input_scratch_hit(count, x, y, radius) === 1) extra.push(index);
                        page.slots[index] = 0; page.id = 0;   // this list is only valid for the current query
                    }
                }
            }
            throw new RangeError('Rust stroke store could not hold the page');
        };
    }
    // <<< stream:editor_input
    // >>> stream:editor_render (only this area edits between its markers)
    // Stroke packing scratch: x,y f64 pairs followed by four f64 of bounds, kept in WASM
    // memory so the host writes coordinates once and Rust reads them in place.
    const STROKE_MAX_POINTS = 4194304, STROKE_KEEP_POINTS = 65536;
    let strokeScratch = null, strokeCapacity = 0;
    function requireCore() {
        if (!core) throw new Error('Rust core is not available; editor rendering requires WebAssembly');
        return core;
    }
    function strokeCoords(count) {
        requireCore();
        if (!Number.isSafeInteger(count) || count < 1 || count > STROKE_MAX_POINTS) throw new RangeError('Stroke point count out of range');
        if (count > strokeCapacity) {
            if (strokeScratch) { release(strokeScratch); strokeScratch = null; strokeCapacity = 0; }
            let capacity = 1024;
            while (capacity < count) capacity *= 2;
            strokeScratch = allocation(capacity * 16 + 32);
            strokeCapacity = capacity;
        }
        return new Float64Array(core.memory.buffer, strokeScratch.pointer, count * 2);
    }
    function strokeBounds(count) {
        if (!strokeScratch || !Number.isSafeInteger(count) || count < 1 || count > strokeCapacity) throw new RangeError('Stroke bounds need packed coordinates');
        const out = strokeScratch.pointer + strokeCapacity * 16;
        if (core.editor_render_stroke_bounds(strokeScratch.pointer, count, out) !== 1) throw new Error('Rust stroke bounds rejected the stroke');
        const bounds = new Float64Array(core.memory.buffer, out, 4);
        const result = [bounds[0], bounds[1], bounds[2], bounds[3]];
        if (strokeCapacity > STROKE_KEEP_POINTS) { release(strokeScratch); strokeScratch = null; strokeCapacity = 0; }
        return result;
    }
    // Per-stroke width of the opaque cover; -1 marks a stroke that is not covered.
    // One scratch block (tools | widths | counts | output, sized for `coverCapacity`) is reused across calls.
    let coverBlock = null, coverCapacity = 0;
    function legacyCoverWidths(strokes) {
        requireCore();
        const n = strokes.length, widths = new Float64Array(n);
        if (!n) return widths;
        if (n > coverCapacity) {
            if (coverBlock) { release(coverBlock); coverBlock = null; coverCapacity = 0; }
            let capacity = 64;
            while (capacity < n) capacity *= 2;
            coverBlock = allocation(capacity * 21 + 16);
            coverCapacity = capacity;
        }
        // Offsets depend on the capacity so every typed array stays aligned.
        const base = coverBlock.pointer, widthsAt = coverCapacity, countsAt = widthsAt + coverCapacity * 8, outAt = countsAt + coverCapacity * 4;
        const buffer = core.memory.buffer;
        const tools = new Uint8Array(buffer, base, n), input = new Float64Array(buffer, base + widthsAt, n), counts = new Uint32Array(buffer, base + countsAt, n);
        for (let i = 0; i < n; i++) {
            const stroke = strokes[i];
            if (!stroke || stroke.tool === 'eraser-stroke') { tools[i] = 2; input[i] = 0; counts[i] = 0; continue; }
            tools[i] = stroke.tool === 'highlighter' ? 1 : 0;
            const points = stroke.points, count = Array.isArray(points) ? points.length : 0;
            counts[i] = count;
            input[i] = count >= (tools[i] ? 1 : 2) ? +(stroke.width || 1) : 0;
        }
        if (core.editor_render_cover_widths(base, base + widthsAt, base + countsAt, n, base + outAt) !== 1)
            throw new Error('Rust cover widths rejected the strokes');
        widths.set(new Float64Array(core.memory.buffer, base + outAt, n));
        if (coverCapacity > 4096) { release(coverBlock); coverBlock = null; coverCapacity = 0; }
        return widths;
    }
    Object.assign(api, { renderStrokeCoords: strokeCoords, renderStrokeBounds: strokeBounds, legacyCoverWidths });
    // <<< stream:editor_render
    // >>> stream:editor_view (only this area edits between its markers)
    // Page layout cache and visible-window planner. Rust owns the layout; there is no JavaScript fallback.
    {
        const need = () => { if (!core?.ev_plan) throw new Error('Rust core is unavailable: the editor page layout requires WebAssembly'); return core; };
        let layoutGap = 0;
        api.editorView = Object.freeze({
            // Uploads the raw page sizes and rebuilds the layout inside Rust. Returns the scalar metrics.
            layout(pages, calendarEnabled, constants) {
                const wasm = need(), pageCount = pages.length;
                const valuePointer = wasm.ev_layout_values(pageCount), flagPointer = wasm.ev_layout_flags(pageCount);
                if (!valuePointer || !flagPointer) throw new RangeError('Page layout upload rejected');
                const values = new Float64Array(wasm.memory.buffer, valuePointer, pageCount * 2);
                const flags = new Uint8Array(wasm.memory.buffer, flagPointer, pageCount);
                for (let index = 0; index < pageCount; index += 1) {
                    const page = pages[index];
                    const width = page?.pageWidth, height = page?.pageHeight;
                    values[index] = typeof width === 'number' ? width : NaN;
                    values[pageCount + index] = typeof height === 'number' ? height : NaN;
                    flags[index] = calendarEnabled && page && page.sidePanel ? 1 : 0;
                }
                layoutGap = constants.gap;
                if (!wasm.ev_layout_build(pageCount, constants.padding, constants.gap, constants.a4Width, constants.a4Height, constants.calendarSpace)) {
                    throw new RangeError('Page layout rejected');
                }
                return {
                    pageCount: wasm.ev_layout_scalar(5), contentBottom: wasm.ev_layout_scalar(0),
                    bounds: { left: wasm.ev_layout_scalar(1), top: wasm.ev_layout_scalar(2), right: wasm.ev_layout_scalar(3), bottom: wasm.ev_layout_scalar(4) }
                };
            },
            // getPageTopOffset over the current layout (index clamping included).
            topOffset(pageIndex) { return need().ev_layout_top(pageIndex, layoutGap); },
            // Visible page window with `buffer` extra pages on both sides. start > end means empty.
            visibleRange(panY, zoom, rectHeight, buffer) {
                const wasm = need(), out = new Float64Array(wasm.memory.buffer, wasm.ev_range(panY, zoom, rectHeight, buffer), 2);
                return { start: out[0], end: out[1] };
            },
            // Planning half of updateVisiblePages. `pending` is a Map keyed by page index. Null means nothing to update.
            plan(view) {
                const wasm = need();
                const pending = view.pending, pendingCount = pending.size;
                const pointer = wasm.ev_plan_input(pendingCount);
                if (!pointer) throw new RangeError('Visible page plan rejected');
                const input = new Int32Array(wasm.memory.buffer, pointer, pendingCount);
                let at = 0;
                for (const index of pending.keys()) input[at++] = index;
                const integer = value => Number.isInteger(value) ? value : NaN;
                const written = wasm.ev_plan(view.panX, view.panY, view.zoom, view.rectWidth, view.rectHeight, view.buffer, view.maxActive,
                    integer(view.selection), integer(view.activePage), pendingCount);
                if (!written) return null;
                const out = new Int32Array(wasm.memory.buffer, wasm.ev_plan_output(), written);
                let cursor = 6;
                const take = length => { const list = out.slice(cursor, cursor + length); cursor += length; return list; };
                const plan = { rangeStart: out[0], rangeEnd: out[1] };
                plan.direct = take(out[2]); plan.ordered = take(out[3]); plan.previewOnly = take(out[4]); plan.prune = take(out[5]);
                return plan;
            }
        });
    }
    // <<< stream:editor_view
    // >>> stream:editor_history (only this area edits between its markers)
    // Item fingerprints (unit eh-1). Strings (ids, tool, color, actor) stay in JS, so lone surrogates and
    // non-string ids behave as before; Rust returns the numeric/hash tails that need per-point work.
    api.history = (() => {
        const RESERVE = 128, GROUP_POINTS = 350000, GROUP_ITEMS = 4096, STRING_CHUNK = 1 << 22, SHORT_TEXT = 256;
        let scratch = null, small = 0, decoder = null, encoder = null;
        function need() {
            if (!core?.history_stroke_batch) throw new Error('Rust core (WASM) is required for item fingerprints and is not loaded');
            decoder ||= new TextDecoder(); encoder ||= new TextEncoder();
            if (!small) small = allocation(256).pointer;
        }
        function reserve(bytes) {
            if (scratch && scratch.bytes >= bytes) return scratch.pointer;
            if (scratch) release(scratch);
            scratch = null;
            scratch = allocation(Math.max(bytes, 4096));
            return scratch.pointer;
        }
        function dropLargeScratch() { if (scratch && scratch.bytes > 32 * 1024 * 1024) { release(scratch); scratch = null; } }
        const finite = value => Number.isFinite(value) ? value : 0;
        function hashChunk(seed, text) {
            const length = text.length;
            let pointer, wide = 1;
            if (length > SHORT_TEXT) {
                pointer = reserve(length);
                const done = encoder.encodeInto(text, new Uint8Array(core.memory.buffer, pointer, length));
                if (done.read === length && done.written === length) wide = 0; // every unit fit one byte: ASCII
            }
            if (wide) {
                pointer = reserve(length * 2);
                const units = new Uint16Array(core.memory.buffer, pointer, length);
                for (let index = 0; index < length; index++) units[index] = text.charCodeAt(index);
            }
            const hash = core.history_hash_update(seed, pointer, length, wide);
            if (hash === -9223372036854775808n) throw new Error('Rust hash rejected its input');
            return Number(hash);
        }
        // Mirrors `str.length` / `charCodeAt` on the legacy hash: null throws, other non-strings hash as empty.
        function hashValue(value) {
            if (typeof value !== 'string') {
                if (value === null || value === undefined || value.length > 0) throw new TypeError('Cannot hash a non-string value');
                return 0;
            }
            let hash = 0;
            for (let start = 0; start < value.length; start += STRING_CHUNK)
                hash = hashChunk(hash, value.length <= STRING_CHUNK ? value : value.slice(start, start + STRING_CHUNK));
            return hash;
        }
        function smallText(length) {
            if (length < 0) throw new Error('Rust fingerprint output rejected');
            return decoder.decode(new Uint8Array(core.memory.buffer, small, length));
        }
        function simpleHash(text) { need(); return smallText(core.history_hash_text(hashValue(text), small, 256)); }
        const stampOf = item => item?.syncStamp || item?._syncStamp
            || (item?.updatedClock || item?.updatedBy ? { clock: item.updatedClock, actor: item.updatedBy } : null);
        const stampText = (clock, actor) => `${clock}|${String(actor || '')}`;
        // Fingerprint records for one group of truthy strokes: "width|count|hash<TAB>clock" lines.
        function strokeGroup(items, from, to, points, collab) {
            const count = to - from, metaBytes = count * 16, offsetBytes = ((count + 1) * 4 + 7) & ~7;
            const base = reserve(metaBytes + offsetBytes + points * 24 + count * RESERVE);
            const buffer = core.memory.buffer;
            const meta = new Float64Array(buffer, base, count * 2), offsets = new Uint32Array(buffer, base + metaBytes, count + 1);
            const coords = new Float64Array(buffer, base + metaBytes + offsetBytes, points * 3);
            let cursor = 0;
            for (let index = 0; index < count; index++) {
                const stroke = items[from + index], list = stroke.points;
                offsets[index] = cursor;
                meta[index * 2] = finite(stroke.width);
                meta[index * 2 + 1] = collab ? Number(stampOf(stroke)?.clock) || 0 : 0;
                if (!Array.isArray(list)) continue;
                for (let at = 0; at < list.length; at++, cursor++) {
                    const point = list[at], slot = cursor * 3;
                    if (point) {
                        const x = point.x, y = point.y, p = point.p;
                        coords[slot] = Number.isFinite(x) ? x : 0; coords[slot + 1] = Number.isFinite(y) ? y : 0; coords[slot + 2] = Number.isFinite(p) ? p : 0;
                    } else if (point === undefined && !(at in list)) { coords[slot] = NaN; coords[slot + 1] = 0; coords[slot + 2] = 0; }
                    else { coords[slot] = 0; coords[slot + 1] = 0; coords[slot + 2] = 0; }
                }
            }
            offsets[count] = cursor;
            if (cursor !== points) throw new Error('Stroke points changed while packing');
            const outPointer = base + metaBytes + offsetBytes + points * 24;
            const written = Number(core.history_stroke_batch(base, base + metaBytes, base + metaBytes + offsetBytes, count, points, outPointer, count * RESERVE));
            if (written < 0) throw new Error('Rust stroke fingerprint rejected its input');
            return decoder.decode(new Uint8Array(core.memory.buffer, outPointer, written)).split('\n');
        }
        function strokeFingerprints(strokes, collab) {
            need();
            const result = new Array(strokes.length);
            const live = [], slots = [];
            for (let index = 0; index < strokes.length; index++) {
                const stroke = strokes[index];
                if (stroke) { live.push(stroke); slots.push(index); } else result[index] = collab ? '|0|' : '';
            }
            let from = 0;
            while (from < live.length) {
                let to = from, points = 0;
                while (to < live.length && to - from < GROUP_ITEMS) {
                    const length = Array.isArray(live[to].points) ? live[to].points.length : 0;
                    if (to > from && points + length > GROUP_POINTS) break;
                    points += length; to++;
                }
                const lines = strokeGroup(live, from, to, points, collab);
                for (let index = from; index < to; index++) {
                    const stroke = live[index], line = lines[index - from], tab = line.indexOf('\t');
                    const sync = `${stroke.id || ''}|${stroke.tool || ''}|${stroke.color || ''}|${line.slice(0, tab)}`;
                    result[slots[index]] = collab ? `${sync}|${stampText(line.slice(tab + 1), stampOf(stroke)?.actor)}` : sync;
                }
                from = to;
            }
            dropLargeScratch();
            return result;
        }
        function imageFingerprint(image, collab) {
            if (!image) return collab ? '|0|' : '';
            need();
            const hash = hashValue(image.src || ''), stamp = collab ? stampOf(image) : null;
            const length = core.history_image_tail(hash, finite(image.x), finite(image.y), finite(image.width), finite(image.height),
                finite(image.rotation), collab ? Number(stamp?.clock) || 0 : 0, small, 256);
            const record = smallText(length), tab = record.indexOf('\t');
            const sync = `${image.id || ''}|${record.slice(0, tab)}`;
            return collab ? `${sync}|${stampText(record.slice(tab + 1), stamp?.actor)}` : sync;
        }
        const imageFingerprints = (images, collab) => images.map(image => imageFingerprint(image, collab));
        // Strokes whose fingerprints differ between two id-keyed snapshots, in legacy insertion order.
        function touchedStrokeIds(sourceById, targetById) {
            const prints = new Map(), pending = [];
            const note = stroke => { if (!prints.has(stroke)) { prints.set(stroke, ''); pending.push(stroke); } };
            sourceById.forEach((source, id) => { const target = targetById.get(id); if (target && target !== source) { note(source); note(target); } });
            if (pending.length) strokeFingerprints(pending, false).forEach((print, index) => prints.set(pending[index], print));
            const differs = (source, target) => !source || !target || (source !== target && prints.get(source) !== prints.get(target));
            const touched = new Set();
            sourceById.forEach((source, id) => { if (differs(source, targetById.get(id))) touched.add(id); });
            targetById.forEach((target, id) => { if (differs(sourceById.get(id), target)) touched.add(id); });
            return touched;
        }
        // True when two equally long item lists differ in any collab fingerprint (early exit per block).
        function collabItemsDiffer(kind, remote, local) {
            const compute = kind === 'image' ? imageFingerprints : strokeFingerprints;
            for (let start = 0; start < remote.length; start += 128) {
                const left = [], right = [];
                for (let index = start; index < Math.min(remote.length, start + 128); index++) {
                    if (remote[index] !== local[index]) { left.push(remote[index]); right.push(local[index]); }
                }
                if (!left.length) continue;
                const a = compute(left, true), b = compute(right, true);
                for (let index = 0; index < a.length; index++) if (a[index] !== b[index]) return true;
            }
            return false;
        }
        return Object.freeze({
            simpleHash,
            strokeFingerprint: stroke => stroke ? strokeFingerprints([stroke], false)[0] : '',
            strokeCollabFingerprint: stroke => strokeFingerprints([stroke], true)[0],
            strokeFingerprints, imageFingerprints,
            imageFingerprint: image => imageFingerprint(image, false),
            imageCollabFingerprint: image => imageFingerprint(image, true),
            touchedStrokeIds, collabItemsDiffer
        });
    })();
    // <<< stream:editor_history
    // >>> stream:collab (only this area edits between its markers)
    // Text in, text out: collaboration-core-v5.js builds the JSON, Rust hashes and canonicalizes it.
    const loneSurrogate = /[\ud800-\udbff](?![\udc00-\udfff])|(?<![\ud800-\udbff])[\udc00-\udfff]/;
    // Method names and the length cell live for the page lifetime: small calls skip two allocations.
    const collabNames = new Map();
    let collabLength = null;
    function collabMethod(name) {
        let method = collabNames.get(name);
        if (!method) {
            codecs();
            const bytes = textEncoder.encode(name), block = allocation(bytes.length);
            view(block).set(bytes);
            method = { pointer: block.pointer, length: bytes.length };
            collabNames.set(name, method);
        }
        return method;
    }
    function collabBytes(name, data) {
        if (!core?.bridge_call) throw new Error('Rust core is not loaded; collaboration hashing is unavailable');
        const method = collabMethod(name);
        collabLength ||= allocation(8);
        let input = null, pointer = 0, total = 0;
        try {
            if (data.length) { input = allocation(data.length); view(input).set(data); }
            pointer = core.bridge_call(method.pointer, method.length, input ? input.pointer : 0, data.length, collabLength.pointer);
            if (!pointer) throw new Error(`Rust call ${name} was rejected`);
            total = new Uint32Array(core.memory.buffer, collabLength.pointer, 1)[0];
            const out = new Uint8Array(core.memory.buffer, pointer, total);
            const text = textDecoder.decode(out.subarray(1));
            if (out[0] !== 0) throw new Error(text);
            return text;
        } finally { release(input); if (pointer) core.free(pointer, total); }
    }
    function collabCall(name, text) { codecs(); return collabBytes(name, textEncoder.encode(text)); }
    function collabHashString(text) {
        if (!core?.bridge_call) throw new Error('Rust core is not loaded; collaboration hashing is unavailable');
        const wellFormed = typeof text.isWellFormed === 'function' ? text.isWellFormed() : !loneSurrogate.test(text);
        if (wellFormed) return collabCall('collab.hash_utf8', text);
        // Unpaired surrogates do not survive UTF-8: hash the exact code units instead.
        const units = new Uint16Array(text.length);
        for (let i = 0; i < units.length; i++) units[i] = text.charCodeAt(i);
        return collabBytes('collab.hash_utf16', new Uint8Array(units.buffer));
    }
    api.collab = Object.freeze({ call: collabCall, hashString: collabHashString });
    // <<< stream:collab
    // >>> stream:offline (only this area edits between its markers)
    // Worker kernels (rust-core/src/offline/worker_kernels.rs): base64, timeline hash, PNG cache eviction plan.
    const BASE64_CHUNK = 24 * 1024 * 1024; // multiple of 3, so chunk boundaries never add padding
    function offlineCore() {
        if (!core?.offline_b64_encode || !core.bridge_call) throw new Error('Rust core is unavailable (offline kernels)');
        return core;
    }
    // btoa over the octets of `bytes`, without building a binary string first.
    api.b64Encode = function b64Encode(bytes) {
        offlineCore();
        if (!(bytes instanceof Uint8Array)) throw new TypeError('b64Encode needs a Uint8Array');
        codecs();
        const parts = [];
        for (let offset = 0; offset < bytes.length; offset += BASE64_CHUNK) {
            const chunk = bytes.subarray(offset, offset + BASE64_CHUNK);
            let input = null, output = null;
            try {
                input = allocation(chunk.length); output = allocation(Math.ceil(chunk.length / 3) * 4);
                view(input).set(chunk);
                const written = core.offline_b64_encode(input.pointer, chunk.length, output.pointer);
                if (written !== output.bytes) throw new Error('Rust base64 encode failed');
                parts.push(textDecoder.decode(view(output)));
            } finally { release(input); release(output); }
        }
        return parts.join('');
    };
    // atob, returning bytes. Rejects exactly the text atob rejects, with the same DOMException.
    api.b64Decode = function b64Decode(text) {
        offlineCore();
        if (typeof text !== 'string') throw new TypeError('b64Decode needs a string');
        if (!text.length) return new Uint8Array(0);
        codecs();
        const invalid = () => new DOMException('The string to be decoded is not correctly encoded.', 'InvalidCharacterError');
        let input = null, output = null;
        try {
            input = allocation(text.length);
            // Valid base64 is ASCII, one byte per code unit. Anything wider cannot fit, so it was not valid anyway.
            if (textEncoder.encodeInto(text, view(input)).read !== text.length) throw invalid();
            output = allocation(Math.floor(text.length / 4) * 3 + 3);
            const written = core.offline_b64_decode(input.pointer, text.length, output.pointer, output.bytes);
            if (written === -1) throw invalid();
            if (written < 0) throw new Error('Rust base64 decode rejected its arguments');
            return view(output).slice(0, written);
        } finally { release(input); release(output); }
    };
    // `<abs(hash) base36>:<length base36>` over the UTF-16 code units of `json`.
    api.timelineHash = function timelineHash(json) {
        offlineCore();
        if (typeof json !== 'string') throw new TypeError('timelineHash needs a string');
        codecs();
        let input = null, output = null;
        try {
            output = allocation(32);
            let length, utf16 = 0;
            if (typeof json.isWellFormed === 'function' && json.isWellFormed()) {
                // Encode straight into WASM memory. JSON is almost always ASCII (one byte per unit), so try that size
                // first and only retry with the worst case when wider characters did not fit.
                for (const factor of [1, 3]) {
                    release(input); input = null;
                    if (factor === 3 && json.length * 3 > 128 * 1024 * 1024) { // too wide for the worst case: size it exactly
                        const exact = textEncoder.encode(json);
                        input = allocation(exact.length); view(input).set(exact); length = exact.length; break;
                    }
                    input = allocation(Math.max(1, json.length * factor));
                    const done = textEncoder.encodeInto(json, view(input));
                    if (done.read === json.length) { length = done.written; break; }
                }
            } else { // lone surrogates cannot travel as UTF-8: hand over the raw code units (little-endian, as every WASM host)
                const units = new Uint16Array(json.length);
                for (let i = 0; i < units.length; i++) units[i] = json.charCodeAt(i);
                input = allocation(Math.max(1, units.byteLength)); view(input).set(new Uint8Array(units.buffer));
                length = units.byteLength; utf16 = 1;
            }
            const hashed = core.offline_timeline_hash(input.pointer, length, utf16, output.pointer, 32);
            if (!hashed) throw new Error('Rust timeline hash failed');
            return textDecoder.decode(view(output).subarray(0, hashed));
        } finally { release(input); release(output); }
    };
    // Prepared-PNG cache: how many of the oldest entries (sizes oldest first) to drop so `incoming` fits `limit`.
    api.pngCachePlan = function pngCachePlan(limit, sizes, incoming) {
        offlineCore();
        const plan = callJson('offline.png_cache_plan', { limit, sizes, incoming });
        if (!plan) throw new Error('Rust PNG cache plan failed');
        return plan;
    };
    // Library catalogue (rust-core/src/offline/library.rs): columnar JSON projections in, Uint32Array indexes out.
    const libraryCall = (name, value) => {
        offlineCore(); codecs();
        const body = callBytes(name, textEncoder.encode(JSON.stringify(value)));
        return new Uint32Array(body.buffer, body.byteOffset, body.length >> 2);
    };
    api.offlineLibrary = Object.freeze({
        // -> {identity, kept, overlay, patches}: identity means every record stays as it is; otherwise the source
        // indexes that survive, their remote overlay (or 0xFFFFFFFF), and (keptPosition, parentPosition, aliasTarget) triples
        merge(columns) {
            const words = libraryCall('offline.library_merge', columns), kept = words[0], patches = words[1];
            if (words[2]) return { identity: true, kept: words.subarray(3, 3), overlay: words.subarray(3, 3), patches: words.subarray(3, 3) };
            return { identity: false, kept: words.subarray(3, 3 + kept), overlay: words.subarray(3 + kept, 3 + 2 * kept), patches: words.subarray(3 + 2 * kept, 3 + 2 * kept + 3 * patches) };
        },
        // -> positions whose timestamp text only JavaScript can parse (answer them with `touch`)
        load(columns) { return libraryCall('offline.library_load', columns); },
        touch(changes) { libraryCall('offline.library_touch', changes); },
        ranks(values) { libraryCall('offline.library_ranks', values); },
        names(values) { libraryCall('offline.library_names', values); },
        folder(parent, needle, local) { return libraryCall('offline.library_folder', { parent, needle, local }); },
        sort(columns) { return libraryCall('offline.library_sort', columns); },
        recents(columns) { return libraryCall('offline.library_recents', columns); },
        home(quick) {
            const words = libraryCall('offline.library_home', { quick }), a = words[0], b = words[1], c = words[2];
            return [words.subarray(3, 3 + a), words.subarray(3 + a, 3 + a + b), words.subarray(3 + a + b, 3 + a + b + c)];
        }
    });
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
    // Ink checkpoint words (rust-core/src/storage.rs). The host reads the live point objects into typed arrays;
    // the layout, validation and range rules are decided in Rust. Both calls throw when the core is missing.
    const storageCore = name => { if (!core?.[name]) throw new Error('Rust core is unavailable (' + name + ')'); return core; };
    api.packInkWords = (raw, tags, counts) => {
        const wasm = storageCore('ink_pack_words');
        if (!(raw instanceof Float64Array) || !(tags instanceof Uint8Array) || !(counts instanceof Uint32Array)
            || raw.length !== tags.length * 3) throw new TypeError('Invalid ink point buffers');
        const blocks = [], put = array => { const block = allocation(Math.max(8, array.byteLength)); blocks.push(block);
            view(block).set(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)); return block; };
        try {
            const input = put(raw), shape = put(tags), strokes = put(counts);
            const words = allocation(Math.max(8, tags.length * 32)); blocks.push(words);
            const offsets = allocation(Math.max(8, counts.length * 8)); blocks.push(offsets);
            const used = wasm.ink_pack_words(input.pointer, shape.pointer, tags.length, strokes.pointer, counts.length, words.pointer, offsets.pointer);
            if (used < 0) throw new Error('Ink point packing rejected its input');
            return { words: new Float64Array(wasm.memory.buffer, words.pointer, used).slice(),
                offsets: new Float64Array(wasm.memory.buffer, offsets.pointer, counts.length).slice() };
        } finally { blocks.forEach(release); }
    };
    // ranges: [offset, count] pairs per stroke. Returns normally when valid and throws the original messages otherwise.
    api.validateInkWords = (words, ranges) => {
        if (!(words instanceof Float64Array) || !(ranges instanceof Float64Array) || ranges.length % 2) throw new TypeError('Invalid ink word buffers');
        if (!core?.ink_validate_words) {
            // Existing offline files must stay readable when WebAssembly cannot load (as with the checkpoint codec above).
            // Reading only: packing a page still requires the core.
            for (let s = 0; s < ranges.length; s += 2) {
                const offset = ranges[s], count = ranges[s + 1];
                if (!Number.isSafeInteger(offset) || !Number.isSafeInteger(count) || offset < 0 || count < 0 || offset % 4 || offset + count * 4 > words.length) throw new Error('Invalid ink point range');
                for (let i = 0; i < count; i++) { const flag = words[offset + i * 4 + 3]; if (flag !== 1 && flag !== 0) throw new Error('Invalid ink pressure flag'); }
            }
            return;
        }
        const wasm = core;
        const blocks = [], put = array => { const block = allocation(Math.max(8, array.byteLength)); blocks.push(block);
            view(block).set(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)); return block; };
        try {
            const status = wasm.ink_validate_words(put(words).pointer, words.length, put(ranges).pointer, ranges.length / 2);
            if (status === 1) throw new Error('Invalid ink point range');
            if (status === 2) throw new Error('Invalid ink pressure flag');
            if (status !== 0) throw new Error('Ink point validation rejected its input');
        } finally { blocks.forEach(release); }
    };
    // <<< stream:storage
    // >>> stream:drive_home (only this area edits between its markers)
    const driveCall = (name, value) => {
        if (!core?.bridge_call) throw new Error(`Rust core is not loaded (${name})`);
        return callJson(name, value);
    };
    const wellFormed = value => { const text = String(value ?? ''); return text.toWellFormed ? text.toWellFormed() : text; };
    // Folder palettes depend only on (colour, night, exact); a Drive account has a couple of dozen colours,
    // so repeated cards cost a Map lookup and one batched Rust call serves every miss.
    const paletteCache = new Map();
    // Fixed CSS values chosen by the Rust contrast decision (dark ink on light folders, white ink on dark ones).
    const DARK_INK = { ink: '#1f1a14', hl: 'rgba(0, 0, 0, 0.07)', hlStrong: 'rgba(0, 0, 0, 0.12)', line: 'rgba(0, 0, 0, 0.25)' };
    const LIGHT_INK = { ink: '#ffffff', hl: 'rgba(255, 255, 255, 0.2)', hlStrong: 'rgba(255, 255, 255, 0.3)', line: 'rgba(255, 255, 255, 0.55)' };
    api.driveFolderColors = (rawValues, fallback) => driveCall('drive_home.resolve_color', { fallback, items: rawValues.map(wellFormed) });
    api.driveFolderPalettes = (bases, night, exact = false) => {
        const mode = `${night ? 'n' : 'd'}${exact ? 'x' : 'b'}`, found = new Map(), misses = [];
        for (const base of bases) {
            if (found.has(base)) continue;
            const hit = paletteCache.get(mode + base);
            if (hit) found.set(base, hit); else { found.set(base, null); misses.push(base); }
        }
        if (misses.length) {
            const batch = driveCall('drive_home.palette', { night: !!night, exact: !!exact, items: misses });
            if (paletteCache.size + misses.length > 1024) paletteCache.clear();
            misses.forEach((base, index) => {
                const dark = batch.dark[index], at = index * 4;
                const palette = Object.freeze({ color: batch.colors[at], cover: batch.colors[at + 1], inside: batch.colors[at + 2], tab: batch.colors[at + 3], ...(dark ? DARK_INK : LIGHT_INK) });
                found.set(base, palette); paletteCache.set(mode + base, palette);
            });
        }
        return bases.map(base => found.get(base));
    };
    api.driveFolderViewTint = (color, night) => driveCall('drive_home.view_tint', { night: !!night, color });
    // <<< stream:drive_home
    // >>> stream:calendar (only this area edits between its markers)
    // <<< stream:calendar
    // >>> stream:ts_engine (only this area edits between its markers)
    // <<< stream:ts_engine
    root.InhouseRust = Object.freeze(api);
})(globalThis);
