//! Unit-cube mesh: 24 vertices (per-face normals for flat neon lighting) and
//! 36 indices. Centered on the origin, spanning [-0.5, 0.5] on each axis so the
//! instance shader can place it at an integer grid coordinate.

use bytemuck::{Pod, Zeroable};

#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl Vertex {
    /// Vertex buffer layout: slot 0, per-vertex, locations 0 (pos) & 1 (normal).
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRS: [wgpu::VertexAttribute; 2] =
            wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &ATTRS,
        }
    }
}

const fn vtx(p: [f32; 3], n: [f32; 3]) -> Vertex {
    Vertex { position: p, normal: n }
}

// Faces: +X, -X, +Y, -Y, +Z, -Z. Four verts each, wound CCW when viewed from
// outside (front face = CCW, matches the pipeline's front_face default).
pub const VERTICES: &[Vertex] = &[
    // +X
    vtx([0.5, -0.5, -0.5], [1.0, 0.0, 0.0]),
    vtx([0.5, 0.5, -0.5], [1.0, 0.0, 0.0]),
    vtx([0.5, 0.5, 0.5], [1.0, 0.0, 0.0]),
    vtx([0.5, -0.5, 0.5], [1.0, 0.0, 0.0]),
    // -X
    vtx([-0.5, -0.5, 0.5], [-1.0, 0.0, 0.0]),
    vtx([-0.5, 0.5, 0.5], [-1.0, 0.0, 0.0]),
    vtx([-0.5, 0.5, -0.5], [-1.0, 0.0, 0.0]),
    vtx([-0.5, -0.5, -0.5], [-1.0, 0.0, 0.0]),
    // +Y
    vtx([-0.5, 0.5, -0.5], [0.0, 1.0, 0.0]),
    vtx([-0.5, 0.5, 0.5], [0.0, 1.0, 0.0]),
    vtx([0.5, 0.5, 0.5], [0.0, 1.0, 0.0]),
    vtx([0.5, 0.5, -0.5], [0.0, 1.0, 0.0]),
    // -Y
    vtx([-0.5, -0.5, 0.5], [0.0, -1.0, 0.0]),
    vtx([-0.5, -0.5, -0.5], [0.0, -1.0, 0.0]),
    vtx([0.5, -0.5, -0.5], [0.0, -1.0, 0.0]),
    vtx([0.5, -0.5, 0.5], [0.0, -1.0, 0.0]),
    // +Z
    vtx([-0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
    vtx([0.5, -0.5, 0.5], [0.0, 0.0, 1.0]),
    vtx([0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
    vtx([-0.5, 0.5, 0.5], [0.0, 0.0, 1.0]),
    // -Z
    vtx([0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
    vtx([-0.5, -0.5, -0.5], [0.0, 0.0, -1.0]),
    vtx([-0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
    vtx([0.5, 0.5, -0.5], [0.0, 0.0, -1.0]),
];

pub const INDICES: &[u16] = &[
    0, 1, 2, 0, 2, 3, // +X
    4, 5, 6, 4, 6, 7, // -X
    8, 9, 10, 8, 10, 11, // +Y
    12, 13, 14, 12, 14, 15, // -Y
    16, 17, 18, 16, 18, 19, // +Z
    20, 21, 22, 20, 22, 23, // -Z
];
