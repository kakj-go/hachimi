use std::{
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    let output = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: bundle-hachimi-cef-host <output-dir> <target-profile-dir>")?;
    let target_profile = arguments
        .next()
        .map(PathBuf::from)
        .ok_or("usage: bundle-hachimi-cef-host <output-dir> <target-profile-dir>")?;
    if arguments.next().is_some() {
        return Err("unexpected extra bundle arguments".into());
    }
    std::fs::create_dir_all(&output)?;
    #[cfg(target_os = "windows")]
    let executable = {
        let generated = cef::build_util::win::bundle(&output, &target_profile, "hachimi_cef_host")?;
        for extension in ["exe", "dll", "pdb", "exe.manifest"] {
            let source = output.join(format!("hachimi_cef_host.{extension}"));
            let destination = output.join(format!("hachimi-cef-host.{extension}"));
            if destination.exists() {
                std::fs::remove_file(&destination)?;
            }
            std::fs::rename(source, destination)?;
        }
        let _ = generated;
        output.join("hachimi-cef-host.exe")
    };
    #[cfg(target_os = "linux")]
    let executable = cef::build_util::linux::bundle(&output, &target_profile, "hachimi-cef-host")?;
    #[cfg(target_os = "macos")]
    let executable = cef::build_util::mac::bundle(&output, &target_profile, "hachimi-cef-host")?;
    write_runtime_manifest(&output)?;
    println!("{}", executable.display());
    Ok(())
}

fn write_runtime_manifest(root: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest_path = root.join("runtime-manifest.json");
    if manifest_path.exists() {
        std::fs::remove_file(&manifest_path)?;
    }
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|left, right| left["path"].as_str().cmp(&right["path"].as_str()));
    let manifest = serde_json::json!({
        "schemaVersion": 1,
        "cefCrateVersion": "151.2.0+151.3.14",
        "cefVersion": "151.3.14+g5d67476",
        "chromiumVersion": "151.0.7922.72",
        "platform": "windows-x64",
        "archiveName": "cef_binary_151.3.14+g5d67476+chromium-151.0.7922.72_windows64_minimal.tar.bz2",
        "archiveUrl": "https://cef-builds.spotifycdn.com/cef_binary_151.3.14%2Bg5d67476%2Bchromium-151.0.7922.72_windows64_minimal.tar.bz2",
        "archiveSha1": "96abc7e46d7dfe31756be682e1c0d423807b498e",
        "archiveSha256": "c63a18909fea077b5c3b5f9a3194f05781cd909efa8a6d7a543cad99c4183a55",
        "files": files,
    });
    std::fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn collect_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<serde_json::Value>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(format!(
                "CEF bundle contains a symbolic link: {}",
                entry.path().display()
            )
            .into());
        }
        if file_type.is_dir() {
            collect_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_string_lossy()
                .replace('\\', "/");
            let mut reader = std::fs::File::open(&path)?;
            let mut digest = Sha256::new();
            let mut buffer = vec![0_u8; 1024 * 1024];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            let sha256 = digest
                .finalize()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            files.push(serde_json::json!({
                "path": relative,
                "size": entry.metadata()?.len(),
                "sha256": sha256,
            }));
        }
    }
    Ok(())
}
