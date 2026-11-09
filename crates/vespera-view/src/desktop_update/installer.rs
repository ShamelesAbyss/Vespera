use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use ring::digest::{Context, SHA256};

use super::Candidate;

pub(super) fn standard_installation() -> bool {
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return false;
    }
    let expected = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .and_then(|base| {
            base.join("Programs/Vespera/bin/vespera-view.exe")
                .canonicalize()
                .ok()
        });
    expected.is_some()
        && std::env::current_exe()
            .ok()
            .and_then(|path| path.canonicalize().ok())
            == expected
}

struct Staging {
    dir: PathBuf,
    file: PathBuf,
    handed_off: bool,
}

impl Staging {
    fn new() -> std::io::Result<Self> {
        let dir =
            std::env::temp_dir().join(format!("vespera-update-{}", vespera_core::TaskId::new()));
        fs::create_dir(&dir)?;
        let file = dir.join("Vespera-Setup.exe");
        Ok(Self {
            dir,
            file,
            handed_off: false,
        })
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        if !self.handed_off {
            let _ = fs::remove_file(&self.file);
            let _ = fs::remove_dir(&self.dir);
        }
    }
}

pub(crate) struct VerifiedSetup {
    // Windows denies writes and deletion while this handle is held.
    _locked: File,
    staging: Staging,
}

impl VerifiedSetup {
    pub(crate) fn launch(mut self) -> std::io::Result<Child> {
        let child = Command::new(&self.staging.file)
            .current_dir(&self.staging.dir)
            .spawn()?;
        self.staging.handed_off = true;
        Ok(child)
    }
}

fn approved_redirect(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some("release-assets.githubusercontent.com" | "objects.githubusercontent.com")
        )
}

pub(super) async fn prepare(
    candidate: Candidate,
    signer_pins: &str,
) -> Result<VerifiedSetup, &'static str> {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() <= 3 && approved_redirect(attempt.url()) {
                attempt.follow()
            } else {
                attempt.error("Unexpected installer redirect")
            }
        }))
        .timeout(Duration::from_secs(300))
        .connect_timeout(Duration::from_secs(12))
        .user_agent(concat!("Vespera/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Could not initialize the Setup download.")?;
    let mut response = client
        .get(&candidate.url)
        .send()
        .await
        .map_err(|_| "Could not download Setup from GitHub.")?;
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|size| size != candidate.bytes)
    {
        return Err("GitHub returned an unexpected Setup response.");
    }
    let staging = Staging::new().map_err(|_| "Could not create a temporary update directory.")?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staging.file)
        .map_err(|_| "Could not stage the installer.")?;
    let mut bytes = 0_u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "The Setup download was interrupted.")?
    {
        bytes += chunk.len() as u64;
        if bytes > candidate.bytes {
            return Err("The installer exceeded its published size.");
        }
        file.write_all(&chunk)
            .map_err(|_| "Could not write the staged installer.")?;
    }
    drop(file);
    verify(staging, &candidate, signer_pins)
}

fn verify(
    staging: Staging,
    candidate: &Candidate,
    signer_pins: &str,
) -> Result<VerifiedSetup, &'static str> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1); // FILE_SHARE_READ only.
    }
    let mut locked = options
        .open(&staging.file)
        .map_err(|_| "Could not lock the staged installer for verification.")?;
    verify_digest(&mut locked, candidate)?;
    verify_signature(&staging.file, signer_pins)?;
    Ok(VerifiedSetup {
        _locked: locked,
        staging,
    })
}

fn verify_digest(mut reader: impl Read, candidate: &Candidate) -> Result<(), &'static str> {
    let mut digest = Context::new(&SHA256);
    let mut buffer = [0; 32 * 1024];
    let mut bytes = 0;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|_| "Could not read the staged installer.")?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        if bytes > candidate.bytes {
            return Err("Setup size does not match the published release.");
        }
        digest.update(&buffer[..count]);
    }
    use std::fmt::Write as _;
    let mut hex = String::with_capacity(64);
    for byte in digest.finish().as_ref() {
        let _ = write!(hex, "{byte:02x}");
    }
    if bytes != candidate.bytes || hex != candidate.sha256 {
        return Err("Setup size or SHA-256 does not match the published release.");
    }
    Ok(())
}

#[cfg(windows)]
fn verify_signature(path: &Path, signer_pins: &str) -> Result<(), &'static str> {
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use std::time::Instant;

    let powershell_home = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or("Windows signature verification is unavailable.")?
        .join("System32/WindowsPowerShell/v1.0");
    let mut child = Command::new(powershell_home.join("powershell.exe"))
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command",
            r#"
            $ErrorActionPreference = 'Stop'
            $s = Microsoft.PowerShell.Security\Get-AuthenticodeSignature -LiteralPath $env:VESPERA_SETUP_VERIFY
            if ($s.Status -ne 'Valid' -or $s.SignatureType -ne 'Authenticode' -or !$s.SignerCertificate) { exit 1 }
            $sha256 = [System.Security.Cryptography.SHA256]::Create()
            $fingerprint = [System.BitConverter]::ToString($sha256.ComputeHash($s.SignerCertificate.RawData)).Replace('-', '')
            if ($env:VESPERA_SETUP_SIGNERS.Split(',') -contains $fingerprint) { exit 0 }
            exit 2
            "#])
        .env("PSModulePath", powershell_home.join("Modules"))
        .env("VESPERA_SETUP_VERIFY", path)
        .env("VESPERA_SETUP_SIGNERS", signer_pins)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .spawn().map_err(|_| "Windows signature verification is unavailable.")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) if status.code() == Some(2) => {
                return Err("Setup was not signed by an approved Vespera publisher.");
            }
            Ok(Some(_)) => return Err("Setup does not have a trusted Windows code signature."),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(100)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("Windows could not complete Setup signature verification.");
            }
        }
    }
}

#[cfg(not(windows))]
fn verify_signature(_: &Path, _: &str) -> Result<(), &'static str> {
    Err("Windows signature verification is unavailable on this platform.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    const UNRELATED_PIN: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    #[test]
    fn rejects_a_truncated_or_modified_payload_and_unapproved_redirects() {
        let candidate = Candidate {
            version: "0.2.0".into(),
            url: String::new(),
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        assert!(verify_digest(&b"abc"[..], &candidate).is_ok());
        for invalid in [b"ab".as_slice(), b"abd", b"abcd"] {
            assert!(verify_digest(invalid, &candidate).is_err());
        }
        assert!(approved_redirect(
            &"https://release-assets.githubusercontent.com/path"
                .parse()
                .unwrap()
        ));
        assert!(!approved_redirect(
            &"https://release-assets.githubusercontent.com.example.org/path"
                .parse()
                .unwrap()
        ));
        assert!(!approved_redirect(
            &"http://release-assets.githubusercontent.com/path"
                .parse()
                .unwrap()
        ));
    }

    #[cfg(windows)]
    #[test]
    fn signature_verification_rejects_unsigned_content() {
        let stage = Staging::new().unwrap();
        fs::write(&stage.file, b"unsigned setup fixture").unwrap();
        assert_eq!(
            verify_signature(&stage.file, UNRELATED_PIN),
            Err("Setup does not have a trusted Windows code signature.")
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "downloads a 67 MB signed GitHub fixture and never runs it"]
    fn rejects_a_signed_github_release_asset_from_an_unrelated_publisher() {
        // A pinned Git for Windows release exercises the real GitHub CDN redirect
        // and Windows trust check while the Vespera channel has no signed asset.
        let candidate = Candidate {
            version: "2.56.0.4".into(),
            url: "https://github.com/git-for-windows/git/releases/download/v2.56.0.windows.4/Git-2.56.0.4-64-bit.exe".into(),
            bytes: 66_850_032,
            sha256: "4961a4a8cc778d8b6e00a5819e5fe4d078b754555aff7aedc2d77a5d74c2de80".into(),
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        assert_eq!(
            runtime.block_on(prepare(candidate, UNRELATED_PIN)).err(),
            Some("Setup was not signed by an approved Vespera publisher.")
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires a trusted embedded-signed Windows executable fixture"]
    fn verifies_and_locks_a_trusted_signed_fixture_without_launching_it() {
        let source = PathBuf::from(
            std::env::var_os("VESPERA_SIGNED_FIXTURE").expect("set a signed fixture executable"),
        );
        let verified = verified_fixture(&source);
        let path = verified.staging.file.clone();
        assert!(OpenOptions::new().write(true).open(&path).is_err());
        drop(verified);
        assert!(!path.exists());
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires Windows SDK SignTool with an embedded Microsoft signature"]
    fn launches_the_verified_executable_from_staging() {
        let source = PathBuf::from(
            std::env::var_os("VESPERA_SIGNTOOL_FIXTURE")
                .expect("set the Windows SDK SignTool path"),
        );
        assert_eq!(
            source.file_name(),
            Some(std::ffi::OsStr::new("signtool.exe"))
        );
        let verified = verified_fixture(&source);
        let path = verified.staging.file.clone();
        let dir = verified.staging.dir.clone();
        // SignTool with no arguments reports usage and exits without acting on any files.
        let status = verified.launch().unwrap().wait().unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(dir).unwrap();
        assert_eq!(status.code(), Some(1));
    }

    #[cfg(windows)]
    fn verified_fixture(source: &Path) -> VerifiedSetup {
        let bytes = fs::read(source).unwrap();
        let hash = ring::digest::digest(&SHA256, &bytes);
        let candidate = Candidate {
            version: "0.2.0".into(),
            url: String::new(),
            bytes: bytes.len() as u64,
            sha256: hash
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        };
        let stage = Staging::new().unwrap();
        fs::write(&stage.file, &bytes).unwrap();
        let fingerprint = std::env::var("VESPERA_TEST_SIGNER_SHA256")
            .expect("set the signed fixture certificate's SHA-256 fingerprint");
        verify(stage, &candidate, &fingerprint)
            .expect("trusted pinned signature and digest must verify")
    }
}
