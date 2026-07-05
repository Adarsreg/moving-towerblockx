//! Angled (near-isometric) camera and the shared `Globals` uniform block.
//!
//! The camera looks down the well from an elevated corner so the player reads
//! depth clearly. The projection is a standard right-handed perspective with a
//! [0,1] depth range (`Mat4::perspective_rh`) which is exactly what wgpu/WebGPU
//! expect — do NOT use the `_gl` variant here.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

/// Uniform block shared by the shader. std140-friendly layout: mat4 (64) +
/// vec4 (16) + vec4 (16) = 96 bytes, each field 16-byte aligned.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Globals {
    pub view_proj: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    /// x = time (s), y = flash intensity, z = shake amplitude, w = level.
    pub params: [f32; 4],
    /// Sky/haze color (rgb) that distant geometry fades into. a = fog density.
    pub sky: [f32; 4],
    /// Light view-projection matrix for shadow mapping.
    pub light_vp: [[f32; 4]; 4],
}

impl Globals {
    pub fn new() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            camera_pos: [0.0; 4],
            params: [0.0; 4],
            sky: [0.05, 0.07, 0.12, 0.014],
            light_vp: Mat4::IDENTITY.to_cols_array_2d(),
        }
    }
}

/// An orbit camera: it circles a fixed `target` at `radius`, parameterised by
/// `yaw` (azimuth) and `pitch` (elevation). Modelling it this way — instead of a
/// frozen eye point — is what lets the view breathe and respond to the mouse,
/// which is most of the difference between "diorama" and "alive".
pub struct Camera {
    pub target: Vec3,
    pub radius: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub eye: Vec3, // recomputed from yaw/pitch/radius; kept for the shader
    pub aspect: f32,
    pub fovy: f32,
    pub znear: f32,
    pub zfar: f32,
}

/// Default framing angles, chosen so the whole 15-tall neon cage fits with
/// margin and the player looks down into the well from a front-right corner.
pub const BASE_YAW: f32 = 0.95;
pub const BASE_PITCH: f32 = 0.52;
/// Pitch clamp so the mouse can't flip under the floor or straight overhead.
pub const MIN_PITCH: f32 = 0.12;
pub const MAX_PITCH: f32 = 1.30;

impl Camera {
    pub fn new(aspect: f32) -> Self {
        let mut cam = Self {
            // `target` and `radius` are driven each frame by the renderer's focus
            // (the camera follows the growing tower).
            target: Vec3::new(0.0, 4.0, 0.0),
            radius: 22.0,
            yaw: BASE_YAW,
            pitch: BASE_PITCH,
            eye: Vec3::ZERO,
            aspect,
            fovy: 50f32.to_radians(),
            znear: 0.1,
            zfar: 600.0,
        };
        cam.recompute_eye();
        cam
    }

    /// Point the camera at `(yaw, pitch)`, clamping pitch to a sane range.
    pub fn orbit_to(&mut self, yaw: f32, pitch: f32) {
        self.yaw = yaw;
        self.pitch = pitch.clamp(MIN_PITCH, MAX_PITCH);
        self.recompute_eye();
    }

    fn recompute_eye(&mut self) {
        let horiz = self.radius * self.pitch.cos();
        self.eye = Vec3::new(
            self.target.x + horiz * self.yaw.cos(),
            self.target.y + self.radius * self.pitch.sin(),
            self.target.z + horiz * self.yaw.sin(),
        );
    }

    pub fn view_proj(&self) -> Mat4 {
        let view = Mat4::look_at_rh(self.eye, self.target, Vec3::Y);
        let proj = Mat4::perspective_rh(self.fovy, self.aspect, self.znear, self.zfar);
        proj * view
    }
}
