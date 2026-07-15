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
async fn fetch_latest(current_version: String) -> Result<Option<LatestRelease>, MhostError> {
    let client = reqwest::Client::builder()
        .user_agent("mHost-Desktop/1.0")
        .build()
        .map_err(|e| MhostError::Network(format!("reqwest build error: {}", e)))?;

    let url = "https://api.github.com/repos/igevin/mHost/releases/latest";
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| MhostError::Network(format!("network error: {}", e)))?;

    if !resp.status().is_success() {
        return Err(MhostError::ExternalApi(format!(
            "GitHub API error: {}",
            resp.status()
        )));
    }

    let gh: GithubReleaseResponse = resp
        .json()
        .await
        .map_err(|e| MhostError::ExternalApi(format!("failed to parse GitHub response: {}", e)))?;

    let latest = LatestRelease {
        tag: gh.tag_name,
        url: gh.html_url,
        title: gh.name,
        body: gh.body,
    };

    // Strip leading "v" prefix for comparison.
    let latest_version = latest.tag.trim_start_matches('v');
    if is_newer(&current_version, latest_version) {
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
