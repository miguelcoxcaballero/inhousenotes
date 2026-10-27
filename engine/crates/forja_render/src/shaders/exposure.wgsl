// Eye adaptation (auto exposure), fully on the GPU:
// 1. `cs_begin` (1 thread, frame start): this frame renders pre-exposed by
//    last frame's exposure; the history keeps its own pre-exposure.
// 2. `cs_histogram` (after TAA): 64-bin histogram of log2 scene luminance
//    (HDR / pre-exposure) over every second pixel, shared-memory atomics.
// 3. `cs_adapt` (1 workgroup): mean log luminance between two percentiles
//    (ignores the black sky and the lamps themselves), a night-aware key
//    value (Krawczyk et al. 2005: the key falls with the adaptation
//    luminance, so night stays dark instead of being pulled to mid grey),
//    and exponential adaptation toward it (faster toward bright, like the
//    eye). The tonemapper reads the result in the same frame.
// Cost: ~0.05 ms at 1440p (a quarter of the pixels, one load each).

struct ExposureParams {
    a: vec4<f32>,   // x: dt (s), y: rate toward brighter (1/s), z: rate toward darker (1/s), w: 1 = instant
    b: vec4<f32>,   // x: min exposure, y: max exposure, z: compensation (x 2^ev), w: low percentile
    c: vec4<f32>,   // x: high percentile, y: histogram log2 min, z: log2 range, w: key calibration
    d: vec4<f32>,   // x: nits per unit, yzw: unused
};

struct ExposureState {
    pre: f32,
    prev_pre: f32,
    exposure: f32,
    avg_log: f32,
};

@group(0) @binding(0) var<uniform> ep: ExposureParams;
@group(0) @binding(1) var<storage, read_write> state: ExposureState;
@group(0) @binding(2) var<storage, read_write> hist: array<atomic<u32>, 64>;
@group(0) @binding(3) var hdr: texture_2d<f32>;

var<workgroup> local_hist: array<atomic<u32>, 64>;

@compute @workgroup_size(1)
fn cs_begin() {
    state.prev_pre = state.pre;
    state.pre = clamp(state.exposure, 1e-6, 1e7);
}

@compute @workgroup_size(16, 16)
fn cs_histogram(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    atomicStore(&local_hist[li % 64u], 0u);
    workgroupBarrier();
    let size = textureDimensions(hdr);
    let p = gid.xy * 2u;
    if (p.x < size.x && p.y < size.y) {
        let c = textureLoad(hdr, p, 0).rgb;
        let l = dot(c, vec3<f32>(0.2126, 0.7152, 0.0722)) / max(state.pre, 1e-8);
        var bin = 0u;
        if (l > 0.0) {
            bin = u32(clamp((log2(l) - ep.c.y) / ep.c.z * 64.0, 0.0, 63.0));
        }
        atomicAdd(&local_hist[bin], 1u);
    }
    workgroupBarrier();
    if (li < 64u) {
        let v = atomicLoad(&local_hist[li]);
        if (v > 0u) { atomicAdd(&hist[li], v); }
    }
}

@compute @workgroup_size(1)
fn cs_adapt() {
    var total = 0.0;
    for (var i = 0u; i < 64u; i++) {
        total += f32(atomicLoad(&hist[i]));
    }
    if (total < 1.0) { return; }
    let lo = total * ep.b.w;
    let hi = total * ep.c.x;
    var cum = 0.0;
    var wsum = 0.0;
    var lsum = 0.0;
    for (var i = 0u; i < 64u; i++) {
        let n = f32(atomicLoad(&hist[i]));
        // Part of this bin inside the [lo, hi] percentile window.
        let a = max(cum, lo);
        let b = min(cum + n, hi);
        cum += n;
        if (b > a) {
            let center = ep.c.y + (f32(i) + 0.5) / 64.0 * ep.c.z;
            wsum += b - a;
            lsum += (b - a) * center;
        }
    }
    let avg_log = select(ep.c.y, lsum / max(wsum, 1e-6), wsum > 0.0);
    let avg = exp2(avg_log);
    let nits = avg * ep.d.x;
    let key = 1.03 - 2.0 / (2.0 + log2(nits + 1.0) / log2(10.0));
    let target_e = clamp(key * ep.c.w * ep.b.z / max(avg, 1e-9), ep.b.x, ep.b.y);
    var e = target_e;
    if (ep.a.w < 0.5 && state.exposure > 0.0) {
        let cur = log2(state.exposure);
        let tgt = log2(target_e);
        let rate = select(ep.a.z, ep.a.y, tgt < cur);
        e = exp2(cur + (tgt - cur) * (1.0 - exp(-ep.a.x * rate)));
    }
    state.exposure = e;
    state.avg_log = avg_log;
}
