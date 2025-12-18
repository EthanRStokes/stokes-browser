use super::element_bindings;
// DOM bindings for JavaScript using mozjs
use super::runtime::JsRuntime;
use crate::dom::{AttributeMap, Dom};
use mozjs::gc::Handle;
use mozjs::jsapi::{
    CallArgs, CurrentGlobalOrNull, HandleValueArray, JSContext, JSNative, JSObject, JS_DefineFunction,
    JS_DefineProperty, JS_NewPlainObject, JS_NewUCStringCopyN, NewArrayObject,
    JSPROP_ENUMERATE,
};
use mozjs::jsval::{BooleanValue, Int32Value, JSVal, ObjectValue, StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers::JS_ValueToSource;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::os::raw::c_uint;
use std::path::PathBuf;
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
use mozjs::rust::CompileOptionsWrapper;

// ============================================================================
// Cookie implementation
// ============================================================================

/// Get the path to the cookies file
fn get_cookies_file_path() -> PathBuf {
    static COOKIES_PATH: OnceLock<PathBuf> = OnceLock::new();
    COOKIES_PATH.get_or_init(|| {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("stokes-browser");

        // Create the directory if it doesn't exist
        if let Err(e) = std::fs::create_dir_all(&config_dir) {
            eprintln!("[Cookies] Warning: Failed to create config directory: {}", e);
        }

        config_dir.join("cookies.json")
    }).clone()
}

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
        let request_path = if request_path.is_empty() { "/" } else { request_path };

        println!("[Cookie] matches_path: cookie_path='{}', request_path='{}'", cookie_path, request_path);

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
            println!("[Cookie] matches_path: prefix but no slash boundary, next_char={:?}", next_char);
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
                (part[..eq_pos].trim().to_lowercase(), Some(part[eq_pos + 1..].trim()))
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

        println!("[Cookie] parse: final cookie name='{}', value='{}', domain={:?}, path='{}'",
            cookie.name, cookie.value, cookie.domain, cookie.path);

        Some(cookie)
    }

    /// Serialize the cookie to a string for document.cookie getter
    /// Only returns "name=value" (attributes are not exposed)
    pub fn to_header_string(&self) -> String {
        format!("{}={}", self.name, self.value)
    }
}

/// Parse a cookie date string (various formats)
fn parse_cookie_date(date_str: &str) -> Option<u64> {
    // Try to parse common date formats
    // RFC 1123: "Sun, 06 Nov 1994 08:49:37 GMT"
    // RFC 1036: "Sunday, 06-Nov-94 08:49:37 GMT"
    // ANSI C: "Sun Nov  6 08:49:37 1994"

    let date_str = date_str.trim();

    // Simple parsing - extract components
    // This is a simplified parser that handles common formats

    // Split by whitespace and common delimiters
    let parts: Vec<&str> = date_str
        .split(|c: char| c.is_whitespace() || c == '-' || c == ',')
        .filter(|s| !s.is_empty())
        .collect();

    if parts.len() < 4 {
        return None;
    }

    let months = [
        "jan", "feb", "mar", "apr", "may", "jun",
        "jul", "aug", "sep", "oct", "nov", "dec",
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
                let s = time_components.get(2).and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
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
                    if num >= 70 { 1900 + num } else { 2000 + num }
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

/// Cookie jar that stores all cookies for a browsing session
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
}

impl CookieJar {
    pub fn new() -> Self {
        CookieJar { cookies: Vec::new() }
    }

    /// Load cookies from disk
    pub fn load_from_disk() -> Self {
        let path = get_cookies_file_path();

        if !path.exists() {
            println!("[Cookies] No cookie file found, starting fresh");
            return CookieJar::new();
        }

        match std::fs::read_to_string(&path) {
            Ok(contents) => {
                match serde_json::from_str::<CookieJar>(&contents) {
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
                }
            }
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
        let persistent_cookies: Vec<Cookie> = self.cookies
            .iter()
            .filter(|c| !c.is_session && !c.is_expired())
            .cloned()
            .collect();

        let jar_to_save = CookieJar { cookies: persistent_cookies };

        match serde_json::to_string_pretty(&jar_to_save) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    eprintln!("[Cookies] Failed to write cookie file: {}", e);
                } else {
                    println!("[Cookies] Saved {} cookies to disk", jar_to_save.cookies.len());
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
            c.name == cookie.name
                && c.domain == cookie.domain
                && c.path == cookie.path
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
    pub fn get_cookies(&mut self, domain: &str, path: &str, include_http_only: bool) -> Vec<&Cookie> {
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
            .filter(|c| {
                c.matches_domain(domain)
                    && c.matches_path(path)
                    && (!c.secure || is_secure)
            })
            .map(|c| c.to_header_string())
            .collect::<Vec<_>>()
            .join("; ")
    }

    /// Parse and store cookies from Set-Cookie response headers
    /// This can set httpOnly cookies (unlike document.cookie)
    pub fn set_from_header(&mut self, set_cookie_header: &str, request_domain: &str, request_path: &str) {
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
                (part[..eq_pos].trim().to_lowercase(), Some(part[eq_pos + 1..].trim()))
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
// Public API for cookie management
// ============================================================================

/// Get the Cookie header value for an HTTP request to the given URL
/// Returns cookies formatted for the Cookie header: "name1=value1; name2=value2"
pub fn get_cookies_for_request(url: &url::Url) -> String {
    ensure_cookie_jar_initialized();

    let domain = url.host_str().unwrap_or("localhost");
    let path = url.path();
    let is_secure = url.scheme() == "https";

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut().get_cookie_header(domain, path, is_secure)
    })
}

/// Store cookies from a Set-Cookie response header
pub fn set_cookie_from_response(set_cookie_header: &str, request_url: &url::Url) {
    ensure_cookie_jar_initialized();

    let domain = request_url.host_str().unwrap_or("localhost");
    let path = request_url.path();

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut().set_from_header(set_cookie_header, domain, path);
    });
}

/// Clear all cookies (for testing or privacy features)
pub fn clear_all_cookies() {
    ensure_cookie_jar_initialized();

    COOKIE_JAR.with(|jar| {
        jar.borrow_mut().clear();
    });
}

// Thread-local storage for DOM reference
thread_local! {
    static DOM_REF: RefCell<Option<*mut Dom>> = RefCell::new(None);
    static USER_AGENT: RefCell<String> = RefCell::new(String::new());
    static LOCAL_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    static SESSION_STORAGE: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    static COOKIE_JAR: RefCell<CookieJar> = RefCell::new(CookieJar::new());
    static COOKIE_JAR_INITIALIZED: RefCell<bool> = const { RefCell::new(false) };
    static DOCUMENT_URL: RefCell<Option<url::Url>> = RefCell::new(None);
}

/// Ensure the cookie jar is initialized (loaded from disk)
fn ensure_cookie_jar_initialized() {
    COOKIE_JAR_INITIALIZED.with(|initialized| {
        if !*initialized.borrow() {
            COOKIE_JAR.with(|jar| {
                *jar.borrow_mut() = CookieJar::load_from_disk();
            });
            *initialized.borrow_mut() = true;
        }
    });
}

/// Set up DOM bindings in the JavaScript context
pub fn setup_dom_bindings(runtime: &mut JsRuntime, document_root: *mut Dom, user_agent: String) -> Result<(), String> {
    let raw_cx = unsafe { runtime.cx().raw_cx() };

    // Store DOM reference in thread-local storage
    DOM_REF.with(|dom| {
        *dom.borrow_mut() = Some(document_root);
    });
    USER_AGENT.with(|ua| {
        *ua.borrow_mut() = user_agent.clone();
    });

    // Store the document URL for cookie handling
    unsafe {
        let dom = &*document_root;
        let url: url::Url = (&dom.url).into();
        println!("[Cookie] Document URL for cookie handling: {}", url);
        println!("[Cookie] URL scheme: {}, host: {:?}, path: {}", url.scheme(), url.host_str(), url.path());

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

    // Also set the DOM reference for element bindings
    element_bindings::set_element_dom_ref(document_root);

    unsafe {
        rooted!(in(raw_cx) let global = CurrentGlobalOrNull(raw_cx));
        if global.get().is_null() {
            return Err("No global object for DOM setup".to_string());
        }

        // Create and set up document object
        setup_document(raw_cx, global.handle().get())?;

        // Set up window object (as alias to global)
        setup_window(raw_cx, global.handle().get(), &user_agent)?;

        // Set up navigator object
        setup_navigator(raw_cx, global.handle().get(), &user_agent)?;

        // Set up location object
        setup_location(raw_cx, global.handle().get())?;

        // Set up localStorage and sessionStorage
        setup_storage(raw_cx, global.handle().get())?;

        // Set up Node constructor with constants
        setup_node_constructor(raw_cx, global.handle().get())?;

        // Set up Element and HTMLElement constructors
        setup_element_constructors(raw_cx, global.handle().get())?;

        // Set up Event and CustomEvent constructors
        setup_event_constructors(raw_cx, global.handle().get())?;

        // Set up XMLHttpRequest constructor
        setup_xhr_constructor(raw_cx, global.handle().get())?;

        // Set up atob/btoa functions
        setup_base64_functions(raw_cx, global.handle().get())?;

        // Set up dataLayer for Google Analytics compatibility
        setup_data_layer(raw_cx, global.handle().get())?;
    }

    println!("[JS] DOM bindings initialized");
    Ok(())
}

/// Set up the document object
unsafe fn setup_document(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let document = JS_NewPlainObject(raw_cx));
    if document.get().is_null() {
        return Err("Failed to create document object".to_string());
    }

    // Define document methods
    define_function(raw_cx, document.get(), "getElementById", Some(document_get_element_by_id), 1)?;
    define_function(raw_cx, document.get(), "getElementsByTagName", Some(document_get_elements_by_tag_name), 1)?;
    define_function(raw_cx, document.get(), "getElementsByClassName", Some(document_get_elements_by_class_name), 1)?;
    define_function(raw_cx, document.get(), "querySelector", Some(document_query_selector), 1)?;
    define_function(raw_cx, document.get(), "querySelectorAll", Some(document_query_selector_all), 1)?;
    define_function(raw_cx, document.get(), "createElement", Some(document_create_element), 1)?;
    define_function(raw_cx, document.get(), "createTextNode", Some(document_create_text_node), 1)?;
    define_function(raw_cx, document.get(), "createDocumentFragment", Some(document_create_document_fragment), 0)?;

    // Add cookie getter and setter helper functions
    define_function(raw_cx, document.get(), "__getCookie", Some(document_get_cookie), 0)?;
    define_function(raw_cx, document.get(), "__setCookie", Some(document_set_cookie), 1)?;

    // Create documentElement (represents <html>) using a proper element with methods
    let doc_elem_val = element_bindings::create_stub_element(raw_cx, "html")?;
    rooted!(in(raw_cx) let doc_elem_val_rooted = doc_elem_val);
    let name = std::ffi::CString::new("documentElement").unwrap();
    rooted!(in(raw_cx) let document_rooted = document.get());
    JS_DefineProperty(
        raw_cx,
        document_rooted.handle().into(),
        name.as_ptr(),
        doc_elem_val_rooted.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Set document on global
    rooted!(in(raw_cx) let document_val = ObjectValue(document.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("document").unwrap();
    if !JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        document_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        return Err("Failed to define document property".to_string());
    }

    // Set up document.cookie as a getter/setter property using Object.defineProperty
    // This is deferred until after setup completes to avoid realm issues
    // The cookie property will be set up on first access via a fallback mechanism

    Ok(())
}

/// Set up the document.cookie property with getter/setter
/// This should be called from the runtime after initialization is complete
pub fn setup_cookie_property_deferred(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
        Object.defineProperty(document, 'cookie', {
            get: function() {
                return document.__getCookie();
            },
            set: function(value) {
                document.__setCookie(value);
            },
            configurable: true,
            enumerable: true
        });
    "#;

    // Use the runtime's execute method which handles realm entry properly
    runtime.execute(script).map_err(|e| {
        println!("[JS] Warning: Failed to set up document.cookie property: {}", e);
        e
    })?;

    Ok(())
}

/// Set up the window object (as alias to global)
// FIXME: Window dimensions, scroll positions, and devicePixelRatio are hardcoded - should get actual values from renderer
unsafe fn setup_window(raw_cx: *mut JSContext, global: *mut JSObject, user_agent: &str) -> Result<(), String> { unsafe {
    rooted!(in(raw_cx) let global_val = ObjectValue(global));
    rooted!(in(raw_cx) let global_rooted = global);

    // window, self, top, parent, globalThis, frames all point to global
    // FIXME: `frames` should be a proper WindowProxy collection that allows indexed access to child iframes (e.g., frames[0], frames['name'])
    for name in &["window", "self", "top", "parent", "globalThis", "frames"] {
        let cname = std::ffi::CString::new(*name).unwrap();
        JS_DefineProperty(
            raw_cx,
            global_rooted.handle().into(),
            cname.as_ptr(),
            global_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        );
    }

    // Define window functions on global
    define_function(raw_cx, global, "alert", Some(window_alert), 1)?;
    define_function(raw_cx, global, "confirm", Some(window_confirm), 1)?;
    define_function(raw_cx, global, "prompt", Some(window_prompt), 2)?;
    define_function(raw_cx, global, "requestAnimationFrame", Some(window_request_animation_frame), 1)?;
    define_function(raw_cx, global, "cancelAnimationFrame", Some(window_cancel_animation_frame), 1)?;
    define_function(raw_cx, global, "getComputedStyle", Some(window_get_computed_style), 1)?;
    define_function(raw_cx, global, "addEventListener", Some(window_add_event_listener), 3)?;
    define_function(raw_cx, global, "removeEventListener", Some(window_remove_event_listener), 3)?;
    define_function(raw_cx, global, "scrollTo", Some(window_scroll_to), 2)?;
    define_function(raw_cx, global, "scrollBy", Some(window_scroll_by), 2)?;

    // Set innerWidth/innerHeight properties
    set_int_property(raw_cx, global, "innerWidth", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport.window_size.0 as i32;
        }
        1920
    }))?;
    set_int_property(raw_cx, global, "innerHeight", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport.window_size.1 as i32;
        }
        1080
    }))?;
    set_int_property(raw_cx, global, "outerWidth", 1920)?;
    set_int_property(raw_cx, global, "outerHeight", 1080)?;
    set_int_property(raw_cx, global, "screenX", 0)?;
    set_int_property(raw_cx, global, "screenY", 0)?;
    set_int_property(raw_cx, global, "scrollX", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport_scroll.x as i32;
        }
        0
    }))?;
    set_int_property(raw_cx, global, "scrollY", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport_scroll.y as i32;
        }
        0
    }))?;
    set_int_property(raw_cx, global, "pageXOffset", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport_scroll.x as i32;
        }
        0
    }))?;
    set_int_property(raw_cx, global, "pageYOffset", DOM_REF.with(|dom| {
        if let Some(ref dom) = *dom.borrow() {
            let dom = &**dom;
            return dom.viewport_scroll.y as i32;
        }
        0
    }))?;
    set_int_property(raw_cx, global, "devicePixelRatio", 1)?;

    Ok(())
} }

/// Set up the navigator object
// TODO: Many navigator properties are hardcoded (language, platform) - should detect from system
unsafe fn setup_navigator(raw_cx: *mut JSContext, global: *mut JSObject, user_agent: &str) -> Result<(), String> {
    rooted!(in(raw_cx) let navigator = JS_NewPlainObject(raw_cx));
    if navigator.get().is_null() {
        return Err("Failed to create navigator object".to_string());
    }

    set_string_property(raw_cx, navigator.get(), "userAgent", user_agent)?;
    set_string_property(raw_cx, navigator.get(), "language", "en-US")?;
    set_string_property(raw_cx, navigator.get(), "platform", std::env::consts::OS)?;
    set_string_property(raw_cx, navigator.get(), "appName", "Stokes Browser")?;
    set_string_property(raw_cx, navigator.get(), "appVersion", "1.0")?;
    set_string_property(raw_cx, navigator.get(), "vendor", "Stokes")?;
    set_bool_property(raw_cx, navigator.get(), "onLine", true)?;
    set_bool_property(raw_cx, navigator.get(), "cookieEnabled", true)?;

    // Set navigator on global
    rooted!(in(raw_cx) let navigator_val = ObjectValue(navigator.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("navigator").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        navigator_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up the location object
// FIXME: Location properties are hardcoded to "about:blank" - should reflect actual page URL
unsafe fn setup_location(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let location = JS_NewPlainObject(raw_cx));
    if location.get().is_null() {
        return Err("Failed to create location object".to_string());
    }

    set_string_property(raw_cx, location.get(), "href", "about:blank")?;
    set_string_property(raw_cx, location.get(), "protocol", "about:")?;
    set_string_property(raw_cx, location.get(), "host", "")?;
    set_string_property(raw_cx, location.get(), "hostname", "")?;
    set_string_property(raw_cx, location.get(), "port", "")?;
    set_string_property(raw_cx, location.get(), "pathname", "blank")?;
    set_string_property(raw_cx, location.get(), "search", "")?;
    set_string_property(raw_cx, location.get(), "hash", "")?;
    set_string_property(raw_cx, location.get(), "origin", "null")?;

    define_function(raw_cx, location.get(), "reload", Some(location_reload), 0)?;
    define_function(raw_cx, location.get(), "assign", Some(location_assign), 1)?;
    define_function(raw_cx, location.get(), "replace", Some(location_replace), 1)?;

    // Set location on global
    rooted!(in(raw_cx) let location_val = ObjectValue(location.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("location").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        location_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up localStorage and sessionStorage
// TODO: Storage length property is set to 0 and not dynamically updated when items are added/removed
unsafe fn setup_storage(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let global_rooted = global);

    // Create localStorage object
    rooted!(in(raw_cx) let local_storage = JS_NewPlainObject(raw_cx));
    if local_storage.get().is_null() {
        return Err("Failed to create localStorage object".to_string());
    }

    define_function(raw_cx, local_storage.get(), "getItem", Some(local_storage_get_item), 1)?;
    define_function(raw_cx, local_storage.get(), "setItem", Some(local_storage_set_item), 2)?;
    define_function(raw_cx, local_storage.get(), "removeItem", Some(local_storage_remove_item), 1)?;
    define_function(raw_cx, local_storage.get(), "clear", Some(local_storage_clear), 0)?;
    define_function(raw_cx, local_storage.get(), "key", Some(local_storage_key), 1)?;
    set_int_property(raw_cx, local_storage.get(), "length", 0)?;

    rooted!(in(raw_cx) let local_storage_val = ObjectValue(local_storage.get()));
    let name = std::ffi::CString::new("localStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        local_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // Create sessionStorage object
    rooted!(in(raw_cx) let session_storage = JS_NewPlainObject(raw_cx));
    if session_storage.get().is_null() {
        return Err("Failed to create sessionStorage object".to_string());
    }

    define_function(raw_cx, session_storage.get(), "getItem", Some(session_storage_get_item), 1)?;
    define_function(raw_cx, session_storage.get(), "setItem", Some(session_storage_set_item), 2)?;
    define_function(raw_cx, session_storage.get(), "removeItem", Some(session_storage_remove_item), 1)?;
    define_function(raw_cx, session_storage.get(), "clear", Some(session_storage_clear), 0)?;
    define_function(raw_cx, session_storage.get(), "key", Some(session_storage_key), 1)?;
    set_int_property(raw_cx, session_storage.get(), "length", 0)?;

    rooted!(in(raw_cx) let session_storage_val = ObjectValue(session_storage.get()));
    let name = std::ffi::CString::new("sessionStorage").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        session_storage_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Node constructor with node type constants
unsafe fn setup_node_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let node = JS_NewPlainObject(raw_cx));
    if node.get().is_null() {
        return Err("Failed to create Node constructor".to_string());
    }

    set_int_property(raw_cx, node.get(), "ELEMENT_NODE", 1)?;
    set_int_property(raw_cx, node.get(), "ATTRIBUTE_NODE", 2)?;
    set_int_property(raw_cx, node.get(), "TEXT_NODE", 3)?;
    set_int_property(raw_cx, node.get(), "CDATA_SECTION_NODE", 4)?;
    set_int_property(raw_cx, node.get(), "ENTITY_REFERENCE_NODE", 5)?;
    set_int_property(raw_cx, node.get(), "ENTITY_NODE", 6)?;
    set_int_property(raw_cx, node.get(), "PROCESSING_INSTRUCTION_NODE", 7)?;
    set_int_property(raw_cx, node.get(), "COMMENT_NODE", 8)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_NODE", 9)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_TYPE_NODE", 10)?;
    set_int_property(raw_cx, node.get(), "DOCUMENT_FRAGMENT_NODE", 11)?;
    set_int_property(raw_cx, node.get(), "NOTATION_NODE", 12)?;

    rooted!(in(raw_cx) let node_val = ObjectValue(node.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Node").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        node_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Element and HTMLElement constructors
unsafe fn setup_element_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    // Element constructor
    rooted!(in(raw_cx) let element = JS_NewPlainObject(raw_cx));
    if element.get().is_null() {
        return Err("Failed to create Element constructor".to_string());
    }
    set_int_property(raw_cx, element.get(), "ELEMENT_NODE", 1)?;

    rooted!(in(raw_cx) let element_val = ObjectValue(element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("Element").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    // HTMLElement constructor (alias for now)
    let name = std::ffi::CString::new("HTMLElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up Event and CustomEvent constructors
unsafe fn setup_event_constructors(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let event = JS_NewPlainObject(raw_cx));
    if event.get().is_null() {
        return Err("Failed to create Event constructor".to_string());
    }

    rooted!(in(raw_cx) let event_val = ObjectValue(event.get()));
    rooted!(in(raw_cx) let global_rooted = global);

    let name = std::ffi::CString::new("Event").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    let name = std::ffi::CString::new("CustomEvent").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        event_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up XMLHttpRequest constructor
unsafe fn setup_xhr_constructor(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    rooted!(in(raw_cx) let xhr = JS_NewPlainObject(raw_cx));
    if xhr.get().is_null() {
        return Err("Failed to create XMLHttpRequest constructor".to_string());
    }

    rooted!(in(raw_cx) let xhr_val = ObjectValue(xhr.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("XMLHttpRequest").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        xhr_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

/// Set up atob/btoa functions
unsafe fn setup_base64_functions(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    define_function(raw_cx, global, "atob", Some(window_atob), 1)?;
    define_function(raw_cx, global, "btoa", Some(window_btoa), 1)?;
    Ok(())
}

/// Set up dataLayer for Google Analytics compatibility
unsafe fn setup_data_layer(raw_cx: *mut JSContext, global: *mut JSObject) -> Result<(), String> {
    // Create an empty array for dataLayer
    rooted!(in(raw_cx) let data_layer = create_empty_array(raw_cx));
    if data_layer.get().is_null() {
        return Err("Failed to create dataLayer array".to_string());
    }

    rooted!(in(raw_cx) let data_layer_val = ObjectValue(data_layer.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("dataLayer").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        data_layer_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

// ============================================================================
// Helper functions
// ============================================================================

/// Create an empty JavaScript array
unsafe fn create_empty_array(raw_cx: *mut JSContext) -> *mut JSObject {
    NewArrayObject(raw_cx, &HandleValueArray::empty())
}

unsafe fn define_function(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    func: JSNative,
    nargs: u32,
) -> Result<(), String> {
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let obj_rooted = obj);
    if JS_DefineFunction(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        func,
        nargs,
        JSPROP_ENUMERATE as u32,
    ).is_null() {
        Err(format!("Failed to define function {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_string_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: &str,
) -> Result<(), String> {
    let utf16: Vec<u16> = value.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    rooted!(in(raw_cx) let val = StringValue(&*str_val.get()));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_int_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: i32,
) -> Result<(), String> {
    rooted!(in(raw_cx) let val = Int32Value(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

unsafe fn set_bool_property(
    raw_cx: *mut JSContext,
    obj: *mut JSObject,
    name: &str,
    value: bool,
) -> Result<(), String> {
    rooted!(in(raw_cx) let val = BooleanValue(value));
    rooted!(in(raw_cx) let obj_rooted = obj);
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        raw_cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string(raw_cx: *mut JSContext, val: JSVal) -> String {
    if val.is_undefined() {
        return "undefined".to_string();
    }
    if val.is_null() {
        return "null".to_string();
    }
    if val.is_boolean() {
        return val.to_boolean().to_string();
    }
    if val.is_int32() {
        return val.to_int32().to_string();
    }
    if val.is_double() {
        return val.to_double().to_string();
    }
    if val.is_string() {
        rooted!(in(raw_cx) let str_val = val.to_string());
        if str_val.get().is_null() {
            return String::new();
        }
        return unsafe { mozjs::conversions::jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap()) };
    }

    // For objects, try to convert to source
    rooted!(in(raw_cx) let str_val = JS_ValueToSource(raw_cx, Handle::from_marked_location(&val)));
    if str_val.get().is_null() {
        return "[object]".to_string();
    }
    unsafe { mozjs::conversions::jsstr_to_string(raw_cx, NonNull::new(str_val.get()).unwrap()) }
}

/// Create a JS string from a Rust string
unsafe fn create_js_string(raw_cx: *mut JSContext, s: &str) -> JSVal {
    let utf16: Vec<u16> = s.encode_utf16().collect();
    rooted!(in(raw_cx) let str_val = JS_NewUCStringCopyN(raw_cx, utf16.as_ptr(), utf16.len()));
    StringValue(&*str_val.get())
}

/// Basic CSS selector matching for single selectors
/// Supports: tag, .class, #id, tag.class, tag#id, [attr], [attr=value]
// TODO: Doesn't support complex selectors (descendant, child, sibling combinators, :pseudo-classes, ::pseudo-elements)
fn matches_selector(selector: &str, tag_name: &str, attributes: &AttributeMap) -> bool {
    // Handle comma-separated selectors (any match)
    if selector.contains(',') {
        return selector.split(',')
            .any(|s| matches_selector(s.trim(), tag_name, attributes));
    }

    // Get element's id and class
    let id_attr = attributes.iter()
        .find(|attr| attr.name.local.as_ref() == "id")
        .map(|attr| attr.value.as_str())
        .unwrap_or("");
    let class_attr = attributes.iter()
        .find(|attr| attr.name.local.as_ref() == "class")
        .map(|attr| attr.value.as_str())
        .unwrap_or("");
    let classes: Vec<&str> = class_attr.split_whitespace().collect();

    let selector = selector.trim();

    // ID selector: #id
    if selector.starts_with('#') {
        let id = &selector[1..];
        // Could be #id.class or #id[attr]
        if let Some(dot_pos) = id.find('.') {
            let (id_part, class_part) = id.split_at(dot_pos);
            return id_attr == id_part && classes.contains(&&class_part[1..]);
        }
        if let Some(bracket_pos) = id.find('[') {
            let id_part = &id[..bracket_pos];
            return id_attr == id_part && matches_attribute_selector(&id[bracket_pos..], attributes);
        }
        return id_attr == id;
    }

    // Class selector: .class
    if selector.starts_with('.') {
        let class_selector = &selector[1..];
        // Could be .class1.class2
        if class_selector.contains('.') {
            return class_selector.split('.').all(|c| !c.is_empty() && classes.contains(&c));
        }
        // Could be .class[attr]
        if let Some(bracket_pos) = class_selector.find('[') {
            let class_part = &class_selector[..bracket_pos];
            return classes.contains(&class_part) && matches_attribute_selector(&class_selector[bracket_pos..], attributes);
        }
        return classes.contains(&class_selector);
    }

    // Attribute selector: [attr] or [attr=value]
    if selector.starts_with('[') {
        return matches_attribute_selector(selector, attributes);
    }

    // Tag selector: tag, tag.class, tag#id, tag[attr]
    let tag_lower = tag_name.to_lowercase();

    // Handle tag.class
    if let Some(dot_pos) = selector.find('.') {
        let tag_part = &selector[..dot_pos];
        let class_part = &selector[dot_pos + 1..];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        // Handle multiple classes: tag.class1.class2
        return class_part.split('.').all(|c| !c.is_empty() && classes.contains(&c));
    }

    // Handle tag#id
    if let Some(hash_pos) = selector.find('#') {
        let tag_part = &selector[..hash_pos];
        let id_part = &selector[hash_pos + 1..];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        return id_attr == id_part;
    }

    // Handle tag[attr]
    if let Some(bracket_pos) = selector.find('[') {
        let tag_part = &selector[..bracket_pos];
        if !tag_part.is_empty() && tag_lower != tag_part.to_lowercase() {
            return false;
        }
        return matches_attribute_selector(&selector[bracket_pos..], attributes);
    }

    // Simple tag match
    if selector == "*" {
        return true;
    }
    tag_lower == selector.to_lowercase()
}

/// Match an attribute selector like [attr], [attr=value], [attr^=value], etc.
fn matches_attribute_selector(selector: &str, attributes: &AttributeMap) -> bool {
    if !selector.starts_with('[') || !selector.ends_with(']') {
        return false;
    }
    let inner = &selector[1..selector.len()-1];

    // [attr=value] or [attr="value"]
    if let Some(eq_pos) = inner.find('=') {
        let operator_start = if eq_pos > 0 {
            match inner.chars().nth(eq_pos - 1) {
                Some('^') | Some('$') | Some('*') | Some('~') | Some('|') => eq_pos - 1,
                _ => eq_pos,
            }
        } else {
            eq_pos
        };

        let attr_name = &inner[..operator_start];
        let operator = &inner[operator_start..eq_pos + 1];
        let mut attr_value = &inner[eq_pos + 1..];

        // Remove quotes if present
        if (attr_value.starts_with('"') && attr_value.ends_with('"'))
            || (attr_value.starts_with('\'') && attr_value.ends_with('\'')) {
            attr_value = &attr_value[1..attr_value.len()-1];
        }

        let actual_value = attributes.iter()
            .find(|attr| attr.name.local.as_ref() == attr_name)
            .map(|attr| attr.value.as_str());

        match actual_value {
            Some(val) => match operator {
                "=" => val == attr_value,
                "^=" => val.starts_with(attr_value),
                "$=" => val.ends_with(attr_value),
                "*=" => val.contains(attr_value),
                "~=" => val.split_whitespace().any(|v| v == attr_value),
                "|=" => val == attr_value || val.starts_with(&format!("{}-", attr_value)),
                _ => false,
            },
            None => false,
        }
    } else {
        // [attr] - just check if attribute exists
        attributes.iter().any(|attr| attr.name.local.as_ref() == inner)
    }
}

// ============================================================================
// Document methods
// ============================================================================

/// document.cookie getter implementation
/// Returns all non-httpOnly cookies as a semicolon-separated string
unsafe extern "C" fn document_get_cookie(raw_cx: *mut JSContext, _argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, 0);

    ensure_cookie_jar_initialized();

    let cookie_string = DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();

            COOKIE_JAR.with(|jar| {
                jar.borrow_mut().get_cookie_string(domain, path)
            })
        } else {
            String::new()
        }
    });

    args.rval().set(create_js_string(raw_cx, &cookie_string));
    true
}

/// document.cookie setter implementation
/// Parses and stores a cookie from "name=value; attributes" format
unsafe extern "C" fn document_set_cookie(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let cookie_str = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        args.rval().set(UndefinedValue());
        return true;
    };

    println!("[JS] document.cookie = '{}' (setting cookie)", cookie_str);

    ensure_cookie_jar_initialized();

    DOCUMENT_URL.with(|doc_url| {
        let url_opt = doc_url.borrow();
        if let Some(ref url) = *url_opt {
            let domain = url.host_str().unwrap_or("localhost");
            let path = url.path();

            if let Some(cookie) = Cookie::parse(&cookie_str, domain, path) {
                COOKIE_JAR.with(|jar| {
                    jar.borrow_mut().set_cookie(cookie);
                });
            } else {
                println!("[JS] Warning: Failed to parse cookie: {}", cookie_str);
            }
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// document.getElementById implementation
unsafe extern "C" fn document_get_element_by_id(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let id = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if id.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.getElementById('{}') called", id);

    // Try to find the element in the DOM and extract its data
    let element_data = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            // Search for element with matching id
            if let Some(&node_id) = dom.nodes_to_id.get(&id) {
                // Get the node and extract tag name and attributes
                if let Some(node) = dom.get_node(node_id) {
                    if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                        let tag_name = elem_data.name.local.to_string();
                        let attributes = elem_data.attributes.clone();
                        return Some((node_id, tag_name, attributes));
                    }
                }
            }
        }
        None
    });

    if let Some((node_id, tag_name, attributes)) = element_data {
        // Create a JS element wrapper with real DOM data
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        println!("[JS] Element with id '{}' not found", id);
        args.rval().set(mozjs::jsval::NullValue());
    }

    true
}

/// document.getElementsByTagName implementation
unsafe extern "C" fn document_get_elements_by_tag_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByTagName('{}') called", tag_name);

    // Collect matching elements from the DOM
    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            let tag_name_lower = tag_name.to_lowercase();

            // Traverse all nodes in the DOM
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    let node_tag = elem_data.name.local.to_string().to_lowercase();
                    // Match if tag names match or if searching for "*" (all elements)
                    if tag_name_lower == "*" || node_tag == tag_name_lower {
                        results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        results
    });

    // Create JS array with the matching elements
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, &attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.getElementsByClassName implementation
unsafe extern "C" fn document_get_elements_by_class_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let class_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.getElementsByClassName('{}') called", class_name);

    // Split the class name into multiple classes (space-separated)
    let search_classes: Vec<&str> = class_name.split_whitespace().collect();

    // Collect matching elements from the DOM
    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;

            // Traverse all nodes in the DOM
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    // Get the class attribute
                    if let Some(class_attr) = elem_data.attributes.iter()
                        .find(|attr| attr.name.local.as_ref() == "class")
                    {
                        let element_classes: Vec<&str> = class_attr.value.split_whitespace().collect();
                        // Check if all search classes are present
                        if search_classes.iter().all(|sc| element_classes.contains(sc)) {
                            results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                        }
                    }
                }
            }
        }
        results
    });

    // Create JS array with the matching elements
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, &attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.querySelector implementation
unsafe extern "C" fn document_query_selector(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelector('{}') called", selector);

    // Find the first matching element
    let element_data = DOM_REF.with(|dom_ref| {
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                        return Some((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        None
    });

    if let Some((node_id, tag_name, attributes)) = element_data {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, node_id, &tag_name, &attributes) {
            args.rval().set(js_elem);
        } else {
            args.rval().set(mozjs::jsval::NullValue());
        }
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// document.querySelectorAll implementation
unsafe extern "C" fn document_query_selector_all(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let selector = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.querySelectorAll('{}') called", selector);

    // Collect all matching elements from the DOM
    let matching_elements: Vec<(usize, String, AttributeMap)> = DOM_REF.with(|dom_ref| {
        let mut results = Vec::new();
        if let Some(ref dom) = *dom_ref.borrow() {
            let dom = &**dom;
            for (node_id, node) in dom.nodes.iter() {
                if let crate::dom::NodeData::Element(ref elem_data) = node.data {
                    if matches_selector(&selector, &elem_data.name.local.to_string(), &elem_data.attributes) {
                        results.push((node_id, elem_data.name.local.to_string(), elem_data.attributes.clone()));
                    }
                }
            }
        }
        results
    });

    // Create JS array with the matching elements
    rooted!(in(raw_cx) let array = create_empty_array(raw_cx));

    for (index, (node_id, tag, attrs)) in matching_elements.iter().enumerate() {
        if let Ok(js_elem) = element_bindings::create_js_element_by_id(raw_cx, *node_id, tag, &attrs) {
            rooted!(in(raw_cx) let elem_val = js_elem);
            rooted!(in(raw_cx) let array_obj = array.get());
            mozjs::rust::wrappers::JS_SetElement(raw_cx, array_obj.handle().into(), index as u32, elem_val.handle().into());
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// document.createElement implementation
unsafe extern "C" fn document_create_element(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let tag_name = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    if tag_name.is_empty() {
        args.rval().set(mozjs::jsval::NullValue());
        return true;
    }

    println!("[JS] document.createElement('{}') called", tag_name);

    // Create a stub element
    match element_bindings::create_stub_element(raw_cx, &tag_name) {
        Ok(elem) => args.rval().set(elem),
        Err(_) => args.rval().set(mozjs::jsval::NullValue()),
    }
    true
}

/// document.createTextNode implementation
unsafe extern "C" fn document_create_text_node(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let text = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] document.createTextNode('{}') called", text);

    // Create a text node object
    rooted!(in(raw_cx) let text_node = JS_NewPlainObject(raw_cx));
    if !text_node.get().is_null() {
        let _ = set_int_property(raw_cx, text_node.get(), "nodeType", 3);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeName", "#text");
        let _ = set_string_property(raw_cx, text_node.get(), "textContent", &text);
        let _ = set_string_property(raw_cx, text_node.get(), "nodeValue", &text);
        args.rval().set(ObjectValue(text_node.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// document.createDocumentFragment implementation
unsafe extern "C" fn document_create_document_fragment(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] document.createDocumentFragment() called");

    // Create a document fragment object
    rooted!(in(raw_cx) let fragment = JS_NewPlainObject(raw_cx));
    if !fragment.get().is_null() {
        let _ = set_int_property(raw_cx, fragment.get(), "nodeType", 11);
        let _ = set_string_property(raw_cx, fragment.get(), "nodeName", "#document-fragment");
        let _ = define_function(raw_cx, fragment.get(), "appendChild", Some(element_append_child), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelector", Some(document_query_selector), 1);
        let _ = define_function(raw_cx, fragment.get(), "querySelectorAll", Some(document_query_selector_all), 1);
        args.rval().set(ObjectValue(fragment.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

// ============================================================================
// Window methods
// ============================================================================

/// window.alert implementation
unsafe extern "C" fn window_alert(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    super::alert_callback::trigger_alert(message);
    args.rval().set(UndefinedValue());
    true
}

/// window.confirm implementation
unsafe extern "C" fn window_confirm(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.confirm('{}') called - returning false", message);
    args.rval().set(BooleanValue(false));
    true
}

/// window.prompt implementation
unsafe extern "C" fn window_prompt(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let message = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.prompt('{}') called - returning null", message);
    args.rval().set(mozjs::jsval::NullValue());
    true
}

/// window.requestAnimationFrame implementation
unsafe extern "C" fn window_request_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] requestAnimationFrame called");
    args.rval().set(Int32Value(1)); // Return a dummy request ID
    true
}

/// window.cancelAnimationFrame implementation
unsafe extern "C" fn window_cancel_animation_frame(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] cancelAnimationFrame called");
    args.rval().set(UndefinedValue());
    true
}

/// window.getComputedStyle implementation
unsafe extern "C" fn window_get_computed_style(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] getComputedStyle called");

    // Return an empty style object
    rooted!(in(raw_cx) let style = JS_NewPlainObject(raw_cx));
    if !style.get().is_null() {
        let _ = define_function(raw_cx, style.get(), "getPropertyValue", Some(style_get_property_value), 1);
        args.rval().set(ObjectValue(style.get()));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// window.addEventListener implementation
unsafe extern "C" fn window_add_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.addEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// window.removeEventListener implementation
unsafe extern "C" fn window_remove_event_listener(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let event_type = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] window.removeEventListener('{}') called", event_type);
    args.rval().set(UndefinedValue());
    true
}

/// window.scrollTo implementation
unsafe extern "C" fn window_scroll_to(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollTo called");
    args.rval().set(UndefinedValue());
    true
}

/// window.scrollBy implementation
unsafe extern "C" fn window_scroll_by(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] window.scrollBy called");
    args.rval().set(UndefinedValue());
    true
}

/// window.atob implementation (base64 decode)
unsafe extern "C" fn window_atob(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let encoded = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    match base64::engine::general_purpose::STANDARD.decode(encoded.as_bytes()) {
        Ok(decoded) => {
            if let Ok(s) = String::from_utf8(decoded) {
                args.rval().set(create_js_string(raw_cx, &s));
            } else {
                args.rval().set(create_js_string(raw_cx, ""));
            }
        }
        Err(_) => {
            args.rval().set(create_js_string(raw_cx, ""));
        }
    }
    true
}

/// window.btoa implementation (base64 encode)
unsafe extern "C" fn window_btoa(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let data = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    use base64::Engine;
    let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_bytes());
    args.rval().set(create_js_string(raw_cx, &encoded));
    true
}

// ============================================================================
// Location methods
// ============================================================================

/// location.reload implementation
unsafe extern "C" fn location_reload(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    println!("[JS] location.reload() called");
    args.rval().set(UndefinedValue());
    true
}

/// location.assign implementation
unsafe extern "C" fn location_assign(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.assign('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

/// location.replace implementation
unsafe extern "C" fn location_replace(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let url = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] location.replace('{}') called", url);
    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// localStorage methods
// ============================================================================

/// localStorage.getItem implementation
unsafe extern "C" fn local_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = LOCAL_STORAGE.with(|storage| {
        storage.borrow().get(&key).cloned()
    });

    if let Some(val) = value {
        args.rval().set(create_js_string(raw_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// localStorage.setItem implementation
unsafe extern "C" fn local_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.removeItem implementation
unsafe extern "C" fn local_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.clear implementation
unsafe extern "C" fn local_storage_clear(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    LOCAL_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

/// localStorage.key implementation
unsafe extern "C" fn local_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = LOCAL_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(raw_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

// ============================================================================
// sessionStorage methods
// ============================================================================

/// sessionStorage.getItem implementation
unsafe extern "C" fn session_storage_get_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    let value = SESSION_STORAGE.with(|storage| {
        storage.borrow().get(&key).cloned()
    });

    if let Some(val) = value {
        args.rval().set(create_js_string(raw_cx, &val));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

/// sessionStorage.setItem implementation
unsafe extern "C" fn session_storage_set_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };
    let value = if argc > 1 {
        js_value_to_string(raw_cx, *args.get(1))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().insert(key, value);
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.removeItem implementation
unsafe extern "C" fn session_storage_remove_item(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let key = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().remove(&key);
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.clear implementation
unsafe extern "C" fn session_storage_clear(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    SESSION_STORAGE.with(|storage| {
        storage.borrow_mut().clear();
    });

    args.rval().set(UndefinedValue());
    true
}

/// sessionStorage.key implementation
unsafe extern "C" fn session_storage_key(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let index = if argc > 0 {
        let val = *args.get(0);
        if val.is_int32() {
            val.to_int32() as usize
        } else if val.is_double() {
            val.to_double() as usize
        } else {
            0
        }
    } else {
        0
    };

    let key = SESSION_STORAGE.with(|storage| {
        let storage = storage.borrow();
        storage.keys().nth(index).cloned()
    });

    if let Some(k) = key {
        args.rval().set(create_js_string(raw_cx, &k));
    } else {
        args.rval().set(mozjs::jsval::NullValue());
    }
    true
}

// ============================================================================
// Element methods (shared)
// ============================================================================

/// element.appendChild implementation
unsafe extern "C" fn element_append_child(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    println!("[JS] element.appendChild() called");

    if argc > 0 {
        args.rval().set(*args.get(0));
    } else {
        args.rval().set(UndefinedValue());
    }
    true
}

/// style.getPropertyValue implementation
unsafe extern "C" fn style_get_property_value(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let property = if argc > 0 {
        js_value_to_string(raw_cx, *args.get(0))
    } else {
        String::new()
    };

    println!("[JS] style.getPropertyValue('{}') called", property);
    args.rval().set(create_js_string(raw_cx, ""));
    true
}


