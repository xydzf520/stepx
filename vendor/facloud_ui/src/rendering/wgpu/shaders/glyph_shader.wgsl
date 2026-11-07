// Brightness-scaled contrast enhancement for glyph alpha masks.
//
// Linear sRGB blending makes light-on-dark text appear too thin because AA fringe
// pixels blend perceptually darker than expected. Dark-on-light text has the opposite
// problem — it already looks heavier than its geometric coverage.
//
// To compensate, we compute the text color's brightness (k) and use it to boost the
// glyph alpha through enhance_contrast(). Brighter text gets a stronger boost;
// dark text is left unchanged.
//
// enhance_contrast() adapted from DWrite_EnhanceContrast in Windows Terminal's DirectWrite shader:
// https://github.com/microsoft/terminal/blob/1283c0f5b99a2961673249fa77c6b986efb5086c/src/renderer/atlas/dwrite.hlsl
// Copyright (c) Microsoft Corporation.
// Licensed under the MIT license.
fn glyph_color_brightness(color: vec3<f32>) -> f32 {
    // REC. 601 luminance coefficients for perceived brightness.
    return dot(color, vec3<f32>(0.30, 0.59, 0.11));
}

fn enhance_contrast(alpha: f32, k: f32) -> f32 {
    return alpha * (k + 1.0) / (alpha * k + 1.0);
}

struct Uniforms {
    viewport_size: vec2<f32>,
    // Padding necessary to ensure that the uniforms is 16 bytes. Some wgpu-supported devices (such as webgl) require
    // buffer bindings to be a multiple of 16 bytes.
    padding: vec2<f32>
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@group(1) @binding(0) var glyphAtlasTexture: texture_2d<f32>;
@group(1) @binding(1) var glyphAtlasSampler: sampler;

struct GlyphVertexShaderInput {
    // The position of the vertex in normalized device coordinates.
    @location(0) vertex_position: vec2<f32>,
    @location(1) bounds: vec4<f32>,
    @location(2) uv_bounds: vec4<f32>,
    @location(3) fade_start: f32,
    @location(4) fade_end: f32,
    @location(5) color: vec4<f32>,
    @location(6) is_emoji: i32,
}

struct GlyphVertexShaderOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) rect_center: vec2<f32>,
    @location(1) rect_corner: vec2<f32>,
    @location(2) texture_coordinate: vec2<f32>,
    @location(3) fade_alpha: f32,
    @location(4) color: vec4<f32>,
    @location(5) is_emoji: i32,
    @location(6) fade_start: f32,
    @location(7) fade_end: f32,
}

@vertex
fn vs_main(
    glyph: GlyphVertexShaderInput,
) -> GlyphVertexShaderOutput {
    var out: GlyphVertexShaderOutput;
    var origin: vec2<f32> = glyph.bounds.xy;
    var size: vec2<f32> = glyph.bounds.zw;
    var pixel_pos: vec2<f32> = glyph.vertex_position * size + origin;

    // Use floor here to vertically align the glyph to the pixel grid.
    // If it's not aligned to the grid, the fragment shader will do its
    // own interpolation, which makes it so we don't use the anti-aliasing
    // from core text, which is what we want.  We don't force the glyph to a
    // horizontal pixel position because we rasterize the glyph at multiple
    // subpixel positions, and so the very slight linear interpolation here
    // won't produce a fuzzy glyph, just a correctly-positioned one.
    pixel_pos = vec2(pixel_pos.x, floor(pixel_pos.y));

    // Evaluating the glyphs fade effect. Note that the fade may go in two different directions:
    // - Right to left (default) - where the opaque side is on the right, and transparent on the left
    //   (in this case, the start_fade < end_fade; start is where the fade is transparent)
    // - Left to right - where the opaque side is on the left, and it fades towards the right side.
    //   In this case, start_fade > end_fade, and the opaque side is on the left (end_fade).
    // To clarify: fade_start is ALWAYS where the fade is transparent, and fade_end is ALWAYS where
    // the opaque part is, this is reflected in how we compute width, dist, and alpha.
    var fade_width: f32 = abs(glyph.fade_end - glyph.fade_start);
    var fade_dist: f32 = pixel_pos.x - min(glyph.fade_start, glyph.fade_end);

    var fade_alpha: f32;
    if glyph.fade_end <= -1.5 {
        // Bloom mode: fade_start encodes bloom progress in [0.0, 1.0].
        let p = clamp(glyph.fade_start, 0.0, 1.0);
        // Smooth cubic ease-out for opacity: 1 - (1 - p)^3
        let inv = 1.0 - p;
        fade_alpha = 1.0 - inv * inv * inv;
    } else if glyph.fade_end < glyph.fade_start { // left-to-right case
        fade_alpha = fade_dist / fade_width;
    } else { // right-to-left case
        fade_alpha = 1. - fade_dist / fade_width;
    }

     // Convert the position of the item from screen coordinates into normalized device coordinates
    var device_pos: vec2<f32> = pixel_pos / uniforms.viewport_size * vec2(2.0, -2.0) + vec2(-1.0, 1.0);

    var texture_coordinate: vec2<f32> = glyph.uv_bounds.xy + glyph.vertex_position * glyph.uv_bounds.zw;

    out.position = vec4<f32>(device_pos, 0.0, 1.0);
    out.rect_corner = size / 2.0;
    out.rect_center = origin + out.rect_corner;
    out.texture_coordinate = texture_coordinate;
    out.fade_alpha = fade_alpha;
    out.color = glyph.color;
    out.is_emoji = glyph.is_emoji;
    out.fade_start = glyph.fade_start;
    out.fade_end = glyph.fade_end;
    return out;
}

@fragment
fn fs_main(in: GlyphVertexShaderOutput) -> @location(0) vec4<f32> {
    let is_emoji_flag = in.is_emoji & 1;
    let is_optical_dark_comp = (in.is_emoji & 2) != 0;

    let atlas_dims = vec2<f32>(textureDimensions(glyphAtlasTexture));
    let texel = 1.0 / max(atlas_dims, vec2<f32>(1.0, 1.0));
    let is_bloom = in.fade_end <= -1.5;
    let bloom_p = select(1.0, clamp(in.fade_start, 0.0, 1.0), is_bloom);
    let radius_px = (1.0 - bloom_p) * (1.0 - bloom_p) * 1.5;
    let d = texel * radius_px;

    // Sample the 5-tap tent kernel in uniform control flow for WGSL derivative uniformity.
    let c0 = textureSample(glyphAtlasTexture, glyphAtlasSampler, in.texture_coordinate);
    let c1 = textureSample(glyphAtlasTexture, glyphAtlasSampler, in.texture_coordinate + vec2<f32>( d.x,  0.0));
    let c2 = textureSample(glyphAtlasTexture, glyphAtlasSampler, in.texture_coordinate + vec2<f32>(-d.x,  0.0));
    let c3 = textureSample(glyphAtlasTexture, glyphAtlasSampler, in.texture_coordinate + vec2<f32>( 0.0,  d.y));
    let c4 = textureSample(glyphAtlasTexture, glyphAtlasSampler, in.texture_coordinate + vec2<f32>( 0.0, -d.y));
    let blurred = c0 * 0.40 + (c1 + c2 + c3 + c4) * 0.15;
    var tex_color: vec4<f32> = select(c0, blurred, is_bloom && radius_px > 0.02);

    // is_emoji_flag is the atlas color-glyph switch. Use the input color for
    // monochrome glyph masks, and preserve sampled RGBA for emoji/color glyphs.
    var color: vec4<f32> = mix(in.color, tex_color, f32(is_emoji_flag));

    // Scale contrast boost by text brightness:
    // light text (white=1) gets boost; dark text (black=0) gets none.
    // For dark-mode optical weight compensation (460 / 560) and high-luminance
    // light-on-dark glyphs, apply halation reduction so strokes stay crisp.
    let luma = glyph_color_brightness(color.rgb);
    let halation_comp = smoothstep(0.55, 0.95, luma) * 0.22;
    let base_k = max(luma - halation_comp, 0.0);
    let weight_comp_k = select(base_k, base_k * 0.76, is_optical_dark_comp);
    let k_crisp = mix(weight_comp_k * 0.35, weight_comp_k, bloom_p);
    let contrasted = enhance_contrast(tex_color.r, k_crisp);
    color.a *= max(contrasted, f32(is_emoji_flag));

    // Apply the fade.
    color.a *= saturate(in.fade_alpha);
    return color;
}
