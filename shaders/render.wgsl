// xHoly game-render shader — Phase 10 placeholder.
// Real game draw calls will bind translated DXIL→WGSL shaders here;
// this is the default fullscreen path used until then.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0), vec2<f32>(-1.0, 1.0), vec2<f32>(3.0, 1.0));
    var o: VsOut;
    o.pos = vec4<f32>(pos[vi], 0.0, 1.0);
    o.uv = pos[vi] * 0.5 + 0.5;
    o.uv.y = 1.0 - o.uv.y;
    return o;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // The bound texture is the CPU's R8 bit-bitmap (see
    // core::gpu::bytes_to_bitmap): bit=1 → trans blue, bit=0 → dark.
    let bit = textureSampleLevel(scene_tex, scene_samp, in.uv, 0.0).r;
    let bg = vec3<f32>(0.071, 0.086, 0.133); // #121622 deep blue-black
    let fg = vec3<f32>(0.357, 0.808, 0.980); // trans blue #5BCEFA
    return vec4<f32>(mix(bg, fg, bit), 1.0);
}
