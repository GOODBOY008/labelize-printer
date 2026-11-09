//! Update discovery and verified full installers. No window or GPU dependency.
pub mod handoff;
pub mod platform;
use anyhow::{ensure, Context, Result};
use fs2::FileExt;
use labelize_printer_core::{paths::private_dir, AppPaths};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub const RELEASES: &str = "https://github.com/GOODBOY008/labelize-studio/releases";
const LEGACY_RELEASES: &str = "https://github.com/GOODBOY008/labelize-printer/releases";
pub const API: &str = "https://api.github.com/repos/GOODBOY008/labelize-studio/releases/latest";
pub const BREW_COMMAND: &str =
    "brew tap goodboy008/labelize-studio && brew update && brew upgrade --cask goodboy008/labelize-studio/labelize-studio";
const MAX_INSTALLER: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum InstallSource {
    Package,
    Homebrew,
    Unknown,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    MacArm,
    MacIntel,
    WindowsX64,
    LinuxX64,
}
impl Target {
    pub fn asset(&self, v: &str) -> String {
        // Historical releases keep their original assets after the repository rename.
        // The version boundary also prevents accepting legacy names for new releases.
        let legacy = Version::parse(v).is_ok_and(|v| v < Version::new(0, 4, 0));
        let product = if legacy {
            "LabelizePrinter"
        } else {
            "LabelizeStudio"
        };
        match self {
            Self::MacArm => format!("{product}-{v}-macos-arm64-release.pkg"),
            Self::MacIntel => format!("{product}-{v}-macos-x86_64-release.pkg"),
            Self::WindowsX64 => format!("{product}-{v}-windows-x64.exe"),
            Self::LinuxX64 => {
                let package = if legacy {
                    "labelize-printer"
                } else {
                    "labelize-studio"
                };
                format!("{package}_{v}_amd64.deb")
            }
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Asset {
    pub name: String,
    pub size: u64,
    pub browser_download_url: String,
    pub digest: Option<String>,
    #[serde(default)]
    pub state: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub body: Option<String>,
    pub draft: bool,
    pub prerelease: bool,
    pub assets: Vec<Asset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Offer {
    pub version: String,
    pub release_url: String,
    pub notes: String,
    pub target: Target,
    pub installer: Asset,
    pub sums: Asset,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verified {
    pub offer: Offer,
    pub path: PathBuf,
    pub sha256: String,
}

pub fn check_due(enabled: bool, last: Option<i64>, now: i64) -> bool {
    enabled && last.is_none_or(|last| now < last || now.saturating_sub(last) >= 24 * 60 * 60)
}
fn digest(asset: &Asset) -> Result<String> {
    let digest = asset
        .digest
        .as_deref()
        .and_then(|s| s.strip_prefix("sha256:"))
        .context("Release asset has no SHA256 digest")?;
    ensure!(
        digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA256 digest"
    );
    Ok(digest.to_ascii_lowercase())
}
fn asset_url(url: &str, tag: &str) -> Result<()> {
    let name = [RELEASES, LEGACY_RELEASES]
        .iter()
        .find_map(|base| url.strip_prefix(&format!("{base}/download/{tag}/")))
        .context("Asset does not belong to the selected official release")?;
    ensure!(
        !name.is_empty() && !name.contains(['/', '?', '#', '%', '\\']),
        "Invalid release asset URL"
    );
    Ok(())
}
pub fn select_release(release: Release, current: &str, target: Target) -> Result<Option<Offer>> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = Version::parse(
        release
            .tag_name
            .strip_prefix('v')
            .unwrap_or(&release.tag_name),
    )?;
    if !version.pre.is_empty() || version <= Version::parse(current)? {
        return Ok(None);
    }
    ensure!(
        [RELEASES, LEGACY_RELEASES]
            .iter()
            .any(|base| release.html_url == format!("{base}/tag/{}", release.tag_name)),
        "Unexpected release page"
    );
    let find = |name: &str| -> Result<Asset> {
        let matches: Vec<_> = release.assets.iter().filter(|a| a.name == name).collect();
        ensure!(
            matches.len() == 1,
            "Release must contain exactly one {name}"
        );
        let asset = matches[0].clone();
        ensure!(
            asset.state == "uploaded" && asset.size > 0 && asset.size <= MAX_INSTALLER,
            "Invalid release asset size or upload state"
        );
        digest(&asset)?;
        asset_url(&asset.browser_download_url, &release.tag_name)?;
        ensure!(
            asset.browser_download_url.ends_with(&format!("/{name}")),
            "Asset name does not match URL"
        );
        Ok(asset)
    };
    let installer = find(&target.asset(&version.to_string()))?;
    let sums = find("SHA256SUMS")?;
    ensure!(sums.size <= 64 * 1024, "Checksum manifest is too large");
    Ok(Some(Offer {
        version: version.to_string(),
        release_url: release.html_url,
        notes: release
            .body
            .unwrap_or_default()
            .chars()
            .take(16000)
            .collect(),
        target,
        installer,
        sums,
    }))
}
fn agent(total: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .max_redirects(5)
        .user_agent(concat!("LabelizeStudio/", env!("CARGO_PKG_VERSION")))
        .timeout_global(Some(total))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_resolve(Some(Duration::from_secs(10)))
        .timeout_per_read(Some(Duration::from_secs(5)))
        .build()
        .into()
}
pub fn check(current: &str, target: Target) -> Result<Option<Offer>> {
    fetch_release(&agent(Duration::from_secs(20)), API, current, target)
}
fn fetch_release(
    client: &ureq::Agent,
    url: &str,
    current: &str,
    target: Target,
) -> Result<Option<Offer>> {
    let mut response = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .context("Could not check GitHub Releases (offline, rate limited, or timed out)")?;
    let mut bytes = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(2 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 2 * 1024 * 1024,
        "Release response is too large"
    );
    select_release(serde_json::from_slice(&bytes)?, current, target)
}
pub fn manifest_digest(text: &str, name: &str) -> Result<String> {
    let mut result = None;
    for line in text.lines() {
        let Some((hash, file)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        if file.trim_start().trim_start_matches('*') != name {
            continue;
        }
        ensure!(result.is_none(), "Duplicate installer checksum");
        ensure!(
            hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid installer checksum"
        );
        result = Some(hash.to_ascii_lowercase());
    }
    result.context("Installer missing from SHA256SUMS")
}
fn cancelled(cancel: &AtomicBool) -> Result<()> {
    ensure!(!cancel.load(Ordering::Acquire), "Download cancelled");
    Ok(())
}
pub fn copy_verified(
    mut input: impl Read,
    mut output: impl Write,
    size: u64,
    hash: &str,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<()> {
    let mut bytes = [0; 65536];
    let mut total = 0;
    let mut sha = Sha256::new();
    loop {
        cancelled(cancel)?;
        let n = input.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        ensure!(
            total <= size && total <= MAX_INSTALLER,
            "Installer is larger than its release metadata"
        );
        sha.update(&bytes[..n]);
        output.write_all(&bytes[..n])?;
        progress(total, size);
    }
    cancelled(cancel)?;
    ensure!(
        total == size,
        "Incomplete download: expected {size} bytes, received {total}"
    );
    ensure!(
        format!("{:x}", sha.finalize()) == hash,
        "SHA256 mismatch; the installer cannot be used"
    );
    Ok(())
}
pub fn verify(file: &Path, size: u64, hash: &str) -> Result<()> {
    let info = fs::symlink_metadata(file)?;
    ensure!(
        info.is_file() && !info.file_type().is_symlink(),
        "Installer must be a regular file"
    );
    copy_verified(
        fs::File::open(file)?,
        std::io::sink(),
        size,
        hash,
        &AtomicBool::new(false),
        |_, _| {},
    )
}
fn cache_lock(paths: &AppPaths) -> Result<fs::File> {
    let root = paths.root.join("updates");
    private_dir(&root)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("download.lock"))?;
    lock.try_lock_exclusive()
        .context("Another Labelize Studio window is downloading an update")?;
    Ok(lock)
}
fn remove_partial_files(paths: &AppPaths) -> Result<()> {
    for directory in fs::read_dir(paths.root.join("updates"))? {
        let directory = directory?;
        if !directory.file_type()?.is_dir() {
            continue;
        }
        for file in fs::read_dir(directory.path())? {
            let file = file?;
            let path = file.path();
            if file.file_type()?.is_file()
                && path.extension().is_some_and(|e| e == "part")
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
            {
                fs::remove_file(path)?;
            }
        }
    }
    Ok(())
}
/// Remove only this updater's orphaned UUID.part files, never another active download.
pub fn cleanup_cache(paths: &AppPaths) -> Result<()> {
    let _lock = cache_lock(paths)?;
    remove_partial_files(paths)
}
pub fn download(
    paths: &AppPaths,
    offer: Offer,
    cancel: &AtomicBool,
    mut progress: impl FnMut(u64, u64),
) -> Result<Verified> {
    cancelled(cancel)?;
    let _lock = cache_lock(paths)?;
    remove_partial_files(paths)?;
    let mut response = agent(Duration::from_secs(20))
        .get(&offer.sums.browser_download_url)
        .call()?;
    let mut manifest = Vec::new();
    copy_verified(
        response.body_mut().as_reader(),
        &mut manifest,
        offer.sums.size,
        &digest(&offer.sums)?,
        cancel,
        |_, _| {},
    )?;
    let sha = manifest_digest(std::str::from_utf8(&manifest)?, &offer.installer.name)?;
    ensure!(
        sha == digest(&offer.installer)?,
        "SHA256SUMS and GitHub asset digest disagree"
    );
    let dir = paths.root.join("updates").join(&offer.version);
    private_dir(&dir)?;
    let destination = dir.join(&offer.installer.name);
    if destination.exists() && verify(&destination, offer.installer.size, &sha).is_ok() {
        cancelled(cancel)?;
        return Ok(Verified {
            offer,
            path: destination,
            sha256: sha,
        });
    }
    let partial = dir.join(format!("{}.part", uuid::Uuid::new_v4()));
    let result = (|| -> Result<()> {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&partial)?;
        labelize_printer_core::paths::private_file(&partial)?;
        let mut response = agent(Duration::from_secs(30 * 60))
            .get(&offer.installer.browser_download_url)
            .call()?;
        copy_verified(
            response.body_mut().as_reader(),
            &mut file,
            offer.installer.size,
            &sha,
            cancel,
            &mut progress,
        )?;
        file.sync_all()?;
        drop(file);
        cancelled(cancel)?;
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&partial, &destination)?;
        Ok(())
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(partial);
        return Err(e);
    }
    Ok(Verified {
        offer,
        path: destination,
        sha256: sha,
    })
}

/// Read installed cask inventory, never infer ownership from brew merely existing.
pub fn brew_installed(bytes: &[u8]) -> Result<bool> {
    let data: serde_json::Value = serde_json::from_slice(bytes)?;
    let casks = data
        .get("casks")
        .and_then(|v| v.as_array())
        .context("Invalid Homebrew inventory")?;
    Ok(casks.iter().any(|c| {
        matches!(
            c["token"].as_str(),
            Some("labelize-printer" | "labelize-studio")
        ) && c.get("installed").is_some_and(|v| match v {
            serde_json::Value::String(s) => !s.is_empty(),
            serde_json::Value::Array(a) => !a.is_empty(),
            _ => false,
        })
    }))
}

pub fn validate_verified(verified: &Verified) -> Result<()> {
    let v = Version::parse(&verified.offer.version)?;
    ensure!(
        v.pre.is_empty() && v > Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Installer is not a newer stable release"
    );
    ensure!(
        verified.offer.installer.name == verified.offer.target.asset(&verified.offer.version),
        "Unexpected installer name"
    );
    ensure!(
        digest(&verified.offer.installer)? == verified.sha256,
        "Installer digest changed"
    );
    verify(
        &verified.path,
        verified.offer.installer.size,
        &verified.sha256,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(v: &str) -> Release {
        let tag = format!("v{v}");
        Release {
            tag_name: tag.clone(),
            html_url: format!("{RELEASES}/tag/{tag}"),
            body: None,
            draft: false,
            prerelease: false,
            assets: [Target::MacArm.asset(v), "SHA256SUMS".into()]
                .into_iter()
                .map(|name| Asset {
                    browser_download_url: format!("{RELEASES}/download/{tag}/{name}"),
                    name,
                    size: 10,
                    digest: Some(format!("sha256:{}", "a".repeat(64))),
                    state: "uploaded".into(),
                })
                .collect(),
        }
    }
    #[test]
    fn stable_semver_and_exact_assets() {
        assert!(select_release(release("0.10.0"), "0.9.0", Target::MacArm)
            .unwrap()
            .is_some());
        for v in ["0.3.0", "0.2.9", "0.4.0-beta.1"] {
            assert!(select_release(release(v), "0.3.0", Target::MacArm)
                .unwrap()
                .is_none());
        }
        let mut r = release("0.4.0");
        r.prerelease = true;
        assert!(select_release(r, "0.3.0", Target::MacArm)
            .unwrap()
            .is_none());
        let mut r = release("0.4.0");
        r.draft = true;
        assert!(select_release(r, "0.3.0", Target::MacArm)
            .unwrap()
            .is_none());
        assert!(select_release(release("0.4.0"), "0.3.0", Target::MacIntel).is_err());
        let mut r = release("0.4.0");
        r.assets[0].digest = None;
        assert!(select_release(r, "0.3.0", Target::MacArm).is_err());
        let mut r = release("0.4.0");
        r.assets[0].browser_download_url = "https://example.com/malware".into();
        assert!(select_release(r, "0.3.0", Target::MacArm).is_err());
    }
    #[test]
    fn scheduling_and_architecture_names() {
        assert!(check_due(true, None, 1));
        assert!(!check_due(false, None, 1));
        assert!(!check_due(true, Some(100), 101));
        assert!(check_due(true, Some(100), 86500));
        assert!(check_due(true, Some(100), 50));
        assert!(Target::MacArm.asset("1.0.0").contains("arm64"));
        assert!(Target::MacIntel.asset("1.0.0").contains("x86_64"));
        assert!(Target::WindowsX64.asset("1.0.0").ends_with(".exe"));
        assert!(Target::LinuxX64.asset("1.0.0").ends_with("_amd64.deb"));
    }
    #[test]
    fn studio_migration_keeps_historical_assets_and_official_repository_boundary() {
        for target in [
            Target::MacArm,
            Target::MacIntel,
            Target::WindowsX64,
            Target::LinuxX64,
        ] {
            assert!(target
                .asset("0.3.0")
                .to_ascii_lowercase()
                .contains("printer"));
            assert!(target
                .asset("0.4.0")
                .to_ascii_lowercase()
                .contains("studio"));
        }
        for base in [RELEASES, LEGACY_RELEASES] {
            let mut old = release("0.3.0");
            old.html_url = format!("{base}/tag/v0.3.0");
            for a in &mut old.assets {
                a.browser_download_url = format!("{base}/download/v0.3.0/{}", a.name);
            }
            assert!(select_release(old, "0.2.0", Target::MacArm)
                .unwrap()
                .is_some());
        }
        let mut wrong_name = release("0.4.0");
        wrong_name.assets[0].name = "LabelizePrinter-0.4.0-macos-arm64-release.pkg".into();
        assert!(select_release(wrong_name, "0.3.0", Target::MacArm).is_err());
        let mut wrong_owner = release("0.4.0");
        wrong_owner.html_url =
            "https://github.com/other/labelize-studio/releases/tag/v0.4.0".into();
        assert!(select_release(wrong_owner, "0.3.0", Target::MacArm).is_err());
        assert!(asset_url(
            "https://github.com/GOODBOY008/labelize-studio-other/releases/download/v0.4.0/app",
            "v0.4.0"
        )
        .is_err());
    }
    #[test]
    fn checksum_and_stream_fail_closed() {
        let hash = format!("{:x}", Sha256::digest(b"abc"));
        assert_eq!(
            manifest_digest(&format!("{hash}  app.exe\n"), "app.exe").unwrap(),
            hash
        );
        assert!(manifest_digest(&format!("{hash} app.exe\n{hash} app.exe"), "app.exe").is_err());
        assert!(manifest_digest("xxx app.exe", "app.exe").is_err());
        let cancel = AtomicBool::new(false);
        assert!(copy_verified(&b"abc"[..], std::io::sink(), 3, &hash, &cancel, |_, _| {}).is_ok());
        for data in [&b"ab"[..], &b"abcd"[..], &b"abd"[..]] {
            assert!(copy_verified(data, std::io::sink(), 3, &hash, &cancel, |_, _| {}).is_err());
        }
        assert!(copy_verified(
            &b"abc"[..],
            std::io::sink(),
            3,
            &hash,
            &AtomicBool::new(true),
            |_, _| {}
        )
        .is_err());
        struct Full;
        impl Write for Full {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::from_raw_os_error(28))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        assert!(copy_verified(&b"abc"[..], Full, 3, &hash, &cancel, |_, _| {}).is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app");
        fs::write(&path, b"abc").unwrap();
        verify(&path, 3, &hash).unwrap();
        fs::write(&path, b"abd").unwrap();
        assert!(verify(&path, 3, &hash).is_err());
    }
    #[test]
    fn homebrew_ownership_requires_installed_record() {
        assert!(
            brew_installed(br#"{"casks":[{"token":"labelize-studio","installed":"0.4.0"}]}"#)
                .unwrap()
        );
        assert!(
            brew_installed(br#"{"casks":[{"token":"labelize-printer","installed":"0.3.0"}]}"#)
                .unwrap()
        );
        assert!(
            !brew_installed(br#"{"casks":[{"token":"labelize-printer","installed":null}]}"#)
                .unwrap()
        );
        assert!(!brew_installed(br#"{"casks":[]}"#).unwrap());
        assert!(brew_installed(b"broken").is_err());
    }
    #[test]
    fn http_failures_and_stalled_servers_are_bounded() {
        use std::net::TcpListener;
        fn server(response: &'static [u8], delay: u64) -> String {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let _ = stream.read(&mut [0; 4096]);
                std::thread::sleep(Duration::from_millis(delay));
                let _ = stream.write_all(response);
            });
            url
        }
        let client: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_millis(100)))
            .build()
            .into();
        for (response, delay) in [
            (
                &b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n"[..],
                0,
            ),
            (
                &b"HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n"[..],
                0,
            ),
            (&b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}"[..], 0),
            (&b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n"[..], 300),
        ] {
            assert!(
                fetch_release(&client, &server(response, delay), "0.3.0", Target::MacArm).is_err()
            );
        }
        // HTTPS is mandatory even on a redirected asset; the test client above is local-only.
        assert!(agent(Duration::from_secs(1))
            .get("http://127.0.0.1:1")
            .call()
            .is_err());
    }
    #[test]
    #[ignore = "Downloads and verifies the current public release using the production HTTPS client"]
    fn live_release_integrity() {
        let offer = check("0.0.0", platform::target().unwrap())
            .unwrap()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().to_path_buf()).unwrap();
        let verified = download(&paths, offer.clone(), &AtomicBool::new(false), |_, _| {}).unwrap();
        verify(&verified.path, offer.installer.size, &verified.sha256).unwrap();
        let cached = download(&paths, offer, &AtomicBool::new(false), |_, _| {
            panic!("Cache must not re-download installer")
        })
        .unwrap();
        assert_eq!(verified.sha256, cached.sha256);
        println!("Verified {} and cache reuse", verified.offer.installer.name);
    }
    #[test]
    fn crash_cleanup_is_scoped_and_respects_active_downloads() {
        let dir = tempfile::tempdir().unwrap();
        let paths = AppPaths::new(dir.path().to_path_buf()).unwrap();
        let release = paths.root.join("updates/1.0.0");
        private_dir(&release).unwrap();
        let orphan = release.join(format!("{}.part", uuid::Uuid::new_v4()));
        fs::write(&orphan, b"partial").unwrap();
        let installer = release.join("app.exe");
        fs::write(&installer, b"keep").unwrap();
        let unrelated = release.join("notes.part");
        fs::write(&unrelated, b"keep").unwrap();
        let lock = cache_lock(&paths).unwrap();
        assert!(cleanup_cache(&paths).is_err());
        assert!(orphan.exists());
        drop(lock);
        cleanup_cache(&paths).unwrap();
        assert!(!orphan.exists());
        assert!(installer.exists() && unrelated.exists());
    }
}
