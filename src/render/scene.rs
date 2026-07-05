//! Procedural scenery for the builder: a big neon ground plot with a grid, and
//! the crane (mast + arm + cable) that carries the current slab. Everything is
//! plain `Instance`s flowing through the one instanced pass.

use super::instance::{boxx, Instance};
use crate::game::{Game, HOOK_AMP, SLAB_H};

const PLOT_HALF: f32 = 95.0; // half-extent of the ground plot
const CRANE_COLOR: [f32; 3] = [1.4, 0.9, 0.15]; // hazard-yellow crane

fn tree(out: &mut Vec<Instance>, x: f32, z: f32) {
    out.push(boxx([x, 1.0, z], [0.22, 2.0, 0.22], [0.24, 0.15, 0.08], 0.02)); // trunk
    out.push(boxx([x, 2.6, z], [1.5, 1.8, 1.5], [0.10, 0.42, 0.16], 0.05)); // canopy
    out.push(boxx([x, 3.4, z], [1.0, 1.1, 1.0], [0.14, 0.5, 0.2], 0.05)); // top tuft
}

/// A NYC-style street level: broad asphalt avenue, wide concrete sidewalks,
/// painted crosswalks + lane lines, sparse planted trees, and a fenced
/// construction lot under the tower. No glowing toy lamps.
pub fn ground(out: &mut Vec<Instance>, cx: f32) {
    let span = PLOT_HALF;

    // Asphalt ground base (dark, textured by the shader's grain).
    out.push(boxx([cx, -0.25, 0.0], [span * 2.0, 0.5, span * 2.0], [0.055, 0.055, 0.065], 0.0));

    // Construction lot under the tower: concrete slab + hazard-striped border.
    out.push(boxx([cx, 0.03, 0.0], [13.0, 0.08, 13.0], [0.28, 0.28, 0.31], 0.03));
    for &(sx, sz, w, d) in &[(0.0, 6.6, 13.0, 0.4), (0.0, -6.6, 13.0, 0.4), (6.6, 0.0, 0.4, 13.0), (-6.6, 0.0, 0.4, 13.0)] {
        out.push(boxx([cx + sx, 0.09, sz], [w, 0.06, d], CRANE_COLOR, 0.4));
    }

    // Wide concrete sidewalks flanking the avenue (roads sit at z = ±15).
    for &s in &[-1.0f32, 1.0] {
        out.push(boxx([cx, 0.05, s * 11.0], [span * 2.0, 0.08, 5.5], [0.22, 0.22, 0.24], 0.02));
        // Sparse street trees set into the sidewalk.
        let mut x = -span + 12.0;
        while x < span - 12.0 {
            tree(out, cx + x, s * 12.6);
            x += 15.0;
        }
    }

    // Painted crosswalks (zebra stripes) across both roads at each block.
    let mut x = -span + 14.0;
    while x < span - 14.0 {
        for &rz in &[-15.0f32, 15.0] {
            for k in 0..7 {
                out.push(boxx([cx + x + (k as f32 - 3.0) * 0.75, 0.12, rz], [0.42, 0.03, 9.0], [0.85, 0.85, 0.85], 0.15));
            }
        }
        x += 30.0;
    }
}

/// The crane: a vertical mast beside the plot, a horizontal arm over the tower,
/// and a cable down to the carried slab. Purely cosmetic but sells the theme.
pub fn crane(out: &mut Vec<Instance>, game: &Game) {
    let Some(slab) = game.carried() else { return };
    let head = game.hook_head();
    let arm_y = head.y + SLAB_H * 1.2;
    let base_x = game.focus_x();

    // Mast: stands at the left edge of the hook's sweep, rising above the arm.
    let mast_x = base_x - HOOK_AMP - 2.0;
    out.push(boxx([mast_x, arm_y * 0.5, 0.0], [0.25, arm_y, 0.25], CRANE_COLOR, 0.5));

    // Arm: spans from the mast out over the hook.
    let arm_len = (head.x - mast_x).abs() + 2.0;
    let arm_cx = (mast_x + head.x) * 0.5;
    out.push(boxx([arm_cx, arm_y, 0.0], [arm_len, 0.18, 0.18], CRANE_COLOR, 0.5));

    // Cable: thin vertical bar from the arm down to the top of the carried slab.
    let cable_top = arm_y;
    let cable_bot = slab.center.y + SLAB_H * 0.5;
    out.push(boxx(
        [head.x, (cable_top + cable_bot) * 0.5, 0.0],
        [0.05, (cable_top - cable_bot).max(0.1), 0.05],
        [1.0, 1.0, 1.0],
        0.6,
    ));
}
