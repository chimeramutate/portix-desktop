use std::{fs, io, path::Path};

use serde::Deserialize;

/// The slice of a saved SSH profile the UI shows. The file is shared with the
/// Flutter app (`~/.portix/profiles.json`), so unknown fields are ignored.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default = "default_group")]
    pub group: String,
    /// Profile color name (`green`, `cyan`, `blue`, `pink`, `amber`).
    #[serde(default)]
    pub color: String,
    #[serde(default)]
    pub host: String,
    /// `u32` so one out-of-range port fails only that profile in `ssh.rs`,
    /// not the load of the whole file.
    #[serde(default)]
    pub port: Option<u32>,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub jump_profile_id: Option<String>,
    #[serde(default)]
    pub forward_agent: bool,
    // Task 8 reads this for the terminal.
    #[cfg_attr(not(test), allow(dead_code))]
    #[serde(default)]
    pub terminal_font_size: Option<f32>,
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

    #[test]
    fn loads_real_shape() {
        let path = std::env::temp_dir().join(format!("portix-shape-{}.json", std::process::id()));
        fs::write(
            &path,
            r#"[{"id":"a","name":"N","group":"G","color":"blue","host":"10.0.0.1","port":22,
            "username":"ops","authMethod":"key","credentialLabel":"x","jumpProfileId":null,
            "forwardAgent":false,"terminalFontSize":13.5,"defaultPath":"/","startupCommand":"",
            "tags":["t"],"status":"ok"},
            {"id":"b","name":"M","jumpProfileId":"a"}]"#,
        )
        .unwrap();
        let p = load(&path).unwrap();
        fs::remove_file(&path).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].port, Some(22));
        assert_eq!(p[0].terminal_font_size, Some(13.5));
        assert_eq!(p[1].jump_profile_id.as_deref(), Some("a"));
    }
}
