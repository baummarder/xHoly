//! Shader resource binding — Phase 10.
//!
//! Models the resource table extracted from a DXIL container
//! (uniform buffers, textures, samplers) and produces the data needed
//! to build wgpu bind groups. Kept wgpu-free so it compiles on all
//! targets; the renderer maps [`ResourceBinding`]s onto
//! `wgpu::BindGroupLayoutEntry`s at pipeline creation.

use serde::{Deserialize, Serialize};

/// Resource kind in a shader's binding table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResourceKind {
    /// Constant/uniform buffer (cb#).
    UniformBuffer,
    /// Shader resource view (t#): texture.
    Texture,
    /// Sampler state (s#).
    Sampler,
    /// Raw/structured buffer (u#).
    Buffer,
}

/// One binding slot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResourceBinding {
    /// HLSL register space (usually 0).
    pub space: u32,
    /// Register kind letter index encoded: 0=cb, 1=t, 2=s, 3=u.
    pub reg_kind: u8,
    /// Register index (#).
    pub reg_index: u32,
    /// Kind.
    pub kind: ResourceKind,
    /// Byte size for buffers (0 = unknown/sized-at-runtime).
    pub size: u32,
}

impl ResourceBinding {
    /// Stable signature bytes for cache keys.
    pub fn signature(&self) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&self.space.to_le_bytes());
        v.push(self.reg_kind);
        v.extend_from_slice(&self.reg_index.to_le_bytes());
        v.push(match self.kind {
            ResourceKind::UniformBuffer => 0,
            ResourceKind::Texture => 1,
            ResourceKind::Sampler => 2,
            ResourceKind::Buffer => 3,
        });
        v.extend_from_slice(&self.size.to_le_bytes());
        v
    }
}

/// A shader's full resource table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ResourceTable {
    pub bindings: Vec<ResourceBinding>,
}

impl ResourceTable {
    /// Concatenated signature for the shader cache key.
    pub fn signature(&self) -> Vec<u8> {
        let mut v = Vec::new();
        for b in &self.bindings {
            v.extend_from_slice(&b.signature());
        }
        v
    }

    /// Find the binding for a texture register.
    pub fn texture(&self, reg: u32) -> Option<&ResourceBinding> {
        self.bindings
            .iter()
            .find(|b| b.kind == ResourceKind::Texture && b.reg_index == reg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_stable_and_distinct() {
        let t1 = ResourceTable {
            bindings: vec![
                ResourceBinding {
                    space: 0,
                    reg_kind: 1,
                    reg_index: 0,
                    kind: ResourceKind::Texture,
                    size: 0,
                },
                ResourceBinding {
                    space: 0,
                    reg_kind: 0,
                    reg_index: 0,
                    kind: ResourceKind::UniformBuffer,
                    size: 256,
                },
            ],
        };
        let t2 = t1.clone();
        let t3 = ResourceTable {
            bindings: vec![ResourceBinding {
                space: 0,
                reg_kind: 1,
                reg_index: 1, // different register
                kind: ResourceKind::Texture,
                size: 0,
            }],
        };
        assert_eq!(t1.signature(), t2.signature());
        assert_ne!(t1.signature(), t3.signature());
        assert!(t1.texture(0).is_some());
        assert!(t1.texture(1).is_none());
    }
}
