//! Step 4 — the render pipeline, GPU resources, and the per-frame render pass.
//!
//! `Renderer` owns the `Gpu` context plus everything needed to draw the scene in
//! a single instanced, depth-tested pass: the cube mesh buffers, a dynamically
//! updated instance buffer, the `Globals` uniform, a depth target, and the
//! neon shader pipeline. It also carries the small amount of effect state
//! (flash/shake) that spikes on a plane-clear and decays over time.

use std::sync::Arc;

use glam::{Mat4, Vec3};
use wgpu::util::DeviceExt;
use winit::window::Window;

use super::camera::{Camera, Globals, BASE_PITCH, BASE_YAW, MAX_PITCH, MIN_PITCH};
use super::cube::{self, Vertex};
use super::gpu::Gpu;
use super::instance::Instance;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// HDR scene target so bright emissive survives >1 for the bloom pass.
const HDR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Shadow map resolution + format (must match the `2048.0` in cube.wgsl).
const SHADOW_SIZE: u32 = 2048;
const SHADOW_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
/// Cap on visible boxes per frame: a tall tower + a growing skyline + crane +
/// particles. The draw clamps to this, so it's a safety bound, not a limit felt
/// in normal play.
const MAX_INSTANCES: usize = 4096;
/// Hard cap on live spark particles.
const MAX_PARTICLES: usize = 600;

/// A single spark from a plane-clear burst. Pure data; integrated on the CPU.
struct Particle {
    pos: Vec3,
    vel: Vec3,
    life: f32,
    max_life: f32,
    color: [f32; 3],
}

pub struct Renderer {
    pub gpu: Gpu,

    pipeline: wgpu::RenderPipeline,
    vertex_buf: wgpu::Buffer,
    index_buf: wgpu::Buffer,
    index_count: u32,

    instance_buf: wgpu::Buffer,

    globals: Globals,
    globals_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup, // main pass: globals + shadow map

    // Shadow mapping.
    shadow_pipe: wgpu::RenderPipeline,
    shadow_view: wgpu::TextureView,
    shadow_bind_group: wgpu::BindGroup, // shadow pass: globals only

    depth_view: wgpu::TextureView,

    // Bloom + tone-map post pass.
    hdr_view: wgpu::TextureView,
    samp: wgpu::Sampler,
    post_pipe: wgpu::RenderPipeline,
    post_bgl: wgpu::BindGroupLayout,
    post_bg: wgpu::BindGroup,

    camera: Camera,

    // Effect state, decayed each frame in `update_effects`.
    flash: f32,
    shake: f32,
    time: f32,

    // Player-driven orbit offsets (accumulated from mouse drag), added on top of
    // the base framing + a gentle time-based sway.
    cam_yaw: f32,
    cam_pitch: f32,
    // True while the mouse is held: pauses the auto-sway so it never fights the
    // player's drag.
    dragging: bool,

    // Build-burst spark particles + a tiny PRNG to vary them.
    particles: Vec<Particle>,
    spark_rng: u32,

    // Smoothed camera focus (follows the growing tower).
    focus_x: f32,
    focus_y: f32,
    focus_r: f32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Self {
        let gpu = Gpu::new(window).await;
        let device = &gpu.device;

        // --- Static mesh buffers -------------------------------------------
        let vertex_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-vertices"),
            contents: bytemuck::cast_slice(cube::VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("cube-indices"),
            contents: bytemuck::cast_slice(cube::INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // --- Dynamic instance buffer (written every frame) -----------------
        let instance_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (std::mem::size_of::<Instance>() * MAX_INSTANCES) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- Globals uniform + bind group ----------------------------------
        let camera = Camera::new(gpu.aspect());
        let mut globals = Globals::new();
        globals.view_proj = camera.view_proj().to_cols_array_2d();
        globals.camera_pos = [camera.eye.x, camera.eye.y, camera.eye.z, 1.0];

        let globals_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("globals"),
            contents: bytemuck::bytes_of(&globals),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let globals_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };

        // Shadow-pass layout: globals only (the depth-only vertex shader).
        let shadow_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("shadow-bgl"),
            entries: &[globals_entry],
        });
        let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("shadow-bg"),
            layout: &shadow_bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });

        // Shadow map depth texture + comparison sampler.
        let shadow_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shadow-map"),
            size: wgpu::Extent3d { width: SHADOW_SIZE, height: SHADOW_SIZE, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: SHADOW_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let shadow_view = shadow_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let shadow_samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("shadow-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            compare: Some(wgpu::CompareFunction::LessEqual),
            ..Default::default()
        });

        // Main-pass layout: globals + shadow depth texture + comparison sampler.
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("main-bgl"),
            entries: &[
                globals_entry,
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("main-bg"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&shadow_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&shadow_samp) },
            ],
        });

        // --- Shaders + pipelines -------------------------------------------
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cube-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/cube.wgsl").into()),
        });

        // Shadow pipeline: depth-only from the light's POV.
        let shadow_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shadow-layout"),
            bind_group_layouts: &[&shadow_bgl],
            push_constant_ranges: &[],
        });
        let shadow_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shadow-pipe"),
            layout: Some(&shadow_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_shadow"),
                buffers: &[Vertex::layout(), Instance::layout()],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: SHADOW_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                // Slope-scaled bias to reduce shadow acne.
                bias: wgpu::DepthBiasState { constant: 2, slope_scale: 2.5, clamp: 0.0 },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cube-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout(), Instance::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: HDR_FORMAT, // render the scene to the HDR target
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // Culling disabled: neon cubes look fine double-sided and this
                // removes winding-order as a possible "invisible geometry" cause.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let depth_view = create_depth_view(device, &gpu.config);

        // --- Post-processing (bloom + ACES tone-map) -----------------------
        let samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let post_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let post_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post-shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/post.wgsl").into()),
        });
        let post_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post-layout"),
            bind_group_layouts: &[&post_bgl],
            push_constant_ranges: &[],
        });
        let post_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("post-pipe"),
            layout: Some(&post_layout),
            vertex: wgpu::VertexState {
                module: &post_shader,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &post_shader,
                entry_point: Some("fs_composite"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: gpu.config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let (hdr_view, post_bg) = create_hdr(device, &gpu.config, &samp, &post_bgl);

        Self {
            gpu,
            pipeline,
            vertex_buf,
            index_buf,
            index_count: cube::INDICES.len() as u32,
            instance_buf,
            globals,
            globals_buf,
            bind_group,
            shadow_pipe,
            shadow_view,
            shadow_bind_group,
            depth_view,
            hdr_view,
            samp,
            post_pipe,
            post_bgl,
            post_bg,
            camera,
            flash: 0.0,
            shake: 0.0,
            time: 0.0,
            cam_yaw: 0.0,
            cam_pitch: 0.0,
            dragging: false,
            particles: Vec::new(),
            spark_rng: 0x9e3779b9,
            focus_x: 0.0,
            focus_y: 4.0,
            focus_r: 22.0,
        }
    }

    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        self.gpu.resize(new_size);
        self.camera.aspect = self.gpu.aspect();
        self.depth_view = create_depth_view(&self.gpu.device, &self.gpu.config);
        let (hv, bg) = create_hdr(&self.gpu.device, &self.gpu.config, &self.samp, &self.post_bgl);
        self.hdr_view = hv;
        self.post_bg = bg;
    }

    /// Accumulate a mouse-drag orbit delta (radians). Yaw is unbounded (full
    /// 360° orbit); pitch is clamped so it can never accumulate past the usable
    /// range — that overshoot was what made the camera feel "stuck" when you
    /// dragged back the other way.
    pub fn orbit_input(&mut self, dyaw: f32, dpitch: f32) {
        self.cam_yaw += dyaw;
        self.cam_pitch =
            (self.cam_pitch + dpitch).clamp(MIN_PITCH - BASE_PITCH, MAX_PITCH - BASE_PITCH);
    }

    /// Whether the player is actively dragging (pauses auto-sway).
    pub fn set_dragging(&mut self, dragging: bool) {
        self.dragging = dragging;
    }

    /// A small camera kick for impact moments (piece lock / hard drop).
    pub fn impact(&mut self, amount: f32) {
        self.shake = (self.shake + amount).min(2.0);
    }

    /// A cheap xorshift float in [0, 1) for particle variation.
    fn frand(&mut self) -> f32 {
        let mut x = self.spark_rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.spark_rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Point the camera focus at a world position + orbit radius (eased in the
    /// render step so the follow is smooth as the tower grows).
    pub fn set_focus(&mut self, x: f32, y: f32, radius: f32) {
        self.focus_x = x;
        self.focus_y = y;
        self.focus_r = radius;
    }

    /// Burst a shower of sparks at a world point — fired on a floor landing and
    /// (bigger) on a completed building. `power` scales count + flash + shake.
    pub fn burst(&mut self, at: [f32; 3], color: [f32; 3], power: f32) {
        self.flash = (self.flash + 0.25 * power).min(1.0);
        self.shake = (self.shake + 0.4 * power).min(2.0);
        let n = (24.0 * power) as usize;
        for _ in 0..n {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            let (rx, ry, rz, rl) = (self.frand(), self.frand(), self.frand(), self.frand());
            let life = 0.5 + rl * 0.6;
            self.particles.push(Particle {
                pos: Vec3::new(at[0], at[1], at[2]),
                vel: Vec3::new((rx - 0.5) * 9.0, ry * 9.0 + 2.0, (rz - 0.5) * 9.0),
                life,
                max_life: life,
                color,
            });
        }
    }

    /// Advance time, decay the effect envelopes, and integrate particles.
    pub fn update_effects(&mut self, dt: f32) {
        self.time += dt;
        let decay = (-dt * 6.0).exp();
        self.flash *= decay;
        self.shake *= decay;

        // Ballistic sparks with gravity + drag, culled when their life expires.
        for p in &mut self.particles {
            p.vel.y -= 14.0 * dt;
            p.vel *= 1.0 - (2.0 * dt).min(0.9);
            p.pos += p.vel * dt;
            p.life -= dt;
        }
        self.particles.retain(|p| p.life > 0.0);

        // Smoothly follow the tower: ease camera target + orbit radius.
        let k = 1.0 - (-dt * 3.0).exp();
        self.camera.target.x += (self.focus_x - self.camera.target.x) * k;
        self.camera.target.y += (self.focus_y - self.camera.target.y) * k;
        self.camera.radius += (self.focus_r - self.camera.radius) * k;
    }

    /// Append the animated Gurgaon environment around the plot: expressways with
    /// moving traffic, an elevated Rapid Metro line with a passing train, and a
    /// distant glass-tower skyline that dissolves into the golden haze. `cx` is
    /// the current tower's X so the world scrolls with the camera.
    pub fn append_environment(&self, out: &mut Vec<Instance>, cx: f32, has_city: bool) {
        let t = self.time;
        let span = 90.0;

        let push = |out: &mut Vec<Instance>, p: [f32; 3], s: [f32; 3], c: [f32; 3], g: f32| {
            out.push(Instance { position: p, scale: s, color: c, glow: g });
        };

        let len = span * 2.0;
        // --- Two multi-lane expressways along X: dark asphalt + dashed markings. ---
        for &rz in &[-15.0f32, 15.0] {
            push(out, [cx, 0.05, rz], [len, 0.1, 9.0], [0.045, 0.045, 0.055], 0.0);
            let mut dx = -span;
            while dx < span {
                push(out, [cx + dx, 0.11, rz], [1.8, 0.03, 0.22], [1.3, 1.2, 0.7], 0.4);
                dx += 4.5;
            }
        }
        // --- Cars: real bodies + cabin + glowing head/tail lights, streaming. ---
        let ncar = 16usize;
        for i in 0..ncar {
            let f = i as f32 / ncar as f32;
            let e = ((t * 11.0 + f * len) % len) - span; // eastbound offset
            let w = span - ((t * 9.5 + f * len) % len); // westbound offset
            emit_car(out, [cx + e, 0.5, -16.6], car_color(i), true);
            emit_car(out, [cx + w, 0.5, -13.4], car_color(i + 5), false);
            emit_car(out, [cx + e, 0.5, 13.4], car_color(i + 11), true);
            emit_car(out, [cx + w, 0.5, 16.6], car_color(i + 3), false);
        }

        // --- Elevated Rapid Metro running ALONG the corridor (parallel to the
        //     expressways), with pillars, rails, a station platform + a train. ---
        let mz = 24.0; // just outside the +Z median
        let ty = 7.0;
        let mut x = -span;
        while x <= span {
            push(out, [cx + x, ty * 0.5, mz], [0.9, ty, 0.9], [0.30, 0.32, 0.40], 0.2); // pillar
            x += 13.0;
        }
        push(out, [cx, ty, mz], [len, 0.6, 1.8], [0.34, 0.36, 0.44], 0.2); // guideway beam
        push(out, [cx, ty + 0.38, mz - 0.5], [len, 0.06, 0.12], [0.6, 0.6, 0.7], 0.3); // rail
        push(out, [cx, ty + 0.38, mz + 0.5], [len, 0.06, 0.12], [0.6, 0.6, 0.7], 0.3); // rail
        push(out, [cx, ty - 0.35, mz], [11.0, 0.3, 4.2], [0.32, 0.32, 0.36], 0.12); // station platform
        push(out, [cx, ty + 2.2, mz], [12.0, 0.25, 5.0], [0.20, 0.22, 0.28], 0.1); // station canopy

        let train_x = ((t * 16.0) % (len + 40.0)) - span - 20.0;
        for c in 0..6 {
            let xc = cx + train_x + c as f32 * 3.35;
            push(out, [xc, ty + 1.0, mz], [3.15, 1.5, 2.2], [0.55, 0.85, 1.6], 0.5); // car
            push(out, [xc, ty + 1.15, mz - 1.12], [2.6, 0.45, 0.06], [2.4, 3.0, 3.4], 1.4); // windows
            push(out, [xc, ty + 1.15, mz + 1.12], [2.6, 0.45, 0.06], [2.4, 3.0, 3.4], 1.4);
        }

        // --- Times-Square-style animated LED billboards facing the plaza. ---
        emit_billboard(out, [cx - 11.0, 10.0, -16.6], 11.0, 6.5, t, 0.0);
        emit_billboard(out, [cx + 3.0, 9.0, -16.6], 8.0, 5.0, t, 3.1);
        emit_billboard(out, [cx + 13.0, 12.0, 16.6], 12.0, 7.0, t, 1.7);
        emit_billboard(out, [cx - 6.0, 11.0, 16.6], 9.0, 6.0, t, 5.0);

        // --- Distant skyline ring — only as a FALLBACK when the real OSM
        //     Gurgaon city hasn't loaded yet. Once it has, the real buildings
        //     replace this. ---
        if has_city {
            return;
        }
        let towers = 44u32;
        for i in 0..towers {
            let h1 = hash1(i);
            let h2 = hash1(i.wrapping_mul(7) ^ 0x51ed);
            let ang = i as f32 * 2.399_963; // golden-angle spread
            let r = 48.0 + h1 * 46.0;
            let h = 10.0 + h2 * 42.0;
            let w = 4.0 + h1 * 6.0;
            let x = cx + r * ang.cos();
            let zz = r * ang.sin();
            // cool bluish glass so the warm haze reads as depth against it.
            let tint = [0.08 + h2 * 0.06, 0.11 + h1 * 0.06, 0.18 + h2 * 0.08];
            push(out, [x, h * 0.5, zz], [w, h, w], tint, 0.06);
        }
    }

    /// Append live particles as glowing instances (shrinking + fading with life).
    pub fn append_particles(&self, out: &mut Vec<Instance>) {
        for p in &self.particles {
            let t = (p.life / p.max_life).clamp(0.0, 1.0);
            let s = 0.08 + 0.16 * t;
            out.push(Instance {
                position: [p.pos.x, p.pos.y, p.pos.z],
                scale: [s, s, s],
                color: p.color,
                glow: 0.6 + 1.6 * t,
            });
        }
    }

    /// Draw one frame from the provided instance list. `level` feeds the shader
    /// (reserved for level-tinted effects).
    pub fn render(
        &mut self,
        instances: &[Instance],
        level: u32,
    ) -> Result<(), wgpu::SurfaceError> {
        let count = instances.len().min(MAX_INSTANCES);

        // Move the camera: base framing + player drag + a slow breathing sway so
        // the view is never dead-still. The sway fades out while dragging so it
        // never fights the player's control.
        let sway = if self.dragging { 0.0 } else { 1.0 };
        let sway_yaw = 0.05 * sway * (self.time * 0.30).sin();
        let sway_pitch = 0.022 * sway * (self.time * 0.47 + 1.0).cos();
        self.camera.orbit_to(
            BASE_YAW + self.cam_yaw + sway_yaw,
            BASE_PITCH + self.cam_pitch + sway_pitch,
        );

        self.globals.view_proj = self.camera.view_proj().to_cols_array_2d();
        self.globals.camera_pos = [self.camera.eye.x, self.camera.eye.y, self.camera.eye.z, 1.0];
        self.globals.params = [self.time, self.flash, self.shake, level as f32];

        // Golden-hour Gurgaon haze — distant towers dissolve into warm amber smog.
        let sky = [
            0.62 + 0.03 * (self.time * 0.05).sin(),
            0.42 + 0.02 * (self.time * 0.06).cos(),
            0.28 + 0.02 * (self.time * 0.04).sin(),
        ];
        self.globals.sky = [sky[0], sky[1], sky[2], 0.012];

        // Sun-shadow: an orthographic light frustum centered on the build area,
        // following the tower so its shadow stays crisp as it grows.
        let sun = Vec3::new(0.55, 0.72, 0.35).normalize();
        let center = Vec3::new(self.focus_x, self.camera.target.y.max(6.0), 0.0);
        let lview = Mat4::look_at_rh(center + sun * 90.0, center, Vec3::Y);
        let r = 52.0;
        let lproj = Mat4::orthographic_rh(-r, r, -r, r, 1.0, 220.0);
        self.globals.light_vp = (lproj * lview).to_cols_array_2d();

        self.gpu
            .queue
            .write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&self.globals));
        self.gpu.queue.write_buffer(
            &self.instance_buf,
            0,
            bytemuck::cast_slice(&instances[..count]),
        );

        let frame = self.gpu.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame-encoder"),
            });

        // --- Shadow pass: render scene depth from the sun into the shadow map. ---
        {
            let mut spass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("shadow-pass"),
                color_attachments: &[],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.shadow_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            spass.set_pipeline(&self.shadow_pipe);
            spass.set_bind_group(0, &self.shadow_bind_group, &[]);
            spass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            spass.set_vertex_buffer(1, self.instance_buf.slice(..));
            spass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            spass.draw_indexed(0..self.index_count, 0, 0..count as u32);
        }

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("main-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.hdr_view, // render the scene into the HDR target
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Clear to the golden-hour sky so it meets the fog seamlessly.
                        load: wgpu::LoadOp::Clear({
                            let lift = self.flash as f64 * 0.18;
                            wgpu::Color {
                                r: (self.globals.sky[0] as f64 + lift).min(1.0),
                                g: (self.globals.sky[1] as f64 + lift).min(1.0),
                                b: (self.globals.sky[2] as f64 + lift).min(1.0),
                                a: 1.0,
                            }
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                occlusion_query_set: None,
                timestamp_writes: None,
            });

            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.vertex_buf.slice(..));
            pass.set_vertex_buffer(1, self.instance_buf.slice(..));
            pass.set_index_buffer(self.index_buf.slice(..), wgpu::IndexFormat::Uint16);
            pass.draw_indexed(0..self.index_count, 0, 0..count as u32);
        }

        // --- Composite pass: HDR scene → bloom + ACES tone-map → the surface. ---
        {
            let mut cpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.post_pipe);
            cpass.set_bind_group(0, &self.post_bg, &[]);
            cpass.draw(0..3, 0..1);
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        frame.present();
        Ok(())
    }
}

/// A Times-Square / Shibuya-style animated LED billboard: a dark frame on a pole
/// filled with a grid of emissive cells that shimmer through colors over time.
/// `t` is the clock; `seed` offsets the animation so multiple screens differ.
fn emit_billboard(out: &mut Vec<Instance>, c: [f32; 3], w: f32, h: f32, t: f32, seed: f32) {
    // Support pole from the ground up to the screen.
    out.push(Instance { position: [c[0], c[1] * 0.5, c[2]], scale: [0.5, c[1], 0.5], color: [0.10, 0.10, 0.12], glow: 0.2 });
    // Dark screen frame/backing.
    out.push(Instance { position: c, scale: [w + 0.6, h + 0.6, 0.35], color: [0.02, 0.02, 0.03], glow: 0.2 });
    // Grid of animated LED cells.
    let nx = 8usize;
    let ny = 5usize;
    for j in 0..ny {
        for i in 0..nx {
            let fx = (i as f32 + 0.5) / nx as f32 - 0.5;
            let fy = (j as f32 + 0.5) / ny as f32 - 0.5;
            let ph = t * 2.0 + i as f32 * 0.5 + j as f32 * 0.35 + seed;
            let col = [
                0.9 + 0.9 * ph.sin(),
                0.9 + 0.9 * (ph + 2.1).sin(),
                0.9 + 0.9 * (ph + 4.2).sin(),
            ];
            out.push(Instance {
                position: [c[0] + fx * w, c[1] + fy * h, c[2]],
                scale: [w / nx as f32 * 0.9, h / ny as f32 * 0.9, 0.4],
                color: col,
                glow: 2.6,
            });
        }
    }
}

/// A varied car body color, NYC-biased toward yellow cabs + black cars.
fn car_color(i: usize) -> [f32; 3] {
    const P: [[f32; 3]; 6] = [
        [0.95, 0.78, 0.10], // taxi yellow
        [0.95, 0.78, 0.10], // taxi yellow
        [0.05, 0.05, 0.06], // black
        [0.06, 0.06, 0.07], // black
        [0.55, 0.57, 0.60], // silver
        [0.85, 0.85, 0.9],  // white
    ];
    P[i % P.len()]
}

/// Emit a recognizable car (body + glass cabin + head/tail lights) at `c`,
/// facing +X when `fwd`, else -X.
fn emit_car(out: &mut Vec<Instance>, c: [f32; 3], col: [f32; 3], fwd: bool) {
    // glow 0.2 keeps the car solid (>= the 0.12 glass-facade threshold).
    out.push(Instance { position: c, scale: [2.3, 0.55, 0.95], color: col, glow: 0.2 });
    out.push(Instance {
        position: [c[0], c[1] + 0.42, c[2]],
        scale: [1.25, 0.5, 0.82],
        color: [0.10, 0.13, 0.18],
        glow: 0.2,
    });
    let d = if fwd { 1.0 } else { -1.0 };
    out.push(Instance {
        position: [c[0] + d * 1.2, c[1] - 0.03, c[2]],
        scale: [0.14, 0.28, 0.82],
        color: [3.2, 3.0, 2.2],
        glow: 2.2,
    }); // headlights
    out.push(Instance {
        position: [c[0] - d * 1.2, c[1] - 0.03, c[2]],
        scale: [0.12, 0.24, 0.82],
        color: [3.4, 0.3, 0.15],
        glow: 2.0,
    }); // taillights
}

/// Create the HDR scene texture (render target + sampled) and its post bind group.
fn create_hdr(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
    samp: &wgpu::Sampler,
    bgl: &wgpu::BindGroupLayout,
) -> (wgpu::TextureView, wgpu::BindGroup) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("hdr-scene"),
        size: wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: HDR_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("post-bg"),
        layout: bgl,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Sampler(samp) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view) },
        ],
    });
    (view, bg)
}

/// Cheap integer hash → [0,1), for deterministic skyline layout.
fn hash1(i: u32) -> f32 {
    let mut x = i.wrapping_mul(2654435761) ^ 0x9e3779b9;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2c1b3c6d);
    x ^= x >> 12;
    ((x >> 8) & 0xffff) as f32 / 65535.0
}

fn create_depth_view(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth-texture"),
        size: wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&wgpu::TextureViewDescriptor::default())
}
