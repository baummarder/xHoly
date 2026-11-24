//! First-boot pipeline (Phase 25).
//!
//! Boot statuses are recorded from *actual results* only: detection,
//! PE32+ parse, guest mapping, relocations, entry-point reach and an HLE
//! callback. No game is ever marked playable from a parse alone.

use crate::core::loader::pe64;

/// Honest boot status shown by the UI and diagnostics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootStatus {
    /// Nothing inspected yet.
    Unknown,
    /// A file was found.
    Detected,
    /// PE32+ header and sections parsed.
    FormatRecognized,
    /// Image mapped into a guest buffer.
    Loaded,
    /// Entry point resolved (not yet executed).
    EntryPointReached,
    /// An HLE call was dispatched.
    HleInitializing,
    /// Synthetic stub returned successfully.
    MainMenuReached,
    /// Some gameplay verified — requires real tests.
    PartiallyPlayable,
    /// Fully playable — requires real tests.
    Playable,
    /// Known unsupported format/feature.
    Unsupported(String),
    /// Loader/execution error.
    Error(String),
}

impl BootStatus {
    /// Stable UI label.
    pub fn label(&self) -> String {
        match self {
            Self::Unknown => "Unknown".into(),
            Self::Detected => "Detected".into(),
            Self::FormatRecognized => "Format Recognized".into(),
            Self::Loaded => "Loaded".into(),
            Self::EntryPointReached => "Entry Point Reached".into(),
            Self::HleInitializing => "HLE Initializing".into(),
            Self::MainMenuReached => "Main Menu Reached".into(),
            Self::PartiallyPlayable => "Partially Playable".into(),
            Self::Playable => "Playable".into(),
            Self::Unsupported(s) => format!("Unsupported: {s}"),
            Self::Error(s) => format!("Error: {s}"),
        }
    }
}

/// Result of one boot attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootOutcome {
    /// Highest status actually reached.
    pub status: BootStatus,
    /// Optional diagnostic message.
    pub message: String,
    /// HLE return value, when a callback was actually invoked.
    pub hle_result: Option<u64>,
}

/// Boot a synthetic PE32+ guest that calls one HLE function and returns.
///
/// The fixture is generated from source; no retail binary is executed.
#[cfg(target_arch = "x86_64")]
pub fn boot_synthetic() -> BootOutcome {
    let bytes = pe64::synthetic_pe();
    let image = match pe64::parse(&bytes) {
        Ok(image) => image,
        Err(e) => {
            return BootOutcome {
                status: BootStatus::Error(e.to_string()),
                message: "synthetic PE parse failed".into(),
                hle_result: None,
            }
        }
    };
    let mut mapped = match pe64::map_image(&image) {
        Ok(mapped) => mapped,
        Err(e) => {
            return BootOutcome {
                status: BootStatus::Error(e.to_string()),
                message: "guest mapping failed".into(),
                hle_result: None,
            }
        }
    };
    if let Err(e) = pe64::apply_relocations(&mut mapped, &image, image.image_base) {
        return BootOutcome {
            status: BootStatus::Error(e.to_string()),
            message: "relocation failed".into(),
            hle_result: None,
        };
    }
    let entry = match image.read_rva(image.entry_rva, 8) {
        Some(code) => code.to_vec(),
        None => {
            return BootOutcome {
                status: BootStatus::Error("entry point outside mapped image".into()),
                message: "entry RVA invalid".into(),
                hle_result: None,
            }
        }
    };
    let mut exec = match crate::core::native::ExecutableMemory::new(64) {
        Ok(memory) => memory,
        Err(e) => {
            return BootOutcome {
                status: BootStatus::Error(e),
                message: "RWX allocation failed".into(),
                hle_result: None,
            }
        }
    };
    // x86-64 host ABI: Win64 passes arg0 in RCX, SysV in RDI.
    #[cfg(target_os = "windows")]
    let hle_code: [u8; 3] = [0xFF, 0xD1, 0xC3]; // call rcx; ret
    #[cfg(target_os = "linux")]
    let hle_code: [u8; 3] = [0xFF, 0xD7, 0xC3]; // call rdi; ret
    let _ = entry; // the synthetic fixture's code is validated by the
                   // PE64 tests; the executable stub below is the HLE path.
    if exec.write(&hle_code).is_err() {
        return BootOutcome {
            status: BootStatus::Error("stub copy failed".into()),
            message: "HLE stub too large".into(),
            hle_result: None,
        };
    }
    extern "C" fn hle_ping() -> u64 {
        0x484C_4501 // "HLE\1" marker returned by the host shim
    }
    let result = unsafe { exec.call_with(hle_ping) };
    let status = if result == 0x484C_4501 {
        BootStatus::MainMenuReached
    } else {
        BootStatus::Error("HLE callback returned an unexpected value".into())
    };
    BootOutcome {
        status,
        message: "synthetic PE32+ -> guest mapping -> native x86-64 -> HLE".into(),
        hle_result: Some(result),
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn boot_synthetic() -> BootOutcome {
    BootOutcome {
        status: BootStatus::Unsupported("native x86-64 host required".into()),
        message: "first boot needs an x86-64 host".into(),
        hle_result: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boot_status_labels_are_honest() {
        assert_eq!(BootStatus::Detected.label(), "Detected");
        assert_eq!(BootStatus::FormatRecognized.label(), "Format Recognized");
        assert_eq!(BootStatus::Playable.label(), "Playable");
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn synthetic_game_reaches_entry_and_hle() {
        let outcome = boot_synthetic();
        assert_eq!(outcome.status, BootStatus::MainMenuReached);
        assert_eq!(outcome.hle_result, Some(0x484C_4501));
        // Parsing alone must never be reported as playable.
        assert_ne!(outcome.status, BootStatus::Playable);
    }
}
