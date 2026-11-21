//! Optical drive abstraction: detection, mounting, hot-swap, eject.
//!
//! [`VirtualDrive`] presents a uniform interface over physical drives,
//! host directories (an already-mounted disc), and ISO image files.

use std::io;
use std::path::{Path, PathBuf};

use super::iso_parser::{DirEntry, Iso9660Image, IsoError};
use super::metadata::{DiscMetadata, FilesystemKind};

/// A filesystem view over mounted media, abstracting ISO images and
/// plain directories so the game scanner works on both.
pub trait DiscFs {
    /// List entries under `path` ("" or "/" = root).
    fn list_dir(&mut self, path: &str) -> io::Result<Vec<DirEntry>>;
    /// Read a whole file into memory.
    fn read_file(&mut self, path: &str) -> io::Result<Vec<u8>>;
    /// Check existence.
    fn exists(&mut self, path: &str) -> bool;
    /// Volume metadata.
    fn metadata(&self) -> DiscMetadata;
}

/// ISO-backed filesystem.
pub struct IsoFs {
    image: Iso9660Image,
}

impl DiscFs for IsoFs {
    fn list_dir(&mut self, path: &str) -> io::Result<Vec<DirEntry>> {
        self.image
            .list_dir(path)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    fn read_file(&mut self, path: &str) -> io::Result<Vec<u8>> {
        self.image
            .read_file(path)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    fn exists(&mut self, path: &str) -> bool {
        self.image.exists(path)
    }

    fn metadata(&self) -> DiscMetadata {
        self.image.metadata()
    }
}

/// Host-directory-backed filesystem (physical drive already mounted
/// by the OS, or an extracted game folder).
pub struct DirFs {
    root: PathBuf,
}

impl DirFs {
    fn to_host(&self, path: &str) -> PathBuf {
        let rel = path.trim_start_matches(['/', '\\']);
        if rel.is_empty() {
            self.root.clone()
        } else {
            self.root.join(rel)
        }
    }
}

impl DiscFs for DirFs {
    fn list_dir(&mut self, path: &str) -> io::Result<Vec<DirEntry>> {
        let host = self.to_host(path);
        let mut out = Vec::new();
        for entry in std::fs::read_dir(&host)? {
            let entry = entry?;
            let md = entry.metadata()?;
            out.push(DirEntry {
                name: entry.file_name().to_string_lossy().to_string(),
                is_dir: md.is_dir(),
                size: md.len() as u32,
            });
        }
        Ok(out)
    }

    fn read_file(&mut self, path: &str) -> io::Result<Vec<u8>> {
        std::fs::read(self.to_host(path))
    }

    fn exists(&mut self, path: &str) -> bool {
        self.to_host(path).exists()
    }

    fn metadata(&self) -> DiscMetadata {
        DiscMetadata {
            volume_label: self
                .root
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default(),
            system_id: "HOST DIRECTORY".into(),
            publisher: None,
            created: None,
            filesystem: FilesystemKind::HostDirectory,
        }
    }
}

/// Kind of media source a drive can mount.
#[derive(Debug, Clone)]
pub enum MediaSource {
    /// An ISO 9660 disc image file (`.iso`).
    IsoImage(PathBuf),
    /// A directory — either a physical drive letter root (`D:\`) or an
    /// extracted game folder.
    Directory(PathBuf),
}

/// Currently mounted media on a virtual drive.
pub struct MountedMedia {
    fs: Box<dyn DiscFs>,
    source: MediaSource,
    /// When this media was mounted (for hot-swap bookkeeping).
    pub mounted_at: std::time::SystemTime,
}

impl MountedMedia {
    pub fn filesystem(&mut self) -> &mut dyn DiscFs {
        &mut *self.fs
    }

    pub fn source(&self) -> &MediaSource {
        &self.source
    }
}

/// A virtual optical drive supporting hot-plug (mount/eject).
pub struct VirtualDrive {
    name: String,
    media: Option<MountedMedia>,
}

impl VirtualDrive {
    /// Create a named virtual drive (e.g. "D:").
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            media: None,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn is_mounted(&self) -> bool {
        self.media.is_some()
    }

    /// Mount media. Replaces any currently mounted disc (hot-swap).
    pub fn mount(&mut self, source: MediaSource) -> io::Result<()> {
        let fs: Box<dyn DiscFs> = match &source {
            MediaSource::IsoImage(path) => {
                let image = Iso9660Image::open(path).map_err(|e| match e {
                    IsoError::UdfUnsupported => {
                        io::Error::new(io::ErrorKind::Unsupported, e.to_string())
                    }
                    other => io::Error::new(io::ErrorKind::InvalidData, other.to_string()),
                })?;
                Box::new(IsoFs { image })
            }
            MediaSource::Directory(path) => Box::new(DirFs { root: path.clone() }),
        };
        self.media = Some(MountedMedia {
            fs,
            source,
            mounted_at: std::time::SystemTime::now(),
        });
        Ok(())
    }

    /// Convenience: mount by path, sniffing ISO vs directory.
    pub fn mount_path(&mut self, path: &Path) -> io::Result<()> {
        if path.is_dir() {
            self.mount(MediaSource::Directory(path.to_path_buf()))
        } else if is_iso_image(path) {
            self.mount(MediaSource::IsoImage(path.to_path_buf()))
        } else {
            // Treat unknown files as ISO attempts; the parser will error
            // with a precise message if the descriptor is wrong.
            self.mount(MediaSource::IsoImage(path.to_path_buf()))
        }
    }

    /// Eject the current disc.
    pub fn eject(&mut self) -> Option<MountedMedia> {
        self.media.take()
    }

    /// Access the mounted filesystem (if any).
    pub fn media(&mut self) -> Option<&mut MountedMedia> {
        self.media.as_mut()
    }
}

/// Heuristic ISO detection by extension.
fn is_iso_image(path: &Path) -> bool {
    path.extension()
        .map(|e| e.eq_ignore_ascii_case("iso"))
        .unwrap_or(false)
}

/// A detected physical optical drive.
#[derive(Debug, Clone)]
pub struct DriveInfo {
    /// Drive letter / mount point (`D:\`, `/dev/sr0`, `/Volumes/...`).
    pub mount_point: PathBuf,
    /// Human-readable drive name from the OS.
    pub name: String,
    /// Media currently in the drive, if reported by the OS.
    pub media_label: Option<String>,
}

/// Detect physical optical drives on the host.
///
/// Windows: queries `Win32_CDROMDrive` via PowerShell.
/// Linux: probes `/dev/sr*` and `/media/*/`. macOS returns an empty list
/// (use `drutil` externally; scaffold limitation).
pub fn detect_drives() -> Vec<DriveInfo> {
    #[cfg(target_os = "windows")]
    {
        detect_drives_windows()
    }
    #[cfg(target_os = "linux")]
    {
        detect_drives_linux()
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Vec::new()
    }
}

#[cfg(target_os = "windows")]
fn detect_drives_windows() -> Vec<DriveInfo> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            "Get-CimInstance Win32_CDROMDrive | Select-Object Drive,Name,VolumeName | ConvertTo-Json -Compress",
        ])
        .output();
    let Ok(out) = output else { return Vec::new() };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_cdrom_json(&text)
}

#[cfg(target_os = "linux")]
fn detect_drives_linux() -> Vec<DriveInfo> {
    let mut drives = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/dev") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("sr") {
                drives.push(DriveInfo {
                    mount_point: e.path(),
                    name: format!("Linux optical ({})", name),
                    media_label: None,
                });
            }
        }
    }
    // Mounted media under /media and /mnt
    for base in ["/media", "/mnt", "/run/media"] {
        if let Ok(users) = std::fs::read_dir(base) {
            for u in users.flatten() {
                if let Ok(subs) = std::fs::read_dir(u.path()) {
                    for s in subs.flatten() {
                        let p = s.path();
                        if p.join(".iso9660").exists() || p.join("VIDEO_TS").exists() {
                            drives.push(DriveInfo {
                                mount_point: p,
                                name: "Mounted optical media".into(),
                                media_label: None,
                            });
                        }
                    }
                }
            }
        }
    }
    drives
}

/// Parse PowerShell `ConvertTo-Json` output (single object or array).
fn parse_cdrom_json(text: &str) -> Vec<DriveInfo> {
    // Minimal JSON extraction without a serde dependency on this path:
    // find "Drive":"X:" style fields per object.
    let mut drives = Vec::new();
    for obj in text
        .split("},{")
        .map(|s| s.trim_matches(|c| c == '[' || c == ']' || c == '{' || c == '}'))
    {
        let drive = extract_json_str(obj, "Drive");
        let name = extract_json_str(obj, "Name").unwrap_or_else(|| "Optical drive".into());
        let vol = extract_json_str(obj, "VolumeName");
        if let Some(d) = drive {
            let letter = d.trim_end_matches(':');
            drives.push(DriveInfo {
                mount_point: PathBuf::from(format!("{}:\\", letter)),
                name,
                media_label: vol.filter(|v| !v.is_empty()),
            });
        }
    }
    drives
}

fn extract_json_str(obj: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\":\"", key);
    let start = obj.find(&pat)? + pat.len();
    let end = obj[start..].find('"')? + start;
    Some(obj[start..end].to_string())
}
