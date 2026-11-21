//! Optical drive subsystem: physical/virtual drives, ISO parsing,
//! game discovery, and metadata extraction.

pub mod drive;
pub mod game_scanner;
pub mod iso_parser;
pub mod metadata;

pub use drive::{detect_drives, DirFs, DiscFs, DriveInfo, MediaSource, MountedMedia, VirtualDrive};
pub use game_scanner::{group_multidisc, scan, ScannedGame};
pub use iso_parser::Iso9660Image;
pub use metadata::{parse_config_file, parse_game_config, GameMetadata, Region};
