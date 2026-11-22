// xHoly UI overlay shader — Phase 10.
// Samples the software-composited UI layer and blends it over the
// game framebuffer (alpha-aware).

@group(0) @binding(0) var ui_tex: texture_2d<f32>;
@group(0) @binding(1) var ui_samp: sampler;
@group(0) @binding(2) var game_tex: texture_2d<f32>;
@group(0) @binding(3) var game_samp: sampler;

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
    let ui = textureSampleLevel(ui_tex, ui_samp, in.uv, 0.0);
    let game = textureSampleLevel(game_tex, game_samp, in.uv, 0.0);
    return mix(game, ui, ui.a);
}
