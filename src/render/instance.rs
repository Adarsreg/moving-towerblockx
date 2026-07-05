//! Per-cube instance data uploaded once per frame. Every visible box (ground,
//! tower slabs, crane, falling slab, particles) is one instance of the unit cube.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Instance {
    /// World-space center of the box.
    pub position: [f32; 3],
    /// Per-axis size (the unit cube is scaled to this).
    pub scale: [f32; 3],
    /// Linear RGB base color (values >1 read as emissive/neon).
    pub color: [f32; 3],
    /// Emissive/mode channel: 0 = lit, >0 = extra glow, <0 = ghost shell.
    pub glow: f32,
}

impl Instance {
    /// Instance buffer layout: slot 1, per-instance, locations 2..=5.
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 4] = wgpu::vertex_attr_array![
            2 => Float32x3, // position
            3 => Float32x3, // scale
            4 => Float32x3, // color
            5 => Float32    // glow
        ];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Instance>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRS,
        }
    }
}

/// A box instance from a world center + full size.
pub fn boxx(center: [f32; 3], size: [f32; 3], color: [f32; 3], glow: f32) -> Instance {
    Instance { position: center, scale: size, color, glow }
}
