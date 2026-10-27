use std::{
    env,
    ffi::OsString,
    fs::{self, File},
    io::{self, IsTerminal, Read},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::Duration,
};

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use owo_colors::OwoColorize;
use reqwest::blocking::Client;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tar::Archive;

const LATEST_MANIFEST: &str =
    "https://github.com/ManiProjs/alaco/releases/latest/download/alacoup-manifest.json";
const ALACOUP_LATEST_RELEASE: &str =
    "https://api.github.com/repos/ManiProjs/alacoup/releases/latest";
const VERSION_MANIFEST: &str =
    "https://github.com/ManiProjs/alaco/releases/download/v{version}/alacoup-manifest.json";
const MAX_ARCHIVE_ENTRIES: usize = 100_000;
const MAX_ARCHIVE_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "alacoup",
    version,
    about = "Install and update the Alaco toolchain"
)]
struct Cli {
    #[command(subcommand)]
    command: CommandLine,
}

#[derive(Debug, Subcommand)]
enum CommandLine {
    /// Show the latest stable release and whether this platform is supported.
    Check,
    /// Install the latest stable release, or a specific version (for example 0.1.0).
    Install {
        /// Optional exact release version.
        version: Option<String>,
        /// Install the binary without replacing the standard library.
        #[arg(long)]
        binary_only: bool,
    },
    /// Print the latest stable version without installing it.
    Latest,
    /// Check GitHub for a newer AlacoUp release.
    SelfUpdate,
    /// Show the install paths used by AlacoUp.
    Paths,
    /// Check the installed compiler and native build prerequisites.
    Doctor,
    /// Run a compiler command explicitly, including commands that share a manager command name.
    #[command(alias = "compiler", trailing_var_arg = true)]
    Exec {
        /// Alaco compiler command, such as check, build, or run.
        command: OsString,
        /// Arguments forwarded to the compiler.
        #[arg(allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Forward a compiler command and its arguments to the installed Alaco compiler.
    #[command(external_subcommand)]
    Compiler(Vec<OsString>),
}

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    version: String,
    tag: String,
    channel: String,
    stdlib: Asset,
    targets: std::collections::BTreeMap<String, TargetAssets>,
}

#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

fn color_enabled() -> bool {
    io::stdout().is_terminal() && env::var_os("NO_COLOR").is_none()
}

fn success(message: impl std::fmt::Display) {
    if color_enabled() {
        println!("{}", format!("✓ {message}").green().bold());
    } else {
        println!("✓ {message}");
    }
}

fn info(message: impl std::fmt::Display) {
    if color_enabled() {
        println!("{}", format!("› {message}").cyan());
    } else {
        println!("› {message}");
    }
}

fn check_self_update() -> Result<()> {
    let current = env!("CARGO_PKG_VERSION");
    let response = client()?
        .get(ALACOUP_LATEST_RELEASE)
        .send()
        .context("could not check for AlacoUp updates")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        info(
            "No AlacoUp releases have been published yet; update checks will work after the first release.",
        );
        return Ok(());
    }
    ensure!(
        response.status().is_success(),
        "AlacoUp update check failed: HTTP {}",
        response.status()
    );
    let release: GitHubRelease = response
        .json()
        .context("GitHub returned an invalid AlacoUp release response")?;
    let latest = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    ensure!(
        is_valid_version(latest),
        "GitHub returned an invalid AlacoUp release tag"
    );
    println!("AlacoUp {current}");
    if latest == current {
        success(format!("You're up to date (latest: {latest})."));
    } else {
        info(format!(
            "AlacoUp {latest} is available (you have {current})."
        ));
        println!("Release notes: {}", release.html_url);
        println!(
            "Update with the latest installer from https://github.com/ManiProjs/alacoup#install-alacoup-and-alaco"
        );
    }
    Ok(())
}

#[derive(Debug, Clone, Deserialize)]
struct Asset {
    filename: String,
    url: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct TargetAssets {
    binary: Asset,
    bundle: Asset,
}

impl Manifest {
    fn parse(json: &str) -> Result<Self> {
        let manifest: Self =
            serde_json::from_str(json).context("invalid AlacoUp release manifest")?;
        ensure!(
            manifest.schema_version == 1,
            "unsupported manifest schema version {}",
            manifest.schema_version
        );
        ensure!(
            is_valid_version(&manifest.version),
            "manifest contains an invalid semantic version"
        );
        ensure!(
            manifest.channel == "stable" || manifest.channel == "prerelease",
            "unknown release channel '{}'",
            manifest.channel
        );
        validate_asset(&manifest.stdlib)?;
        for assets in manifest.targets.values() {
            validate_asset(&assets.binary)?;
            validate_asset(&assets.bundle)?;
        }
        Ok(manifest)
    }

    fn target(&self) -> Result<(&str, &TargetAssets)> {
        let target = current_target()?;
        self.targets
            .get_key_value(target)
            .map(|(name, assets)| (name.as_str(), assets))
            .ok_or_else(|| {
                anyhow::anyhow!("Alaco does not publish a release for this platform ({target})")
            })
    }
}

fn validate_asset(asset: &Asset) -> Result<()> {
    ensure!(
        !asset.filename.is_empty()
            && !asset.filename.contains('/')
            && !asset.filename.contains('\\')
            && Path::new(&asset.filename)
                .file_name()
                .and_then(|n| n.to_str())
                == Some(asset.filename.as_str()),
        "invalid asset filename '{}'; expected a plain filename",
        asset.filename
    );
    ensure!(
        is_trusted_asset_url(&asset.url),
        "refusing untrusted release URL for {}",
        asset.filename
    );
    ensure!(
        asset.sha256.len() == 64 && asset.sha256.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid SHA-256 for {}",
        asset.filename
    );
    Ok(())
}

fn is_trusted_asset_url(value: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(value) else {
        return false;
    };
    url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.host_str() == Some("github.com")
        && url
            .path()
            .starts_with("/ManiProjs/alaco/releases/download/")
        && !url.path().contains("/../")
}

fn is_valid_version(version: &str) -> bool {
    if version.is_empty()
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
    {
        return false;
    }
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let components: Vec<_> = core.split('.').collect();
    components.len() == 3
        && components
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn current_target() -> Result<&'static str> {
    match (env::consts::OS, env::consts::ARCH) {
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, arch) => bail!("unsupported host platform: {os}-{arch}"),
    }
}

fn client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent(concat!("AlacoUp/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(300))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let trusted_host = matches!(
                attempt.url().host_str(),
                Some(
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            );
            if attempt.url().scheme() != "https" || !trusted_host || attempt.previous().len() >= 10
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}

fn fetch_manifest(version: Option<&str>) -> Result<Manifest> {
    let url = match version {
        Some(version) => {
            let version = version.strip_prefix('v').unwrap_or(version);
            ensure!(
                is_valid_version(version),
                "invalid version '{version}'; expected a semantic version such as 0.1.0"
            );
            VERSION_MANIFEST.replace("{version}", version)
        }
        None => LATEST_MANIFEST.to_owned(),
    };
    let response = client()?
        .get(&url)
        .send()
        .with_context(|| format!("could not fetch release manifest from {url}"))?;
    ensure!(
        response.status().is_success(),
        "release manifest request failed: HTTP {}",
        response.status()
    );
    let manifest = Manifest::parse(
        &response
            .text()
            .context("could not read release manifest response")?,
    )?;
    if let Some(requested) = version {
        let requested = requested.strip_prefix('v').unwrap_or(requested);
        ensure!(
            manifest.version == requested,
            "requested Alaco {requested}, but the release manifest describes {}",
            manifest.version
        );
    }
    Ok(manifest)
}

fn alacoup_home() -> Result<PathBuf> {
    if let Some(path) = env::var_os("ALACOUP_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("could not determine your home directory; set ALACOUP_HOME")?;
    Ok(home.join(".alacoup"))
}

fn alaco_home() -> Result<PathBuf> {
    let home = env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .context("could not determine your home directory")?;
    Ok(home.join(".alaco"))
}

fn download_asset(client: &Client, asset: &Asset, destination: &Path) -> Result<()> {
    info(format!("Downloading {}", asset.filename));
    let mut response = client
        .get(&asset.url)
        .send()
        .with_context(|| format!("download failed for {}", asset.filename))?;
    ensure!(
        response.status().is_success(),
        "download failed for {}: HTTP {}",
        asset.filename,
        response.status()
    );
    ensure!(
        response.url().scheme() == "https",
        "download for {} redirected away from HTTPS",
        asset.filename
    );
    if let Some(length) = response.content_length() {
        ensure!(
            length <= MAX_ARCHIVE_BYTES,
            "download for {} exceeds the 1 GiB size limit",
            asset.filename
        );
    }
    let mut file = File::create(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    let progress = if io::stderr().is_terminal() {
        let progress = response
            .content_length()
            .map(ProgressBar::new)
            .unwrap_or_else(ProgressBar::new_spinner);
        let template = if env::var_os("NO_COLOR").is_none() {
            "  {spinner:.green} [{bar:32.cyan/blue}] {bytes}/{total_bytes} ({eta})"
        } else {
            "  {spinner} [{bar:32}] {bytes}/{total_bytes} ({eta})"
        };
        let style = ProgressStyle::with_template(template)
            .unwrap_or_else(|_| ProgressStyle::default_bar())
            .progress_chars("━━╸─");
        progress.set_style(style);
        progress.set_message(asset.filename.clone());
        Some(progress)
    } else {
        None
    };
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 32 * 1024];
    let mut downloaded = 0_u64;
    loop {
        let read = response.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(read as u64);
        if let Some(progress) = &progress {
            progress.set_position(downloaded);
        }
        ensure!(
            downloaded <= MAX_ARCHIVE_BYTES,
            "download for {} exceeds the 1 GiB size limit",
            asset.filename
        );
        hasher.update(&buffer[..read]);
        io::Write::write_all(&mut file, &buffer[..read])?;
    }
    let actual = format!("{:x}", hasher.finalize());
    verify_sha256(&asset.filename, &asset.sha256, &actual)?;
    if let Some(progress) = progress {
        progress.finish_and_clear();
    }
    success(format!("Downloaded and verified {}", asset.filename));
    Ok(())
}

fn verify_sha256(filename: &str, expected: &str, actual: &str) -> Result<()> {
    ensure!(
        actual.eq_ignore_ascii_case(expected),
        "SHA-256 verification failed for {filename} (expected {expected}, got {actual})"
    );
    Ok(())
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    let mut entries_seen = 0_usize;
    let mut bytes_seen = 0_u64;
    for entry in archive.entries().context("invalid tar archive")? {
        let entry = entry.context("invalid tar archive entry")?;
        entries_seen += 1;
        ensure!(
            entries_seen <= MAX_ARCHIVE_ENTRIES,
            "archive contains too many entries"
        );
        let path = entry.path().context("archive contains an invalid path")?;
        ensure!(
            !path.as_os_str().is_empty()
                && path
                    .components()
                    .all(|component| matches!(component, std::path::Component::Normal(_))),
            "archive contains an unsafe path"
        );
        let kind = entry.header().entry_type();
        ensure!(
            kind.is_file() || kind.is_dir(),
            "archive contains an unsupported link or special file"
        );
        if kind.is_file() {
            bytes_seen = bytes_seen.saturating_add(entry.size());
            ensure!(
                bytes_seen <= MAX_ARCHIVE_BYTES,
                "archive exceeds the 1 GiB extraction limit"
            );
        }
    }
    let file = File::open(archive_path)?;
    let decoder = GzDecoder::new(file);
    let mut archive = Archive::new(decoder);
    archive
        .unpack(destination)
        .with_context(|| format!("could not extract {}", archive_path.display()))?;
    Ok(())
}

fn install(version: Option<&str>, binary_only: bool) -> Result<()> {
    let manifest = fetch_manifest(version)?;
    let (target, assets) = manifest.target()?;
    info(format!(
        "Alaco {} ({target}, {})",
        manifest.version, manifest.channel
    ));

    let home = alacoup_home()?;
    let versions_dir = home.join("versions");
    let bin_dir = home.join("bin");
    fs::create_dir_all(&home)?;
    let staging = tempfile::Builder::new()
        .prefix("install-")
        .tempdir_in(&home)
        .context("could not create a temporary installation directory")?;
    let asset = if binary_only {
        &assets.binary
    } else {
        &assets.bundle
    };
    let archive_path = staging.path().join(&asset.filename);
    download_asset(&client()?, asset, &archive_path)?;
    let extracted = staging.path().join("extracted");
    extract_archive(&archive_path, &extracted)?;

    let binary_name = if cfg!(windows) { "alaco.exe" } else { "alaco" };
    let extracted_binary = extracted.join(binary_name);
    ensure!(
        extracted_binary.is_file(),
        "archive did not contain the expected compiler executable '{binary_name}'"
    );
    if !binary_only {
        ensure!(
            extracted.join("std").is_dir(),
            "bundle did not contain a std/ directory"
        );
    }

    fs::create_dir_all(&versions_dir)?;
    // Never replace a version directory while the live launcher may point to it.
    // Keep each installed payload immutable and activate it only after validation.
    let version_dir = tempfile::Builder::new()
        .prefix(&format!("{}-", manifest.version))
        .tempdir_in(&versions_dir)?;
    let staged_version = version_dir.path().join("payload");
    fs::rename(&extracted, &staged_version)?;
    let final_version_dir = version_dir.keep();
    let version_dir = final_version_dir.join("payload");

    // Create all required activation directories before touching the live stdlib.
    fs::create_dir_all(&bin_dir)?;
    let mut stdlib_backup: Option<(PathBuf, PathBuf)> = None;
    let mut stdlib_transaction = None;
    let mut updated_stdlib: Option<PathBuf> = None;
    if !binary_only {
        let stdlib_dir = alaco_home()?.join("stdlib").join("std");
        let staged_stdlib = version_dir.join("std");
        ensure!(
            staged_stdlib.is_dir(),
            "bundle did not contain a std/ directory"
        );
        let parent = stdlib_dir
            .parent()
            .context("standard-library destination has no parent")?;
        fs::create_dir_all(parent)?;
        let backup_root = tempfile::Builder::new()
            .prefix("alacoup-update-")
            .tempdir_in(parent)?;
        let backup = backup_root.path().join("previous-stdlib");
        if stdlib_dir.exists() {
            fs::rename(&stdlib_dir, &backup)
                .context("could not stage the existing standard library for rollback")?;
            stdlib_backup = Some((backup, stdlib_dir.clone()));
        }
        stdlib_transaction = Some(backup_root);
        if let Err(error) = replace_directory(&staged_stdlib, &stdlib_dir) {
            if let Some((backup, destination)) = stdlib_backup.take()
                && let Err(rollback) = fs::rename(&backup, &destination)
            {
                return Err(error).context(format!(
                    "standard-library update failed and rollback also failed: {rollback}"
                ));
            }
            return Err(error);
        }
        updated_stdlib = Some(stdlib_dir);
    }

    if let Err(error) = install_launcher(&bin_dir, &version_dir, binary_name) {
        if let Some((backup, destination)) = stdlib_backup.take() {
            if destination.exists() {
                fs::remove_dir_all(&destination)
                    .context("could not remove failed standard-library update before rollback")?;
            }
            if let Err(rollback) = fs::rename(&backup, &destination) {
                return Err(error).context(format!("launcher activation failed and standard-library rollback also failed: {rollback}"));
            }
        } else if let Some(destination) = updated_stdlib.take()
            && let Err(rollback) = fs::remove_dir_all(&destination)
        {
            return Err(error).context(format!(
                "launcher activation failed and cleanup of the newly installed standard library also failed: {rollback}"
            ));
        }
        let _ = fs::remove_dir_all(&final_version_dir);
        return Err(error)
            .context("could not activate the new compiler; previous launcher remains active");
    }
    // Keep the transaction directory alive until launcher activation succeeds;
    // its backup is intentionally discarded only after this commit point.
    drop(stdlib_transaction);

    success(format!(
        "Installed Alaco {} to {}",
        manifest.version,
        version_dir.display()
    ));
    println!("Launcher: {}", bin_dir.join(binary_name).display());
    if binary_only {
        println!(
            "Standard library unchanged. Set ALACO_STDLIB_DIR if it is installed outside ~/.alaco/stdlib."
        );
    }
    print_path_hint(&bin_dir);
    Ok(())
}

fn replace_directory(source: &Path, destination: &Path) -> Result<()> {
    let parent = destination
        .parent()
        .context("standard-library destination has no parent")?;
    fs::create_dir_all(parent)?;
    let transaction = tempfile::Builder::new()
        .prefix("alacoup-stdlib-")
        .tempdir_in(parent)
        .context("could not create a temporary standard-library directory")?;
    let staged = transaction.path().join("new");
    let backup = transaction.path().join("previous");
    copy_directory(source, &staged)?;
    if destination.exists() {
        fs::rename(destination, &backup)
            .with_context(|| format!("could not back up {}", destination.display()))?;
        if let Err(error) = fs::rename(&staged, destination) {
            if let Err(restore_error) = fs::rename(&backup, destination) {
                return Err(error).with_context(|| {
                    format!(
                        "could not install standard library at {}; rollback also failed: {restore_error}",
                        destination.display()
                    )
                });
            }
            return Err(error).with_context(|| {
                format!(
                    "could not install standard library at {}",
                    destination.display()
                )
            });
        }
    } else {
        fs::rename(&staged, destination).with_context(|| {
            format!(
                "could not install standard library at {}",
                destination.display()
            )
        })?;
    }
    Ok(())
}

#[cfg(test)]
fn replace_version_directory(source: &Path, destination: &Path, backup: &Path) -> Result<()> {
    if destination.exists() {
        fs::rename(destination, backup).with_context(|| {
            format!(
                "could not back up existing installation {}",
                destination.display()
            )
        })?;
    }
    if let Err(error) = fs::rename(source, destination) {
        if backup.exists() {
            let _ = fs::rename(backup, destination);
        }
        return Err(error).with_context(|| {
            format!(
                "could not finalize the versioned installation at {}",
                destination.display()
            )
        });
    }
    if backup.exists() {
        fs::remove_dir_all(backup).with_context(|| {
            format!(
                "installed successfully but could not remove backup {}",
                backup.display()
            )
        })?;
    }
    Ok(())
}

fn compiler_path() -> Result<PathBuf> {
    let name = if cfg!(windows) { "alaco.exe" } else { "alaco" };
    let managed = alacoup_home()?.join("bin").join(name);
    if managed.is_file() {
        return Ok(managed);
    }
    if let Some(path) = env::var_os("PATH") {
        for directory in env::split_paths(&path) {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    bail!(
        "Alaco compiler not found; run `alacoup install` or add an existing `alaco` executable to PATH"
    )
}

fn run_compiler(args: Vec<OsString>) -> Result<ExitCode> {
    ensure!(
        !args.is_empty(),
        "provide an Alaco compiler command; try `alacoup exec --help`"
    );
    let path = compiler_path()?;
    let status = Command::new(&path)
        .args(&args)
        .status()
        .with_context(|| format!("could not start Alaco compiler at {}", path.display()))?;
    Ok(ExitCode::from(
        status.code().unwrap_or(1).clamp(0, 255) as u8
    ))
}

fn find_program(name: &str) -> Option<PathBuf> {
    env::var_os("PATH").and_then(|path| {
        env::split_paths(&path)
            .map(|directory| directory.join(name))
            .find(|candidate| candidate.is_file())
    })
}

fn doctor() -> Result<bool> {
    println!("AlacoUp {}", env!("CARGO_PKG_VERSION"));
    let mut healthy = true;
    match compiler_path() {
        Ok(path) => {
            println!("Alaco compiler: {}", path.display());
            match Command::new(&path).arg("--version").output() {
                Ok(output) if output.status.success() => {
                    let version = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    if version.is_empty() {
                        println!("Compiler version: available (no version text)");
                    } else {
                        println!("Compiler version: {version}");
                    }
                }
                Ok(output) => {
                    healthy = false;
                    println!("Compiler version: could not query (exit {})", output.status)
                }
                Err(error) => {
                    healthy = false;
                    println!("Compiler version: could not run ({error})")
                }
            }
        }
        Err(error) => {
            healthy = false;
            println!("Alaco compiler: missing — {error}")
        }
    }

    let stdlib = env::var_os("ALACO_STDLIB_DIR")
        .map(PathBuf::from)
        .unwrap_or(alaco_home()?.join("stdlib"))
        .join("std");
    println!(
        "Standard library: {} ({})",
        stdlib.display(),
        if stdlib.is_dir() {
            "present"
        } else {
            "missing"
        }
    );
    if !stdlib.is_dir() {
        healthy = false;
    }
    for program in ["clang++", "g++", "node", "git"] {
        match find_program(program) {
            Some(path) => println!("{program}: {}", path.display()),
            None => {
                println!("{program}: not found on PATH");
                if program == "clang++" && (cfg!(target_os = "macos") || cfg!(target_os = "linux"))
                {
                    healthy = false;
                }
            }
        }
    }
    if cfg!(target_os = "macos") || cfg!(target_os = "linux") {
        println!("Native Alaco builds normally require clang++.");
    }
    if healthy {
        println!("Environment looks ready.");
    } else {
        println!("Environment is incomplete; fix the missing requirements above.");
    }
    Ok(healthy)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in
        fs::read_dir(source).with_context(|| format!("cannot read {}", source.display()))?
    {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path)?;
        } else {
            bail!(
                "unsupported file type in standard library archive: {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

#[cfg(unix)]
fn install_launcher(bin_dir: &Path, version_dir: &Path, binary_name: &str) -> Result<()> {
    use std::os::unix::fs::symlink;
    let launcher = bin_dir.join(binary_name);
    let transaction = tempfile::Builder::new()
        .prefix(".alacoup-launcher-")
        .tempdir_in(bin_dir)
        .context("could not create temporary launcher directory")?;
    let temporary = transaction.path().join(binary_name);
    symlink(version_dir.join(binary_name), &temporary)
        .context("could not create compiler launcher symlink")?;
    // Renaming a sibling symlink over the old launcher is atomic on Unix.
    fs::rename(temporary, launcher)?;
    Ok(())
}

#[cfg(windows)]
fn install_launcher(bin_dir: &Path, version_dir: &Path, binary_name: &str) -> Result<()> {
    let launcher = bin_dir.join(binary_name);
    let transaction = tempfile::Builder::new()
        .prefix("alacoup-launcher-")
        .tempdir_in(bin_dir)
        .context("could not create temporary launcher directory")?;
    let staged = transaction.path().join(binary_name);
    let backup = transaction.path().join("previous-launcher");
    fs::copy(version_dir.join(binary_name), &staged)
        .context("could not stage compiler launcher")?;
    if launcher.exists() {
        fs::rename(&launcher, &backup).context("could not back up existing compiler launcher")?;
    }
    if let Err(error) = fs::rename(&staged, &launcher) {
        if backup.exists() {
            if let Err(restore_error) = fs::rename(&backup, &launcher) {
                return Err(error).with_context(|| {
                    format!(
                        "could not install compiler launcher; rollback also failed: {restore_error}"
                    )
                });
            }
        }
        return Err(error).context("could not install compiler launcher");
    }
    Ok(())
}

fn print_path_hint(bin_dir: &Path) {
    let path = env::var_os("PATH").unwrap_or_default();
    let present = env::split_paths(&path).any(|entry| entry == bin_dir);
    if !present {
        println!("Add this directory to your PATH: {}", bin_dir.display());
    }
}

fn run() -> Result<ExitCode> {
    match Cli::parse().command {
        CommandLine::Check => {
            let manifest = fetch_manifest(None)?;
            println!(
                "Latest stable Alaco: {} ({})",
                manifest.version, manifest.tag
            );
            match manifest.target() {
                Ok((target, _)) => println!("This platform is supported: {target}"),
                Err(error) => println!("This platform is not supported: {error}"),
            }
        }
        CommandLine::Latest => println!("{}", fetch_manifest(None)?.version),
        CommandLine::SelfUpdate => check_self_update()?,
        CommandLine::Install {
            version,
            binary_only,
        } => install(version.as_deref(), binary_only)?,
        CommandLine::Doctor => {
            return Ok(if doctor()? {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            });
        }
        CommandLine::Exec { command, args } => {
            let mut forwarded = Vec::with_capacity(args.len() + 1);
            forwarded.push(command);
            forwarded.extend(args);
            return run_compiler(forwarded);
        }
        CommandLine::Compiler(args) => {
            return run_compiler(args);
        }
        CommandLine::Paths => {
            println!("AlacoUp home: {}", alacoup_home()?.display());
            println!(
                "Compiler launcher: {}",
                alacoup_home()?
                    .join("bin")
                    .join(if cfg!(windows) { "alaco.exe" } else { "alaco" })
                    .display()
            );
            println!(
                "Standard library: {}",
                alaco_home()?.join("stdlib").display()
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            if io::stderr().is_terminal() && env::var_os("NO_COLOR").is_none() {
                eprintln!("{}", format!("✗ error: {error:#}").red().bold());
            } else {
                eprintln!("error: {error:#}");
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_MANIFEST: &str = r#"{
      "schema_version": 1,
      "version": "0.1.0",
      "tag": "v0.1.0",
      "channel": "stable",
      "stdlib": {"filename":"alaco-0.1.0-stdlib.tar.gz","url":"https://github.com/ManiProjs/alaco/releases/download/v0.1.0/stdlib.tar.gz","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},
      "targets": {
        "aarch64-apple-darwin": {
          "binary": {"filename":"alaco-binary.tar.gz","url":"https://github.com/ManiProjs/alaco/releases/download/v0.1.0/binary.tar.gz","sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"},
          "bundle": {"filename":"alaco-bundle.tar.gz","url":"https://github.com/ManiProjs/alaco/releases/download/v0.1.0/bundle.tar.gz","sha256":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"}
        }
      }
    }"#;

    #[test]
    fn parses_manifest_and_selects_platform() {
        let manifest = Manifest::parse(VALID_MANIFEST).unwrap();
        assert_eq!(manifest.version, "0.1.0");
        assert!(manifest.targets.contains_key("aarch64-apple-darwin"));
    }

    #[test]
    fn rejects_unknown_schema_and_insecure_asset_urls() {
        assert!(
            Manifest::parse(
                &VALID_MANIFEST.replace("\"schema_version\": 1", "\"schema_version\": 2")
            )
            .is_err()
        );
        for url in [
            "https://evil.example/alaco.tar.gz",
            "https://github.com.evil.example/ManiProjs/alaco/releases/download/v0.1.0/alaco.tar.gz",
            "https://user@github.com/ManiProjs/alaco/releases/download/v0.1.0/alaco.tar.gz",
            "https://github.com/other/repo/releases/download/v0.1.0/alaco.tar.gz",
        ] {
            assert!(
                Manifest::parse(&VALID_MANIFEST.replace(
                    "https://github.com/ManiProjs/alaco/releases/download/v0.1.0/stdlib.tar.gz",
                    url
                ))
                .is_err(),
                "accepted untrusted URL {url}"
            );
        }
        assert!(is_trusted_asset_url(
            "https://github.com/ManiProjs/alaco/releases/download/v0.1.0/alaco.tar.gz"
        ));
        assert!(
            Manifest::parse(&VALID_MANIFEST.replace(
                "https://github.com/ManiProjs/alaco/releases/download/v0.1.0/stdlib.tar.gz",
                "http://github.com/ManiProjs/alaco/releases/download/v0.1.0/stdlib.tar.gz"
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_asset_path_traversal() {
        assert!(
            Manifest::parse(
                &VALID_MANIFEST.replace("alaco-0.1.0-stdlib.tar.gz", "../stdlib.tar.gz")
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_release_versions_and_rejects_path_like_versions() {
        assert!(is_valid_version("0.1.0"));
        assert!(is_valid_version("0.2.0-rc.1"));
        assert!(!is_valid_version(".."));
        assert!(!is_valid_version("../0.1.0"));
        assert!(!is_valid_version("release"));
    }

    #[test]
    fn verifies_downloaded_asset_hash() {
        let bytes = b"test asset";
        let expected = format!("{:x}", Sha256::digest(bytes));
        verify_sha256("test.tar.gz", &expected, &expected).unwrap();
        assert!(verify_sha256("test.tar.gz", &expected, &"0".repeat(64)).is_err());
    }

    #[test]
    fn archive_extraction_rejects_symlinks_without_writing_outside_staging() {
        use flate2::{Compression, write::GzEncoder};

        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("malicious.tar.gz");
        let encoder = GzEncoder::new(File::create(&archive_path).unwrap(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_link_name("../../outside").unwrap();
        header.set_cksum();
        builder
            .append_data(&mut header, "escape", io::empty())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap();

        let destination = temp.path().join("staging");
        assert!(extract_archive(&archive_path, &destination).is_err());
        assert!(!temp.path().join("outside").exists());
        assert!(!destination.join("escape").exists());
    }

    #[test]
    fn rejects_asset_filenames_that_are_not_plain_names() {
        for filename in ["", ".", "..", "nested/file.tar.gz", "nested\\file.tar.gz"] {
            let escaped = filename.replace('\\', "\\\\");
            let json = VALID_MANIFEST.replace("alaco-0.1.0-stdlib.tar.gz", &escaped);
            assert!(Manifest::parse(&json).is_err(), "accepted {filename:?}");
        }
    }

    #[test]
    fn version_validation_rejects_empty_components_and_non_numeric_core() {
        for version in ["1..0", "1.0", "x.0.0", "1.0.0/../../tmp"] {
            assert!(!is_valid_version(version), "accepted {version:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn failed_standard_library_copy_preserves_existing_destination() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("stdlib").join("std");
        fs::create_dir_all(&source).unwrap();
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("keep.txt"), "original").unwrap();
        symlink("missing-target", source.join("unsupported-link")).unwrap();

        assert!(replace_directory(&source, &destination).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("keep.txt")).unwrap(),
            "original"
        );
        assert!(!temp.path().join("stdlib/std.alacoup-new").exists());
        assert!(!temp.path().join("stdlib/std.alacoup-backup").exists());
    }

    #[cfg(unix)]
    #[test]
    fn launcher_install_preserves_preexisting_temporary_name() {
        let temp = tempfile::tempdir().unwrap();
        let bin = temp.path().join("bin");
        let version = temp.path().join("versions/0.1.0");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&version).unwrap();
        fs::write(version.join("alaco"), "compiler").unwrap();
        fs::write(bin.join(".alaco.new"), "keep this file").unwrap();

        install_launcher(&bin, &version, "alaco").unwrap();

        assert_eq!(
            fs::read_to_string(bin.join(".alaco.new")).unwrap(),
            "keep this file"
        );
        assert_eq!(
            fs::read_link(bin.join("alaco")).unwrap(),
            version.join("alaco")
        );
    }

    #[test]
    fn failed_version_replacement_restores_existing_installation() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("missing-source");
        let destination = temp.path().join("version");
        let backup = temp.path().join("transaction/previous-version");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("alaco"), "working compiler").unwrap();
        fs::create_dir_all(backup.parent().unwrap()).unwrap();

        assert!(replace_version_directory(&source, &destination, &backup).is_err());
        assert_eq!(
            fs::read_to_string(destination.join("alaco")).unwrap(),
            "working compiler"
        );
        assert!(!backup.exists());
    }
}
