//! Post-processing configuration — Phase 9.
//!
//! Pure-data types (no wgpu dependency) describing the GPU post-FX
//! pipeline: bloom, tone mapping and anti-aliasing. The wgpu-side
//! implementation lives in [`crate::gui::windowed::postfx`]; the
//! software fallback ignores these settings.

use serde::{Deserialize, Serialize};

/// Tone-mapping operator applied after bloom in the composite pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToneMapping {
    /// No tone mapping (clamp only).
    Linear,
    /// Reinhard: `c / (1 + c)` — cheap, slightly desaturating.
    #[default]
    Reinhard,
    /// Filmic (Hable-ish curve) — more contrast in highlights.
    Filmic,
    /// ACES fitted — filmic, good highlight rolloff.
    Acs,
}

impl ToneMapping {
    /// Parse a CLI/config string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "linear" | "none" | "off" => Some(Self::Linear),
            "reinhard" => Some(Self::Reinhard),
            "filmic" | "hable" => Some(Self::Filmic),
            "aces" | "acs" => Some(Self::Acs),
            _ => None,
        }
    }

    /// CLI-friendly name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Reinhard => "reinhard",
            Self::Filmic => "filmic",
            Self::Acs => "aces",
        }
    }

    /// Discriminant matching the WGSL uniform (`u32`).
    pub fn uniform_index(&self) -> u32 {
        match self {
            Self::Linear => 0,
            Self::Reinhard => 1,
            Self::Filmic => 2,
            Self::Acs => 3,
        }
    }
}

/// Anti-aliasing method for the final composite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AaMethod {
    /// No AA.
    Off,
    /// FXAA-lite in the composite shader (luma-edge blend).
    #[default]
    Fxaa,
    /// 4x MSAA — reserved; the blit pipeline is single-sample today.
    /// Selecting it currently behaves like `Fxaa` and is reported as
    /// such in the settings panel.
    Msaa4,
}

impl AaMethod {
    /// Parse a CLI/config string.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "off" | "none" => Some(Self::Off),
            "fxaa" => Some(Self::Fxaa),
            "msaa" | "msaa4" | "4x" => Some(Self::Msaa4),
            "taa" => Some(Self::Fxaa), // TAA needs history; falls back to FXAA.
            _ => None,
        }
    }

    /// CLI-friendly name.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Fxaa => "fxaa",
            Self::Msaa4 => "msaa4",
        }
    }

    /// Whether the composite shader should run the AA pass.
    pub fn enabled(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Full post-FX configuration.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PostFxConfig {
    /// Enable the bloom chain (bright-pass + separable blur).
    pub bloom: bool,
    /// Bloom intensity added on top of the scene (0.0–2.0 typical).
    pub bloom_strength: f32,
    /// Luminance threshold for the bright pass.
    pub bloom_threshold: f32,
    /// Tone-mapping operator.
    pub tone_mapping: ToneMapping,
    /// Anti-aliasing method.
    pub aa: AaMethod,
}

impl Default for PostFxConfig {
    fn default() -> Self {
        Self {
            bloom: true,
            bloom_strength: 0.6,
            bloom_threshold: 0.75,
            tone_mapping: ToneMapping::Reinhard,
            aa: AaMethod::Fxaa,
        }
    }
}

impl PostFxConfig {
    /// Whether any post-processing is active (otherwise the renderer
    /// can use the plain blit path).
    pub fn active(&self) -> bool {
        self.bloom || self.tone_mapping != ToneMapping::Linear || self.aa.enabled()
    }

    /// Pack parameters into the WGSL uniform layout:
    /// `vec4(bloom_strength, bloom_threshold, tonemap_mode, aa_enable)`
    /// followed by `vec2(texel_w, texel_h)`.
    pub fn uniforms(&self, width: u32, height: u32) -> [f32; 6] {
        [
            if self.bloom { self.bloom_strength } else { 0.0 },
            self.bloom_threshold,
            self.tone_mapping.uniform_index() as f32,
            if self.aa.enabled() { 1.0 } else { 0.0 },
            1.0 / width.max(1) as f32,
            1.0 / height.max(1) as f32,
        ]
    }

    /// CPU reference for Reinhard tone mapping (tests + software path).
    pub fn tonemap_reinhard(c: f32) -> f32 {
        c / (1.0 + c)
    }

    /// CPU reference for the filmic curve.
    pub fn tonemap_filmic(c: f32) -> f32 {
        let x = c * (2.51 * c + 0.03) / (c * (2.43 * c + 0.59) + 0.14);
        x.clamp(0.0, 1.0)
    }

    /// CPU reference for ACES (fitted, per-channel).
    pub fn tonemap_aces(c: f32) -> f32 {
        let a = 2.51;
        let b = 0.03;
        let cc = 2.43;
        let d = 0.59;
        let e = 0.14;
        let out = (c * (a * c + b)) / (c * (cc * c + d) + e);
        out.clamp(0.0, 1.0)
    }

    /// Apply the configured tone map to one channel.
    pub fn tonemap(&self, c: f32) -> f32 {
        match self.tone_mapping {
            ToneMapping::Linear => c.clamp(0.0, 1.0),
            ToneMapping::Reinhard => Self::tonemap_reinhard(c),
            ToneMapping::Filmic => Self::tonemap_filmic(c),
            ToneMapping::Acs => Self::tonemap_aces(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_roundtrip() {
        assert_eq!(ToneMapping::parse("ACES"), Some(ToneMapping::Acs));
        assert_eq!(ToneMapping::parse("hable"), Some(ToneMapping::Filmic));
        assert_eq!(AaMethod::parse("msaa4"), Some(AaMethod::Msaa4));
        assert_eq!(AaMethod::parse("bogus"), None);
        assert_eq!(ToneMapping::Acs.as_str(), "aces");
    }

    #[test]
    fn tonemap_bounds() {
        for c in [0.0, 0.25, 1.0, 4.0, 100.0] {
            for m in [
                ToneMapping::Linear,
                ToneMapping::Reinhard,
                ToneMapping::Filmic,
                ToneMapping::Acs,
            ] {
                let cfg = PostFxConfig {
                    tone_mapping: m,
                    ..Default::default()
                };
                let v = cfg.tonemap(c);
                assert!((0.0..=1.0).contains(&v), "{m:?}({c}) = {v}");
            }
        }
    }

    #[test]
    fn uniforms_layout() {
        let cfg = PostFxConfig::default();
        let u = cfg.uniforms(1920, 1080);
        assert_eq!(u[2], ToneMapping::Reinhard.uniform_index() as f32);
        assert!((u[4] - 1.0 / 1920.0).abs() < 1e-6);
        let off = PostFxConfig {
            bloom: false,
            aa: AaMethod::Off,
            tone_mapping: ToneMapping::Linear,
            ..Default::default()
        };
        assert!(!off.active());
    }
}
