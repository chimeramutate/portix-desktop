use std::{fs, io, path::Path};

use serde::Deserialize;

/// The slice of a saved SSH profile the UI shows. The file is shared with the
/// Flutter app (`~/.portix/profiles.json`), so unknown fields are ignored.
#[derive(Debug, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default = "default_group")]
    pub group: String,
    /// Profile color name (`green`, `cyan`, `blue`, `pink`, `amber`).
    #[serde(default)]
    pub color: String,
}

/// The Flutter app's default group.
fn default_group() -> String {
    "Production".into()
}

/// A missing file means no profiles yet; any other failure is reported, not
/// swallowed, so a broken file never looks like an empty one.
pub fn load(path: &Path) -> Result<Vec<Profile>, String> {
    match fs::read_to_string(path) {
        Ok(json) => serde_json::from_str(&json).map_err(|e| e.to_string()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_handles_missing_valid_and_broken_files() {
        let path = std::env::temp_dir().join(format!("portix-ui-{}.json", std::process::id()));

        let _ = fs::remove_file(&path);
        assert!(load(&path).unwrap().is_empty());

        fs::write(&path, r#"[{"id":"a","name":"Prod","host":"10.0.0.1","port":22}]"#).unwrap();
        let profiles = load(&path).unwrap();
        assert_eq!(profiles[0].name, "Prod");

        fs::write(&path, "{not json").unwrap();
        assert!(load(&path).is_err());

        fs::remove_file(&path).unwrap();
    }
}
