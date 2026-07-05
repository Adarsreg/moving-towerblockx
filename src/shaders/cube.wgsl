// Neon block shader (Step 4).
//
// Vertex stage places an instanced unit cube at its grid cell and applies a
// uniform-driven screen-shake. Fragment stage fakes a glowing neon material via
// an emissive body + Fresnel rim + a white flash spike on plane-clear. Output is
// linear color into an sRGB surface (hardware does the gamma encode).

struct Globals {
    view_proj  : mat4x4<f32>,
    camera_pos : vec4<f32>,
    // x = time (s), y = flash intensity, z = shake amplitude, w = level
    params     : vec4<f32>,
    // rgb = sky/haze color distant geometry fades into, a = fog density
    sky        : vec4<f32>,
    light_vp   : mat4x4<f32>,   // light view-projection for shadow mapping
};

@group(0) @binding(0) var<uniform> G : Globals;
@group(0) @binding(1) var shadowTex : texture_depth_2d;
@group(0) @binding(2) var shadowSamp : sampler_comparison;

// Depth-only vertex pass from the light's point of view (shadow map render).
@vertex
fn vs_shadow(in : VIn) -> @builtin(position) vec4<f32> {
    let world_pos = in.position * in.i_scale + in.i_pos;
    return G.light_vp * vec4<f32>(world_pos, 1.0);
}

// Fraction of light reaching `world_pos` (1 = lit, 0 = shadowed), 3x3 PCF.
fn shadow_factor(world_pos : vec3<f32>) -> f32 {
    let lp = G.light_vp * vec4<f32>(world_pos, 1.0);
    let ndc = lp.xyz / lp.w;
    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, ndc.y * -0.5 + 0.5);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0 || ndc.z > 1.0) {
        return 1.0; // outside the light frustum → treat as lit
    }
    let bias = 0.0015;
    let texel = 1.0 / 2048.0;
    var sum = 0.0;
    for (var y = -1; y <= 1; y = y + 1) {
        for (var x = -1; x <= 1; x = x + 1) {
            let off = vec2<f32>(f32(x), f32(y)) * texel;
            // ...Level variant samples at LOD 0 (no derivatives) so it's legal in
            // loops / after a conditional return — required by WGSL uniformity.
            sum = sum + textureSampleCompareLevel(shadowTex, shadowSamp, uv + off, ndc.z - bias);
        }
    }
    return sum / 9.0;
}

struct VIn {
    @location(0) position : vec3<f32>,   // cube-local, [-0.5, 0.5]
    @location(1) normal   : vec3<f32>,
    @location(2) i_pos    : vec3<f32>,   // world cell center (instance)
    @location(3) i_scale  : vec3<f32>,   // per-axis scale (instance)
    @location(4) i_color  : vec3<f32>,   // neon base (instance)
    @location(5) i_glow   : f32,         // emissive / mode (instance)
};

struct VOut {
    @builtin(position) clip         : vec4<f32>,
    @location(0)       world_normal : vec3<f32>,
    @location(1)       world_pos    : vec3<f32>,
    @location(2)       color        : vec3<f32>,
    @location(3)       glow         : f32,
};

@vertex
fn vs_main(in : VIn) -> VOut {
    var out : VOut;

    // Per-instance scale turns the unit cube into either a block or a thin bar.
    let world_pos = in.position * in.i_scale + in.i_pos;
    var clip = G.view_proj * vec4<f32>(world_pos, 1.0);

    // Screen-shake: jitter in clip space, scaled by w so it is resolution- and
    // depth-stable. Driven by the shake amplitude uniform (spikes on clear).
    let t     = G.params.x;
    let shake = G.params.z;
    clip.x += sin(t * 97.0)  * shake * clip.w * 0.03;
    clip.y += cos(t * 113.0) * shake * clip.w * 0.03;

    out.clip         = clip;
    out.world_normal = in.normal;
    out.world_pos    = world_pos;
    out.color        = in.i_color;
    out.glow         = in.i_glow;
    return out;
}

// Per-face tint keyed to the cube's axis. Independent of light direction, this
// alone makes the three planes (X / Y / Z) read as distinct surfaces — the main
// trick that turns flat-looking cubes into a legible 3D solid.
fn face_tint(n : vec3<f32>) -> f32 {
    let a = abs(n);
    if (a.y > 0.5) {
        // Top faces catch the most light; bottoms fall into shadow.
        if (n.y > 0.0) { return 1.18; } else { return 0.45; }
    }
    if (a.x > 0.5) { return 0.82; } // side walls
    return 0.62;                    // front/back (Z) → darkest, pushes depth
}

// --- Procedural texture: value noise for concrete/asphalt grain -------------
fn hash2(p : vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(41.31, 289.17))) * 43758.5453);
}
fn vnoise(p : vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let a = hash2(i);
    let b = hash2(i + vec2<f32>(1.0, 0.0));
    let c = hash2(i + vec2<f32>(0.0, 1.0));
    let d = hash2(i + vec2<f32>(1.0, 1.0));
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
// Two-octave fractal noise → richer surface texture.
fn fbm(p : vec2<f32>) -> f32 {
    return 0.65 * vnoise(p) + 0.35 * vnoise(p * 2.3 + 7.1);
}

@fragment
fn fs_main(in : VOut) -> @location(0) vec4<f32> {
    let N = normalize(in.world_normal);
    let V = normalize(G.camera_pos.xyz - in.world_pos);

    // --- Ghost shell: dark body with a bright Fresnel edge → wireframe look. ---
    if (in.glow < 0.0) {
        let grim = pow(1.0 - max(dot(N, V), 0.0), 1.8);
        var g = in.color * 0.06 + in.color * grim * 1.1;
        g += vec3<f32>(1.0) * G.params.y * 0.5;
        g = g / (g + vec3<f32>(1.0));
        return vec4<f32>(g, 1.0);
    }

    // Golden-hour key light + a cool sky fill = realistic modern lighting.
    let L = normalize(vec3<f32>(0.55, 0.72, 0.35));
    let ndl = max(dot(N, L), 0.0);
    let sky_fill = 0.35 + 0.35 * N.y;                 // brighter facing the sky
    let fres = pow(1.0 - max(dot(N, V), 0.0), 3.0);
    let sh = shadow_factor(in.world_pos);             // 1 = lit, 0 = shadowed

    // Diffuse base: sky-fill ambient (always) + sun (shadowed).
    var col = in.color * (0.22 * sky_fill + 0.85 * ndl * sh);

    let side = 1.0 - abs(N.y);
    // Procedural concrete/asphalt grain on the matte base (glass overrides below).
    var tc = in.world_pos.xz;
    if (side > 0.5) { tc = mix(in.world_pos.xy, in.world_pos.zy, step(0.5, abs(N.x))); }
    col = col * (0.85 + 0.28 * fbm(tc * 2.6));

    // --- Glass curtain-wall — only real building walls (vertical + low glow). --
    if (side > 0.5 && in.glow < 0.12) {
        // Grid along the wall (X on Z-walls, Z on X-walls) and up Y (per floor).
        let u = mix(in.world_pos.x, in.world_pos.z, step(0.5, abs(N.x)));
        let gu = fract(u * 1.35);
        let gv = fract(in.world_pos.y * 1.30);

        // Mullion mask: dark thin frame lines between the glass panes.
        let mu = smoothstep(0.0, 0.05, gu) * smoothstep(0.0, 0.05, 1.0 - gu);
        let mv = smoothstep(0.0, 0.08, gv) * smoothstep(0.0, 0.08, 1.0 - gv);
        let glassmask = mu * mv;

        // Reflective glass: reflects the sky, brighter at grazing angles (fresnel).
        let refl = G.sky.rgb * (0.35 + 0.9 * fres);
        var glass = vec3<f32>(0.06, 0.10, 0.16) + refl;      // cool tinted glass
        // Opaque spandrel band across the top of each floor → reads as floors.
        let spandrel = smoothstep(0.80, 0.90, gv);
        glass = mix(glass, in.color * 0.30, spandrel);
        // A scatter of lit interior windows for life.
        let cell = floor(u * 1.35) * 12.9 + floor(in.world_pos.y * 1.30) * 78.2;
        let lit = step(0.72, fract(sin(cell) * 43758.5));
        glass += vec3<f32>(1.0, 0.85, 0.55) * lit * 0.4 * glassmask;

        // Dark frame at the mullions, glass in the panes.
        col = mix(in.color * 0.10, glass, glassmask);
    }

    // Sun glint (Blinn-Phong specular) — a sharp highlight sweeping the glass.
    let H = normalize(L + V);
    let spec = pow(max(dot(N, H), 0.0), 80.0) * 0.8 * side;
    col += vec3<f32>(1.0, 0.95, 0.8) * spec * sh;

    // Emissive items (car lights, carried block, crane) + the clear flash.
    col += in.color * in.glow;
    col += vec3<f32>(1.0) * G.params.y;

    // NOTE: no tone-map here — we render to an HDR target so bright emissive
    // (lights, billboards, glints) stays >1 for the bloom pass to pick up. The
    // composite pass applies the filmic tone-map.

    // Atmospheric haze: fade distant geometry into the sky color for depth.
    let dist = length(G.camera_pos.xyz - in.world_pos);
    let fog = clamp(1.0 - exp(-dist * G.sky.a), 0.0, 0.92);
    col = mix(col, G.sky.rgb, fog);

    return vec4<f32>(col, 1.0);
}
