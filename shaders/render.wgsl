// xHoly render shader — samples an R8 "text bitmap" texture and outputs
// gold-on-dark pixels. Used by the Phase-2 GPU stub to display the
// "Hello, xHoly!" byte stream produced by the ARM64 CPU interpreter.

@group(0) @binding(0)
var tex: texture_2d<f32>;

@group(0) @binding(1)
var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// Fullscreen triangle (3 vertices cover the whole target).
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
    out.uv.y = 1.0 - out.uv.y; // flip for texture row order
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let bit = textureSampleLevel(tex, samp, in.uv, 0.0).r;
    let bg = vec3<f32>(0.10, 0.10, 0.10);   // #1a1a1a-ish
    let fg = vec3<f32>(1.00, 0.84, 0.00);   // gold #FFD700
    return vec4<f32>(mix(bg, fg, bit), 1.0);
}
