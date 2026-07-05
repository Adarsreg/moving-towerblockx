//! Rendering layer: wgpu context + instanced neon pipeline. Depends on `game`.

pub mod camera;
pub mod cube;
pub mod gpu;
pub mod instance;
pub mod pipeline;
pub mod scene;

pub use instance::{boxx, Instance};
pub use pipeline::Renderer;

use crate::game::{Game, SLAB_D, SLAB_H};

/// Build the per-frame instance list from the game: ground plot, every placed
/// tower/skyline slab, the carried slab, the falling slab, and the crane.
pub fn build_instances(game: &Game) -> Vec<Instance> {
    let mut out = Vec::with_capacity(1024);

    scene::ground(&mut out, game.focus_x());

    // Real city buildings from OpenStreetMap. We keep a clear CONSTRUCTION
    // CORRIDOR down the middle (|z| < CLEAR) so the real city flanks the site on
    // both sides — with the expressways — instead of spawning on top of the
    // tower you're stacking.
    const CLEAR: f32 = 17.0;
    for b in &game.city {
        if b.z.abs() < CLEAR {
            continue; // leave the build lane + roads open
        }
        let t = (b.h / 40.0).clamp(0.0, 1.0);
        let color = [
            0.34 * (1.0 - t) + 0.18 * t,
            0.32 * (1.0 - t) + 0.28 * t,
            0.28 * (1.0 - t) + 0.46 * t,
        ];
        out.push(boxx([b.x, b.h * 0.5, b.z], [b.w, b.h, b.d], color, 0.05));
    }

    // Placed slabs (current tower + finished skyline). Low glow so the glass
    // curtain-wall shader applies (buildings use glow < 0.12).
    for s in &game.slabs {
        out.push(boxx(
            [s.center.x, s.center.y, s.center.z],
            [s.w, SLAB_H, s.d],
            s.color,
            0.08,
        ));
    }

    // The slab hanging from the crane hook.
    if let Some(s) = game.carried() {
        out.push(boxx(
            [s.center.x, s.center.y, s.center.z],
            [s.w, SLAB_H, s.d],
            s.color,
            0.95,
        ));
    }

    // A slab mid-fall.
    if let Some(f) = &game.falling {
        out.push(boxx(
            [f.center.x, f.center.y, f.center.z],
            [f.w, SLAB_H, SLAB_D],
            f.color,
            0.95,
        ));
    }

    scene::crane(&mut out, game);

    out
}
