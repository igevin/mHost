use std::time::Duration;

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
/// release. Returns `Some(latest)` if a newer version exists, `None` if the
/// current version is already up-to-date, or an error if the network/check
/// failed.
#[tauri::command]
pub async fn check_update(current_version: String) -> Result<Option<LatestRelease>, MhostError> {
    fetch_latest(current_version).await
}

/// Fetches the latest GitHub release.
///
/// Runs the blocking `ureq` call inside `spawn_blocking` so the tokio
/// worker pool isn't tied up waiting on the network. `ureq` is sync
/// (issue #180 — replacing `reqwest` shrank the dependency graph by
/// removing h2 / aws-lc-sys / hyper transitively).
async fn fetch_latest(current_version: String) -> Result<Option<LatestRelease>, MhostError> {
    tokio::task::spawn_blocking(move || fetch_latest_blocking(&current_version))
        .await
        .map_err(|e| MhostError::Network(format!("spawn_blocking join: {}", e)))?
}

fn fetch_latest_blocking(current_version: &str) -> Result<Option<LatestRelease>, MhostError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .user_agent("mHost-Desktop/1.0")
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();

    let url = "https://api.github.com/repos/igevin/mHost/releases/latest";
    let resp = agent
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call();

    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => {
            return Err(MhostError::ExternalApi(format!(
                "GitHub API error: {}",
                code
            )));
        }
        Err(e) => {
            return Err(MhostError::Network(format!("network error: {}", e)));
        }
    };

    // Drain the body into a String so any JSON parse error stays close to the
    // raw bytes. Use ureq's built-in `read_json` with a 8 MB cap on the
    // response body (the GitHub release endpoint returns <100 KB in practice
    // — release notes + body — but we cap generously for safety).
    let mut resp = resp;
    let gh: GithubReleaseResponse = resp
        .body_mut()
        .with_config()
        .limit(8 * 1024 * 1024)
        .read_json()
        .map_err(|e| MhostError::ExternalApi(format!("read GitHub response JSON: {}", e)))?;

    let latest = LatestRelease {
        tag: gh.tag_name,
        url: gh.html_url,
        title: gh.name,
        body: gh.body,
    };

    // Strip leading "v" prefix for comparison.
    let latest_version = latest.tag.trim_start_matches('v');
    if is_newer(current_version, latest_version) {
        Ok(Some(latest))
    } else {
        Ok(None)
    }
}

/// Returns true if `latest` is strictly greater than `current`.
/// Both strings are already stripped of any leading "v" prefix.
fn is_newer(current: &str, latest: &str) -> bool {
    let mut current_parts = current.split('.').fuse();
    let mut latest_parts = latest.split('.').fuse();

    loop {
        match (current_parts.next(), latest_parts.next()) {
            (None, None) => return false,    // equal
            (None, Some(_)) => return false, // current shorter → e.g. "1.0" vs "1.0.1" → not newer
            (Some(_), None) => return true,  // current longer → e.g. "1.0.1" vs "1.0" → newer
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        // Equal
        assert!(!is_newer("0.3.2", "0.3.2"));
        assert!(!is_newer("v0.3.2", "v0.3.2"));

        // Major/minor/patch increases
        assert!(is_newer("0.3.2", "0.3.3"));
        assert!(is_newer("0.3.2", "0.4.0"));
        assert!(is_newer("0.3.2", "1.0.0"));
        assert!(!is_newer("0.3.3", "0.3.2"));
        assert!(!is_newer("0.4.0", "0.3.2"));

        // Unequal length (real-world GitHub tags are always 3-part: x.y.z)
        assert!(!is_newer("0.3", "0.3.1")); // current shorter → not newer
        assert!(is_newer("0.3.1", "0.3")); // current longer → newer
    }
}
