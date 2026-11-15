//! Redirect policy for Hub and Xet token requests.
//!
//! Every request is built on one client with this policy attached. Redirects
//! are followed only to the Hugging Face hosts, the Xet CAS host, or the host
//! the client was configured with (a mirror or a local test server). The chain
//! is capped and plain HTTP is only tolerated when it stays on the configured
//! endpoint host, which keeps local test servers usable without weakening the
//! rule for public hosts.

use reqwest::Url;

/// Maximum number of redirect hops that will be followed.
pub(crate) const MAX_REDIRECTS: usize = 8;

/// Hosts that any Hub client may be redirected to.
const PUBLIC_HOSTS: [&str; 2] = ["huggingface.co", "cas-server.xethub.hf.co"];

/// Whether a host is a Hugging Face or Xet host that may receive redirects.
pub(crate) fn host_is_public(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    PUBLIC_HOSTS.iter().any(|allowed| host == *allowed)
        || host == "hf.co"
        || host.ends_with(".hf.co")
}

/// Whether `next` is an acceptable redirect target for a client pointed at
/// `endpoint_host`.
pub(crate) fn target_allowed(next: &Url, endpoint_host: &str) -> bool {
    let Some(host) = next.host_str() else {
        return false;
    };
    let same_as_endpoint = host.eq_ignore_ascii_case(endpoint_host);
    if !host_is_public(host) && !same_as_endpoint {
        return false;
    }
    match next.scheme() {
        "https" => true,
        // Local test servers speak plain HTTP; an HTTP mirror may too. Either
        // way the request never leaves the host the caller configured.
        "http" => same_as_endpoint,
        _ => false,
    }
}

/// Render a URL for an error message without user information.
pub(crate) fn display_url(url: &Url) -> String {
    if url.username().is_empty() && url.password().is_none() {
        return url.to_string();
    }
    let mut redacted = url.clone();
    let _ = redacted.set_username("");
    let _ = redacted.set_password(None);
    redacted.to_string()
}

/// Build a redirect policy pinned to the configured endpoint host.
pub(crate) fn policy(endpoint_host: String) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error("too many redirects");
        }
        let next = attempt.url();
        if target_allowed(next, &endpoint_host) {
            attempt.follow()
        } else {
            let reason = format!("redirect target {} is not allowed", display_url(next));
            attempt.error(reason)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(value: &str) -> Url {
        Url::parse(value).unwrap()
    }

    #[test]
    fn public_hosts_are_allowed() {
        assert!(host_is_public("huggingface.co"));
        assert!(host_is_public("HF.CO"));
        assert!(host_is_public("cdn-lfs.hf.co"));
        assert!(host_is_public("cas-server.xethub.hf.co"));
        assert!(!host_is_public("huggingface.co.evil.example"));
        assert!(!host_is_public("evil.example"));
        assert!(!host_is_public("hf.co.evil.example"));
    }

    #[test]
    fn https_redirects_to_public_hosts_are_allowed() {
        assert!(target_allowed(
            &url("https://huggingface.co/api/models/x"),
            "huggingface.co"
        ));
        assert!(target_allowed(
            &url("https://us.aws.cdn.hf.co/xet-bridge-us/file"),
            "huggingface.co"
        ));
        assert!(!target_allowed(
            &url("https://evil.example/file"),
            "huggingface.co"
        ));
    }

    #[test]
    fn plain_http_is_only_allowed_on_the_configured_host() {
        assert!(target_allowed(
            &url("http://127.0.0.1:8080/canonical"),
            "127.0.0.1"
        ));
        assert!(!target_allowed(
            &url("http://127.0.0.1:8080/canonical"),
            "huggingface.co"
        ));
        assert!(!target_allowed(
            &url("http://other.example/canonical"),
            "127.0.0.1"
        ));
    }

    #[test]
    fn non_http_schemes_are_refused() {
        assert!(!target_allowed(
            &url("ftp://huggingface.co/x"),
            "huggingface.co"
        ));
        assert!(!target_allowed(
            &url("file:///etc/passwd"),
            "huggingface.co"
        ));
    }

    #[test]
    fn display_url_drops_credentials() {
        let rendered = display_url(&url("https://user:secret@example.com/path"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("user"));
        assert!(rendered.starts_with("https://example.com/path"));
    }
}
