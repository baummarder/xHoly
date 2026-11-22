//! Game-shader cache — Phase 10.
//!
//! Compiled shader artifacts (`.spv` + `.wgsl`) keyed by
//! FNV-1a hash of (DXIL bitcode || resource-binding signature).
//! Lives under the platform cache dir next to the GUI shader cache:
//! `%LOCALAPPDATA%\xHoly\cache\shaders\game\` (Win) /
//! `$XDG_CACHE_HOME/xholy/shaders/game/`.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

/// One compiled shader artifact pair.
#[derive(Debug, Clone)]
pub struct ShaderArtifact {
    /// Cache key (hex).
    pub key: String,
    /// WGSL source.
    pub wgsl: String,
    /// SPIR-V binary (may be empty when generation was unsupported).
    pub spirv: Vec<u8>,
    /// Warnings collected during lowering/emission.
    pub warnings: Vec<String>,
}

/// On-disk shader artifact cache.
pub struct GameShaderCache {
    dir: PathBuf,
}

impl GameShaderCache {
    /// Open/create under the platform cache dir.
    pub fn open_default() -> Result<Self, String> {
        Self::open(
            &crate::gui::textures::cache_root()
                .join("shaders")
                .join("game"),
        )
    }

    /// Open/create at `dir`.
    pub fn open(dir: &Path) -> Result<Self, String> {
        fs::create_dir_all(dir).map_err(|e| format!("shader cache dir: {e}"))?;
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    /// Cache key from DXIL bitcode + binding signature bytes.
    pub fn key(dxil: &[u8], bindings: &[u8]) -> String {
        let mut h = DefaultHasher::new();
        dxil.hash(&mut h);
        bindings.hash(&mut h);
        format!("{:016x}", h.finish())
    }

    /// Look up a cached artifact; verifies the stored key file.
    pub fn get(&self, key: &str) -> Option<ShaderArtifact> {
        let wgsl_path = self.dir.join(format!("{key}.wgsl"));
        let spv_path = self.dir.join(format!("{key}.spv"));
        let wgsl = fs::read_to_string(&wgsl_path).ok()?;
        let spirv = fs::read(&spv_path).unwrap_or_default();
        Some(ShaderArtifact {
            key: key.to_string(),
            wgsl,
            spirv,
            warnings: Vec::new(),
        })
    }

    /// Store an artifact.
    pub fn put(&self, artifact: &ShaderArtifact) -> Result<(), String> {
        fs::write(
            self.dir.join(format!("{}.wgsl", artifact.key)),
            &artifact.wgsl,
        )
        .map_err(|e| format!("write wgsl: {e}"))?;
        if !artifact.spirv.is_empty() {
            fs::write(
                self.dir.join(format!("{}.spv", artifact.key)),
                &artifact.spirv,
            )
            .map_err(|e| format!("write spv: {e}"))?;
        }
        Ok(())
    }
}

/// Full compile path: DXIL bytes → cache lookup → IR → WGSL+SPIR-V.
///
/// `bindings` is a caller-computed signature of the shader's resource
/// table (uniform/texture layout) so shaders with identical code but
/// different bindings get distinct cache entries.
pub fn compile_shader(
    cache: &GameShaderCache,
    dxil_bitcode: &[u8],
    bindings_sig: &[u8],
) -> Result<ShaderArtifact, String> {
    let key = GameShaderCache::key(dxil_bitcode, bindings_sig);
    if let Some(hit) = cache.get(&key) {
        return Ok(hit);
    }

    // DXBC container unwrap first (callers may pass raw DXBC).
    let bitcode = if dxil_bitcode.starts_with(b"DXBC") {
        super::dxil_parser::parse_dxbc(dxil_bitcode)
            .map_err(|e| e.to_string())?
            .bitcode
    } else {
        dxil_bitcode.to_vec()
    };

    let (wgsl, spirv, warnings) = match super::hlsl_compiler::lower(&bitcode) {
        Ok(ir) => {
            let (wgsl, warnings) = super::wgsl_emitter::emit_wgsl(&ir);
            let spirv = super::spirv_generator::generate(&ir, "main").unwrap_or_default();
            (wgsl, spirv, warnings)
        }
        Err(e) => (
            super::wgsl_emitter::blit_wgsl(),
            Vec::new(),
            vec![format!("lower failed ({e}) — blit fallback")],
        ),
    };

    let artifact = ShaderArtifact {
        key,
        wgsl,
        spirv,
        warnings,
    };
    cache.put(&artifact)?;
    Ok(artifact)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_hit_and_miss() {
        let dir = std::env::temp_dir().join("xholy_game_shader_cache");
        let _ = fs::remove_dir_all(&dir);
        let cache = GameShaderCache::open(&dir).unwrap();

        // Mini-program DXIL (host-generated, supported subset).
        let body =
            super::super::hlsl_compiler::Node::Const(super::super::hlsl_compiler::Const::F32(1.0));
        let mini = super::super::hlsl_compiler::encode_mini(b'f', &body);

        let a1 = compile_shader(&cache, &mini, b"bind0").unwrap();
        assert!(a1.wgsl.contains("fs_main"));
        // Cache hit: same input → same key, served from disk.
        let a2 = compile_shader(&cache, &mini, b"bind0").unwrap();
        assert_eq!(a1.key, a2.key);
        assert_eq!(a1.wgsl, a2.wgsl);
        // Different bindings → different key.
        let a3 = compile_shader(&cache, &mini, b"bind1").unwrap();
        assert_ne!(a1.key, a3.key);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unsupported_dxil_gets_blit_fallback() {
        let dir = std::env::temp_dir().join("xholy_game_shader_fb");
        let _ = fs::remove_dir_all(&dir);
        let cache = GameShaderCache::open(&dir).unwrap();
        let realish = b"\xde\xc0\x17\x0bLLVM bitcode blob";
        let art = compile_shader(&cache, realish, b"").unwrap();
        assert!(art.wgsl.contains("fs_main")); // blit fallback has fs_main
        assert!(!art.warnings.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
