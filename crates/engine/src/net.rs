//! The internet, for the assistant (docs/agent-chat.md, "The internet"): `web.fetch` reads a page
//! and `file.download` saves the CAD files a link gives. The engine makes no connection itself;
//! the desktop app installs a fetcher ([`install`]) and says whether the user allows it
//! ([`set_allowed`], Preferences → AI). Every address is checked here first ([`check_url`]):
//! HTTPS only, never a name or address on the user's own network, so a page can't steer a
//! download at a router or a service on this computer. The fetcher checks the addresses a name
//! resolves to as well ([`is_public`]).

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// What a fetch brought back.
#[derive(Clone, Debug, Default)]
pub struct Fetched {
    /// The HTTP status.
    pub status: u16,
    /// The address the body came from, after any redirects.
    pub url: String,
    /// `Content-Type`, lower-case, without parameters ("" when none).
    pub content_type: String,
    /// The file name `Content-Disposition` gives, if any.
    pub file_name: Option<String>,
    pub body: Vec<u8>,
}

/// Fetches `url` (already checked), reading at most the given number of bytes of body, or says
/// why it could not. Redirects are followed only to addresses [`check_url`] allows.
pub type Fetcher = dyn Fn(&str, usize) -> Result<Fetched, String> + Send + Sync;

static FETCHER: RwLock<Option<Arc<Fetcher>>> = RwLock::new(None);
static ALLOWED: AtomicBool = AtomicBool::new(true);
static DOWNLOADS: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Make `fetcher` this process's way onto the internet.
pub fn install(fetcher: Arc<Fetcher>) {
    if let Ok(mut f) = FETCHER.write() {
        *f = Some(fetcher);
    }
}

/// Whether the user lets the assistant use the internet (on unless turned off).
pub fn set_allowed(allowed: bool) {
    ALLOWED.store(allowed, Ordering::Relaxed);
}

pub fn allowed() -> bool {
    ALLOWED.load(Ordering::Relaxed)
}

/// Where `file.download` saves when the call names no folder: the app's choice (a Parts
/// Library's Downloads folder), else [`default_downloads`].
pub fn set_downloads_dir(dir: Option<PathBuf>) {
    if DOWNLOADS.read().is_ok_and(|d| *d == dir) {
        return;
    }
    if let Ok(mut d) = DOWNLOADS.write() {
        *d = dir;
    }
}

pub fn downloads_dir() -> Option<PathBuf> {
    DOWNLOADS.read().ok().and_then(|d| d.clone()).or_else(default_downloads)
}

/// `Downloads/SolidCraft` in the user's home folder.
pub fn default_downloads() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).filter(|h| !h.is_empty())?;
    Some(PathBuf::from(home).join("Downloads").join("SolidCraft"))
}

/// `url` fetched, at most `limit` bytes of body, or why not: the internet turned off, no fetcher
/// in this process, an address [`check_url`] refuses, or the fetcher's error.
pub fn get(url: &str, limit: usize) -> Result<Fetched, String> {
    get_with(allowed(), FETCHER.read().ok().and_then(|f| f.clone()), url, limit)
}

fn get_with(allowed: bool, fetcher: Option<Arc<Fetcher>>, url: &str, limit: usize) -> Result<Fetched, String> {
    if !allowed {
        return Err("the assistant's internet access is off (Preferences → AI)".into());
    }
    check_url(url)?;
    let fetcher = fetcher.ok_or("this SolidCraft can't reach the internet (the desktop app can)")?;
    let got = fetcher(url, limit)?;
    check_url(&got.url)?;
    Ok(got)
}

/// The host of an `https://` address, lower-case, or why it isn't one SolidCraft goes to:
/// another scheme, a user name or password in it, a port other than 443, an address on this
/// computer or its network (an IP literal that isn't public, `localhost`, `.local` and the like).
pub fn check_url(url: &str) -> Result<String, String> {
    let shown: String = url.chars().take(200).collect();
    if url.len() > 4096 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(format!("`{shown}` is not a web address"));
    }
    let rest = url
        .get(..8)
        .filter(|s| s.eq_ignore_ascii_case("https://"))
        .and_then(|_| url.get(8..))
        .ok_or_else(|| format!("`{shown}`: only https:// addresses are fetched"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.contains('@') {
        return Err(format!("`{shown}`: an address with a user name or password is not fetched"));
    }
    let (host, port) = match authority.strip_prefix('[') {
        // An IPv6 literal: never fetched (a public service has a name).
        Some(_) => return Err(format!("`{shown}`: an IP address is not fetched")),
        None => match authority.rsplit_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (authority, None),
        },
    };
    if port.is_some_and(|p| p != "443") {
        return Err(format!("`{shown}`: only the standard HTTPS port is fetched"));
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') {
        return Err(format!("`{shown}` names no host SolidCraft can look up"));
    }
    let local =
        host == "localhost" || [".localhost", ".local", ".internal", ".lan", ".home.arpa", ".intranet", ".corp"].iter().any(|s| host.ends_with(s));
    if local || !host.contains('.') {
        return Err(format!("`{shown}`: a name on this computer or its network is not fetched"));
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>()
        && !is_public(IpAddr::V4(ip))
    {
        return Err(format!("`{shown}`: an address on this computer or its network is not fetched"));
    }
    Ok(host)
}

/// Is `ip` an address on the public internet (not this computer, a private network, link-local,
/// shared, multicast, documentation or reserved)?
pub fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let [a, b, ..] = v.octets();
            !(v.is_private()
                || v.is_loopback()
                || v.is_link_local()
                || v.is_unspecified()
                || v.is_broadcast()
                || v.is_multicast()
                || v.is_documentation()
                || a == 0
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0)
                || (a == 198 && (18..20).contains(&b))
                || a >= 240)
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v.segments()[0];
            !(v.is_loopback()
                || v.is_unspecified()
                || v.is_multicast()
                || (first & 0xfe00) == 0xfc00
                || (first & 0xffc0) == 0xfe80
                || (first == 0x2001 && v.segments()[1] == 0x0db8)
                || v == Ipv6Addr::LOCALHOST)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn only_public_https_addresses_are_fetched() {
        for ok in ["https://www.example.com/parts/t200.step", "https://bluerobotics.com:443/x?y=1#z", "HTTPS://Example.COM."] {
            assert!(check_url(ok).is_ok(), "{ok}");
        }
        assert_eq!(check_url("https://Example.com/a").unwrap(), "example.com");
        for bad in [
            "http://example.com/a.step",
            "ftp://example.com/a",
            "file:///etc/passwd",
            "https://localhost/a",
            "https://printer.local/a",
            "https://router/admin",
            "https://192.168.1.1/a",
            "https://127.0.0.1/a",
            "https://10.0.0.5/a",
            "https://169.254.169.254/latest/meta-data",
            "https://100.110.6.85/a",
            "https://[::1]/a",
            "https://user:pass@example.com/a",
            "https://example.com:8443/a",
            "https://exa mple.com/a",
            "https://",
        ] {
            assert!(check_url(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn private_and_reserved_addresses_are_not_public() {
        for ip in ["93.184.216.34", "2606:2800:220:1:248:1893:25c8:1946", "1.1.1.1"] {
            assert!(is_public(ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.0.1",
            "127.0.0.1",
            "169.254.1.1",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "fe80::1",
            "::ffff:192.168.1.1",
            "240.0.0.1",
        ] {
            assert!(!is_public(ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn nothing_is_fetched_with_the_internet_off_no_fetcher_or_a_redirect_home() {
        let fetcher: Arc<Fetcher> =
            Arc::new(|url: &str, _| Ok(Fetched { status: 200, url: url.replace("example.com", "192.168.1.1"), ..Default::default() }));
        assert!(get_with(false, Some(fetcher.clone()), "https://example.com/", 10).unwrap_err().contains("off"));
        assert!(get_with(true, None, "https://example.com/", 10).unwrap_err().contains("can't reach"));
        assert!(get_with(true, Some(fetcher.clone()), "http://example.com/", 10).is_err());
        assert!(get_with(true, Some(fetcher), "https://example.com/", 10).is_err(), "ending on a private address is refused");
    }
}
