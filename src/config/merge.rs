//! Config merge — overlay precedence: CLI > file > defaults.
//!
//! Works on `serde_json::Value` trees so it applies uniformly to TOML
//! and JSON sources (both deserialize into `Value` losslessly enough
//! for our flat-ish config shapes).

use serde::de::Error as _;
use serde_json::Value;

/// Deep-merge `overlay` onto `base`: objects merge recursively,
/// scalars/arrays in overlay replace base values.
pub fn deep_merge(base: &mut Value, overlay: &Value) {
    match (base, overlay) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(bv) => deep_merge(bv, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Merge a list of layers left→right (later wins).
pub fn merge_layers<'a>(layers: impl IntoIterator<Item = &'a Value>) -> Value {
    let mut out = Value::Object(serde_json::Map::new());
    for l in layers {
        deep_merge(&mut out, l);
    }
    out
}

/// Parse TOML text into a mergeable `Value`.
pub fn toml_to_value(text: &str) -> Result<Value, toml::de::Error> {
    let v: toml::Value = toml::from_str(text)?;
    serde_json::to_value(v).map_err(|e| {
        // serde_json errors can't become toml::de::Error; wrap via custom
        // path — in practice toml::Value→json::Value is infallible for
        // our shapes, so unwrap_or_default keeps this total.
        let _ = e;
        toml::de::Error::custom("json conversion failed")
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scalar_overlay_wins() {
        let mut base = json!({"a": 1, "b": {"c": 2}});
        deep_merge(&mut base, &json!({"a": 9, "b": {"c": 3}}));
        assert_eq!(base, json!({"a": 9, "b": {"c": 3}}));
    }

    #[test]
    fn merge_layers_order() {
        let v = merge_layers([&json!({"x": 1, "y": 1}), &json!({"x": 2}), &json!({"z": 3})]);
        assert_eq!(v, json!({"x": 2, "y": 1, "z": 3}));
    }

    #[test]
    fn toml_parses() {
        let v = toml_to_value("[save]\nauto_save = true\n").unwrap();
        assert_eq!(v["save"]["auto_save"], json!(true));
    }
}
