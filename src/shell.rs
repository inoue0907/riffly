//! Telling Windows about Riffly, for the current user only (no admin rights):
//!
//! - "Open in Riffly" on the right-click menu of Markdown, HTML and PDF files,
//! - Riffly in the "Open with" list for them,
//! - Riffly as a choice on Settings › Apps › Default apps.
//!
//! Windows does not let an app make itself the default; the person picks it
//! in Settings (or "Open with" › "Always"). [`open_default_apps_settings`]
//! takes them straight to Riffly's page there.
//!
//! Run `Riffly.exe --register` once (again after moving the exe) and
//! `Riffly.exe --unregister` to take it all back out.

use std::path::Path;

/// What Riffly opens, as (extension, ProgID, description).
pub const FILE_TYPES: &[(&str, &str, &str)] = &[
    (".md", "Riffly.Markdown", "Markdown document"),
    (".markdown", "Riffly.Markdown", "Markdown document"),
    (".html", "Riffly.Html", "HTML document"),
    (".htm", "Riffly.Html", "HTML document"),
    (".pdf", "Riffly.Pdf", "PDF document"),
];

/// The icon a ProgID's files get: a resource id in the executable (see
/// `build.rs`), a page saying what kind of file it is with Riffly's badge in
/// the corner. Id 2, the plain page, is for files opened with "Open with".
pub fn file_icon_id(progid: &str) -> u32 {
    match progid {
        "Riffly.Markdown" => 3,
        "Riffly.Html" => 4,
        "Riffly.Pdf" => 5,
        _ => GENERIC_FILE_ICON,
    }
}

pub const GENERIC_FILE_ICON: u32 = 2;

/// The registry root everything is written under, below HKEY_CURRENT_USER.
pub const ROOT: &str = "Software";

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use anyhow::{Result, bail};
    use windows::Win32::Globalization::GetUserDefaultUILanguage;
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RRF_RT_REG_SZ,
        RegCloseKey, RegCreateKeyExW, RegDeleteKeyValueW, RegDeleteKeyW, RegDeleteTreeW,
        RegGetValueW,
        RegSetValueExW,
    };
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows::core::{HSTRING, PCWSTR};

    use super::FILE_TYPES;

    /// Set a string value; `name` "" is the key's default value.
    fn set(key: &str, name: &str, value: &str) -> Result<()> {
        let mut handle = HKEY::default();
        // SAFETY: plain registry calls on HKCU with owned, NUL-terminated
        // strings; the handle is closed before returning.
        unsafe {
            let status = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                None,
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE | KEY_READ,
                None,
                &mut handle,
                None,
            );
            if status.is_err() {
                bail!("could not create HKCU\\{key}: {status:?}");
            }
            let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
            let bytes = std::slice::from_raw_parts(wide.as_ptr().cast::<u8>(), wide.len() * 2);
            let status = RegSetValueExW(handle, &HSTRING::from(name), None, REG_SZ, Some(bytes));
            let _ = RegCloseKey(handle);
            if status.is_err() {
                bail!("could not write HKCU\\{key}\\{name}: {status:?}");
            }
        }
        Ok(())
    }

    /// Read a string value, if it is there.
    pub fn get(key: &str, name: &str) -> Option<String> {
        let mut buffer = vec![0u16; 2048];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: the buffer and its size describe the same allocation.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                &HSTRING::from(key),
                &HSTRING::from(name),
                RRF_RT_REG_SZ,
                None,
                Some(buffer.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if status.is_err() {
            return None;
        }
        let len = (size as usize / 2).saturating_sub(1);
        Some(String::from_utf16_lossy(&buffer[..len]))
    }

    /// Remove a key and everything under it; a missing key is fine.
    fn delete_tree(key: &str) {
        let key = HSTRING::from(key);
        // SAFETY: deleting HKCU subkeys by path. RegDeleteTreeW empties the
        // key, RegDeleteKeyW then removes the key itself.
        unsafe {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, &key);
            let _ = RegDeleteKeyW(HKEY_CURRENT_USER, &key);
        }
    }

    /// Remove one value; a missing one is fine.
    fn delete_value(key: &str, name: &str) {
        // SAFETY: deleting one named value of an HKCU key.
        let _ = unsafe {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, &HSTRING::from(key), &HSTRING::from(name))
        };
    }

    /// "Riffly で開く" on a Japanese Windows, "Open in Riffly" elsewhere.
    pub fn verb_label() -> &'static str {
        // SAFETY: no arguments, returns the UI language id.
        let language = unsafe { GetUserDefaultUILanguage() };
        if language & 0x3ff == 0x11 { "Riffly で開く" } else { "Open in Riffly" }
    }

    fn notify_shell() {
        // SAFETY: a broadcast with no items.
        unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    }

    pub fn register_under(root: &str, exe: &Path) -> Result<()> {
        let exe = std::path::absolute(exe)?;
        let exe = exe.to_string_lossy();
        let command = format!("\"{exe}\" \"%1\"");
        // The app's own icon, and a file icon by resource id (negative).
        let icon = format!("\"{exe}\",0");
        let file_icon = |id: u32| format!("\"{exe}\",-{id}");
        let classes = format!("{root}\\Classes");

        // One ProgID per kind of file: its name, icon and how it opens.
        for (_, progid, description) in FILE_TYPES {
            let key = format!("{classes}\\{progid}");
            set(&key, "", description)?;
            set(&key, "FriendlyTypeName", description)?;
            set(&format!("{key}\\DefaultIcon"), "", &file_icon(super::file_icon_id(progid)))?;
            set(&format!("{key}\\shell\\open\\command"), "", &command)?;
        }

        for (ext, progid, _) in FILE_TYPES {
            // "Open with" lists Riffly for the extension…
            set(&format!("{classes}\\{ext}\\OpenWithProgids"), progid, "")?;
            // …and the right-click menu offers it outright.
            let verb = format!("{classes}\\SystemFileAssociations\\{ext}\\shell\\Riffly");
            set(&verb, "", verb_label())?;
            set(&verb, "MUIVerb", verb_label())?;
            set(&verb, "Icon", &icon)?;
            set(&format!("{verb}\\command"), "", &command)?;
        }

        // The application itself, as "Open with" and Default apps see it.
        let app = format!("{classes}\\Applications\\Riffly.exe");
        set(&app, "FriendlyAppName", "Riffly")?;
        set(&format!("{app}\\DefaultIcon"), "", &file_icon(super::GENERIC_FILE_ICON))?;
        set(&format!("{app}\\shell\\open\\command"), "", &command)?;
        for (ext, _, _) in FILE_TYPES {
            set(&format!("{app}\\SupportedTypes"), ext, "")?;
        }

        let capabilities = format!("{root}\\Riffly\\Capabilities");
        set(&capabilities, "ApplicationName", "Riffly")?;
        set(
            &capabilities,
            "ApplicationDescription",
            "Read, review and fix Markdown, HTML and PDF files",
        )?;
        set(&capabilities, "ApplicationIcon", &icon)?;
        for (ext, progid, _) in FILE_TYPES {
            set(&format!("{capabilities}\\FileAssociations"), ext, progid)?;
        }
        set(&format!("{root}\\RegisteredApplications"), "Riffly", &format!("{root}\\Riffly\\Capabilities"))?;

        notify_shell();
        Ok(())
    }

    pub fn unregister_under(root: &str) -> Result<()> {
        let classes = format!("{root}\\Classes");
        let mut progids: Vec<&str> = FILE_TYPES.iter().map(|(_, p, _)| *p).collect();
        progids.dedup();
        for progid in progids {
            delete_tree(&format!("{classes}\\{progid}"));
        }
        for (ext, progid, _) in FILE_TYPES {
            delete_value(&format!("{classes}\\{ext}\\OpenWithProgids"), progid);
            delete_tree(&format!("{classes}\\SystemFileAssociations\\{ext}\\shell\\Riffly"));
        }
        delete_tree(&format!("{classes}\\Applications\\Riffly.exe"));
        delete_tree(&format!("{root}\\Riffly"));
        delete_value(&format!("{root}\\RegisteredApplications"), "Riffly");
        notify_shell();
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use anyhow::{Result, bail};

    pub fn register_under(_: &str, _: &Path) -> Result<()> {
        bail!("registering file types is only needed on Windows")
    }

    pub fn unregister_under(_: &str) -> Result<()> {
        Ok(())
    }

    pub fn get(_: &str, _: &str) -> Option<String> {
        None
    }

    pub fn verb_label() -> &'static str {
        "Open in Riffly"
    }
}

pub use imp::{get as registry_value, register_under, unregister_under, verb_label};

/// Register `exe` for the current user.
pub fn register(exe: &Path) -> anyhow::Result<()> {
    register_under(ROOT, exe)
}

/// Take every registration back out.
pub fn unregister() -> anyhow::Result<()> {
    unregister_under(ROOT)
}

/// Open Settings › Default apps on Riffly's own page.
pub fn open_default_apps_settings() {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg("ms-settings:defaultapps?registeredAppUser=Riffly")
            .spawn();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn registers_and_unregisters_under_a_scratch_key() {
        let root = format!("Software\\RifflyTest-{}", std::process::id());
        let exe = std::path::PathBuf::from(r"C:\Program Files\Riffly Test\Riffly.exe");
        register_under(&root, &exe).unwrap();

        let classes = format!("{root}\\Classes");
        let command = format!("\"{}\" \"%1\"", exe.display());
        assert_eq!(
            registry_value(&format!("{classes}\\Riffly.Markdown\\shell\\open\\command"), "").as_deref(),
            Some(command.as_str())
        );
        assert_eq!(
            registry_value(&format!("{classes}\\SystemFileAssociations\\.pdf\\shell\\Riffly"), "MUIVerb")
                .as_deref(),
            Some(verb_label())
        );
        assert_eq!(registry_value(&format!("{classes}\\.html\\OpenWithProgids"), "Riffly.Html").as_deref(), Some(""));
        // Files get a document icon from the exe, by resource id.
        assert_eq!(
            registry_value(&format!("{classes}\\Riffly.Pdf\\DefaultIcon"), "").as_deref(),
            Some(format!("\"{}\",-5", exe.display()).as_str())
        );
        assert_eq!(
            registry_value(&format!("{root}\\Riffly\\Capabilities\\FileAssociations"), ".md").as_deref(),
            Some("Riffly.Markdown")
        );
        assert_eq!(
            registry_value(&format!("{root}\\RegisteredApplications"), "Riffly").as_deref(),
            Some(format!("{root}\\Riffly\\Capabilities").as_str())
        );

        unregister_under(&root).unwrap();
        assert_eq!(registry_value(&format!("{classes}\\Riffly.Markdown\\shell\\open\\command"), ""), None);
        assert_eq!(registry_value(&format!("{classes}\\.html\\OpenWithProgids"), "Riffly.Html"), None);
        assert_eq!(registry_value(&format!("{root}\\RegisteredApplications"), "Riffly"), None);

        // Leave nothing of the scratch root behind.
        let _ = std::process::Command::new("reg")
            .args(["delete", &format!("HKCU\\{root}"), "/f"])
            .output();
    }
}
