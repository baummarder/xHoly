//! Guest/host graphics seam for the Gen9 HLE runtime (Phase 28).
//!
//! This module only defines the **architecture** that separates guest
//! graphics interfaces from host rendering backends. It deliberately
//! contains no fake D3D12 device, no resource that pretends to be
//! renderable and no shader translation: `d3d12_xs.dll` imports are not
//! implemented (see the registry), and nothing here claims Xbox
//! Direct3D 12 compatibility.
//!
//! Host backends are vendor-neutral: wgpu already selects Vulkan /
//! OpenGL / software at runtime, and Vulkan may be evaluated as the
//! cross-platform path for NVIDIA and AMD hosts. No Microsoft GDK or
//! GDKX dependency is introduced here.

/// Host rendering backend families xHoly may bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostBackendKind {
    /// Vulkan (candidate cross-platform backend).
    Vulkan,
    /// OpenGL.
    OpenGl,
    /// CPU/software path (always available, no GPU requirement).
    Software,
}

impl HostBackendKind {
    /// Stable display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Vulkan => "Vulkan",
            Self::OpenGl => "OpenGL",
            Self::Software => "software",
        }
    }
}

/// A host-side rendering backend.
///
/// Implementations wrap the existing wgpu renderer; the trait exists so
/// the guest-facing HLE side never depends on a concrete GPU API or
/// vendor.
pub trait HostRenderBackend: Send {
    /// Backend family.
    fn kind(&self) -> HostBackendKind;
    /// Human-readable backend name (for diagnostics).
    fn name(&self) -> &str;
    /// Whether the backend has been initialised successfully.
    fn is_ready(&self) -> bool;
}

/// Errors from the graphics runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphicsError {
    /// No host backend is bound — device creation is refused rather than
    /// simulated.
    NoBackend,
    /// The bound backend is not ready.
    BackendNotReady,
    /// The requested operation has no implementation yet.
    Unsupported(&'static str),
}

impl std::fmt::Display for GraphicsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoBackend => f.write_str("no host graphics backend is bound"),
            Self::BackendNotReady => f.write_str("host graphics backend is not ready"),
            Self::Unsupported(what) => write!(f, "not implemented: {what}"),
        }
    }
}

/// Guest-visible device state tracked by the HLE side.
///
/// This is bookkeeping only: it records that the guest asked for a
/// device, not that any host device exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuestDeviceModel {
    /// Debug label supplied by the caller (echoed in diagnostics).
    pub label: String,
    /// Backend family that would serve the device, if any.
    pub backend: Option<HostBackendKind>,
}

/// The graphics HLE runtime: owns the optional host backend and the
/// guest device model.
#[derive(Default)]
pub struct GraphicsRuntime {
    backend: Option<Box<dyn HostRenderBackend>>,
    device: Option<GuestDeviceModel>,
}

impl std::fmt::Debug for GraphicsRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraphicsRuntime")
            .field(
                "backend",
                &self
                    .backend
                    .as_ref()
                    .map(|b| (b.name(), b.kind(), b.is_ready())),
            )
            .field("device", &self.device)
            .finish()
    }
}

impl GraphicsRuntime {
    /// Runtime without a backend: every device request fails explicitly.
    pub fn new() -> Self {
        Self::default()
    }

    /// Runtime bound to a host backend.
    pub fn with_backend(backend: Box<dyn HostRenderBackend>) -> Self {
        Self {
            backend: Some(backend),
            device: None,
        }
    }

    /// The bound backend's family, if any.
    pub fn backend_kind(&self) -> Option<HostBackendKind> {
        self.backend.as_ref().map(|b| b.kind())
    }

    /// The guest device model, if a device was created.
    pub fn device(&self) -> Option<&GuestDeviceModel> {
        self.device.as_ref()
    }

    /// Attempt to create the guest device model.
    ///
    /// Refuses without a ready host backend — a device is never
    /// fabricated. Creating a device model does **not** imply rendering
    /// capability: no draw path exists yet.
    pub fn create_device(&mut self, label: &str) -> Result<(), GraphicsError> {
        let Some(backend) = self.backend.as_ref() else {
            return Err(GraphicsError::NoBackend);
        };
        if !backend.is_ready() {
            return Err(GraphicsError::BackendNotReady);
        }
        let kind = backend.kind();
        self.device = Some(GuestDeviceModel {
            label: label.to_string(),
            backend: Some(kind),
        });
        Ok(())
    }

    /// Guest-side draw submission — explicitly unimplemented.
    pub fn submit_draw(&self, _draw_count: u32) -> Result<(), GraphicsError> {
        Err(GraphicsError::Unsupported(
            "guest draw-call translation (no Xbox shader path)",
        ))
    }

    /// Guest-side resource creation — explicitly unimplemented.
    pub fn create_resource(&mut self, _description: &str) -> Result<(), GraphicsError> {
        Err(GraphicsError::Unsupported(
            "guest resource creation (xg_xs/d3d12_xs not implemented)",
        ))
    }
}

/// A backend used by tests and headless runs: reports its family but
/// performs no rendering.
#[derive(Debug, Clone, Copy)]
pub struct MockBackend {
    kind: HostBackendKind,
    ready: bool,
}

impl MockBackend {
    /// Backend of the given family with the given readiness.
    pub fn new(kind: HostBackendKind, ready: bool) -> Self {
        Self { kind, ready }
    }
}

impl HostRenderBackend for MockBackend {
    fn kind(&self) -> HostBackendKind {
        self.kind
    }

    fn name(&self) -> &str {
        "mock"
    }

    fn is_ready(&self) -> bool {
        self.ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_backend_refuses_device_creation() {
        let mut rt = GraphicsRuntime::new();
        assert_eq!(rt.backend_kind(), None);
        assert_eq!(rt.create_device("game"), Err(GraphicsError::NoBackend));
        assert!(rt.device().is_none(), "no device may be fabricated");
    }

    #[test]
    fn not_ready_backend_refuses_device_creation() {
        let mut rt = GraphicsRuntime::with_backend(Box::new(MockBackend::new(
            HostBackendKind::Vulkan,
            false,
        )));
        assert_eq!(
            rt.create_device("game"),
            Err(GraphicsError::BackendNotReady)
        );
        assert!(rt.device().is_none());
    }

    #[test]
    fn ready_backend_creates_a_bookkeeping_device_only() {
        let mut rt = GraphicsRuntime::with_backend(Box::new(MockBackend::new(
            HostBackendKind::Vulkan,
            true,
        )));
        rt.create_device("game").expect("ready backend");
        let device = rt.device().expect("device model");
        assert_eq!(device.label, "game");
        assert_eq!(device.backend, Some(HostBackendKind::Vulkan));
        // Still no rendering capability.
        assert!(matches!(
            rt.submit_draw(1),
            Err(GraphicsError::Unsupported(_))
        ));
        assert!(matches!(
            rt.create_resource("texture"),
            Err(GraphicsError::Unsupported(_))
        ));
    }

    #[test]
    fn backend_kinds_are_vendor_neutral_labels() {
        assert_eq!(HostBackendKind::Vulkan.label(), "Vulkan");
        assert_eq!(HostBackendKind::Software.label(), "software");
        let rt = GraphicsRuntime::with_backend(Box::new(MockBackend::new(
            HostBackendKind::Software,
            true,
        )));
        assert_eq!(rt.backend_kind(), Some(HostBackendKind::Software));
    }
}
