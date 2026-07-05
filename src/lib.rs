//! Neon 3D Tetris — crate root.
//!
//! Wires the Wasm entry point to the app shell (winit event loop + wgpu
//! renderer + game simulation).

pub mod app;
pub mod game;
pub mod render;

/// Platform-agnostic bootstrap: initialize logging + panic reporting.
fn init_logging() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            console_error_panic_hook::set_once();
            let _ = console_log::init_with_level(log::Level::Info);
        } else {
            let _ = env_logger::try_init();
        }
    }
}

/// Wasm entry point invoked by index.html's JS bootstrap after wasm init.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn run() {
    init_logging();
    log::info!("Tower Builder booting — {} buildings in catalog", game::BUILDINGS.len());
    app::start();
}

/// Native entry so we can `cargo run --example ...` / test on desktop.
#[cfg(not(target_arch = "wasm32"))]
pub fn run() {
    init_logging();
    app::start();
}
