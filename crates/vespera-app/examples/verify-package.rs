//! Read-only integrity checks for a staged or extracted local package.
use std::collections::BTreeSet;
use std::error::Error;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use clap::Parser;
use ring::digest::{Context, SHA256};
use serde::Deserialize;

#[derive(Parser)]
#[command(about = "Verify listed Vespera package files without executing them")]
struct Args {
    /// Staged or extracted package directory containing manifest.json.
    #[arg(long)]
    package: PathBuf,
}

#[derive(Deserialize)]
struct Manifest {
    schema_version: u32,
    files: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    bytes: u64,
    sha256: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn main() -> Result<(), Box<dyn Error>> {
    let count = verify(&Args::parse().package)?;
    println!(
        "Verified {count} manifest-listed files. Authenticity and unlisted files are not checked."
    );
    Ok(())
}

fn verify(root: &Path) -> Result<usize, Box<dyn Error>> {
    let root = root.canonicalize()?;
    let manifest_path = root.join("manifest.json");
    let metadata = manifest_path.symlink_metadata()?;
    if !metadata.is_file() || metadata.len() > 1024 * 1024 {
        return Err(invalid("manifest must be a regular file of at most 1 MiB").into());
    }
    let mut bytes = Vec::new();
    File::open(manifest_path)?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 1024 * 1024 {
        return Err(invalid("manifest exceeds 1 MiB").into());
    }
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.schema_version != 1 || manifest.files.is_empty() || manifest.files.len() > 4096 {
        return Err(invalid("unsupported schema or invalid file count").into());
    }
    let mut seen = BTreeSet::new();
    for entry in &manifest.files {
        // Manifest paths use portable, forward-slash-separated relative names.
        if entry.path.contains(['\\', ':'])
            || entry
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
            || !seen.insert(entry.path.to_lowercase())
            || entry.path.eq_ignore_ascii_case("manifest.json")
        {
            return Err(invalid("unsafe or duplicate manifest path").into());
        }
        let mut path = root.clone();
        for component in entry.path.split('/') {
            path.push(component);
            if path.symlink_metadata()?.file_type().is_symlink() {
                return Err(invalid("package entries must not traverse symbolic links").into());
            }
        }
        if !path.canonicalize()?.starts_with(&root) || !path.symlink_metadata()?.is_file() {
            return Err(invalid("package entry is not a regular file inside the package").into());
        }
        let mut source = File::open(&path)?;
        if source.metadata()?.len() != entry.bytes {
            return Err(invalid(&format!("size mismatch: {}", entry.path)).into());
        }
        let mut digest = Context::new(&SHA256);
        let mut buffer = [0; 32768];
        let mut count = 0_u64;
        loop {
            let read = source.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            count += read as u64;
            digest.update(&buffer[..read]);
        }
        let hash: String = digest
            .finish()
            .as_ref()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if count != entry.bytes || hash != entry.sha256 {
            return Err(invalid(&format!("content mismatch: {}", entry.path)).into());
        }
    }
    Ok(manifest.files.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn accepts_intact_files_and_rejects_damage_missing_files_and_unsafe_manifests() {
        let root =
            std::env::temp_dir().join(format!("vespera-package-check-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let data = root.join("payload");
        fs::write(&data, b"abc").unwrap();
        let entry = serde_json::json!({"path":"payload","bytes":3,"sha256":"ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"});
        let manifest = |entries: serde_json::Value| {
            fs::write(
                root.join("manifest.json"),
                serde_json::to_vec(&serde_json::json!({"schema_version":1,"files":entries}))
                    .unwrap(),
            )
            .unwrap();
        };
        manifest(serde_json::json!([entry]));
        assert_eq!(verify(&root).unwrap(), 1);
        fs::write(&data, b"abd").unwrap();
        assert!(verify(&root).is_err());
        fs::write(&data, b"abcd").unwrap();
        assert!(verify(&root).is_err());
        fs::remove_file(&data).unwrap();
        assert!(verify(&root).is_err());
        fs::write(&data, b"abc").unwrap();
        manifest(serde_json::json!([entry, entry]));
        assert!(verify(&root).is_err());
        for path in [
            "../payload",
            "/payload",
            "C:/payload",
            "dir\\payload",
            "./payload",
            "manifest.json",
        ] {
            let mut unsafe_entry = entry.clone();
            unsafe_entry["path"] = path.into();
            manifest(serde_json::json!([unsafe_entry]));
            assert!(verify(&root).is_err(), "accepted {path}");
        }
        fs::remove_file(data).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("manifest.json"), root.join("payload")).unwrap();
            manifest(serde_json::json!([entry]));
            assert!(verify(&root).is_err());
            fs::remove_file(root.join("payload")).unwrap();
        }
        fs::remove_file(root.join("manifest.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
