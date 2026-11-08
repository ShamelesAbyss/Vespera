//! Build and stage a local, allowlisted release candidate for operator validation.

use std::collections::BTreeMap;
use std::error::Error;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Parser;
use ring::digest::{Context, SHA256};
use serde::Serialize;
use serde_json::Value;

const DOCUMENTS: &[&str] = &[
    "LICENSE",
    "README.md",
    "artifacts/live-ollama-2026-11-06.json",
    "artifacts/live-ollama-context-2026-11-06.json",
    "artifacts/live-workflow-2026-11-06.json",
    "assets/branding/vespera-original.jpg",
    "assets/branding/vespera.png",
    "assets/branding/vespera.ico",
    "assets/branding/vespera.icns",
    "config/local.toml",
    "config/world.example.json",
    "config/workflow.example.json",
    "config/decomposition.example.json",
    "docs/AGENTS.md",
    "docs/ARCHITECTURE.md",
    "docs/CLI.md",
    "docs/DEPENDENCIES.md",
    "docs/DEVELOPMENT_LOG.md",
    "docs/IMPLEMENTATION_REPORT.md",
    "docs/INITIAL_PLAN.md",
    "docs/MEMORY.md",
    "docs/MILESTONE_LEDGER.md",
    "docs/ORCHESTRATION.md",
    "docs/PROTOCOL.md",
    "docs/PROVIDERS.md",
    "docs/REGISTRY.md",
    "docs/RELEASE.md",
    "docs/ROADMAP.md",
    "docs/SCHEDULER.md",
];

#[derive(Parser)]
#[command(about = "Stage a verified-input local Vespera release candidate")]
struct Args {
    /// New directory for the candidate. Its parent must exist.
    #[arg(long)]
    output: PathBuf,
    /// Build the CLI without the optional Atlas executable.
    #[arg(long)]
    headless: bool,
    /// Require Cargo to use cached dependencies only.
    #[arg(long)]
    offline: bool,
}

#[derive(Serialize)]
struct FileRecord {
    path: String,
    bytes: u64,
    sha256: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    if args.output.symlink_metadata().is_ok() {
        return Err(io::Error::new(io::ErrorKind::AlreadyExists, "output already exists").into());
    }
    let parent = args
        .output
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if !parent.is_dir() {
        return Err(io::Error::new(io::ErrorKind::NotFound, "output parent is missing").into());
    }
    let revision = clean_revision(&root)?;
    let names: &[&str] = if args.headless {
        &["vespera"]
    } else {
        &["vespera", "vespera-view", "vespera-launcher"]
    };
    let artifacts = build(&root, &args, names)?;
    if clean_revision(&root)? != revision {
        return Err(io::Error::other("source revision changed during build").into());
    }
    fs::create_dir(&args.output)?;
    let mut records = Vec::with_capacity(DOCUMENTS.len() + names.len());
    for name in names {
        let source = &artifacts[*name];
        let relative =
            Path::new("bin").join(source.file_name().ok_or_else(|| {
                io::Error::other("Cargo returned an executable without a file name")
            })?);
        let destination = create_parent(&args.output, &relative)?;
        fs::copy(source, &destination)?;
        records.push(record(&relative, &destination)?);
    }
    for &relative in DOCUMENTS {
        let specification = format!("{revision}:{relative}");
        let bytes = git(&root, &["show", &specification])?;
        let relative = Path::new(relative);
        let destination = create_parent(&args.output, relative)?;
        fs::write(&destination, bytes)?;
        records.push(record(relative, &destination)?);
    }
    let lockfile = git(&root, &["show", &format!("{revision}:Cargo.lock")])?;
    let manifest = serde_json::json!({
        "schema_version": 1,
        "source_revision": revision,
        "vespera_version": env!("CARGO_PKG_VERSION"),
        "build": "cargo release, locked",
        "build_host_os": std::env::consts::OS,
        "build_host_arch": std::env::consts::ARCH,
        "graphics": !args.headless,
        "cargo_lock_sha256": hex(ring::digest::digest(&SHA256, &lockfile).as_ref()),
        "files": records,
        "scope": "local release validation",
    });
    serde_json::to_writer_pretty(File::create(args.output.join("manifest.json"))?, &manifest)?;
    println!("Staged local candidate at {}", args.output.display());
    Ok(())
}

fn git(root: &Path, args: &[&str]) -> io::Result<Vec<u8>> {
    let output = Command::new("git").current_dir(root).args(args).output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!("git {} failed", args[0])));
    }
    Ok(output.stdout)
}

fn clean_revision(root: &Path) -> Result<String, Box<dyn Error>> {
    if !git(root, &["status", "--porcelain", "--untracked-files=no"])?.is_empty() {
        return Err(io::Error::other("commit or stash tracked changes before packaging").into());
    }
    Ok(
        String::from_utf8(git(root, &["rev-parse", "--verify", "HEAD"])?)?
            .trim()
            .to_owned(),
    )
}

fn build(
    root: &Path,
    args: &Args,
    names: &[&str],
) -> Result<BTreeMap<String, PathBuf>, Box<dyn Error>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut command = Command::new(cargo);
    command.current_dir(root).stderr(Stdio::inherit()).args([
        "build",
        "--release",
        "--locked",
        "--message-format=json-render-diagnostics",
        "-p",
        "vespera-app",
        "--bin",
        "vespera",
    ]);
    if !args.headless {
        command.args([
            "-p",
            "vespera-view",
            "--features",
            "vespera-view/graphics",
            "--bin",
            "vespera-view",
            "--bin",
            "vespera-launcher",
        ]);
    }
    if args.offline {
        command.arg("--offline");
    }
    let result = command.output()?;
    if !result.status.success() {
        io::Write::write_all(&mut io::stderr(), &result.stdout)?;
        return Err(io::Error::other("locked release build failed").into());
    }
    executable_artifacts(&result.stdout, names)
}

fn executable_artifacts(
    output: &[u8],
    names: &[&str],
) -> Result<BTreeMap<String, PathBuf>, Box<dyn Error>> {
    let mut artifacts = BTreeMap::new();
    for line in output
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let entry: Value = serde_json::from_slice(line)?;
        if entry["reason"] == "compiler-artifact"
            && let (Some(name), Some(path)) = (
                entry["target"]["name"].as_str(),
                entry["executable"].as_str(),
            )
            && names.contains(&name)
            && entry["target"]["kind"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"))
            && artifacts
                .insert(name.to_owned(), PathBuf::from(path))
                .is_some()
        {
            return Err(io::Error::other("Cargo returned duplicate executable artifacts").into());
        }
    }
    if artifacts.len() != names.len() {
        return Err(io::Error::other("Cargo did not return every requested executable").into());
    }
    Ok(artifacts)
}

fn create_parent(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().expect("file in package root"))?;
    Ok(path)
}

fn record(relative: &Path, file: &Path) -> io::Result<FileRecord> {
    let mut source = File::open(file)?;
    let mut digest = Context::new(&SHA256);
    let mut buffer = [0; 32768];
    let mut bytes = 0;
    loop {
        let read = source.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        digest.update(&buffer[..read]);
    }
    Ok(FileRecord {
        path: relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        bytes,
        sha256: hex(digest.finish().as_ref()),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_only_requested_cargo_binaries_and_rejects_missing_or_duplicate_artifacts() {
        let messages = br#"{"reason":"compiler-artifact","target":{"name":"vespera","kind":["bin"]},"executable":"/out/vespera"}
{"reason":"compiler-artifact","target":{"name":"vespera-view","kind":["bin"]},"executable":"/out/vespera-view"}
{"reason":"compiler-artifact","target":{"name":"vespera-launcher","kind":["bin"]},"executable":"/out/vespera-launcher"}
{"reason":"compiler-artifact","target":{"name":"vespera","kind":["example"]},"executable":"/out/example"}
{"reason":"build-finished","success":true}
"#;
        let artifacts =
            executable_artifacts(messages, &["vespera", "vespera-view", "vespera-launcher"])
                .unwrap();
        assert_eq!(artifacts["vespera"], Path::new("/out/vespera"));
        assert_eq!(artifacts["vespera-view"], Path::new("/out/vespera-view"));
        assert_eq!(
            artifacts["vespera-launcher"],
            Path::new("/out/vespera-launcher")
        );
        assert!(executable_artifacts(messages, &["other"]).is_err());
        assert!(executable_artifacts(&messages.repeat(2), &["vespera"]).is_err());
    }
}
