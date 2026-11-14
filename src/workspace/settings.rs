//! Preferences that outlive the window, kept as `settings.json` next to the
//! recent-files list.

use std::path::Path;

use serde::{Deserialize, Serialize};

const FILE: &str = "settings.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Appearance {
    Light,
    Dark,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    /// The palette the reader picked; `None` follows the system.
    #[serde(default)]
    pub appearance: Option<Appearance>,
}

impl Settings {
    /// Settings read from `dir`, or the defaults when there are none.
    pub fn load(dir: Option<&Path>) -> Self {
        dir.and_then(|dir| std::fs::read_to_string(dir.join(FILE)).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: Option<&Path>) {
        let Some(dir) = dir else { return };
        let _ = std::fs::create_dir_all(dir);
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join(FILE), text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_the_appearance() {
        let dir = std::env::temp_dir().join(format!("riffly-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(Some(&dir)).appearance, None);

        let settings = Settings {
            appearance: Some(Appearance::Dark),
        };
        settings.save(Some(&dir));
        assert_eq!(Settings::load(Some(&dir)), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
