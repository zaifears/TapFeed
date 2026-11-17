//! "Update available" check against GitHub releases.

use serde::Serialize;
use std::time::Duration;

// Renamed from zaifears/tapo-viewer; GitHub redirects the old path for older builds.
const LATEST: &str = "https://api.github.com/repos/zaifears/TapFeed/releases/latest";

#[derive(Serialize, Clone)]
pub struct Release {
    pub version: String,
    pub name: String,
    pub url: String,
    pub newer: bool,
}

/// `v1.10.2` -> [1, 10, 2]; anything after a `-` or `+` is ignored.
fn parse(v: &str) -> Vec<u64> {
    v.trim()
        .trim_start_matches(['v', 'V'])
        .split(['-', '+'])
        .next()
        .unwrap_or("")
        .split('.')
        .map(|p| p.parse().unwrap_or(0))
        .collect()
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    let (mut a, mut b) = (parse(candidate), parse(current));
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    a > b
}

/// What the user should see. Automatic checks stay quiet for a skipped version;
/// "Check now" always reports.
pub fn policy(rel: Release, manual: bool, skipped: &str) -> Option<Release> {
    if !manual && rel.newer && skipped.trim_start_matches(['v', 'V']) == rel.version {
        None
    } else {
        Some(rel)
    }
}

pub fn latest() -> Result<Release, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(4)).build();
    let body: serde_json::Value = agent
        .get(LATEST)
        .set("User-Agent", concat!("TapFeed/", env!("CARGO_PKG_VERSION")))
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    let tag = body["tag_name"].as_str().unwrap_or_default().to_string();
    if tag.is_empty() {
        return Err("no tag in release".into());
    }
    Ok(Release {
        newer: is_newer(&tag, env!("CARGO_PKG_VERSION")),
        version: tag.trim_start_matches(['v', 'V']).to_string(),
        name: body["name"].as_str().unwrap_or_default().to_string(),
        url: body["html_url"]
            .as_str()
            .unwrap_or("https://github.com/zaifears/TapFeed/releases/latest")
            .to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::{is_newer, latest, policy, Release};

    fn rel(v: &str, newer: bool) -> Release {
        Release { version: v.into(), name: String::new(), url: String::new(), newer }
    }

    #[test]
    fn auto_check_respects_skip() {
        assert!(policy(rel("2.1.0", true), false, "2.1.0").is_none());
        assert!(policy(rel("2.1.0", true), false, "v2.1.0").is_none());
        assert!(policy(rel("2.2.0", true), false, "2.1.0").is_some(), "a newer release than the skipped one shows");
        assert!(policy(rel("2.1.0", true), true, "2.1.0").is_some(), "Check now ignores skip");
        assert!(!policy(rel("1.1.3", false), false, "").unwrap().newer);
    }

    /// Real request to GitHub: `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn reaches_github() {
        let r = latest().expect("GitHub release check failed");
        println!("latest on GitHub: v{} ({}) newer than {}: {}", r.version, r.url, env!("CARGO_PKG_VERSION"), r.newer);
        assert!(!r.version.is_empty());
        assert!(r.url.starts_with("https://github.com/zaifears/TapFeed"), "{}", r.url);
    }
    #[test]
    fn compares_versions() {
        assert!(is_newer("v2.0.1", "2.0.0"));
        assert!(is_newer("v1.10.0", "1.9.9"));
        assert!(!is_newer("v1.1.3", "2.0.0"));
        assert!(!is_newer("v2.0.0", "2.0.0"));
        assert!(is_newer("2.1", "2.0.9"));
    }
}
