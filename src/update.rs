//! Updating Riffly in place from its GitHub releases.
//!
//! Riffly ships as a zip you unzip and run, so there is no installer to ask
//! about new versions. The sidebar's update button does that instead: it
//! asks GitHub for the latest release, compares it with the version built
//! into this executable, and — once the person says yes — downloads the
//! release zip, takes `Riffly.exe` out of it and puts it where this
//! executable stands.
//!
//! A running executable cannot be written over, but it can be renamed: the
//! old one is moved aside to `<name>.old` and the new one takes its place.
//! The next start deletes what was moved aside ([`clean_up`]).

use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The repository releases are published from.
const REPO: &str = "inoue0907/riffly";

/// What this executable was built as.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Riffly's own name in requests, as GitHub's API asks of every caller.
fn user_agent() -> String {
    format!("riffly/{}", current_version())
}

/// A release newer than this build, and the zip to fetch it from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The version without the tag's leading `v`, such as `0.1.6`.
    pub version: String,
    /// The release zip holding `Riffly.exe`.
    pub download_url: String,
    /// The release's page, for the person who would rather read first.
    pub page_url: String,
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    /// This build is the latest release (or newer than it).
    UpToDate,
    /// A newer release is published.
    Newer(Release),
}

/// Ask GitHub for the latest release. Blocking: call it off the main thread.
pub fn check() -> Result<Check, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = get(&url, "application/vnd.github+json", 4 * 1024 * 1024)?;
    let json: serde_json::Value = serde_json::from_slice(&body)
        .map_err(|err| format!("GitHub's answer could not be read ({err})"))?;
    let release = parse_release(&json)?;
    Ok(if is_newer(&release.version, current_version()) {
        Check::Newer(release)
    } else {
        Check::UpToDate
    })
}

/// Pick the version and the Windows zip out of a release's JSON.
fn parse_release(json: &serde_json::Value) -> Result<Release, String> {
    let tag = json["tag_name"]
        .as_str()
        .ok_or("the latest release has no tag")?;
    let version = tag.trim_start_matches('v').to_string();
    let page_url = json["html_url"].as_str().unwrap_or_default().to_string();
    let download_url = json["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|asset| {
            let name = asset["name"].as_str()?.to_ascii_lowercase();
            (name.ends_with(".zip") && name.contains("windows"))
                .then(|| asset["browser_download_url"].as_str())
                .flatten()
        })
        .ok_or_else(|| format!("release {tag} has no Windows zip to download"))?
        .to_string();
    Ok(Release {
        version,
        download_url,
        page_url,
    })
}

/// Whether `candidate` is a later version than `current`. Versions are
/// compared number by number, so 0.1.10 comes after 0.1.9; anything that is
/// not a number sorts before one, which keeps a pre-release such as
/// `0.2.0-rc1` behind `0.2.0`.
fn is_newer(candidate: &str, current: &str) -> bool {
    let parts = |version: &str| -> Vec<i64> {
        version
            .split(['.', '-', '+'])
            .map(|part| part.parse::<i64>().unwrap_or(-1))
            .collect()
    };
    parts(candidate) > parts(current)
}

/// Download the release and put its `Riffly.exe` where this executable
/// stands. Blocking: call it off the main thread. Returns the path the new
/// executable now has (this executable's own path).
pub fn install(release: &Release) -> Result<PathBuf, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Riffly cannot tell where it is installed ({err})"))?;
    // 200 MB: the zip holds one stripped executable, far below that.
    let zip = get(&release.download_url, "application/octet-stream", 200 * 1024 * 1024)?;
    let bytes = executable_in(&zip)?;
    swap_in(&exe, &bytes)?;
    crate::log::write(format!(
        "updated {} to {} at {exe:?}",
        current_version(),
        release.version
    ));
    Ok(exe)
}

/// The bytes of `Riffly.exe` inside a release zip.
fn executable_in(zip: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip))
        .map_err(|err| format!("the downloaded zip is damaged ({err})"))?;
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|err| format!("the downloaded zip is damaged ({err})"))?;
        let name = entry
            .name()
            .map_err(|err| format!("the downloaded zip is damaged ({err})"))?
            .to_ascii_lowercase();
        if !name.ends_with("riffly.exe") {
            continue;
        }
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry
            .read_to_end(&mut bytes)
            .map_err(|err| format!("the new Riffly could not be unpacked ({err})"))?;
        return Ok(bytes);
    }
    Err("the downloaded zip holds no Riffly.exe".into())
}

/// Put `bytes` at `exe`, moving what is there now aside. The running
/// executable is the one being replaced, so it is renamed rather than
/// deleted; Windows allows that while it runs.
fn swap_in(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let fresh = beside(exe, "new");
    let aside = beside(exe, "old");
    let _ = std::fs::remove_file(&fresh);
    let _ = std::fs::remove_file(&aside);
    std::fs::write(&fresh, bytes)
        .map_err(|err| format!("the new Riffly could not be written to {fresh:?} ({err})"))?;

    if let Err(err) = std::fs::rename(exe, &aside) {
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("{exe:?} could not be moved aside ({err})"));
    }
    if let Err(err) = std::fs::rename(&fresh, exe) {
        // Put the running executable back, so Riffly is still there.
        let _ = std::fs::rename(&aside, exe);
        let _ = std::fs::remove_file(&fresh);
        return Err(format!("the new Riffly could not be put in place ({err})"));
    }
    Ok(())
}

/// `riffly.exe` → `riffly.exe.old`, kept beside the executable.
fn beside(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(suffix);
    exe.with_file_name(name)
}

/// Delete the executable an earlier update moved aside. Called at start,
/// when the file is no longer running and can go.
pub fn clean_up() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let aside = beside(&exe, "old");
    if aside.exists() && std::fs::remove_file(&aside).is_ok() {
        crate::log::write(format!("removed the replaced executable {aside:?}"));
    }
}

/// Start the executable at `exe` afresh, with `files` to open. The caller
/// quits once this returns, leaving the new process in its place.
pub fn restart(exe: &Path, files: &[PathBuf]) -> Result<(), String> {
    std::process::Command::new(exe)
        .args(files)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("the new Riffly could not be started ({err})"))
}

/// Fetch a URL, with a ceiling on how much is read.
fn get(url: &str, accept: &str, limit: u64) -> Result<Vec<u8>, String> {
    let mut response = ureq::get(url)
        .header("Accept", accept)
        .header("User-Agent", user_agent())
        .call()
        .map_err(|err| format!("GitHub could not be reached ({err})"))?;
    response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|err| format!("the download stopped early ({err})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert!(is_newer("0.1.6", "0.1.5"));
        assert!(is_newer("0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.99"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.5", "0.1.5"));
        assert!(!is_newer("0.1.4", "0.1.5"));
        // A pre-release is behind the release it leads to.
        assert!(!is_newer("0.2.0-rc1", "0.2.0"));
        assert!(is_newer("0.2.0", "0.2.0-rc1"));
    }

    #[test]
    fn the_windows_zip_is_picked_out_of_a_release() {
        let json = serde_json::json!({
            "tag_name": "v0.1.6",
            "html_url": "https://github.com/inoue0907/riffly/releases/tag/v0.1.6",
            "assets": [
                { "name": "Riffly-0.1.6-windows-x64.zip",
                  "browser_download_url": "https://example.test/Riffly-0.1.6-windows-x64.zip" },
                { "name": "notes.txt", "browser_download_url": "https://example.test/notes.txt" },
            ],
        });
        let release = parse_release(&json).expect("a release");
        assert_eq!(release.version, "0.1.6");
        assert_eq!(
            release.download_url,
            "https://example.test/Riffly-0.1.6-windows-x64.zip"
        );
    }

    #[test]
    fn a_release_without_a_zip_is_an_error() {
        let json = serde_json::json!({
            "tag_name": "v0.1.6",
            "assets": [{ "name": "notes.txt", "browser_download_url": "https://example.test/n" }],
        });
        assert!(parse_release(&json).is_err());
    }

    #[test]
    fn the_replaced_executable_is_kept_beside_it() {
        let exe = Path::new("C:/apps/riffly/Riffly.exe");
        assert_eq!(beside(exe, "old"), Path::new("C:/apps/riffly/Riffly.exe.old"));
    }

    #[test]
    fn an_executable_is_taken_out_of_a_zip() {
        let mut buffer = Vec::new();
        {
            use std::io::Write as _;
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buffer));
            let options: zip::write::FileOptions<'_, '_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            writer.start_file("README.md", options).expect("entry");
            writer.write_all(b"not the program").expect("write");
            writer.start_file("Riffly.exe", options).expect("entry");
            writer.write_all(b"MZ the program").expect("write");
            writer.finish().expect("zip");
        }
        assert_eq!(executable_in(&buffer).expect("an exe"), b"MZ the program");
    }
}
