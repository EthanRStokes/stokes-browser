// Cookie implementation for browser storage
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// Cookie file path
// ============================================================================

/// Get the path to the cookies file
fn get_cookies_file_path() -> PathBuf {
    static COOKIES_PATH: OnceLock<PathBuf> = OnceLock::new();
    COOKIES_PATH
        .get_or_init(|| {
            let config_dir = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("stokes-browser");

            // Create the directory if it doesn't exist
            if let Err(e) = std::fs::create_dir_all(&config_dir) {
                eprintln!(
                    "[Cookies] Warning: Failed to create config directory: {}",
                    e
                );
            }

            config_dir.join("cookies.json")
        })
        .clone()
}

// ============================================================================
// Cookie struct
// ============================================================================

/// Represents a single HTTP cookie with all its attributes
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Cookie {
    /// Cookie name
    pub name: String,
    /// Cookie value
    pub value: String,
    /// Domain the cookie is valid for (None = current document's domain)
    pub domain: Option<String>,
    /// Path the cookie is valid for (defaults to "/")
    pub path: String,
    /// Expiration time as Unix timestamp in milliseconds (None = session cookie)
    pub expires: Option<u64>,
    /// Max-Age in seconds (takes precedence over expires)
    pub max_age: Option<i64>,
    /// Whether the cookie should only be sent over HTTPS
    pub secure: bool,
    /// Whether the cookie is inaccessible to JavaScript (Set-Cookie only, not readable via document.cookie)
    pub http_only: bool,
    /// SameSite attribute: "Strict", "Lax", or "None"
    pub same_site: Option<String>,
    /// Creation time as Unix timestamp in milliseconds
    pub creation_time: u64,
    /// Whether this is a session cookie (should not be persisted)
    #[serde(default)]
    pub is_session: bool,
}

impl Cookie {
    /// Create a new cookie with default attributes
    pub fn new(name: String, value: String) -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        Cookie {
            name,
            value,
            domain: None,
            path: "/".to_string(),
            expires: None,
            max_age: None,
            secure: false,
            http_only: false,
            same_site: None,
            creation_time: now,
            is_session: true, // Default to session cookie until expires/max-age is set
        }
    }

    /// Check if the cookie has expired
    pub fn is_expired(&self) -> bool {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        // Check max-age first (it takes precedence)
        if let Some(max_age) = self.max_age {
            if max_age <= 0 {
                return true;
            }
            let expires_at = self.creation_time + (max_age as u64 * 1000);
            return now > expires_at;
        }

        // Then check expires
        if let Some(expires) = self.expires {
            return now > expires;
        }

        // Session cookie - never expires during the session
        false
    }

    /// Check if the cookie matches the given domain
    pub fn matches_domain(&self, request_domain: &str) -> bool {
        match &self.domain {
            None => true, // Host-only cookie matches exactly
            Some(cookie_domain) => {
                let cookie_domain = cookie_domain.to_lowercase();
                let request_domain = request_domain.to_lowercase();

                // Exact match
                if cookie_domain == request_domain {
                    return true;
                }

                // Domain cookie (starts with .) - matches subdomains
                if cookie_domain.starts_with('.') {
                    let domain_suffix = &cookie_domain[1..];
                    return request_domain == domain_suffix
                        || request_domain.ends_with(&format!(".{}", domain_suffix));
                }

                // Cookie domain without leading dot - also matches subdomains per spec
                request_domain == cookie_domain
                    || request_domain.ends_with(&format!(".{}", cookie_domain))
            }
        }
    }

    /// Check if the cookie matches the given path
    pub fn matches_path(&self, request_path: &str) -> bool {
        let cookie_path = &self.path;
        let request_path = if request_path.is_empty() {
            "/"
        } else {
            request_path
        };

        println!(
            "[Cookie] matches_path: cookie_path='{}', request_path='{}'",
            cookie_path, request_path
        );

        // Exact match
        if cookie_path == request_path {
            println!("[Cookie] matches_path: exact match");
            return true;
        }

        // Cookie path is a prefix of request path
        if request_path.starts_with(cookie_path) {
            // Cookie path ends with /
            if cookie_path.ends_with('/') {
                println!("[Cookie] matches_path: prefix match (cookie ends with /)");
                return true;
            }
            // Request path has / after cookie path
            let next_char = request_path.chars().nth(cookie_path.len());
            if next_char == Some('/') {
                println!("[Cookie] matches_path: prefix match (next char is /)");
                return true;
            }
            println!(
                "[Cookie] matches_path: prefix but no slash boundary, next_char={:?}",
                next_char
            );
        }

        println!("[Cookie] matches_path: no match");
        false
    }

    /// Parse a cookie string from document.cookie assignment format
    /// Format: "name=value; attr1=val1; attr2=val2; ..."
    pub fn parse(cookie_str: &str, document_domain: &str, document_path: &str) -> Option<Cookie> {
        let parts: Vec<&str> = cookie_str.split(';').collect();
        if parts.is_empty() {
            return None;
        }

        // First part is name=value
        let name_value = parts[0].trim();
        let eq_pos = name_value.find('=')?;
        let name = name_value[..eq_pos].trim().to_string();
        let value = name_value[eq_pos + 1..].trim().to_string();

        if name.is_empty() {
            return None;
        }

        let mut cookie = Cookie::new(name, value);
        cookie.domain = Some(document_domain.to_lowercase());

        // Default path is the directory of the current document
        cookie.path = if document_path.contains('/') {
            let last_slash = document_path.rfind('/').unwrap_or(0);
            if last_slash == 0 {
                "/".to_string()
            } else {
                document_path[..last_slash].to_string()
            }
        } else {
            "/".to_string()
        };

        // Parse attributes
        for part in parts.iter().skip(1) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (attr_name, attr_value) = if let Some(eq_pos) = part.find('=') {
                (
                    part[..eq_pos].trim().to_lowercase(),
                    Some(part[eq_pos + 1..].trim()),
                )
            } else {
                (part.to_lowercase(), None)
            };

            match attr_name.as_str() {
                "expires" => {
                    if let Some(val) = attr_value {
                        if let Some(timestamp) = parse_cookie_date(val) {
                            cookie.expires = Some(timestamp);
                        }
                    }
                }
                "max-age" => {
                    if let Some(val) = attr_value {
                        if let Ok(seconds) = val.parse::<i64>() {
                            cookie.max_age = Some(seconds);
                        }
                    }
                }
                "domain" => {
                    if let Some(val) = attr_value {
                        let mut domain = val.to_lowercase();
                        // Remove leading dot for storage, we'll handle it in matching
                        if domain.starts_with('.') {
                            domain = domain[1..].to_string();
                        }
                        // Validate domain - must be same or parent of document domain
                        let doc_domain = document_domain.to_lowercase();
                        if doc_domain == domain || doc_domain.ends_with(&format!(".{}", domain)) {
                            cookie.domain = Some(domain);
                        }
                        // If domain doesn't match, cookie is rejected (we keep original)
                    }
                }
                "path" => {
                    if let Some(val) = attr_value {
                        if val.starts_with('/') {
                            cookie.path = val.to_string();
                        }
                    }
                }
                "secure" => {
                    cookie.secure = true;
                }
                "httponly" => {
                    // Note: httpOnly cookies set via document.cookie are ignored
                    // but we parse it anyway for completeness
                    cookie.http_only = true;
                }
                "samesite" => {
                    if let Some(val) = attr_value {
                        let val_lower = val.to_lowercase();
                        if val_lower == "strict" || val_lower == "lax" || val_lower == "none" {
                            cookie.same_site = Some(val.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // httpOnly cookies cannot be set via JavaScript
        if cookie.http_only {
            return None;
        }

        // Determine if this is a session cookie
        cookie.is_session = cookie.expires.is_none() && cookie.max_age.is_none();

        println!(
            "[Cookie] parse: final cookie name='{}', value='{}', domain={:?}, path='{}'",
            cookie.name, cookie.value, cookie.domain, cookie.path
        );

        Some(cookie)
    }

    /// Serialize the cookie to a string for document.cookie getter
    /// Only returns "name=value" (attributes are not exposed)
    pub fn to_header_string(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

// ============================================================================
// Date parsing
// ============================================================================

/// Parse a cookie date string (various formats)
fn parse_cookie_date(date_str: &str) -> Option<u64> {
    // Try to parse common date formats
    // RFC 1123: "Sun, 06 Nov 1994 08:49:37 GMT"
    // RFC 1036: "Sunday, 06-Nov-94 08:49:37 GMT"
    // ANSI C: "Sun Nov  6 08:49:37 1994"

    let date_str = date_str.trim();

    // Split by whitespace and common delimiters
    let parts: Vec<&str> = date_str
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',')
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 4 {
        return None;
    }

    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];

    let mut day: Option<u32> = None;
    let mut month: Option<u32> = None;
    let mut year: Option<i32> = None;
    let mut time_parts: Option<(u32, u32, u32)> = None;

    for part in parts {
        let part_lower = part.to_lowercase();

        // Check for month name
        if let Some(m) = months.iter().position(|&m| part_lower.starts_with(m)) {
            month = Some(m as u32 + 1);
            continue;
        }

        // Check for time (HH:MM:SS)
        if part.contains(':') {
            let time_components: Vec<&str> = part.split(':').collect();
            if time_components.len() >= 2 {
                let h = time_components[0].parse::<u32>().ok();
                let m = time_components[1].parse::<u32>().ok();
                let s = time_components
                    .get(2)
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                if let (Some(h), Some(m)) = (h, m) {
                    time_parts = Some((h, m, s));
                }
            }
            continue;
        }

        // Check for numeric values (day or year)
        if let Ok(num) = part.parse::<i32>() {
            if num > 31 {
                // Year
                year = Some(if num < 100 {
                    if num >= 70 {
                        1900 + num
                    } else {
                        2000 + num
                    }
                } else {
                    num
                });
            } else if num >= 1 && num <= 31 && day.is_none() {
                day = Some(num as u32);
            }
        }
    }

    // Convert to timestamp
    if let (Some(d), Some(m), Some(y)) = (day, month, year) {
        let (h, min, s) = time_parts.unwrap_or((0, 0, 0));

        // Simple calculation (not accounting for leap seconds, etc.)
        // Days from year 1970
        let mut total_days: i64 = 0;

        // Years
        for year in 1970..y {
            total_days += if is_leap_year(year) { 366 } else { 365 };
        }

        // Months
        let days_in_month = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
        for month_idx in 0..(m - 1) as usize {
            total_days += days_in_month[month_idx] as i64;
            if month_idx == 1 && is_leap_year(y) {
                total_days += 1;
            }
        }

        // Days
        total_days += (d - 1) as i64;

        // Convert to milliseconds
        let timestamp = (total_days * 24 * 60 * 60 * 1000)
            + (h as i64 * 60 * 60 * 1000)
            + (min as i64 * 60 * 1000)
            + (s as i64 * 1000);

        if timestamp >= 0 {
            return Some(timestamp as u64);
        }
    }

    None
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0)
}

// ============================================================================
// Cookie jar
// ============================================================================

/// Cookie jar that stores all cookies for a browsing session
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar {
            cookies: Vec::new(),
        }
    }

    /// Load cookies from disk
    pub fn load_from_disk() -> Self {
        let path = get_cookies_file_path();

        if !path.exists() {
            println!("[Cookies] No cookie file found, starting fresh");
            return CookieJar::new();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<CookieJar>(&contents) {
                Ok(mut jar) => {
                    // Remove expired cookies on load
                    jar.remove_expired();
                    // Remove session cookies (they shouldn't persist)
                    jar.cookies.retain(|c| !c.is_session);
                    println!("[Cookies] Loaded {} cookies from disk", jar.cookies.len());
                    jar
                }
                Err(e) => {
                    eprintln!("[Cookies] Failed to parse cookie file: {}", e);
                    CookieJar::new()
                }
            },
            Err(e) => {
                eprintln!("[Cookies] Failed to read cookie file: {}", e);
                CookieJar::new()
            }
        }
    }

    /// Save cookies to disk (only persistent cookies)
    pub fn save_to_disk(&self) {
        let path = get_cookies_file_path();

        // Create a filtered jar with only persistent, non-expired cookies
        let persistent_cookies: Vec<Cookie> = self
            .cookies
            .iter()
            .filter(|c| !c.is_session && !c.is_expired())
            .cloned()
            .collect();

        let jar_to_save = CookieJar {
            cookies: persistent_cookies,
        };

        match serde_json::to_string_pretty(&jar_to_save) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    eprintln!("[Cookies] Failed to write cookie file: {}", e);
                } else {
                    println!(
                        "[Cookies] Saved {} cookies to disk",
                        jar_to_save.cookies.len()
                    );
                }
            }
            Err(e) => {
                eprintln!("[Cookies] Failed to serialize cookies: {}", e);
            }
        }
    }

    /// Add or update a cookie
    pub fn set_cookie(&mut self, mut cookie: Cookie) {
        // Remove expired cookies first
        self.remove_expired();

        // Determine if this is a session cookie
        // A cookie is persistent if it has expires or max-age set
        cookie.is_session = cookie.expires.is_none() && cookie.max_age.is_none();

        // Check for existing cookie with same name, domain, and path
        let existing_idx = self.cookies.iter().position(|c| {
            c.name == cookie.name && c.domain == cookie.domain && c.path == cookie.path
        });

        if let Some(idx) = existing_idx {
            // If the new cookie has expired or max-age <= 0, remove it
            if cookie.is_expired() || cookie.max_age.map_or(false, |ma| ma <= 0) {
                self.cookies.remove(idx);
            } else {
                // Update existing cookie
                self.cookies[idx] = cookie;
            }
        } else if !cookie.is_expired() && !cookie.max_age.map_or(false, |ma| ma <= 0) {
            // Add new cookie if not expired
            self.cookies.push(cookie);
        }

        // Save to disk after modification
        self.save_to_disk();
    }

    /// Get all non-expired, non-httpOnly cookies that match the given domain and path
    pub fn get_cookies(
        &mut self,
        domain: &str,
        path: &str,
        include_http_only: bool,
    ) -> Vec<&Cookie> {
        self.remove_expired();

        self.cookies
            .iter()
            .filter(|c| {
                (include_http_only || !c.http_only)
                    && c.matches_domain(domain)
                    && c.matches_path(path)
            })
            .collect()
    }

    /// Get the cookie string for document.cookie getter
    pub fn get_cookie_string(&mut self, domain: &str, path: &str) -> String {
        let cookies = self.get_cookies(domain, path, false);
        cookies
            .iter()
            .map(|c| c.to_header_string())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Remove all expired cookies
    fn remove_expired(&mut self) {
        self.cookies.retain(|c| !c.is_expired());
    }

    /// Clear all cookies
    pub fn clear(&mut self) {
        self.cookies.clear();
        self.save_to_disk();
    }

    /// Get the Cookie header value for an HTTP request
    /// This includes all cookies (including httpOnly) that match the request
    pub fn get_cookie_header(&mut self, domain: &str, path: &str, is_secure: bool) -> String {
        self.remove_expired();

        self.cookies
            .iter()
            .filter(|c| c.matches_domain(domain) && c.matches_path(path) && (!c.secure || is_secure))
            .map(|c| c.to_header_string())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Parse and store cookies from Set-Cookie response headers
    /// This can set httpOnly cookies (unlike document.cookie)
    pub fn set_from_header(
        &mut self,
        set_cookie_header: &str,
        request_domain: &str,
        request_path: &str,
    ) {
        // Parse the Set-Cookie header (similar format to document.cookie but allows httpOnly)
        let parts: Vec<&str> = set_cookie_header.split(';').collect();
        if parts.is_empty() {
            return;
        }

        // First part is name=value
        let name_value = parts[0].trim();
        let eq_pos = match name_value.find('=') {
            Some(p) => p,
            None => return,
        };
        let name = name_value[..eq_pos].trim().to_string();
        let value = name_value[eq_pos + 1..].trim().to_string();

        if name.is_empty() {
            return;
        }

        let mut cookie = Cookie::new(name, value);
        cookie.domain = Some(request_domain.to_lowercase());
        cookie.path = if request_path.contains('/') {
            let last_slash = request_path.rfind('/').unwrap_or(0);
            if last_slash == 0 {
                "/".to_string()
            } else {
                request_path[..last_slash].to_string()
            }
        } else {
            "/".to_string()
        };

        // Parse attributes
        for part in parts.iter().skip(1) {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (attr_name, attr_value) = if let Some(eq_pos) = part.find('=') {
                (
                    part[..eq_pos].trim().to_lowercase(),
                    Some(part[eq_pos + 1..].trim()),
                )
            } else {
                (part.to_lowercase(), None)
            };

            match attr_name.as_str() {
                "expires" => {
                    if let Some(val) = attr_value {
                        if let Some(timestamp) = parse_cookie_date(val) {
                            cookie.expires = Some(timestamp);
                        }
                    }
                }
                "max-age" => {
                    if let Some(val) = attr_value {
                        if let Ok(seconds) = val.parse::<i64>() {
                            cookie.max_age = Some(seconds);
                        }
                    }
                }
                "domain" => {
                    if let Some(val) = attr_value {
                        let mut domain = val.to_lowercase();
                        if domain.starts_with('.') {
                            domain = domain[1..].to_string();
                        }
                        let doc_domain = request_domain.to_lowercase();
                        if doc_domain == domain || doc_domain.ends_with(&format!(".{}", domain)) {
                            cookie.domain = Some(domain);
                        }
                    }
                }
                "path" => {
                    if let Some(val) = attr_value {
                        if val.starts_with('/') {
                            cookie.path = val.to_string();
                        }
                    }
                }
                "secure" => {
                    cookie.secure = true;
                }
                "httponly" => {
                    cookie.http_only = true;
                }
                "samesite" => {
                    if let Some(val) = attr_value {
                        let val_lower = val.to_lowercase();
                        if val_lower == "strict" || val_lower == "lax" || val_lower == "none" {
                            cookie.same_site = Some(val.to_string());
                        }
                    }
                }
                _ => {}
            }
        }

        // Determine if this is a session cookie
        cookie.is_session = cookie.expires.is_none() && cookie.max_age.is_none();

        self.set_cookie(cookie);
    }
}

// ============================================================================
// Thread-local storage and public API
// ============================================================================

thread_local! {
    pub(crate) static COOKIE_JAR: RefCell<CookieJar> = RefCell::new(CookieJar::new());
    static COOKIE_JAR_INITIALIZED: RefCell<bool> = const { RefCell::new(false) };
    pub(crate) static DOCUMENT_URL: RefCell<Option<url::Url>> = RefCell::new(None);
}

/// Ensure the cookie jar is initialized (loaded from disk)
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

/// Get the Cookie header value for an HTTP request to the given URL
/// Returns cookies formatted for the Cookie header: "name1=value1; name2=value2"
pub fn get_cookies_for_request(url: &url::Url) -> String {
    ensure_cookie_jar_initialized();

    let domain = url.host_str().unwrap_or("localhost");
    let path = url.path();
    let is_secure = url.scheme() == "https";

    COOKIE_JAR.with(|jar| jar.borrow_mut().get_cookie_header(domain, path, is_secure))
}

/// Store cookies from a Set-Cookie response header
pub fn set_cookie_from_response(set_cookie_header: &str, request_url: &url::Url) {
    ensure_cookie_jar_initialized();

    let domain = request_url.host_str().unwrap_or("localhost");
    let path = request_url.path();

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut()
            .set_from_header(set_cookie_header, domain, path);
    });
}

/// Clear all cookies (for testing or privacy features)
pub fn clear_all_cookies() {
    ensure_cookie_jar_initialized();

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut().clear();
    });
}

/// Set the document URL for cookie handling
pub fn set_document_url(url: url::Url) {
    println!("[Cookie] Document URL for cookie handling: {}", url);
    println!(
        "[Cookie] URL scheme: {}, host: {:?}, path: {}",
        url.scheme(),
        url.host_str(),
        url.path()
    );

    // For data: URLs or URLs without a proper host, create a localhost URL
    // This allows cookies to work in test scenarios
    let effective_url = if url.scheme() == "data" || url.host_str().is_none() {
        println!("[Cookie] Using localhost fallback for cookie domain");
        url::Url::parse("http://localhost/").unwrap()
    } else {
        url
    };

    DOCUMENT_URL.with(|doc_url| {
        *doc_url.borrow_mut() = Some(effective_url);
    });
}

