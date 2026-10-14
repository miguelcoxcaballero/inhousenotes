/* Small, optional Rust/WASM adapter. No document or network state enters Rust. */
(function (root) {
    'use strict';
    const scriptUrl = typeof document !== 'undefined' && document.currentScript?.src;
    const base = scriptUrl || new URL(root.location.pathname.includes('/scanner/') ? '../../rust-core-v5.js' : 'rust-core-v5.js', root.location.href).href;
    const url = new URL('wasm/inhouse_notes_core.wasm', base);
    url.search = new URL(scriptUrl || root.location.href).search;
    let core = null;
    const ready = (async () => {
        try {
            if (typeof WebAssembly === 'undefined') return false;
            const response = await fetch(url, { signal: AbortSignal.timeout(4000) });
            if (!response.ok) return false;
            const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), {});
            if (instance.exports.core_abi_version() !== 1) return false;
            core = instance.exports;
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
    function scanColors(rgba, original, width, height, field, mask, fw, fh, parameters) {
        if (!core?.correct_scan_rows || ![width,height,fw,fh].every(value => Number.isSafeInteger(value) && value > 0)
            || width > 16384 || height > 16384 || rgba.length !== width * height * 4 || original.length !== rgba.length || field.length !== fw * fh
            || parameters.length !== 61 || (mask && mask.length !== fw * fh)) return null;
        const blocks = [];
        const copy = array => { const block = allocation(array.byteLength); blocks.push(block);
            view(block).set(new Uint8Array(array.buffer, array.byteOffset, array.byteLength)); return block; };
        try {
            const data = copy(rgba), raw = copy(original), light = copy(field), drawing = mask ? copy(mask) : null, params = copy(parameters);
            let disposed = false;
            return {
                rows(from, to) { if (!disposed) core.correct_scan_rows(data.pointer, raw.pointer, width, height,
                    light.pointer, drawing?.pointer || 0, fw, fh, params.pointer, from, to); },
                copyTo(destination) { if (!disposed) destination.set(view(data)); },
                dispose() { if (!disposed) { disposed = true; blocks.forEach(release); } }
            };
        } catch (_) { blocks.forEach(release); return null; }
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
    function inkAxes(rgba, width, height) {
        if (!core?.ink_axes || !Number.isSafeInteger(width) || !Number.isSafeInteger(height)
            || width < 2 || height < 2 || width > 4096 || height > 4096 || width*height > 12000000
            || rgba?.length !== width*height*4) return null;
        let pixels, mask, radii;
        try {
            pixels=allocation(rgba.length); mask=allocation(width*height); radii=allocation(width*height*2);
            view(pixels).set(rgba);
            const status=core.ink_axes(pixels.pointer,width,height,mask.pointer,radii.pointer);
            if (status===2) return {rejected:true};
            if (status!==1) return null;
            return {mask:view(mask).slice(), radii:new Uint16Array(core.memory.buffer,radii.pointer,width*height).slice(), backend:'rust'};
        } catch (_) { return null; }
        finally { release(pixels); release(mask); release(radii); }
    }
    root.InhouseRust = Object.freeze({ ready, warp, splitRgba, scanColors, separateInk, surfaceFit, inkAxes, available: () => !!core });
})(globalThis);
