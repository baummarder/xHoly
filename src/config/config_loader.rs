//! Config loader — precedence: CLI overrides > file > defaults.
//!
//! Produces a merged `serde_json::Value` which callers deserialize
//! into [`super::settings::Config`] and/or [`super::infra::InfraConfig`].

use serde_json::Value;

use super::config_file;
use super::merge::{merge_layers, toml_to_value};

/// CLI-style flat overrides (key path → JSON value).
/// e.g. `[("save.auto_save", json!(false))]`.
pub type CliOverrides = Vec<(String, Value)>;

/// Load the effective config: defaults ← file ← CLI.
///
/// `defaults_toml` is the built-in baseline (usually `include_str!` of
/// a shipped `default.toml`); `file` is the user config path (may not
/// exist); `cli` is the override list.
pub fn load_effective(
    defaults_toml: &str,
    file: Option<&std::path::Path>,
    cli: &CliOverrides,
) -> Result<Value, String> {
    let defaults = toml_to_value(defaults_toml).map_err(|e| e.to_string())?;

    let file_value = match file {
        Some(p) if p.exists() => Some(config_file::load_file(p).map_err(|e| e.to_string())?),
        _ => None,
    };

    let mut cli_obj = serde_json::Map::new();
    for (path, v) in cli {
        set_path(&mut cli_obj, path, v.clone());
    }
    let cli_value = Value::Object(cli_obj);

    let layers: Vec<&Value> = [Some(&defaults), file_value.as_ref(), Some(&cli_value)]
        .into_iter()
        .flatten()
        .collect();
    Ok(merge_layers(layers))
}

/// Set `a.b.c = v` inside a JSON object tree.
fn set_path(obj: &mut serde_json::Map<String, Value>, path: &str, v: Value) {
    let mut parts = path.split('.').peekable();
    let mut cur = obj;
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            cur.insert(part.to_string(), v);
            return;
        }
        let entry = cur
            .entry(part.to_string())
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if !entry.is_object() {
            *entry = Value::Object(serde_json::Map::new());
        }
        cur = entry.as_object_mut().unwrap();
    }
}

/// Deserialize the merged value into `T`.
pub fn into<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, serde_json::Error> {
    serde_json::from_value(v.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn precedence_cli_file_default() {
        let dir = std::env::temp_dir().join("xholy_cfgload");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.toml");
        std::fs::write(&f, "[save]\nauto_save = false\n").unwrap();

        let v = load_effective(
            "[save]\nauto_save = true\ncompress = true\n",
            Some(&f),
            &vec![("save.compress".to_string(), json!(false))],
        )
        .unwrap();
        // file overrode default, CLI overrode file
        assert_eq!(v["save"]["auto_save"], json!(false));
        assert_eq!(v["save"]["compress"], json!(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_path_builds_tree() {
        let mut m = serde_json::Map::new();
        set_path(&mut m, "a.b.c", json!(1));
        assert_eq!(m["a"]["b"]["c"], json!(1));
    }
}
