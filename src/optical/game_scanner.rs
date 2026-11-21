//! Disc game scanner.
//!
//! Walks a mounted [`DiscFs`] looking for Xbox game content: config files
//! (`xboxconfig.xml`, `GameConfig.xml`), standard payload folders
//! (`GAME`, `PATCH`, `DATA`, `DLC`, `XBOX`), and multi-disc layouts.

use std::io;

use super::drive::DiscFs;
use super::metadata::{parse_game_config, GameMetadata, Region};

/// A game discovered on mounted media.
#[derive(Debug, Clone)]
pub struct ScannedGame {
    pub metadata: GameMetadata,
    /// Filesystem path to the game payload ("" = disc root).
    pub payload_path: String,
    /// Estimated payload size in bytes (files under payload_path).
    pub size_bytes: u64,
    /// File count under the payload.
    pub file_count: u32,
}

/// Filenames treated as game configuration manifests, checked in order.
const CONFIG_CANDIDATES: &[&str] = &[
    "xboxconfig.xml",
    "XBOXCONFIG.XML",
    "gameconfig.xml",
    "GAMECONFIG.XML",
    "default.xex",
];

/// Directory names that indicate Xbox game content on a disc.
const CONTENT_DIRS: &[&str] = &["GAME", "PATCH", "DATA", "DLC", "XBOX", "GAMES"];

/// Maximum directory depth to scan for config files (bounds runtime on
/// deeply nested UDF/ISO trees).
const MAX_SCAN_DEPTH: u32 = 3;

/// Scan mounted media and return all discovered games.
pub fn scan(fs: &mut dyn DiscFs) -> io::Result<Vec<ScannedGame>> {
    let mut games = Vec::new();

    // 1. Shallow search for config files.
    scan_for_configs(fs, "", 0, &mut games)?;

    // 2. Content-folder fallback: titles without configs still get listed.
    if games.is_empty() {
        for entry in fs.list_dir("").unwrap_or_default() {
            if entry.is_dir
                && CONTENT_DIRS
                    .iter()
                    .any(|d| entry.name.eq_ignore_ascii_case(d))
            {
                games.push(ScannedGame {
                    metadata: GameMetadata {
                        title_name: Some(entry.name.clone()),
                        region: Region::Other,
                        payload_path: Some(entry.name.clone()),
                        ..Default::default()
                    },
                    payload_path: entry.name.clone(),
                    size_bytes: dir_size(fs, &entry.name),
                    file_count: count_files(fs, &entry.name, 0),
                });
            }
        }
    }

    // 3. Root fallback: if the disc has files but nothing matched, report
    //    the volume itself as a single anonymous title.
    if games.is_empty() {
        let root = fs.list_dir("")?;
        if !root.is_empty() {
            games.push(ScannedGame {
                metadata: GameMetadata {
                    title_name: fs.metadata().volume_label.clone().into(),
                    payload_path: Some(String::new()),
                    ..Default::default()
                },
                payload_path: String::new(),
                size_bytes: dir_size(fs, ""),
                file_count: root.len() as u32,
            });
        }
    }

    // Sort multi-disc titles together: title id, then disc number.
    games.sort_by(|a, b| {
        a.metadata
            .title_id
            .cmp(&b.metadata.title_id)
            .then(a.metadata.disc_number.cmp(&b.metadata.disc_number))
    });
    Ok(games)
}

/// Recursively look for config files up to `MAX_SCAN_DEPTH`.
fn scan_for_configs(
    fs: &mut dyn DiscFs,
    dir: &str,
    depth: u32,
    games: &mut Vec<ScannedGame>,
) -> io::Result<()> {
    if depth > MAX_SCAN_DEPTH {
        return Ok(());
    }
    let entries = fs.list_dir(dir)?;
    for entry in &entries {
        let child = join(dir, &entry.name);
        if entry.is_dir {
            scan_for_configs(fs, &child, depth + 1, games)?;
        } else if is_config_name(&entry.name) && is_xml_or_xex(&entry.name) {
            if let Ok(bytes) = fs.read_file(&child) {
                let text = String::from_utf8_lossy(&bytes);
                let payload = parent_dir(&child);
                let mut meta = parse_game_config(&text, &payload);
                if meta.title_name.is_none() {
                    meta.title_name = Some(humanize(&entry.name));
                }
                games.push(ScannedGame {
                    size_bytes: dir_size(fs, &payload),
                    file_count: count_files(fs, &payload, 0),
                    payload_path: payload,
                    metadata: meta,
                });
            }
        }
    }
    Ok(())
}

fn is_config_name(name: &str) -> bool {
    CONFIG_CANDIDATES
        .iter()
        .any(|c| name.eq_ignore_ascii_case(c))
}

fn is_xml_or_xex(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.ends_with(".xml") || lower.ends_with(".xex")
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", dir.trim_end_matches('/'), name)
    }
}

fn parent_dir(path: &str) -> String {
    path.rfind('/')
        .map(|i| path[..i].to_string())
        .unwrap_or_default()
}

fn humanize(name: &str) -> String {
    name.trim_end_matches(".xml")
        .trim_end_matches(".XML")
        .replace('_', " ")
        .to_string()
}

/// Sum file sizes under `dir` (bounded recursion, 4 levels).
fn dir_size(fs: &mut dyn DiscFs, dir: &str) -> u64 {
    let mut total = 0u64;
    accumulate_size(fs, dir, 0, &mut total);
    total
}

fn accumulate_size(fs: &mut dyn DiscFs, dir: &str, depth: u32, total: &mut u64) {
    if depth > 4 {
        return;
    }
    if let Ok(entries) = fs.list_dir(dir) {
        for e in entries {
            if e.is_dir {
                accumulate_size(fs, &join(dir, &e.name), depth + 1, total);
            } else {
                *total += e.size as u64;
            }
        }
    }
}

/// Count files under `dir` (bounded recursion).
fn count_files(fs: &mut dyn DiscFs, dir: &str, depth: u32) -> u32 {
    if depth > 4 {
        return 0;
    }
    let mut n = 0u32;
    if let Ok(entries) = fs.list_dir(dir) {
        for e in entries {
            if e.is_dir {
                n += count_files(fs, &join(dir, &e.name), depth + 1);
            } else {
                n += 1;
            }
        }
    }
    n
}

/// Group scanned games into multi-disc titles for display.
pub fn group_multidisc(games: &[ScannedGame]) -> Vec<Vec<&ScannedGame>> {
    let mut groups: Vec<Vec<&ScannedGame>> = Vec::new();
    for g in games {
        let key = g
            .metadata
            .title_id
            .clone()
            .unwrap_or_else(|| g.metadata.title_name.clone().unwrap_or_default());
        if let Some(group) = groups.iter_mut().find(|grp| {
            grp.first()
                .map(|f| {
                    f.metadata
                        .title_id
                        .clone()
                        .unwrap_or_else(|| f.metadata.title_name.clone().unwrap_or_default())
                        == key
                })
                .unwrap_or(false)
        }) {
            group.push(g);
        } else {
            groups.push(vec![g]);
        }
    }
    groups
}
