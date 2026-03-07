use std::fs;
use std::path::Path;

const GITHUB_API_LATEST: &str =
    "https://api.github.com/repos/solisoft/solidb/releases/latest";

pub fn execute() -> anyhow::Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: v{}", current_version);
    println!("Checking for updates...");

    let client = reqwest::blocking::Client::builder()
        .user_agent("solidb-updater")
        .build()?;

    let release: serde_json::Value = client.get(GITHUB_API_LATEST).send()?.json()?;

    let tag = release["tag_name"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Could not read tag_name from release"))?;

    let latest_version = tag.strip_prefix('v').unwrap_or(tag);

    if latest_version == current_version {
        println!("Already up to date (v{}).", current_version);
        return Ok(());
    }

    println!("New version available: v{}", latest_version);

    let asset_name = get_asset_name()?;
    let download_url = release["assets"]
        .as_array()
        .and_then(|assets| {
            assets.iter().find_map(|a| {
                if a["name"].as_str() == Some(&asset_name) {
                    a["browser_download_url"].as_str().map(|s| s.to_string())
                } else {
                    None
                }
            })
        })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No release asset found for this platform ({})",
                asset_name
            )
        })?;

    println!("Downloading {}...", asset_name);

    let response = client.get(&download_url).send()?;
    let bytes = response.bytes()?;

    let decoder = flate2::read::GzDecoder::new(&bytes[..]);
    let mut archive = tar::Archive::new(decoder);

    let current_exe = std::env::current_exe()?;
    let exe_dir = current_exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine executable directory"))?;

    let tmp_dir = tempfile::tempdir_in(exe_dir)?;

    // Extract all binaries to temp dir
    for entry in archive.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let file_name = path
            .file_name()
            .ok_or_else(|| anyhow::anyhow!("Invalid entry in archive"))?
            .to_owned();
        let dest = tmp_dir.path().join(&file_name);
        entry.unpack(&dest)?;
    }

    // Replace binaries
    let binaries = ["solidb", "solidb-dump", "solidb-restore"];
    for name in &binaries {
        let src = tmp_dir.path().join(name);
        if !src.exists() {
            continue;
        }
        let dest = exe_dir.join(name);
        replace_binary(&src, &dest)?;
        println!("  Updated {}", name);
    }

    println!("Successfully updated to v{}!", latest_version);
    Ok(())
}

fn get_asset_name() -> anyhow::Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;

    let os_str = match os {
        "linux" => "linux",
        "macos" => "darwin",
        _ => anyhow::bail!("Unsupported OS: {}", os),
    };

    let arch_str = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        _ => anyhow::bail!("Unsupported architecture: {}", arch),
    };

    Ok(format!("solidb-{}-{}.tar.gz", os_str, arch_str))
}

fn replace_binary(src: &Path, dest: &Path) -> anyhow::Result<()> {
    let backup = dest.with_extension("old");

    // Move current binary out of the way (if it exists)
    if dest.exists() {
        fs::rename(dest, &backup)?;
    }

    // Move new binary into place
    match fs::rename(src, dest) {
        Ok(()) => {
            // Clean up backup
            let _ = fs::remove_file(&backup);
        }
        Err(e) => {
            // Restore backup on failure
            if backup.exists() {
                let _ = fs::rename(&backup, dest);
            }
            return Err(e.into());
        }
    }

    // Ensure executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dest, fs::Permissions::from_mode(0o755))?;
    }

    Ok(())
}
