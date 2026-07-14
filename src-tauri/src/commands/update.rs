use mhost_core::MhostError;
use serde::{Deserialize, Serialize};

/// Response shape from GitHub Releases API
/// GET https://api.github.com/repos/{owner}/{repo}/releases/latest
#[derive(Deserialize, Debug)]
struct GithubReleaseResponse {
    tag_name: String,
    html_url: String,
    name: Option<String>,
    body: Option<String>,
}

/// Latest release info fetched from GitHub Releases.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LatestRelease {
    /// The GitHub tag, e.g. "v0.3.3". Always prefixed with "v".
    pub tag: String,
    /// URL to the release page on GitHub.
    pub url: String,
    /// Release title (subject to localization).
    pub title: Option<String>,
    /// Release notes body.
    pub body: Option<String>,
}

/// Check whether a newer mHost release exists on GitHub.
///
/// Compares `current_version` against the `tag_name` of the latest GitHub
/// release. Returns the release if a newer version exists, or an error if
/// already up-to-date or if the check failed.
#[tauri::command]
pub async fn check_update(current_version: String) -> Result<LatestRelease, MhostError> {
    let latest = tauri::async_runtime::spawn_blocking(move || fetch_latest(current_version))
        .await
        .map_err(|e| MhostError::InvalidInput(e.to_string()))?;
    latest
}

/// Fetches the latest GitHub release (runs in blocking thread).
fn fetch_latest(current_version: String) -> Result<LatestRelease, MhostError> {
    let client = reqwest::blocking::Client::builder()
        .user_agent("mHost-Desktop/1.0")
        .build()
        .map_err(|e| MhostError::InvalidInput(format!("reqwest build error: {}", e)))?;

    let url = "https://api.github.com/repos/igevin/mHost/releases/latest";
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|e| MhostError::InvalidInput(format!("network error: {}", e)))?;

    if !resp.status().is_success() {
        return Err(MhostError::InvalidInput(format!(
            "GitHub API error: {}",
            resp.status()
        )));
    }

    let gh: GithubReleaseResponse = resp
        .json()
        .map_err(|e| MhostError::InvalidInput(format!("failed to parse GitHub response: {}", e)))?;

    let latest = LatestRelease {
        tag: gh.tag_name,
        url: gh.html_url,
        title: gh.name,
        body: gh.body,
    };

    // Strip leading "v" prefix for comparison.
    let latest_version = latest.tag.trim_start_matches('v');
    if is_newer(&current_version, latest_version) {
        Ok(latest)
    } else {
        Err(MhostError::InvalidInput("already_up_to_date".to_string()))
    }
}

/// Returns true if `latest` is strictly greater than `current`.
fn is_newer(current: &str, latest: &str) -> bool {
    let current = current.trim_start_matches('v');
    let latest = latest.trim_start_matches('v');

    let mut current_parts = current.split('.').fuse();
    let mut latest_parts = latest.split('.').fuse();

    loop {
        match (current_parts.next(), latest_parts.next()) {
            (None, None) => return false,
            (None, Some(_)) => return false,
            (Some(_), None) => return true,
            (Some(c), Some(l)) => {
                let c: u64 = c.parse().unwrap_or(0);
                let l: u64 = l.parse().unwrap_or(0);
                if l != c {
                    return l > c;
                }
            }
        }
    }
}
