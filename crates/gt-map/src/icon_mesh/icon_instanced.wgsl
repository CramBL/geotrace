// Instanced icon-template rendering.
//
// The color pipeline mirrors egui's own `egui.wgsl` (egui_wgpu, MIT) so
// icon pixels match what the CPU mesh path produces through epaint: colors
// stay in gamma space through the multiply, the framebuffer entry points
// convert (or not) exactly like egui's, and the same interleaved-gradient
// dither is applied.

struct Locals {
    screen_size_in_points: vec2<f32>,
    // 1 if dithering is enabled, 0 otherwise.
    dithering: u32,
    _padding: u32,
};
@group(0) @binding(0) var<uniform> r_locals: Locals;

struct VertexOutput {
    @location(0) color: vec4<f32>, // gamma 0-1, premultiplied
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(
    // Template vertex (per vertex).
    @location(0) t_pos: vec2<f32>,
    @location(1) t_color: vec4<f32>,
    @location(2) t_tint_slot: u32,
    // Instance (per instance).
    @location(3) i_center: vec2<f32>,
    @location(4) i_col_x: vec2<f32>,
    @location(5) i_col_y: vec2<f32>,
    @location(6) i_tint0: vec4<f32>,
    @location(7) i_tint1: vec4<f32>,
) -> VertexOutput {
    var out: VertexOutput;
    let screen_pos = i_center + i_col_x * t_pos.x + i_col_y * t_pos.y;
    out.position = vec4<f32>(
        2.0 * screen_pos.x / r_locals.screen_size_in_points.x - 1.0,
        1.0 - 2.0 * screen_pos.y / r_locals.screen_size_in_points.y,
        0.0,
        1.0,
    );
    let tint = select(i_tint0, i_tint1, t_tint_slot == 1u);
    // Componentwise gamma-space multiply of premultiplied colors: identical
    // to the CPU path's Color32 multiply, up to float rounding.
    out.color = t_color * tint;
    return out;
}

// The noise/dither/conversion helpers below are copied from egui.wgsl so
// the two pipelines quantize identically.

fn interleaved_gradient_noise(n: vec2<f32>) -> f32 {
    let f = 0.06711056 * n.x + 0.00583715 * n.y;
    return fract(52.9829189 * fract(f));
}

fn dither_interleaved(rgb: vec3<f32>, levels: f32, frag_coord: vec4<f32>) -> vec3<f32> {
    var noise = interleaved_gradient_noise(frag_coord.xy);
    noise = (noise - 0.5) * 0.95;
    return rgb + noise / (levels - 1.0);
}

fn linear_from_gamma_rgb(srgb: vec3<f32>) -> vec3<f32> {
    let cutoff = srgb < vec3<f32>(0.04045);
    let lower = srgb / vec3<f32>(12.92);
    let higher = pow((srgb + vec3<f32>(0.055)) / vec3<f32>(1.055), vec3<f32>(2.4));
    return select(higher, lower, cutoff);
}

@fragment
fn fs_main_linear_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    var out_color_gamma = in.color;
    if r_locals.dithering == 1u {
        let rgb = dither_interleaved(out_color_gamma.rgb, 256.0, in.position);
        out_color_gamma = vec4<f32>(rgb, out_color_gamma.a);
    }
    return vec4<f32>(linear_from_gamma_rgb(out_color_gamma.rgb), out_color_gamma.a);
}

@fragment
fn fs_main_gamma_framebuffer(in: VertexOutput) -> @location(0) vec4<f32> {
    var out_color_gamma = in.color;
    if r_locals.dithering == 1u {
        let rgb = dither_interleaved(out_color_gamma.rgb, 256.0, in.position);
        out_color_gamma = vec4<f32>(rgb, out_color_gamma.a);
    }
    return out_color_gamma;
}
