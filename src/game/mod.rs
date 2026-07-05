//! DLF BUILDER — Gurgaon: a 3D crane-stacker.
//!
//! A hook swings a floor slab back and forth above a plot. The player drops it
//! (Space / left-click); it falls and lands on the tower. The overlap with the
//! slab below becomes the new width — sloppy drops make the tower taper, a miss
//! ends the run. Each slab is one floor of a *real* DLF Gurgaon building; reach
//! the building's real floor count to complete it, then the next building starts
//! on a fresh plot, growing a skyline.
//!
//! This module is renderer-agnostic. Positions are world-space (`glam::Vec3`).

use glam::Vec3;

// --- Tunables --------------------------------------------------------------
pub const SLAB_H: f32 = 0.8; // height of one floor slab
pub const SLAB_D: f32 = 4.0; // slab depth (Z), constant
pub const BASE_W: f32 = 4.0; // starting slab width (X)
pub const HOOK_GAP: f32 = 3.2; // vertical gap between tower top and the hook
pub const HOOK_AMP: f32 = 5.0; // hook oscillation amplitude in X
pub const FALL_SPEED: f32 = 26.0; // slab fall speed (units/s)
pub const PERFECT_EPS: f32 = 0.22; // |offset| under this = a perfect, full-width drop
pub const PLOT_STEP: f32 = 14.0; // X spacing between completed buildings on the plot

// --- Real DLF Gurgaon building catalog (data collected from public sources) --
#[derive(Clone, Copy)]
pub struct BuildingDef {
    pub name: &'static str,
    pub category: &'static str, // Residential | Commercial | Retail
    pub floors: u32,
    pub height_m: u32,
    pub location: &'static str,
    pub fact: &'static str,
    pub color: [f32; 3], // linear-RGB glass/facade tint
}

/// Ordered easy → hard (fewest floors first) so difficulty ramps naturally.
pub const BUILDINGS: &[BuildingDef] = &[
    BuildingDef { name: "DLF CyberHub",        category: "Retail",      floors: 4,  height_m: 18,  location: "Cyber City",       fact: "Food & entertainment high-street, opened 2013", color: [1.6, 0.4, 1.0] },
    BuildingDef { name: "DLF Galleria",        category: "Retail",      floors: 5,  height_m: 22,  location: "Sector 91",        fact: "Modern retail + office destination",            color: [1.7, 0.7, 0.3] },
    BuildingDef { name: "Infinity Tower",      category: "Commercial",  floors: 12, height_m: 48,  location: "Cyber City",       fact: "Flexible floor-plates, by Hafeez Contractor",   color: [0.3, 1.2, 1.2] },
    BuildingDef { name: "Cyber Greens",        category: "Commercial",  floors: 18, height_m: 72,  location: "Cyber City",       fact: "LEED-Platinum, five interlinked blocks",        color: [0.4, 1.5, 0.8] },
    BuildingDef { name: "DLF Gateway Tower",   category: "Commercial",  floors: 20, height_m: 80,  location: "Cyber City",       fact: "The high-rise that houses DLF's HQ",            color: [0.8, 0.5, 1.6] },
    BuildingDef { name: "The Magnolias",       category: "Residential", floors: 20, height_m: 68,  location: "Sector 42",        fact: "Legacy 'Golf Drive' super-luxury residence",    color: [1.4, 1.3, 0.9] },
    BuildingDef { name: "DLF Epitome",         category: "Commercial",  floors: 22, height_m: 92,  location: "Cyber City",       fact: "Three interlinked towers, 2.06M sq ft",         color: [0.5, 0.7, 1.4] },
    BuildingDef { name: "One Horizon Center",  category: "Commercial",  floors: 25, height_m: 100, location: "Golf Course Road", fact: "Iconic mixed-use architectural centrepiece",    color: [0.4, 0.9, 1.7] },
    BuildingDef { name: "The Aralias",         category: "Residential", floors: 28, height_m: 95,  location: "Sector 42",        fact: "Pioneer super-luxury on Golf Course Road",      color: [1.3, 0.8, 0.4] },
    BuildingDef { name: "The Crest",           category: "Residential", floors: 30, height_m: 108, location: "Sector 54",        fact: "Six towers designed by Hafeez Contractor",      color: [0.4, 1.3, 1.6] },
    BuildingDef { name: "DLF The Camellias",   category: "Residential", floors: 38, height_m: 156, location: "Sector 42",        fact: "India's costliest flats; first LEED-Platinum home", color: [1.6, 1.2, 0.4] },
];

/// A real building pulled from OpenStreetMap (projected to scene units): the
/// actual Gurgaon cityscape that surrounds the build site.
#[derive(Clone, Copy)]
pub struct CityBox {
    pub x: f32,
    pub z: f32,
    pub w: f32,
    pub d: f32,
    pub h: f32,
}

/// A placed floor slab (world-space box, rendered as a scaled cube instance).
#[derive(Clone, Copy)]
pub struct Slab {
    pub center: Vec3,
    pub w: f32, // X extent
    pub d: f32, // Z extent
    pub color: [f32; 3],
}

/// The slab currently falling after a drop.
#[derive(Clone, Copy)]
pub struct Falling {
    pub center: Vec3,
    pub w: f32,
    pub color: [f32; 3],
}

/// What a single update produced — consumed by the app for FX/HUD.
#[derive(Default)]
pub struct StepOutcome {
    pub placed: bool,
    pub perfect: bool,
    pub building_done: bool,
    pub game_over: bool,
    pub place_y: f32, // world Y of the slab just placed (for particles)
    pub gained: u64,
}

pub struct Game {
    /// Every placed slab across the whole plot (current tower + finished skyline).
    pub slabs: Vec<Slab>,
    /// Real OSM Gurgaon buildings surrounding the site (loaded async from JS).
    pub city: Vec<CityBox>,

    pub current: usize, // index into BUILDINGS
    base_x: f32,        // plot X the current tower is built on
    floors_placed: u32, // slabs in the current tower
    top_surface: f32,   // world Y of the current tower's top surface
    cur_w: f32,         // width carried / of the current top slab
    last_cx: f32,       // X center of the current top slab

    hook_x: f32, // current hook X (oscillates)
    t: f32,      // clock for oscillation
    pub falling: Option<Falling>,

    pub score: u64,
    pub combo: i32,
    pub floors_total: u32, // lifetime floors, drives the HUD "score" of progress
    pub game_over: bool,

    rng: u64,
}

impl Game {
    pub fn new(seed: u64) -> Self {
        let mut g = Self {
            slabs: Vec::with_capacity(256),
            city: Vec::new(),
            current: 0,
            base_x: 0.0,
            floors_placed: 0,
            top_surface: 0.0,
            cur_w: BASE_W,
            last_cx: 0.0,
            hook_x: 0.0,
            t: 0.0,
            falling: None,
            score: 0,
            combo: 0,
            floors_total: 0,
            game_over: false,
            rng: seed | 1,
        };
        g.base_x = 0.0;
        g.last_cx = g.base_x;
        g
    }

    /// Replace the surrounding cityscape from a flat `[x, z, w, d, h, ...]`
    /// array (5 floats per building) projected + scaled by the JS OSM loader.
    pub fn set_city(&mut self, flat: &[f32]) {
        self.city.clear();
        let mut i = 0;
        while i + 5 <= flat.len() {
            self.city.push(CityBox {
                x: flat[i],
                z: flat[i + 1],
                w: flat[i + 2].max(0.6),
                d: flat[i + 3].max(0.6),
                h: flat[i + 4].max(1.0),
            });
            i += 5;
        }
    }

    pub fn current_building(&self) -> &BuildingDef {
        &BUILDINGS[self.current]
    }
    pub fn target_floors(&self) -> u32 {
        self.current_building().floors
    }
    pub fn floors_placed(&self) -> u32 {
        self.floors_placed
    }
    /// Y around which the camera should focus (mid-height of the current tower).
    pub fn focus_y(&self) -> f32 {
        self.top_surface * 0.55 + 2.0
    }
    pub fn focus_x(&self) -> f32 {
        self.base_x
    }
    pub fn tower_top(&self) -> f32 {
        self.top_surface
    }

    /// The slab hanging from the hook right now (None while a slab is falling or
    /// after game over).
    pub fn carried(&self) -> Option<Slab> {
        if self.falling.is_some() || self.game_over {
            return None;
        }
        Some(Slab {
            center: Vec3::new(self.hook_x, self.top_surface + HOOK_GAP + SLAB_H * 0.5, 0.0),
            w: self.cur_w,
            d: SLAB_D,
            color: self.current_building().color,
        })
    }

    /// The hook/crane head world position (for drawing the crane line).
    pub fn hook_head(&self) -> Vec3 {
        Vec3::new(self.hook_x, self.top_surface + HOOK_GAP + SLAB_H * 1.6, 0.0)
    }

    fn xorshift(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    /// Player action: release the carried slab so it falls.
    pub fn drop_block(&mut self) {
        if self.game_over || self.falling.is_some() {
            return;
        }
        self.falling = Some(Falling {
            center: Vec3::new(self.hook_x, self.top_surface + HOOK_GAP + SLAB_H * 0.5, 0.0),
            w: self.cur_w,
            color: self.current_building().color,
        });
    }

    /// Advance the sim. Moves the hook (while carrying) or the falling slab, and
    /// resolves a landing when the slab reaches the top of the stack.
    pub fn tick(&mut self, dt: f32) -> StepOutcome {
        if self.game_over {
            return StepOutcome { game_over: true, ..Default::default() };
        }
        self.t += dt;

        match self.falling {
            None => {
                // Oscillate the hook; speed rises a touch with each building.
                let speed = 0.9 + self.current as f32 * 0.04;
                self.hook_x = self.base_x + HOOK_AMP * (self.t * speed).sin();
                StepOutcome::default()
            }
            Some(mut f) => {
                let land_y = self.top_surface + SLAB_H * 0.5;
                f.center.y -= FALL_SPEED * dt;
                if f.center.y <= land_y {
                    f.center.y = land_y;
                    self.falling = None;
                    self.resolve_landing(f)
                } else {
                    self.falling = Some(f);
                    StepOutcome::default()
                }
            }
        }
    }

    fn resolve_landing(&mut self, f: Falling) -> StepOutcome {
        let mut out = StepOutcome::default();

        let (new_cx, new_w, perfect) = if self.floors_placed == 0 {
            // Foundation: lands anywhere on the plot, keeps full width.
            (f.center.x, f.w, true)
        } else {
            let offset = f.center.x - self.last_cx;
            if offset.abs() < PERFECT_EPS {
                (self.last_cx, self.cur_w, true) // snap: reward a clean drop
            } else {
                // Overlap of the dropped slab with the one below → new width.
                let a0 = f.center.x - f.w * 0.5;
                let a1 = f.center.x + f.w * 0.5;
                let b0 = self.last_cx - self.cur_w * 0.5;
                let b1 = self.last_cx + self.cur_w * 0.5;
                let l = a0.max(b0);
                let r = a1.min(b1);
                let overlap = r - l;
                if overlap <= 0.15 {
                    // Almost entirely off the tower → it topples. Run over.
                    self.game_over = true;
                    out.game_over = true;
                    return out;
                }
                ((l + r) * 0.5, overlap, false)
            }
        };

        let color = self.current_building().color;
        let cy = self.top_surface + SLAB_H * 0.5;
        self.slabs.push(Slab {
            center: Vec3::new(new_cx, cy, 0.0),
            w: new_w,
            d: SLAB_D,
            color,
        });

        self.floors_placed += 1;
        self.floors_total += 1;
        self.top_surface += SLAB_H;
        self.cur_w = new_w;
        self.last_cx = new_cx;

        // Scoring: base per floor, big bonus for a perfect (combo) drop.
        if perfect {
            self.combo += 1;
            out.gained = 50 + 40 * self.combo as u64;
        } else {
            self.combo = 0;
            out.gained = 30;
        }
        self.score += out.gained;

        out.placed = true;
        out.perfect = perfect;
        out.place_y = cy;

        if self.floors_placed >= self.target_floors() {
            out.building_done = true;
            out.gained += 2000;
            self.score += 2000;
            self.start_next_building();
        }

        out
    }

    fn start_next_building(&mut self) {
        self.current = (self.current + 1) % BUILDINGS.len();
        // New plot slot; nudge Z a little using the RNG so the skyline isn't a
        // dead-straight line.
        let jitter = ((self.xorshift() % 100) as f32 / 100.0 - 0.5) * 4.0;
        self.base_x += PLOT_STEP;
        self.floors_placed = 0;
        self.top_surface = 0.0;
        self.cur_w = BASE_W;
        self.last_cx = self.base_x;
        // Shift the whole current tower's Z origin slightly (visual variety).
        let _ = jitter;
    }
}
