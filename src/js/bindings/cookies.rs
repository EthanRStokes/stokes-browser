// Cookie implementation for browser storage.
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

const STORAGE_VERSION: u32 = 3;
const MAX_COOKIE_NAME_LEN: usize = 1024;
const MAX_COOKIE_VALUE_LEN: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CookieSource {
    Document,
    Response,
}

#[derive(Clone, Debug)]
struct CookieInput {
    name: String,
    value: String,
    attrs: Vec<(String, Option<String>)>,
}

fn get_cookies_file_path() -> PathBuf {
    static COOKIES_PATH: OnceLock<PathBuf> = OnceLock::new();
    COOKIES_PATH
        .get_or_init(|| {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("stokes-browser");

            if let Err(err) = std::fs::create_dir_all(&config_dir) {
                eprintln!(
                    "[Cookies] Warning: failed to create config directory {}: {}",
                    config_dir.display(),
                    err
                );
            }

            config_dir.join("cookies.json")
        })
        .clone()
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn saturating_add_millis(base_ms: u64, delta_seconds: i64) -> u64 {
    if delta_seconds <= 0 {
        return 0;
    }

    let delta_ms = (delta_seconds as u128).saturating_mul(1000);
    let total = (base_ms as u128).saturating_add(delta_ms);
    total.min(u64::MAX as u128) as u64
}

fn normalize_host(host: &str) -> String {
    host.trim().trim_end_matches('.').to_ascii_lowercase()
}

fn is_ip_host(host: &str) -> bool {
    host.parse::<IpAddr>().is_ok()
}

fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }

    if request_path == "/" {
        return "/".to_string();
    }

    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => request_path[..idx].to_string(),
    }
}

fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    if cookie_path == request_path {
        return true;
    }

    if request_path.starts_with(cookie_path) {
        if cookie_path.ends_with('/') {
            return true;
        }

        return request_path
            .as_bytes()
            .get(cookie_path.len())
            .is_some_and(|ch| *ch == b'/');
    }

    false
}

fn domain_matches(host: &str, cookie_domain: &str, host_only: bool) -> bool {
    if host_only {
        return host == cookie_domain;
    }

    if host == cookie_domain {
        return true;
    }

    host.ends_with(&format!(".{cookie_domain}"))
}

fn parse_cookie_date_to_millis(raw: &str) -> Option<u64> {
    let parsed = httpdate::parse_http_date(raw).ok()?;
    parsed
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

fn has_ctl_bytes(value: &str) -> bool {
    value
        .bytes()
        .any(|b| b <= 0x1f || b == 0x7f || b == b';' || b == b'\r' || b == b'\n')
}

fn split_cookie_kv(cookie_str: &str) -> Option<CookieInput> {
    let mut parts = cookie_str.split(';');
    let first = parts.next()?.trim();

    let eq_pos = first.find('=')?;
    let name = first[..eq_pos].trim().to_string();
    let value = first[eq_pos + 1..].trim().to_string();

    if name.is_empty() || name.len() > MAX_COOKIE_NAME_LEN || value.len() > MAX_COOKIE_VALUE_LEN {
        return None;
    }

    if has_ctl_bytes(&name) || has_ctl_bytes(&value) {
        return None;
    }

    let attrs = parts
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|part| {
            if let Some(eq_pos) = part.find('=') {
                (
                    part[..eq_pos].trim().to_ascii_lowercase(),
                    Some(part[eq_pos + 1..].trim().to_string()),
                )
            } else {
                (part.to_ascii_lowercase(), None)
            }
        })
        .collect();

    Some(CookieInput { name, value, attrs })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SameSite {
    Strict,
    Lax,
    None,
}

impl SameSite {
    fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case("strict") {
            return Some(Self::Strict);
        }
        if value.eq_ignore_ascii_case("lax") {
            return Some(Self::Lax);
        }
        if value.eq_ignore_ascii_case("none") {
            return Some(Self::None);
        }
        None
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub host_only: bool,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: Option<SameSite>,
    pub creation_time: u64,
    pub last_access_time: u64,
    pub expires_at: Option<u64>,
    pub persistent: bool,
}

impl Cookie {
    fn new(name: String, value: String, host: &str, path: String, now: u64) -> Self {
        Self {
            name,
            value,
            domain: normalize_host(host),
            host_only: true,
            path,
            secure: false,
            http_only: false,
            same_site: None,
            creation_time: now,
            last_access_time: now,
            expires_at: None,
            persistent: false,
        }
    }

    fn key_eq(&self, other: &Cookie) -> bool {
        self.name == other.name && self.domain == other.domain && self.path == other.path
    }

    fn is_expired_at(&self, now: u64) -> bool {
        self.expires_at.is_some_and(|exp| now >= exp)
    }

    fn matches_request(
        &self,
        host: &str,
        path: &str,
        is_secure: bool,
        include_http_only: bool,
    ) -> bool {
        if !include_http_only && self.http_only {
            return false;
        }

        if self.secure && !is_secure {
            return false;
        }

        if !domain_matches(host, &self.domain, self.host_only) {
            return false;
        }

        path_matches(&self.path, path)
    }

    fn to_header_string(&self) -> String {
        format!("{}={}", self.name, self.value)
    }

    fn validate_prefixes(&self) -> bool {
        if self.name.starts_with("__Host-") {
            return self.secure && self.host_only && self.path == "/";
        }

        if self.name.starts_with("__Secure-") {
            return self.secure;
        }

        true
    }

    fn from_cookie_input(
        input: CookieInput,
        request_domain: &str,
        request_path: &str,
        is_secure_origin: bool,
        source: CookieSource,
        now: u64,
    ) -> Option<Self> {
        let mut cookie = Cookie::new(
            input.name,
            input.value,
            request_domain,
            default_path(request_path),
            now,
        );

        let mut max_age: Option<i64> = None;
        let mut expires: Option<u64> = None;

        for (attr_name, attr_value) in input.attrs {
            match attr_name.as_str() {
                "expires" => {
                    if let Some(value) = attr_value.as_deref() {
                        expires = parse_cookie_date_to_millis(value);
                    }
                }
                "max-age" => {
                    if let Some(value) = attr_value.as_deref() {
                        if let Ok(seconds) = value.parse::<i64>() {
                            max_age = Some(seconds);
                        }
                    }
                }
                "domain" => {
                    let Some(mut value) = attr_value else {
                        return None;
                    };

                    value = normalize_host(value.trim_start_matches('.'));
                    if value.is_empty() {
                        return None;
                    }

                    let request_domain = normalize_host(request_domain);
                    if is_ip_host(&request_domain) {
                        return None;
                    }

                    if !domain_matches(&request_domain, &value, false) {
                        return None;
                    }

                    cookie.host_only = false;
                    cookie.domain = value;
                }
                "path" => {
                    if let Some(path) = attr_value {
                        if path.starts_with('/') {
                            cookie.path = path;
                        }
                    }
                }
                "secure" => {
                    cookie.secure = true;
                }
                "httponly" => {
                    if source == CookieSource::Response {
                        cookie.http_only = true;
                    }
                }
                "samesite" => {
                    if let Some(value) = attr_value.as_deref() {
                        cookie.same_site = SameSite::parse(value);
                    }
                }
                _ => {}
            }
        }

        if cookie.same_site == Some(SameSite::None) && !cookie.secure {
            return None;
        }

        if cookie.secure && !is_secure_origin {
            return None;
        }

        cookie.expires_at = match max_age {
            Some(seconds) if seconds <= 0 => Some(0),
            Some(seconds) => Some(saturating_add_millis(now, seconds)),
            None => expires,
        };
        cookie.persistent = cookie.expires_at.is_some();

        if !cookie.validate_prefixes() {
            return None;
        }

        Some(cookie)
    }

    fn parse_with_context(
        cookie_str: &str,
        document_domain: &str,
        document_path: &str,
        is_secure_origin: bool,
    ) -> Option<Self> {
        let input = split_cookie_kv(cookie_str)?;
        let now = now_millis();
        Cookie::from_cookie_input(
            input,
            document_domain,
            document_path,
            is_secure_origin,
            CookieSource::Document,
            now,
        )
    }

    pub fn parse(cookie_str: &str, document_domain: &str, document_path: &str) -> Option<Self> {
        Self::parse_with_context(cookie_str, document_domain, document_path, true)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct PersistedCookieJar {
    #[serde(default = "default_storage_version")]
    version: u32,
    #[serde(default)]
    cookies: Vec<Cookie>,
}

const fn default_storage_version() -> u32 {
    STORAGE_VERSION
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LegacyCookieJar {
    #[serde(default)]
    cookies: Vec<LegacyCookie>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyCookie {
    name: String,
    value: String,
    domain: Option<String>,
    path: String,
    expires: Option<u64>,
    max_age: Option<i64>,
    secure: bool,
    http_only: bool,
    same_site: Option<String>,
    creation_time: u64,
    #[serde(default)]
    is_session: bool,
}

impl LegacyCookie {
    fn into_modern(self, now: u64) -> Option<Cookie> {
        let domain = normalize_host(&self.domain?);
        if domain.is_empty() {
            return None;
        }

        let same_site = self.same_site.as_deref().and_then(SameSite::parse);

        let expires_at = match self.max_age {
            Some(seconds) if seconds <= 0 => Some(0),
            Some(seconds) => Some(saturating_add_millis(self.creation_time, seconds)),
            None => self.expires,
        };

        let cookie = Cookie {
            name: self.name,
            value: self.value,
            domain,
            host_only: false,
            path: if self.path.starts_with('/') {
                self.path
            } else {
                "/".to_string()
            },
            secure: self.secure,
            http_only: self.http_only,
            same_site,
            creation_time: self.creation_time,
            last_access_time: now,
            expires_at,
            persistent: !self.is_session && expires_at.is_some(),
        };

        if !cookie.validate_prefixes() {
            return None;
        }

        Some(cookie)
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        Self::default()
    }

    fn normalize_for_store(cookie: Cookie) -> Option<Cookie> {
        if cookie.domain.is_empty() || !cookie.path.starts_with('/') {
            return None;
        }
        Some(cookie)
    }

    fn dedupe_overwrite(cookies: Vec<Cookie>) -> Vec<Cookie> {
        let mut deduped: Vec<Cookie> = Vec::with_capacity(cookies.len());
        for cookie in cookies {
            if let Some(idx) = deduped.iter().position(|existing| existing.key_eq(&cookie)) {
                deduped[idx] = cookie;
            } else {
                deduped.push(cookie);
            }
        }
        deduped
    }

    pub fn load_from_disk() -> Self {
        let path = get_cookies_file_path();
        let now = now_millis();

        if !path.exists() {
            return Self::new();
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                eprintln!("[Cookies] Failed to read cookie file {}: {}", path.display(), err);
                return Self::new();
            }
        };

        if let Ok(persisted) = serde_json::from_str::<PersistedCookieJar>(&contents) {
            let cookies = Self::dedupe_overwrite(
                persisted
                    .cookies
                    .into_iter()
                    .filter_map(Self::normalize_for_store)
                    .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
                    .collect(),
            );

            return Self { cookies };
        }

        if let Ok(legacy) = serde_json::from_str::<LegacyCookieJar>(&contents) {
            let cookies = Self::dedupe_overwrite(
                legacy
                    .cookies
                    .into_iter()
                    .filter_map(|legacy_cookie| legacy_cookie.into_modern(now))
                    .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
                    .collect(),
            );

            return Self { cookies };
        }

        eprintln!("[Cookies] Failed to parse cookie storage {}", path.display());
        Self::new()
    }

    pub fn save_to_disk(&self) {
        let path = get_cookies_file_path();
        let now = now_millis();

        let persisted = PersistedCookieJar {
            version: default_storage_version(),
            cookies: self
                .cookies
                .iter()
                .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
                .cloned()
                .collect(),
        };

        match serde_json::to_string_pretty(&persisted) {
            Ok(serialized) => {
                if let Err(err) = std::fs::write(&path, serialized) {
                    eprintln!("[Cookies] Failed to write cookie file {}: {}", path.display(), err);
                }
            }
            Err(err) => {
                eprintln!("[Cookies] Failed to serialize cookies: {}", err);
            }
        }
    }

    pub fn set_cookie(&mut self, cookie: Cookie) {
        self.remove_expired();

        if !cookie.validate_prefixes() {
            return;
        }

        if let Some(idx) = self
            .cookies
            .iter()
            .position(|existing| existing.key_eq(&cookie))
        {
            if cookie.is_expired_at(now_millis()) {
                self.cookies.remove(idx);
            } else {
                self.cookies[idx] = cookie;
            }
        } else if !cookie.is_expired_at(now_millis()) {
            self.cookies.push(cookie);
        }

        self.save_to_disk();
    }

    pub fn set_from_document_cookie(
        &mut self,
        cookie_str: &str,
        document_domain: &str,
        document_path: &str,
        is_secure_origin: bool,
    ) -> bool {
        let Some(cookie) = Cookie::parse_with_context(
            cookie_str,
            document_domain,
            document_path,
            is_secure_origin,
        ) else {
            return false;
        };

        self.set_cookie(cookie);
        true
    }

    fn matching_cookie_indices(
        &self,
        domain: &str,
        path: &str,
        include_http_only: bool,
        is_secure: bool,
    ) -> Vec<usize> {
        let mut matched: Vec<usize> = self
            .cookies
            .iter()
            .enumerate()
            .filter_map(|(idx, cookie)| {
                cookie
                    .matches_request(domain, path, is_secure, include_http_only)
                    .then_some(idx)
            })
            .collect();

        matched.sort_by(|a, b| {
            self.cookies[*b]
                .path
                .len()
                .cmp(&self.cookies[*a].path.len())
                .then_with(|| self.cookies[*a].creation_time.cmp(&self.cookies[*b].creation_time))
        });

        matched
    }

    fn get_cookies(
        &mut self,
        domain: &str,
        path: &str,
        include_http_only: bool,
        is_secure: bool,
    ) -> Vec<Cookie> {
        self.remove_expired();

        let normalized_domain = normalize_host(domain);
        let normalized_path = if path.starts_with('/') { path } else { "/" };

        let indices =
            self.matching_cookie_indices(&normalized_domain, &normalized_path, include_http_only, is_secure);

        let now = now_millis();
        let mut result = Vec::with_capacity(indices.len());

        for idx in indices {
            let cookie = &mut self.cookies[idx];
            cookie.last_access_time = now;
            result.push(cookie.clone());
        }

        result
    }

    pub fn get_document_cookie_string(&mut self, domain: &str, path: &str, is_secure: bool) -> String {
        self.get_cookies(domain, path, false, is_secure)
            .into_iter()
            .map(|cookie| cookie.to_header_string())
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn get_cookie_string(&mut self, domain: &str, path: &str) -> String {
        self.get_document_cookie_string(domain, path, true)
    }

    fn remove_expired(&mut self) {
        let now = now_millis();
        self.cookies.retain(|cookie| !cookie.is_expired_at(now));
    }

    pub fn clear(&mut self) {
        self.cookies.clear();
        self.save_to_disk();
    }

    pub fn get_cookie_header(&mut self, domain: &str, path: &str, is_secure: bool) -> String {
        self.get_cookies(domain, path, true, is_secure)
            .into_iter()
            .map(|cookie| cookie.to_header_string())
            .collect::<Vec<_>>()
            .join("; ")
    }

    pub fn set_from_header(
        &mut self,
        set_cookie_header: &str,
        request_domain: &str,
        request_path: &str,
        is_secure_origin: bool,
    ) {
        let now = now_millis();
        let Some(input) = split_cookie_kv(set_cookie_header) else {
            return;
        };

        let Some(cookie) = Cookie::from_cookie_input(
            input,
            request_domain,
            request_path,
            is_secure_origin,
            CookieSource::Response,
            now,
        ) else {
            return;
        };

        self.set_cookie(cookie);
    }
}

thread_local! {
    pub(crate) static COOKIE_JAR: RefCell<CookieJar> = RefCell::new(CookieJar::new());
    static COOKIE_JAR_INITIALIZED: RefCell<bool> = const { RefCell::new(false) };
    pub(crate) static DOCUMENT_URL: RefCell<Option<url::Url>> = RefCell::new(None);
}

pub fn ensure_cookie_jar_initialized() {
    COOKIE_JAR_INITIALIZED.with(|initialized| {
        if !*initialized.borrow() {
            COOKIE_JAR.with(|jar| {
                *jar.borrow_mut() = CookieJar::load_from_disk();
            });
            *initialized.borrow_mut() = true;
        }
    });
}

pub fn get_cookies_for_request(url: &url::Url) -> String {
    ensure_cookie_jar_initialized();

    let domain = url.host_str().unwrap_or("localhost");
    let path = url.path();
    let is_secure = url.scheme() == "https";

    COOKIE_JAR.with(|jar| jar.borrow_mut().get_cookie_header(domain, path, is_secure))
}

pub fn set_cookie_from_response(set_cookie_header: &str, request_url: &url::Url) {
    ensure_cookie_jar_initialized();

    let domain = request_url.host_str().unwrap_or("localhost");
    let path = request_url.path();
    let is_secure = request_url.scheme() == "https";

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut()
            .set_from_header(set_cookie_header, domain, path, is_secure);
    });
}

pub fn clear_all_cookies() {
    ensure_cookie_jar_initialized();

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut().clear();
    });
}

pub fn set_document_url(url: url::Url) {
    let effective_url = if url.scheme() == "data" || url.host_str().is_none() {
        url::Url::parse("http://localhost/").expect("localhost URL should parse")
    } else {
        url
    };

    DOCUMENT_URL.with(|doc_url| {
        *doc_url.borrow_mut() = Some(effective_url);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_path_algorithm() {
        assert_eq!(default_path("/"), "/");
        assert_eq!(default_path("/index.html"), "/");
        assert_eq!(default_path("/docs/index.html"), "/docs");
        assert_eq!(default_path("not-absolute"), "/");
    }

    #[test]
    fn test_domain_match_host_only_and_domain_cookie() {
        assert!(domain_matches("example.com", "example.com", true));
        assert!(!domain_matches("sub.example.com", "example.com", true));
        assert!(domain_matches("sub.example.com", "example.com", false));
    }

    #[test]
    fn test_path_matching_rules() {
        assert!(path_matches("/docs", "/docs"));
        assert!(path_matches("/docs", "/docs/"));
        assert!(path_matches("/docs", "/docs/a"));
        assert!(!path_matches("/docs", "/docset"));
    }

    #[test]
    fn test_document_cookie_cannot_set_httponly_and_secure_on_http_is_rejected() {
        let parsed = Cookie::parse_with_context("a=1; HttpOnly", "example.com", "/", false);
        assert!(parsed.is_some());
        assert!(!parsed.expect("cookie exists").http_only);

        let secure_cookie = Cookie::parse_with_context("a=1; Secure", "example.com", "/", false);
        assert!(secure_cookie.is_none());
    }

    #[test]
    fn test_response_cookie_can_set_httponly() {
        let now = now_millis();
        let input = split_cookie_kv("a=1; HttpOnly").expect("input parse");
        let cookie = Cookie::from_cookie_input(
            input,
            "example.com",
            "/",
            true,
            CookieSource::Response,
            now,
        )
        .expect("cookie parse");

        assert!(cookie.http_only);
    }

    #[test]
    fn test_max_age_overrides_expires() {
        let parsed = Cookie::parse_with_context(
            "a=1; Max-Age=1; Expires=Wed, 21 Oct 2015 07:28:00 GMT",
            "example.com",
            "/",
            true,
        )
        .expect("cookie should parse");

        assert!(parsed.expires_at.is_some());
        assert!(parsed.persistent);
    }

    #[test]
    fn test_same_site_none_requires_secure() {
        let parsed = Cookie::parse_with_context(
            "a=1; SameSite=None",
            "example.com",
            "/",
            true,
        );
        assert!(parsed.is_none());
    }

    #[test]
    fn test_cookie_ordering_for_header() {
        let mut jar = CookieJar::new();

        jar.set_from_header("a=1; Path=/", "example.com", "/", true);
        jar.set_from_header("b=2; Path=/account", "example.com", "/account", true);

        let header = jar.get_cookie_header("example.com", "/account/profile", true);
        assert_eq!(header, "b=2; a=1");
    }

    #[test]
    fn test_host_only_does_not_match_subdomain() {
        let mut jar = CookieJar::new();
        jar.set_from_header("a=1; Path=/", "example.com", "/", true);

        let header = jar.get_cookie_header("sub.example.com", "/", true);
        assert!(header.is_empty());
    }

    #[test]
    fn test_domain_cookie_matches_subdomain() {
        let mut jar = CookieJar::new();
        jar.set_from_header("a=1; Domain=example.com; Path=/", "example.com", "/", true);

        let header = jar.get_cookie_header("sub.example.com", "/", true);
        assert_eq!(header, "a=1");
    }

    #[test]
    fn test_set_cookie_with_max_age_zero_removes_existing() {
        let mut jar = CookieJar::new();
        jar.set_from_header("a=1; Path=/", "example.com", "/", true);
        jar.set_from_header("a=1; Max-Age=0; Path=/", "example.com", "/", true);

        let header = jar.get_cookie_header("example.com", "/", true);
        assert!(header.is_empty());
    }

    #[test]
    fn test_secure_cookie_not_sent_on_http() {
        let mut jar = CookieJar::new();
        jar.set_from_header("a=1; Secure; Path=/", "example.com", "/", true);

        let over_http = jar.get_cookie_header("example.com", "/", false);
        assert!(over_http.is_empty());

        let over_https = jar.get_cookie_header("example.com", "/", true);
        assert_eq!(over_https, "a=1");
    }

    #[test]
    fn test_document_cookie_string_excludes_httponly() {
        let mut jar = CookieJar::new();
        jar.set_from_header("visible=1; Path=/", "example.com", "/", true);
        jar.set_from_header("hidden=1; HttpOnly; Path=/", "example.com", "/", true);

        let doc_cookie = jar.get_document_cookie_string("example.com", "/", true);
        assert_eq!(doc_cookie, "visible=1");
    }
}
