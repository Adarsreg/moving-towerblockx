//! Application shell: window + event loop, crane input, build loop, HUD bridge.
//!
//! Controls (deliberately minimal): Space / Left-click = drop the slab,
//! Right-drag = orbit the camera, P = pause, F = fullscreen (handled in JS).

use std::sync::Arc;

use winit::event::{ElementState, Event, KeyEvent, MouseButton, TouchPhase, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowBuilder};

use crate::game::{Game, StepOutcome};
use crate::render::{build_instances, Renderer};

/// Mouse-drag → orbit sensitivity (radians per pixel).
const ORBIT_SENS: f32 = 0.006;
/// Clamp on a single frame's dt so a backgrounded tab can't dump a huge step.
const MAX_DT: f64 = 0.1;

pub struct App {
    window: Arc<Window>,
    renderer: Renderer,
    game: Game,
    last_time: f64,
    frames: u64,

    dragging: bool,
    last_cursor: Option<(f64, f64)>,
    /// Active touch: (id, last_x, last_y, moved?) — for tap-to-drop / drag-to-look.
    touch: Option<(u64, f64, f64, bool)>,
    paused: bool,
}

impl App {
    pub fn new(window: Arc<Window>, renderer: Renderer) -> Self {
        let game = Game::new(seed_now());
        push_hud(&game);
        push_building(&game);
        Self {
            window,
            renderer,
            game,
            last_time: now_secs(),
            frames: 0,
            dragging: false,
            last_cursor: None,
            touch: None,
            paused: false,
        }
    }

    fn handle(&mut self, event: Event<()>, elwt: &EventLoopWindowTarget<()>) {
        elwt.set_control_flow(ControlFlow::Poll);

        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::CloseRequested => elwt.exit(),
                WindowEvent::Resized(size) => self.renderer.resize(size),

                // Left button / Space drop the slab. Right button orbits.
                WindowEvent::MouseInput { state, button, .. } => match button {
                    MouseButton::Left if state == ElementState::Pressed => self.drop_action(),
                    MouseButton::Right => {
                        self.dragging = state == ElementState::Pressed;
                        self.renderer.set_dragging(self.dragging);
                        if !self.dragging {
                            self.last_cursor = None;
                        }
                    }
                    _ => {}
                },
                WindowEvent::CursorMoved { position, .. } => {
                    if self.dragging {
                        if let Some((lx, ly)) = self.last_cursor {
                            let dx = (position.x - lx) as f32;
                            let dy = (position.y - ly) as f32;
                            self.renderer.orbit_input(dx * ORBIT_SENS, -dy * ORBIT_SENS);
                        }
                        self.last_cursor = Some((position.x, position.y));
                    }
                }

                // Touch (mobile): tap = drop, drag = orbit the camera.
                WindowEvent::Touch(t) => match t.phase {
                    TouchPhase::Started => {
                        self.touch = Some((t.id, t.location.x, t.location.y, false));
                    }
                    TouchPhase::Moved => {
                        if let Some((id, lx, ly, moved)) = self.touch {
                            if id == t.id {
                                let dx = (t.location.x - lx) as f32;
                                let dy = (t.location.y - ly) as f32;
                                let big = dx.abs() > 3.0 || dy.abs() > 3.0;
                                if big {
                                    self.renderer.orbit_input(dx * ORBIT_SENS, -dy * ORBIT_SENS);
                                }
                                self.touch = Some((id, t.location.x, t.location.y, moved || big));
                            }
                        }
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if let Some((id, _, _, moved)) = self.touch {
                            if id == t.id {
                                if !moved && t.phase == TouchPhase::Ended {
                                    self.drop_action(); // a tap drops the slab
                                }
                                self.touch = None;
                            }
                        }
                    }
                },

                WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            physical_key: PhysicalKey::Code(code),
                            state: ElementState::Pressed,
                            repeat,
                            ..
                        },
                    ..
                } => self.on_key(code, repeat),

                WindowEvent::RedrawRequested => self.frame(),
                _ => {}
            },
            Event::AboutToWait => self.window.request_redraw(),
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode, repeat: bool) {
        match code {
            KeyCode::KeyP if !self.game.game_over => {
                self.paused = !self.paused;
                set_overlay(if self.paused { OVERLAY_PAUSED } else { OVERLAY_NONE });
            }
            KeyCode::Space if !repeat => self.drop_action(),
            _ => {
                // Any key restarts after a topple.
                if self.game.game_over {
                    self.restart();
                }
            }
        }
    }

    fn drop_action(&mut self) {
        if self.game.game_over {
            self.restart();
            return;
        }
        if self.paused {
            return;
        }
        self.game.drop_block();
        sfx(SFX_DROP);
    }

    fn restart(&mut self) {
        stop_sfx(); // silence the lingering game-over chord immediately
        let city = std::mem::take(&mut self.game.city); // keep the loaded cityscape
        self.game = Game::new(seed_now());
        self.game.city = city;
        self.paused = false;
        set_overlay(OVERLAY_NONE);
        push_hud(&self.game);
        push_building(&self.game);
    }

    fn frame(&mut self) {
        // Pick up real OSM city buildings once the JS loader delivers them.
        #[cfg(target_arch = "wasm32")]
        if let Some(data) = PENDING_CITY.with(|c| c.borrow_mut().take()) {
            self.game.set_city(&data);
            log::info!("Loaded {} real city buildings from OSM", self.game.city.len());
        }

        // Keep the surface matched to the canvas (web rarely emits Resized).
        let size = self.desired_size();
        if size.width > 0
            && size.height > 0
            && (size.width != self.renderer.gpu.config.width
                || size.height != self.renderer.gpu.config.height)
        {
            self.renderer.resize(size);
        }

        let now = now_secs();
        let dt = ((now - self.last_time).max(0.0)).min(MAX_DT);
        self.last_time = now;

        // Tick only while actively playing. Crucially, stop once game-over so the
        // game-over sound + overlay fire exactly once instead of every frame —
        // that per-frame replay was the "game over won't clear" bug.
        if !self.paused && !self.game.game_over {
            let outcome = self.game.tick(dt as f32);
            self.apply_outcome(outcome);
        }

        // Follow the tower: focus the camera on its mid-height, radius grows tall.
        let top = self.game.tower_top();
        self.renderer
            .set_focus(self.game.focus_x(), self.game.focus_y(), 20.0 + top * 0.7);
        self.renderer.update_effects(dt as f32);

        let mut instances = build_instances(&self.game);
        self.renderer
            .append_environment(&mut instances, self.game.focus_x(), !self.game.city.is_empty());
        self.renderer.append_particles(&mut instances);

        if self.frames == 0 {
            log::info!(
                "first frame: surface {}x{}, {} instances",
                self.renderer.gpu.config.width,
                self.renderer.gpu.config.height,
                instances.len()
            );
        }
        self.frames += 1;

        match self.renderer.render(&instances, self.game.floors_placed()) {
            Ok(()) => {}
            Err(wgpu::SurfaceError::Lost | wgpu::SurfaceError::Outdated) => {
                self.renderer.resize(self.renderer.gpu.size);
            }
            Err(wgpu::SurfaceError::OutOfMemory) => log::error!("GPU OOM"),
            Err(wgpu::SurfaceError::Timeout) => {}
            Err(e) => log::warn!("surface error: {e:?}"),
        }
    }

    fn apply_outcome(&mut self, outcome: StepOutcome) {
        if outcome.placed {
            let color = self.game.current_building().color;
            let at = [self.game.focus_x(), outcome.place_y, 0.0];
            if outcome.perfect {
                self.renderer.burst(at, color, 1.2);
                sfx(SFX_ROTATE);
                if self.game.combo >= 2 {
                    game_event(EVENT_COMBO, self.game.combo as u32);
                }
            } else {
                self.renderer.burst(at, color, 0.5);
                sfx(SFX_LOCK);
            }
            push_hud(&self.game);
        }

        if outcome.building_done {
            // Big celebration + the next building's info.
            self.renderer
                .burst([self.game.focus_x(), 3.0, 0.0], [1.6, 1.3, 0.4], 2.2);
            game_event(EVENT_PERFECT, 0);
            sfx(SFX_LEVEL);
            push_building(&self.game);
        }

        if outcome.game_over {
            sfx(SFX_GAMEOVER);
            set_overlay(OVERLAY_GAMEOVER);
            log::info!("Topple — final score {}", self.game.score);
        }
    }

    fn desired_size(&self) -> winit::dpi::PhysicalSize<u32> {
        #[cfg(target_arch = "wasm32")]
        {
            if let Some((w, h)) = web_canvas_size() {
                return winit::dpi::PhysicalSize::new(w, h);
            }
        }
        self.window.inner_size()
    }
}

// --- Platform bootstraps ---------------------------------------------------

#[cfg(target_arch = "wasm32")]
pub fn start() {
    use winit::platform::web::{EventLoopExtWebSys, WindowBuilderExtWebSys};

    let event_loop = EventLoop::new().expect("event loop");
    let canvas = get_canvas();
    let window = Arc::new(
        WindowBuilder::new()
            .with_canvas(Some(canvas))
            .build(&event_loop)
            .expect("window"),
    );
    wasm_bindgen_futures::spawn_local(async move {
        let renderer = Renderer::new(window.clone()).await;
        let mut app = App::new(window, renderer);
        event_loop.spawn(move |event, elwt| app.handle(event, elwt));
    });
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start() {
    let event_loop = EventLoop::new().expect("event loop");
    let window = Arc::new(
        WindowBuilder::new()
            .with_title("Tower Builder")
            .build(&event_loop)
            .expect("window"),
    );
    let renderer = pollster::block_on(Renderer::new(window.clone()));
    let mut app = App::new(window, renderer);
    event_loop
        .run(move |event, elwt| app.handle(event, elwt))
        .expect("event loop run");
}

#[cfg(target_arch = "wasm32")]
fn web_canvas_size() -> Option<(u32, u32)> {
    let win = web_sys::window()?;
    let el = win.document()?.get_element_by_id("game-canvas")?;
    // Native pixel density → razor-sharp on hi-DPI / 4K. Capped at 3x.
    let dpr = win.device_pixel_ratio().clamp(1.0, 3.0);
    let w = (el.client_width().max(0) as f64 * dpr) as u32;
    let h = (el.client_height().max(0) as f64 * dpr) as u32;
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

#[cfg(target_arch = "wasm32")]
fn get_canvas() -> web_sys::HtmlCanvasElement {
    use wasm_bindgen::JsCast;
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("game-canvas"))
        .expect("#game-canvas not found")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("#game-canvas is not a canvas")
}

// --- HUD / info bridge (Rust -> DOM) ---------------------------------------

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = __hud)]
    fn hud_js(score: f64, floor: u32, target: u32);
    #[wasm_bindgen(js_namespace = window, js_name = __building)]
    fn building_js(name: &str, category: &str, location: &str, fact: &str, floors: u32, height: u32);
}

fn push_hud(game: &Game) {
    #[cfg(target_arch = "wasm32")]
    hud_js(game.score as f64, game.floors_placed(), game.target_floors());
    #[cfg(not(target_arch = "wasm32"))]
    log::info!(
        "score={} floor {}/{}",
        game.score,
        game.floors_placed(),
        game.target_floors()
    );
}

fn push_building(game: &Game) {
    let b = game.current_building();
    #[cfg(target_arch = "wasm32")]
    building_js(b.name, b.category, b.location, b.fact, b.floors, b.height_m);
    #[cfg(not(target_arch = "wasm32"))]
    log::info!("Now building: {} ({} floors)", b.name, b.floors);
}

// --- Sound + overlay + toast bridges ---------------------------------------

const SFX_ROTATE: u32 = 1;
const SFX_LOCK: u32 = 2;
const SFX_LEVEL: u32 = 4;
const SFX_GAMEOVER: u32 = 5;
const SFX_DROP: u32 = 6;

const OVERLAY_NONE: u32 = 0;
const OVERLAY_PAUSED: u32 = 1;
const OVERLAY_GAMEOVER: u32 = 2;

const EVENT_COMBO: u32 = 1;
const EVENT_PERFECT: u32 = 3;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = window, js_name = __sfx)]
    fn sfx_js(kind: u32);
    #[wasm_bindgen(js_namespace = window, js_name = __stopsfx)]
    fn sfx_stop_js();
    #[wasm_bindgen(js_namespace = window, js_name = __overlay)]
    fn overlay_js(kind: u32);
    #[wasm_bindgen(js_namespace = window, js_name = __event)]
    fn event_js(kind: u32, n: u32);
}

#[cfg(target_arch = "wasm32")]
fn sfx(kind: u32) {
    sfx_js(kind);
}
#[cfg(target_arch = "wasm32")]
fn stop_sfx() {
    sfx_stop_js();
}
#[cfg(target_arch = "wasm32")]
fn set_overlay(kind: u32) {
    overlay_js(kind);
}
#[cfg(target_arch = "wasm32")]
fn game_event(kind: u32, n: u32) {
    event_js(kind, n);
}

#[cfg(not(target_arch = "wasm32"))]
fn sfx(_kind: u32) {}
#[cfg(not(target_arch = "wasm32"))]
fn stop_sfx() {}
#[cfg(not(target_arch = "wasm32"))]
fn set_overlay(_kind: u32) {}
#[cfg(not(target_arch = "wasm32"))]
fn game_event(_kind: u32, _n: u32) {}

// --- Time source -----------------------------------------------------------

#[cfg(target_arch = "wasm32")]
fn now_secs() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now() / 1000.0)
        .unwrap_or(0.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn now_secs() -> f64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

fn seed_now() -> u64 {
    let micros = (now_secs() * 1_000_000.0) as u64;
    let mut z = micros.wrapping_add(0x9E3779B97F4A7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^ (z >> 31)
}

// --- Real-city bridge (JS OSM loader -> Rust) ------------------------------
// JS fetches OpenStreetMap building footprints, projects them to scene units,
// and calls `set_city(flatArray)`. We stash them here; the App picks them up on
// its next frame. A thread-local is fine because wasm runs single-threaded.

#[cfg(target_arch = "wasm32")]
thread_local! {
    static PENDING_CITY: std::cell::RefCell<Option<Vec<f32>>> = const { std::cell::RefCell::new(None) };
}

/// Exported to JS as `set_city(Float32Array)`: `[x, z, w, d, h, ...]` per real
/// building, already projected + scaled into scene units.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_city(data: Vec<f32>) {
    PENDING_CITY.with(|c| *c.borrow_mut() = Some(data));
}
