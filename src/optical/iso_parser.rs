//! Minimal ISO 9660 reader for optical disc images.
//!
//! Reads Primary Volume Descriptors and walks directory records on
//! demand (no full-image buffering — Blu-ray images can be 50 GB).
//! UDF (Blu-ray Video) media is detected and reported as unsupported.
//!
//! Xbox One game discs use ISO 9660 directory structures, so this parser
//! covers retail game media; UDF-only BDAV video discs are out of scope.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use super::metadata::{DiscMetadata, FilesystemKind};

const SECTOR_SIZE: u64 = 2048;
const PVD_LBA: u64 = 16;
const VD_ID: &[u8; 5] = b"CD001";

/// A mounted ISO 9660 disc image.
pub struct Iso9660Image {
    file: File,
    /// Root directory record extent, parsed from the PVD.
    root: DirRecord,
    volume_id: String,
    system_id: String,
    created: Option<String>,
}

/// A single ISO9660 directory record.
#[derive(Debug, Clone)]
struct DirRecord {
    /// LBA of the extent containing file data / child directory records.
    extent_lba: u32,
    /// Length of the extent in bytes.
    data_len: u32,
    /// File flags (bit 1 = directory).
    flags: u8,
    /// File identifier (e.g. `GAME`, `.`, `..`, `FILE.BIN;1`).
    name: String,
}

impl DirRecord {
    fn is_dir(&self) -> bool {
        self.flags & 0x02 != 0
    }
}

/// A directory listing entry exposed to the game scanner.
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    pub size: u32,
}

/// Error type for ISO parsing.
#[derive(Debug, thiserror::Error)]
pub enum IsoError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("not an ISO 9660 image (bad volume descriptor)")]
    NotIso,
    #[error("UDF filesystem detected — UDF parsing not yet implemented")]
    UdfUnsupported,
    #[error("path not found in image: {0}")]
    NotFound(String),
    #[error("malformed directory record at sector {0}")]
    Malformed(u32),
}

impl Iso9660Image {
    /// Open and validate an ISO image file.
    pub fn open(path: &Path) -> Result<Self, IsoError> {
        let mut file = File::open(path)?;

        // UDF detection: scan early sectors + anchor area for UDF signatures.
        if Self::looks_like_udf(&mut file)? {
            return Err(IsoError::UdfUnsupported);
        }

        // Primary Volume Descriptor at sector 16.
        let mut pvd = [0u8; SECTOR_SIZE as usize];
        file.seek(SeekFrom::Start(PVD_LBA * SECTOR_SIZE))?;
        file.read_exact(&mut pvd)?;

        if &pvd[1..6] != VD_ID || pvd[0] != 0x01 {
            return Err(IsoError::NotIso);
        }

        let system_id = read_a_bytes(&pvd[40..72]);
        let volume_id = read_a_bytes(&pvd[72..104]);
        let created =
            decode_recording_date(&pvd[813..830]).or_else(|| decode_dir_date(&pvd[18..25]));
        let root = parse_dir_record(&pvd[156..190], 0)?;

        Ok(Self {
            file,
            root,
            volume_id,
            system_id,
            created,
        })
    }

    /// Volume metadata for display in the game list.
    pub fn metadata(&self) -> DiscMetadata {
        DiscMetadata {
            volume_label: self.volume_id.clone(),
            system_id: self.system_id.clone(),
            publisher: None,
            created: self.created.clone(),
            filesystem: FilesystemKind::Iso9660,
        }
    }

    /// Check whether bytes look like a UDF descriptor sequence.
    fn looks_like_udf(file: &mut File) -> Result<bool, IsoError> {
        const UDF_IDS: [&[u8]; 5] = [b"BEA01", b"NSR02", b"NSR03", b"TEA01", b"TD"];
        for lba in [256u64, 257, 16, 17, 18, 19, 20] {
            let mut buf = [0u8; 32];
            file.seek(SeekFrom::Start(lba * SECTOR_SIZE))?;
            if file.read(&mut buf)? < 5 {
                continue;
            }
            // UDF descriptors may start at offset 16 within the sector block.
            if UDF_IDS
                .iter()
                .any(|id| buf.starts_with(id) || buf[16..].starts_with(id))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Read `buf.len()` bytes at absolute byte offset.
    fn read_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<(), IsoError> {
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(buf)?;
        Ok(())
    }

    /// Read an entire extent (up to `len` bytes) into memory.
    fn read_extent(&mut self, rec: &DirRecord) -> Result<Vec<u8>, IsoError> {
        let mut buf = vec![0u8; rec.data_len as usize];
        self.read_at(rec.extent_lba as u64 * SECTOR_SIZE, &mut buf)?;
        Ok(buf)
    }

    /// Parse all records in a directory extent.
    fn parse_dir_extent(&mut self, rec: &DirRecord) -> Result<Vec<DirRecord>, IsoError> {
        let data = self.read_extent(rec)?;
        let mut records = Vec::new();
        let mut off = 0usize;
        while off < data.len() {
            let len = data[off] as usize;
            if len == 0 {
                // Padding — rest of sector is unused.
                break;
            }
            if off + len > data.len() || len < 34 {
                return Err(IsoError::Malformed(rec.extent_lba));
            }
            let name_len = data[off + 32] as usize;
            if len < 33 + name_len {
                return Err(IsoError::Malformed(rec.extent_lba));
            }
            records.push(parse_dir_record(&data[off..off + len], rec.extent_lba)?);
            off += len;
        }
        Ok(records)
    }

    /// Resolve a `/`-separated path (ISO-style, `;1` suffixes tolerated)
    /// to a directory record, starting from the root.
    fn resolve(&mut self, path: &str) -> Result<DirRecord, IsoError> {
        let mut current = self.root.clone();
        let clean = path.trim_start_matches(['/', '\\']);
        if clean.is_empty() {
            return Ok(current);
        }
        for comp in clean.split(['/', '\\']) {
            let want = normalize_name(comp);
            if want == "." {
                continue;
            }
            if want == ".." {
                return Err(IsoError::NotFound(path.to_string()));
            }
            let entries = self.parse_dir_extent(&current)?;
            let found = entries
                .iter()
                .find(|e| e.name != "." && e.name != ".." && normalize_name(&e.name) == want);
            match found {
                Some(e) => current = e.clone(),
                None => return Err(IsoError::NotFound(path.to_string())),
            }
        }
        Ok(current)
    }

    /// List a directory (path relative to image root, `/`-separated).
    pub fn list_dir(&mut self, path: &str) -> Result<Vec<DirEntry>, IsoError> {
        let rec = self.resolve(path)?;
        if !rec.is_dir() {
            return Err(IsoError::NotFound(path.to_string()));
        }
        let entries = self.parse_dir_extent(&rec)?;
        Ok(entries
            .into_iter()
            .filter(|e| e.name != "." && e.name != "..")
            .map(|e| DirEntry {
                name: strip_version(&e.name).to_string(),
                is_dir: e.is_dir(),
                size: e.data_len,
            })
            .collect())
    }

    /// Read a file's full contents.
    pub fn read_file(&mut self, path: &str) -> Result<Vec<u8>, IsoError> {
        let rec = self.resolve(path)?;
        if rec.is_dir() {
            return Err(IsoError::NotFound(path.to_string()));
        }
        self.read_extent(&rec)
    }

    /// Check whether a path exists.
    pub fn exists(&mut self, path: &str) -> bool {
        self.resolve(path).is_ok()
    }
}

/// Parse one 33+N byte directory record.
fn parse_dir_record(bytes: &[u8], sector: u32) -> Result<DirRecord, IsoError> {
    if bytes.len() < 34 {
        return Err(IsoError::Malformed(sector));
    }
    let len = bytes[0] as usize;
    if len < 34 || len > bytes.len() {
        return Err(IsoError::Malformed(sector));
    }
    let rec = &bytes[..len];
    let extent_lba = u32::from_le_bytes([rec[2], rec[3], rec[4], rec[5]]);
    let data_len = u32::from_le_bytes([rec[10], rec[11], rec[12], rec[13]]);
    let flags = rec[25];
    let name_len = rec[32] as usize;
    if 33 + name_len > len {
        return Err(IsoError::Malformed(sector));
    }
    let raw_name = &rec[33..33 + name_len];
    // Name field may be padded to an even boundary; name_len already
    // excludes padding per ECMA-119.
    let name = String::from_utf8_lossy(raw_name).to_string();
    Ok(DirRecord {
        extent_lba,
        data_len,
        flags,
        name,
    })
}

/// Normalize an ISO file identifier for comparison: uppercase, strip `;ver`.
fn normalize_name(name: &str) -> String {
    strip_version(name).to_ascii_uppercase()
}

/// Strip the `;1`-style version suffix from an ISO file id.
fn strip_version(name: &str) -> &str {
    match name.rfind(';') {
        Some(i) if name[i + 1..].chars().all(|c| c.is_ascii_digit()) => &name[..i],
        _ => name,
    }
}

/// Decode a 7-byte ISO directory-record date into ISO-8601-ish text.
fn decode_dir_date(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 7 {
        return None;
    }
    let year = 1900 + bytes[0] as u32;
    let month = bytes[1];
    let day = bytes[2];
    let hour = bytes[3];
    let min = bytes[4];
    let sec = bytes[5];
    // byte 6 is timezone offset in 15-min units; omitted from display.
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    ))
}

/// Decode the 17-byte both-endian recording timestamp in the PVD
/// (offset 813). Format: 16 bytes of both-endian date/time + tz.
fn decode_recording_date(bytes: &[u8]) -> Option<String> {
    // Both-endian 16-bit values: LE then BE for each field.
    if bytes.len() < 16 {
        return None;
    }
    let year = u16::from_le_bytes([bytes[0], bytes[1]]) as u32;
    let month = u16::from_le_bytes([bytes[2], bytes[3]]);
    let day = u16::from_le_bytes([bytes[4], bytes[5]]);
    let hour = u16::from_le_bytes([bytes[6], bytes[7]]);
    let min = u16::from_le_bytes([bytes[8], bytes[9]]);
    let sec = u16::from_le_bytes([bytes[10], bytes[11]]);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}Z",
        year, month, day, hour, min, sec
    ))
}

/// Read an ISO-a-string field (a-characters, space-padded).
fn read_a_bytes(field: &[u8]) -> String {
    let s = String::from_utf8_lossy(field);
    s.trim_end_matches(' ').trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SECTOR: usize = 2048;

    /// Write a both-endian 32-bit field (LE then BE) into `buf`.
    fn both_endian_32(buf: &mut [u8], le_off: usize, be_off: usize, v: u32) {
        buf[le_off..le_off + 4].copy_from_slice(&v.to_le_bytes());
        buf[be_off..be_off + 4].copy_from_slice(&v.to_be_bytes());
    }

    /// Build a minimal ISO9660 image containing `GAME/XBOXCONFIG.XML;1`.
    fn build_test_iso(path: &Path, file_payload: &[u8]) {
        let mut img = vec![0u8; 30 * SECTOR];

        // --- PVD at sector 16 ---
        let pvd = &mut img[16 * SECTOR..17 * SECTOR];
        pvd[0] = 0x01;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[6] = 0x01;
        let mut sysid = [b' '; 32];
        sysid[..14].copy_from_slice(b"MICROSOFT XBOX");
        pvd[40..72].copy_from_slice(&sysid);
        let mut volid = [b' '; 32];
        volid[..8].copy_from_slice(b"TESTDISC");
        pvd[72..104].copy_from_slice(&volid);
        // Root directory record at offset 156: extent LBA 20, len 2048.
        let root = &mut pvd[156..190];
        root[0] = 34;
        both_endian_32(root, 2, 6, 20);
        both_endian_32(root, 10, 14, SECTOR as u32);
        root[25] = 0x02; // directory
        root[32] = 1;
        root[33] = 0x00; // "."

        // --- Root dir extent at sector 20 ---
        let dir = &mut img[20 * SECTOR..21 * SECTOR];
        let mut off = 0usize;
        // "." record
        dir[off] = 34;
        both_endian_32(&mut dir[off..], 2, 6, 20);
        both_endian_32(&mut dir[off..], 10, 14, SECTOR as u32);
        dir[off + 25] = 0x02;
        dir[off + 32] = 1;
        dir[off + 33] = 0x00;
        off += 34;
        // ".." record
        dir[off] = 34;
        both_endian_32(&mut dir[off..], 2, 6, 20);
        both_endian_32(&mut dir[off..], 10, 14, SECTOR as u32);
        dir[off + 25] = 0x02;
        dir[off + 32] = 1;
        dir[off + 33] = 0x01;
        off += 34;
        // "GAME" record (name_len 4 → padded len 38)
        dir[off] = 38;
        both_endian_32(&mut dir[off..], 2, 6, 21);
        both_endian_32(&mut dir[off..], 10, 14, SECTOR as u32);
        dir[off + 25] = 0x02;
        dir[off + 32] = 4;
        dir[off + 33..off + 37].copy_from_slice(b"GAME");

        // --- GAME dir extent at sector 21 ---
        let gdir = &mut img[21 * SECTOR..22 * SECTOR];
        let mut off = 0usize;
        // "."
        gdir[off] = 34;
        both_endian_32(&mut gdir[off..], 2, 6, 21);
        both_endian_32(&mut gdir[off..], 10, 14, SECTOR as u32);
        gdir[off + 25] = 0x02;
        gdir[off + 32] = 1;
        gdir[off + 33] = 0x00;
        off += 34;
        // ".."
        gdir[off] = 34;
        both_endian_32(&mut gdir[off..], 2, 6, 20);
        both_endian_32(&mut gdir[off..], 10, 14, SECTOR as u32);
        gdir[off + 25] = 0x02;
        gdir[off + 32] = 1;
        gdir[off + 33] = 0x01;
        off += 34;
        // "XBOXCONFIG.XML;1" (16 chars → padded len 50)
        let name = b"XBOXCONFIG.XML;1";
        gdir[off] = 50;
        both_endian_32(&mut gdir[off..], 2, 6, 22);
        both_endian_32(&mut gdir[off..], 10, 14, file_payload.len() as u32);
        gdir[off + 25] = 0x00; // file
        gdir[off + 32] = name.len() as u8;
        gdir[off + 33..off + 33 + name.len()].copy_from_slice(name);

        // --- File data at sector 22 ---
        img[22 * SECTOR..22 * SECTOR + file_payload.len()].copy_from_slice(file_payload);

        let mut f = File::create(path).unwrap();
        f.write_all(&img).unwrap();
    }

    #[test]
    fn parses_pvd_and_walks_directories() {
        let dir = std::env::temp_dir().join(format!("xholy-iso-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iso = dir.join("test.iso");
        let payload = br#"<TitleId>TEST-00000-00</TitleId><DisplayName>Test Game</DisplayName>"#;
        build_test_iso(&iso, payload);

        let mut image = Iso9660Image::open(&iso).expect("open iso");
        let meta = image.metadata();
        assert_eq!(meta.volume_label, "TESTDISC");
        assert_eq!(meta.filesystem, FilesystemKind::Iso9660);

        let root = image.list_dir("").unwrap();
        assert!(root.iter().any(|e| e.name == "GAME" && e.is_dir));

        let data = image.read_file("GAME/XBOXCONFIG.XML;1").unwrap();
        assert_eq!(data, payload);
        // version-suffix-less path should also resolve
        assert!(image.exists("game/xboxconfig.xml"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rejects_non_iso() {
        let dir = std::env::temp_dir().join(format!("xholy-iso-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let bad = dir.join("bad.iso");
        std::fs::write(&bad, vec![0u8; 64 * SECTOR]).unwrap();
        assert!(Iso9660Image::open(&bad).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
