// xHoly post-processing shader — Phase 9.
//
// Pipeline: scene (RGBA8) → bright-pass (half res) → H blur → V blur
// → composite (bloom add + tone map + FXAA-lite).
//
// Entry points:
//   vs_main        — fullscreen triangle (shared)
//   bright_fs      — luminance threshold → half-res bright texture
//   blur_fs        — separable gaussian; direction via params.z sign? No:
//                    direction passed via `dir` uniform (see BlurParams)
//   composite_fs   — bloom + tonemap + FXAA-lite → final target

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0)
var<uniform> params: vec4<f32>; // x=bloom_strength y=bloom_threshold
                                // z=tonemap_mode w=aa_enable
@group(0) @binding(1)
var<uniform> size_params: vec4<f32>; // x=1/w y=1/h z=dir_x w=dir_y

// ---- bright pass ----------------------------------------------------

@group(1) @binding(0)
var scene_tex: texture_2d<f32>;
@group(1) @binding(1)
var scene_samp: sampler;

@fragment
fn bright_fs(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(scene_tex, scene_samp, in.uv, 0.0);
    let luma = dot(c.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let knee = params.y;
    // Soft-knee bright pass: keeps highlights, suppresses the rest.
    let soft = clamp(luma - knee + 0.25, 0.0, 0.5) * 2.0;
    let w = max(soft, luma - knee) / max(luma, 1e-4);
    return vec4<f32>(c.rgb * max(w, 0.0), 1.0);
}

// ---- separable blur --------------------------------------------------
// dir = (+1,0) horizontal or (0,+1) vertical, in pixels.

@group(1) @binding(0)
var blur_tex: texture_2d<f32>;
@group(1) @binding(1)
var blur_samp: sampler;

fn gaussian_weight(i: f32) -> f32 {
    // 9-tap [0.05 0.09 0.12 0.15 0.18 0.15 0.12 0.09 0.05]-ish via exp falloff
    return exp(-0.5 * i * i / 4.0);
}

@fragment
fn blur_fs(in: VsOut) -> @location(0) vec4<f32> {
    let texel = vec2<f32>(size_params.x, size_params.y);
    let dir = vec2<f32>(size_params.z, size_params.w);
    var acc = textureSampleLevel(blur_tex, blur_samp, in.uv, 0.0) * gaussian_weight(0.0);
    var wsum = gaussian_weight(0.0);
    for (var i = 1; i <= 4; i = i + 1) {
        let o = dir * texel * f32(i);
        let wp = gaussian_weight(f32(i));
        acc += textureSampleLevel(blur_tex, blur_samp, in.uv + o, 0.0) * wp;
        acc += textureSampleLevel(blur_tex, blur_samp, in.uv - o, 0.0) * wp;
        wsum += wp * 2.0;
    }
    return acc / wsum;
}

// ---- composite -------------------------------------------------------

@group(2) @binding(0)
var bloom_tex: texture_2d<f32>;
@group(2) @binding(1)
var bloom_samp: sampler;

fn luma(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn tonemap(c: vec3<f32>, mode: u32) -> vec3<f32> {
    switch mode {
        case 0u: { // linear
            return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
        }
        case 1u: { // reinhard
            return c / (vec3<f32>(1.0) + c);
        }
        case 2u: { // filmic (hable-ish)
            let a = 2.51; let b = 0.03;
            let cc = 2.43; let d = 0.59; let e = 0.14;
            return clamp(c * (a * c + b) / (c * (cc * c + d) + e),
                         vec3<f32>(0.0), vec3<f32>(1.0));
        }
        default: { // aces
            let a = 2.51; let b = 0.03;
            let cc = 2.43; let d = 0.59; let e = 0.14;
            return clamp(c * (a * c + b) / (c * (cc * c + d) + e),
                         vec3<f32>(0.0), vec3<f32>(1.0));
        }
    }
}

// FXAA-lite: blend toward neighbors on high-luma edges.
fn fxaa_lite(uv: vec2<f32>, col: vec4<f32>) -> vec4<f32> {
    let texel = vec2<f32>(size_params.x, size_params.y);
    let lc = luma(col.rgb);
    var edge = 0.0;
    var num = vec3<f32>(0.0);
    var den = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            if x == 0 && y == 0 { continue; }
            let s = textureSampleLevel(scene_tex, scene_samp,
                                       uv + vec2<f32>(f32(x), f32(y)) * texel, 0.0);
            let ls = luma(s.rgb);
            let w = 1.0 - abs(ls - lc); // similar luminance weighs more
            num += s.rgb * w;
            den += w;
            edge = max(edge, abs(ls - lc));
        }
    }
    // Only blend when an edge is present; strength grows with edge size.
    let t = clamp(edge * 2.0, 0.0, 0.6);
    let soft = num / max(den, 1e-4);
    return vec4<f32>(mix(col.rgb, soft, t), col.a);
}

@fragment
fn composite_fs(in: VsOut) -> @location(0) vec4<f32> {
    var col = textureSampleLevel(scene_tex, scene_samp, in.uv, 0.0);

    // Bloom add (bloom_strength is 0 when bloom disabled).
    if params.x > 0.0 {
        let b = textureSampleLevel(bloom_tex, bloom_samp, in.uv, 0.0);
        col = vec4<f32>(col.rgb + b.rgb * params.x, col.a);
    }

    // FXAA-lite before tonemapping (operates on scene HDR-ish values).
    if params.w > 0.5 {
        col = fxaa_lite(in.uv, col);
    }

    let mode = u32(params.z + 0.5);
    let rgb = tonemap(col.rgb, mode);
    return vec4<f32>(rgb, col.a);
}

// ---- vertex (shared) --------------------------------------------------

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0,  1.0),
        vec2<f32>( 3.0,  1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    out.uv = pos[vi] * 0.5 + 0.5;
    out.uv.y = 1.0 - out.uv.y;
    return out;
}
