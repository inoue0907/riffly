//! Preferences that outlive the window, kept as `settings.json` next to the
//! recent-files list.

use std::path::Path;

use gpui_kit::{Bounds, Pixels, point, px, size};

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
    /// Where the window was when it was last closed.
    #[serde(default)]
    pub window: Option<Placement>,
    /// The welcome document has been shown. A settings file written before
    /// this was kept means Riffly has run before, so it counts as shown.
    #[serde(default = "already_welcomed")]
    pub welcomed: bool,
}

fn already_welcomed() -> bool {
    true
}

/// A window's place on screen, in logical pixels. For a maximized window it
/// is the size it goes back to when restored.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    #[serde(default)]
    pub maximized: bool,
    /// The same place as Windows keeps it, which comes back right on any
    /// monitor whatever its scale; preferred over the fields above.
    #[serde(default)]
    pub native: Option<crate::placement::NativePlacement>,
}

/// How much of `a` lies inside `b`, as an area.
fn overlap(a: Bounds<Pixels>, b: Bounds<Pixels>) -> f32 {
    let width = f32::from(a.right().min(b.right()) - a.left().max(b.left()));
    let height = f32::from(a.bottom().min(b.bottom()) - a.top().max(b.top()));
    if width <= 0. || height <= 0. { 0. } else { width * height }
}

impl Placement {
    /// The place, moved and shrunk as needed to sit wholly on the screen it
    /// is mostly on (`screens` are their usable areas). None when it is on
    /// no screen any more — a monitor that was unplugged, say — or too small
    /// to be a real window.
    pub fn fit(&self, screens: &[Bounds<Pixels>]) -> Option<Bounds<Pixels>> {
        if self.width < 320. || self.height < 240. {
            return None;
        }
        let saved = Bounds::new(point(px(self.x), px(self.y)), size(px(self.width), px(self.height)));
        let area = screens
            .iter()
            .copied()
            .max_by(|a, b| overlap(*a, saved).total_cmp(&overlap(*b, saved)))
            .filter(|area| overlap(*area, saved) > 0.)?;
        let width = saved.size.width.min(area.size.width);
        let height = saved.size.height.min(area.size.height);
        let x = saved.origin.x.max(area.left()).min(area.right() - width);
        let y = saved.origin.y.max(area.top()).min(area.bottom() - height);
        Some(Bounds::new(point(x, y), size(width, height)))
    }
}

impl Settings {
    /// Settings read from `dir`, or the defaults when there are none.
    pub fn load(dir: Option<&Path>) -> Self {
        dir.and_then(|dir| std::fs::read_to_string(dir.join(FILE)).ok())
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write the settings to `dir`, returning the file written.
    pub fn save(&self, dir: Option<&Path>) -> std::io::Result<std::path::PathBuf> {
        let Some(dir) = dir else {
            return Err(std::io::Error::other("there is no settings folder"));
        };
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let path = dir.join(FILE);
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(w), px(h)))
    }

    fn place(x: f32, y: f32, width: f32, height: f32) -> Placement {
        Placement { x, y, width, height, maximized: false, native: None }
    }

    /// Three screens like a laptop with two monitors above it.
    fn screens() -> Vec<Bounds<Pixels>> {
        vec![
            screen(0., 0., 1280., 752.),
            screen(-1899., -1440., 2560., 1392.),
            screen(661., -1440., 2560., 1392.),
        ]
    }

    #[test]
    fn a_window_that_fits_stays_put() {
        let fitted = place(1000., -1200., 1100., 800.).fit(&screens()).unwrap();
        assert_eq!(fitted, screen(1000., -1200., 1100., 800.));
    }

    #[test]
    fn a_window_hanging_off_a_screen_is_pulled_onto_it() {
        // Taller than the laptop screen and above its top edge.
        let fitted = place(90., -54., 1100., 860.).fit(&screens()).unwrap();
        assert_eq!(fitted, screen(90., 0., 1100., 752.));
    }

    #[test]
    fn a_window_on_an_unplugged_monitor_is_not_restored() {
        assert_eq!(place(5000., 3000., 900., 700.).fit(&screens()), None);
        assert_eq!(place(10., 10., 100., 100.).fit(&screens()), None, "too small");
    }

    #[test]
    fn settings_from_before_the_welcome_flag_count_as_welcomed() {
        let old: Settings = serde_json::from_str(r#"{ "appearance": "dark" }"#).unwrap();
        assert!(old.welcomed);
        assert!(!Settings::default().welcomed, "a first start has no settings");
    }

    #[test]
    fn round_trips_the_appearance() {
        let dir = std::env::temp_dir().join(format!("riffly-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(Settings::load(Some(&dir)).appearance, None);

        let settings = Settings {
            appearance: Some(Appearance::Dark),
            window: Some(Placement {
                x: 120.,
                y: 80.,
                width: 1000.,
                height: 700.,
                maximized: true,
                native: Some(crate::placement::NativePlacement {
                    left: -1800,
                    top: -1300,
                    right: -600,
                    bottom: -500,
                    maximized: true,
                }),
            }),
            welcomed: true,
        };
        settings.save(Some(&dir)).unwrap();
        assert_eq!(Settings::load(Some(&dir)), settings);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
