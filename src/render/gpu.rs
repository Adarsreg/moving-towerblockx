//! Step 3 — wgpu context bring-up.
//!
//! Owns the `Instance`, `Surface`, `Adapter`-derived `Device`/`Queue`, and the
//! surface configuration. On wasm this targets the WebGPU backend and binds the
//! surface to the existing `<canvas>` (via winit's raw-window-handle). Adapter
//! and device requests are async, so `Gpu::new` is an async fn — the caller
//! drives it with `wasm_bindgen_futures::spawn_local` on web / `pollster` native.

use std::sync::Arc;

use winit::window::Window;

pub struct Gpu {
    pub surface: wgpu::Surface<'static>,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub config: wgpu::SurfaceConfiguration,
    pub size: winit::dpi::PhysicalSize<u32>,
}

impl Gpu {
    pub async fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        // Guard against a zero-sized canvas on first layout.
        let width = size.width.max(1);
        let height = size.height.max(1);

        // On wasm, PRIMARY resolves to BROWSER_WEBGPU (we did not enable webgl).
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // Arc<Window> is 'static + implements the raw handle traits, giving us a
        // Surface<'static> we can store alongside the device.
        let surface = instance
            .create_surface(window.clone())
            .expect("create_surface failed");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("no suitable GPU adapter found");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("tetris3d-device"),
                required_features: wgpu::Features::empty(),
                // WebGPU baseline limits — plenty for a uniform + two vertex
                // buffers. Capped by the adapter for safety.
                required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("request_device failed");

        let caps = surface.get_capabilities(&adapter);
        // Prefer an sRGB format so the shader can output linear color and let the
        // hardware do the gamma encode (no manual pow in the fragment shader).
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo, // vsync; universally supported
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);
        log::info!(
            "GPU ready: {:?} adapter, surface {}x{} {:?}",
            adapter.get_info().backend,
            config.width,
            config.height,
            config.format
        );

        Self { surface, device, queue, config, size }
    }

    /// Reconfigure the surface after a canvas resize.
    pub fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
    }

    pub fn aspect(&self) -> f32 {
        self.config.width as f32 / self.config.height as f32
    }
}
