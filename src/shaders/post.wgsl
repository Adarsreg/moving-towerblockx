// Post-processing: HDR scene → cheap radial bloom → ACES filmic tone-map → sRGB.
//
// A single fullscreen pass. It samples the HDR scene texture in a ring pattern,
// keeps only the parts above 1.0 (the emissive lights / billboards / glints),
// accumulates them as a soft glow, adds it back, then tone-maps. Bilinear
// sampling at wide offsets fakes a gaussian blur cheaply — no ping-pong targets.

struct VOut {
    @builtin(position) pos : vec4<f32>,
    @location(0) uv : vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi : u32) -> VOut {
    // Fullscreen triangle.
    var xs = array<f32, 3>(-1.0, 3.0, -1.0);
    var ys = array<f32, 3>(-1.0, -1.0, 3.0);
    var out : VOut;
    let x = xs[vi];
    let y = ys[vi];
    out.pos = vec4<f32>(x, y, 0.0, 1.0);
    out.uv = vec2<f32>((x + 1.0) * 0.5, (1.0 - y) * 0.5);
    return out;
}

@group(0) @binding(0) var samp : sampler;
@group(0) @binding(1) var scene : texture_2d<f32>;

fn bright(c : vec3<f32>) -> vec3<f32> {
    let m = max(max(c.r, c.g), c.b);
    // Higher threshold → only genuinely bright emissive (lights/billboards) blooms.
    let k = max(m - 1.6, 0.0) / max(m, 0.0001);
    return c * k;
}

@fragment
fn fs_composite(in : VOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(scene));
    let texel = 1.0 / dims;

    let base = textureSample(scene, samp, in.uv).rgb;

    // Ring-sampled bloom at three radii.
    var bloom = vec3<f32>(0.0);
    var total = 0.0;
    for (var r = 1; r <= 3; r = r + 1) {
        let rad = f32(r) * 3.2;
        let wgt = 1.0 / f32(r);
        for (var a = 0; a < 8; a = a + 1) {
            let ang = f32(a) * 0.7853982; // 45° steps
            let off = vec2<f32>(cos(ang), sin(ang)) * rad * texel;
            bloom = bloom + bright(textureSample(scene, samp, in.uv + off).rgb) * wgt;
            total = total + wgt;
        }
    }
    bloom = bloom / max(total, 0.0001);

    var col = base + bloom * 0.55;

    // ACES filmic tone-map (Narkowicz approximation).
    col = (col * (2.51 * col + vec3<f32>(0.03))) / (col * (2.43 * col + vec3<f32>(0.59)) + vec3<f32>(0.14));
    col = clamp(col, vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(col, 1.0);
}
