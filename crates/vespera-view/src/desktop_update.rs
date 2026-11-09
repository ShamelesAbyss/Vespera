//! Release selection for the official Windows desktop update channel.

use std::sync::{Mutex, mpsc};
use std::time::Duration;

use bevy::prelude::Resource;
use serde_json::Value;

const REPOSITORY: &str = "ShamelesAbyss/Vespera";
const RELEASE_API: &str = "https://api.github.com/repos/ShamelesAbyss/Vespera/releases/latest";
const MAX_METADATA: usize = 256 * 1024;
const MAX_SETUP: u64 = 512 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
struct Candidate {
    version: String,
    url: String,
    bytes: u64,
    sha256: String,
}

type Check = Result<Option<Candidate>, &'static str>;

#[derive(Resource)]
pub(super) struct Updates {
    pending: Option<Mutex<mpsc::Receiver<Check>>>,
    pub status: String,
}

impl Default for Updates {
    fn default() -> Self {
        Self {
            pending: None,
            status: format!("Running Vespera {}", env!("CARGO_PKG_VERSION")),
        }
    }
}

impl Updates {
    pub(super) fn check(&mut self) {
        if self.pending.is_some() {
            return;
        }
        if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
            self.status = "The desktop update channel is available on Windows x64.".into();
            return;
        }
        let (send, receive) = mpsc::sync_channel(1);
        self.pending = Some(Mutex::new(receive));
        self.status = "Checking the official Vespera release channel...".into();
        std::thread::spawn(move || {
            let result = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|_| "Could not start the update check.")
                .and_then(|runtime| runtime.block_on(check_release()));
            let _ = send.send(result);
        });
    }

    pub(super) fn poll(&mut self) {
        let Some(pending) = &mut self.pending else {
            return;
        };
        let result = match pending
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .try_recv()
        {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("The update check stopped unexpectedly."),
        };
        self.pending = None;
        self.status = match result {
            Ok(Some(candidate)) => format!(
                "Vespera {} is available from the official release channel.",
                candidate.version
            ),
            Ok(None) => format!(
                "No newer published release found for Vespera {}.",
                env!("CARGO_PKG_VERSION")
            ),
            Err(error) => error.into(),
        };
    }
}

async fn check_release() -> Check {
    let client = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("Vespera/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|_| "Could not initialize the update connection.")?;
    let mut response = client
        .get(RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|_| "Could not reach the Vespera release channel.")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if response.status() != reqwest::StatusCode::OK {
        return Err("The Vespera release channel returned an error.");
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the release metadata.")?
    {
        if body.len() + chunk.len() > MAX_METADATA {
            return Err("Release metadata exceeded the permitted size.");
        }
        body.extend_from_slice(&chunk);
    }
    let release = serde_json::from_slice(&body).map_err(|_| "Release metadata was invalid.")?;
    select_candidate(&release, env!("CARGO_PKG_VERSION"))
}

fn version(text: &str) -> Option<[u32; 3]> {
    let mut parts = text.split('.');
    let mut result = [0; 3];
    for part in &mut result {
        let raw = parts.next()?;
        if !raw.bytes().all(|byte| byte.is_ascii_digit()) || (raw.starts_with('0') && raw.len() > 1)
        {
            return None;
        }
        *part = raw.parse().ok()?;
    }
    parts.next().is_none().then_some(result)
}

fn select_candidate(release: &Value, current: &str) -> Check {
    let tag = release["tag_name"]
        .as_str()
        .ok_or("The release has no version tag.")?;
    let text = tag
        .strip_prefix('v')
        .ok_or("The release has an unsupported version tag.")?;
    let newer = version(text).ok_or("The release has an unsupported version tag.")?;
    let installed =
        version(current).ok_or("The installed version is unsupported by the update channel.")?;
    if release["draft"].as_bool() != Some(false) || release["prerelease"].as_bool() != Some(false) {
        return Err("The release is not a published stable update.");
    }
    if newer <= installed {
        return Ok(None);
    }
    if release["immutable"].as_bool() != Some(true) {
        return Err("The newer release is not immutable. No update selected.");
    }
    let name = format!("Vespera-Setup-{text}.exe");
    let assets = release["assets"]
        .as_array()
        .ok_or("The release has no installer assets.")?;
    let mut matching = assets
        .iter()
        .filter(|asset| asset["name"].as_str() == Some(&name));
    let asset = matching
        .next()
        .ok_or("The release has no matching Windows installer.")?;
    let url = format!("https://github.com/{REPOSITORY}/releases/download/{tag}/{name}");
    if matching.next().is_some()
        || asset["state"].as_str() != Some("uploaded")
        || asset["browser_download_url"].as_str() != Some(&url)
    {
        return Err("The release installer identity was invalid.");
    }
    let bytes = asset["size"]
        .as_u64()
        .filter(|bytes| (1..=MAX_SETUP).contains(bytes))
        .ok_or("The release installer size was invalid.")?;
    let sha256 = asset["digest"]
        .as_str()
        .and_then(|value| value.strip_prefix("sha256:"))
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or("The release installer has no valid SHA-256 digest.")?
        .to_ascii_lowercase();
    Ok(Some(Candidate {
        version: text.into(),
        url,
        bytes,
        sha256,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_a_newer_immutable_stable_release_and_exact_installer_identity() {
        let mut release = serde_json::json!({
            "tag_name":"v0.1.10", "draft":false, "prerelease":false, "immutable":true,
            "assets":[{"name":"Vespera-Setup-0.1.10.exe", "state":"uploaded", "size":100,
                "digest":format!("sha256:{}", "a".repeat(64)),
                "browser_download_url":"https://github.com/ShamelesAbyss/Vespera/releases/download/v0.1.10/Vespera-Setup-0.1.10.exe"}]
        });
        assert_eq!(
            select_candidate(&release, "0.1.9")
                .unwrap()
                .unwrap()
                .version,
            "0.1.10"
        );
        assert!(select_candidate(&release, "0.1.11").unwrap().is_none());
        release["immutable"] = false.into();
        assert!(select_candidate(&release, "0.1.9").is_err());
        release["immutable"] = true.into();
        let good = release["assets"][0].clone();
        release["assets"][0]["digest"] = "sha256:invalid".into();
        assert!(select_candidate(&release, "0.1.9").is_err());
        release["assets"] = serde_json::json!([good, good]);
        assert!(select_candidate(&release, "0.1.9").is_err());
        release["assets"] = serde_json::json!([good]);
        release["assets"][0]["browser_download_url"] = "https://example.org/Setup.exe".into();
        assert!(select_candidate(&release, "0.1.9").is_err());
    }
}
