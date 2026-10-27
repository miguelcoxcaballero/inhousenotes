// Depth of field in a single gather pass (after Gustafsson 2018, "Bokeh
// depth of field in a single pass", and the scatter-as-gather idea of
// Jimenez 2014, "Next Generation Post Processing in Call of Duty: Advanced
// Warfare"): samples on a golden-angle spiral whose radius grows to the
// largest circle of confusion; each sample counts when its own CoC reaches
// the centre pixel, so blurred foreground spreads over sharp background
// while sharp foreground is not bled over by the blurred background (the
// background's CoC is clamped to the centre's). The circle of confusion is
// the thin-lens one of a physical camera (look.rs `DepthOfField`).
// Runs on the HDR image before bloom, so bright highlights become bright
// bokeh discs.
// Cost: up to ~64 colour + 64 depth taps per pixel (~1 ms at 1440p).
// `common.wgsl` is prepended at load time.

struct DofParams {
    p: vec4<f32>,  // x: k (px), y: focus distance (m), z: max radius (px), w: radius step
};

@group(1) @binding(0) var color_tex: texture_2d<f32>;
@group(1) @binding(1) var unused_tex: texture_2d<f32>;
@group(1) @binding(2) var depth_tex: texture_depth_2d;
@group(1) @binding(3) var lin_samp: sampler;
@group(1) @binding(4) var<uniform> dof: DofParams;

struct FsIn {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) i: u32) -> FsIn {
    let uv = vec2<f32>(f32((i << 1u) & 2u), f32(i & 2u));
    var o: FsIn;
    o.pos = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);
    o.uv = uv;
    return o;
}

fn coc_at(p: vec2<i32>) -> vec2<f32> {
    let q = clamp(p, vec2<i32>(0), vec2<i32>(frame.screen.xy) - vec2<i32>(1));
    let z = min(linear_depth(textureLoad(depth_tex, q, 0)), 1e5);
    return vec2<f32>(z, min(abs(dof.p.x * (1.0 - dof.p.y / z)), dof.p.z));
}

const GOLDEN: f32 = 2.39996323;

@fragment
fn fs_dof(in: FsIn) -> @location(0) vec4<f32> {
    let pix = vec2<i32>(in.pos.xy);
    let centre = coc_at(pix);
    var acc = textureLoad(color_tex, pix, 0).rgb;
    var tot = 1.0;
    let stp = dof.p.w;
    var radius = stp;
    // Per-pixel rotation hides the spiral's structure.
    var ang = ign(in.pos.xy) * 6.2831853;
    for (var i = 0; i < 96; i++) {
        if (radius >= dof.p.z) { break; }
        let o = vec2<f32>(cos(ang), sin(ang)) * radius;
        let sp = in.pos.xy + o;
        let s = coc_at(vec2<i32>(sp));
        var size = s.y;
        // Background behind the centre cannot blur over it.
        if (s.x > centre.x) { size = min(size, centre.y * 2.0); }
        let m = smoothstep(radius - 0.5, radius + 0.5, size);
        let col = textureSampleLevel(color_tex, lin_samp, sp * frame.screen.zw, 0.0).rgb;
        acc += mix(acc / tot, col, m);
        tot += 1.0;
        radius += stp / radius;
        ang += GOLDEN;
    }
    return vec4<f32>(acc / tot, 1.0);
}
