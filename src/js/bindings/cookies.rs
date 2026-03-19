// Cookie implementation for browser storage.
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};
#[cfg(target_os = "linux")]
use dbus::blocking::Connection as DbusConnection;
#[cfg(target_os = "linux")]
use std::time::Duration;
use rand::Rng;

const STORAGE_VERSION: u32 = 3;
const SQLITE_SCHEMA_VERSION: u32 = 1;
const MAX_COOKIE_NAME_LEN: usize = 1024;
const MAX_COOKIE_VALUE_LEN: usize = 4096;
const COOKIE_DB_FILE: &str = "cookies.sqlite";
const LEGACY_COOKIE_FILE: &str = "cookies.json";
const COOKIE_KEYRING_SERVICE: &str = "stokes-browser";
const COOKIE_KEYRING_USERNAME: &str = "cookie-encryption-key-v1";
#[cfg(target_os = "linux")]
const KWALLET_FOLDER: &str = "StokesBrowser";

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

fn get_cookies_config_dir() -> PathBuf {
    static COOKIES_CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
    COOKIES_CONFIG_DIR
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

            config_dir
        })
        .clone()
}

fn get_cookies_db_path() -> PathBuf {
    get_cookies_config_dir().join(COOKIE_DB_FILE)
}

fn get_legacy_cookies_file_path() -> PathBuf {
    get_cookies_config_dir().join(LEGACY_COOKIE_FILE)
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

#[derive(Debug)]
struct CookieCrypto {
    key: Option<[u8; 32]>,
    decrypt_keys: Vec<[u8; 32]>,
}

impl CookieCrypto {
    fn decode_key(encoded: &str) -> Option<[u8; 32]> {
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .ok()?;
        if decoded.len() != 32 {
            return None;
        }

        let mut key = [0_u8; 32];
        key.copy_from_slice(&decoded);
        Some(key)
    }

    fn keyring_read_key(entry: &keyring::Entry) -> Option<[u8; 32]> {
        entry
            .get_password()
            .ok()
            .and_then(|encoded| Self::decode_key(&encoded))
    }

    fn keyring_write_key(entry: &keyring::Entry, key: &[u8; 32]) -> bool {
        let encoded = base64::engine::general_purpose::STANDARD.encode(key);
        if entry.set_password(&encoded).is_err() {
            return false;
        }

        entry
            .get_password()
            .ok()
            .and_then(|roundtrip| Self::decode_key(&roundtrip))
            .is_some_and(|roundtrip| roundtrip == *key)
    }

    #[cfg(target_os = "linux")]
    fn kwallet_services() -> [(&'static str, &'static str); 2] {
        [
            ("org.kde.kwalletd6", "/modules/kwalletd6"),
            ("org.kde.kwalletd5", "/modules/kwalletd5"),
        ]
    }

    #[cfg(target_os = "linux")]
    fn kwallet_read_key() -> Option<[u8; 32]> {
        let conn = DbusConnection::new_session().ok()?;

        for (service, path) in Self::kwallet_services() {
            let proxy = conn.with_proxy(service, path, Duration::from_millis(1500));
            let wallet = match proxy.method_call("org.kde.KWallet", "networkWallet", ()) as Result<(String,), _> {
                Ok((wallet,)) => wallet,
                Err(_) => continue,
            };

            let handle = match proxy.method_call(
                "org.kde.KWallet",
                "open",
                (wallet, 0_i64, COOKIE_KEYRING_SERVICE.to_string()),
            ) as Result<(i32,), _> {
                Ok((handle,)) => handle,
                Err(_) => continue,
            };

            if handle < 0 {
                continue;
            }

            let has_folder = proxy
                .method_call(
                    "org.kde.KWallet",
                    "hasFolder",
                    (handle, KWALLET_FOLDER.to_string(), COOKIE_KEYRING_SERVICE.to_string()),
                )
                .ok()
                .map(|r: (bool,)| r.0)
                .unwrap_or(false);

            if !has_folder {
                continue;
            }

            let password = (proxy.method_call(
                "org.kde.KWallet",
                "readPassword",
                (
                    handle,
                    KWALLET_FOLDER.to_string(),
                    COOKIE_KEYRING_USERNAME.to_string(),
                    COOKIE_KEYRING_SERVICE.to_string(),
                ),
            ) as Result<(String,), _>)
                .ok()
                .map(|r| r.0);

            if let Some(password) = password {
                if let Some(key) = Self::decode_key(&password) {
                    return Some(key);
                }
            }
        }

        None
    }

    #[cfg(target_os = "linux")]
    fn kwallet_write_key(key: &[u8; 32]) -> bool {
        let conn = match DbusConnection::new_session() {
            Ok(conn) => conn,
            Err(_) => return false,
        };

        let encoded = base64::engine::general_purpose::STANDARD.encode(key);

        for (service, path) in Self::kwallet_services() {
            let proxy = conn.with_proxy(service, path, Duration::from_millis(1500));
            let wallet = match proxy.method_call("org.kde.KWallet", "networkWallet", ()) as Result<(String,), _> {
                Ok((wallet,)) => wallet,
                Err(_) => continue,
            };

            let handle = match (proxy.method_call(
                "org.kde.KWallet",
                "open",
                (wallet, 0_i64, COOKIE_KEYRING_SERVICE.to_string()),
            ) as Result<(i32,), _>) {
                Ok((handle,)) => handle,
                Err(_) => continue,
            };

            if handle < 0 {
                continue;
            }

            let has_folder = proxy
                .method_call(
                    "org.kde.KWallet",
                    "hasFolder",
                    (handle, KWALLET_FOLDER.to_string(), COOKIE_KEYRING_SERVICE.to_string()),
                )
                .ok()
                .map(|r: (bool,)| r.0)
                .unwrap_or(false);

            if !has_folder {
                let created = proxy
                    .method_call(
                        "org.kde.KWallet",
                        "createFolder",
                        (handle, KWALLET_FOLDER.to_string(), COOKIE_KEYRING_SERVICE.to_string()),
                    )
                    .ok()
                    .map(|r: (bool,)| r.0)
                    .unwrap_or(false);
                if !created {
                    continue;
                }
            }

            let write_ok = (proxy
                .method_call(
                    "org.kde.KWallet",
                    "writePassword",
                    (
                        handle,
                        KWALLET_FOLDER.to_string(),
                        COOKIE_KEYRING_USERNAME.to_string(),
                        encoded.clone(),
                        COOKIE_KEYRING_SERVICE.to_string(),
                    ),
                ) as Result<(i32,), _>)
                .ok()
                .map(|r| r.0 == 0)
                .unwrap_or(false);

            if !write_ok {
                continue;
            }

            let verified = (proxy
                .method_call(
                    "org.kde.KWallet",
                    "readPassword",
                    (
                        handle,
                        KWALLET_FOLDER.to_string(),
                        COOKIE_KEYRING_USERNAME.to_string(),
                        COOKIE_KEYRING_SERVICE.to_string(),
                    ),
                ) as Result<(String,), _>)
                .ok()
                .map(|r| r.0)
                .and_then(|value| Self::decode_key(&value))
                .is_some_and(|decoded| decoded == *key);

            if verified {
                return true;
            }
        }

        false
    }

    fn load_or_create() -> Self {
        let keyring_entry = keyring::Entry::new(COOKIE_KEYRING_SERVICE, COOKIE_KEYRING_USERNAME).ok();

        let mut active_key = keyring_entry
            .as_ref()
            .and_then(Self::keyring_read_key);

        #[cfg(target_os = "linux")]
        if active_key.is_none() {
            active_key = Self::kwallet_read_key();
        }

        if active_key.is_none() {
            let mut generated_key = [0_u8; 32];
            let mut rng = rand::rng();
            rng.fill_bytes(&mut generated_key);
            active_key = Some(generated_key);
        }

        let Some(key) = active_key else {
            eprintln!("[Cookies] Warning: cookie encryption key unavailable; persistence is disabled");
            return Self {
                key: None,
                decrypt_keys: Vec::new(),
            };
        };

        #[cfg(target_os = "linux")]
        {
            if !Self::kwallet_write_key(&key) {
                eprintln!("[Cookies] Warning: cookie encryption key unavailable; persistence is disabled");
                return Self {
                    key: None,
                    decrypt_keys: Vec::new(),
                };
            }
        }

        if let Some(entry) = keyring_entry.as_ref() {
            let _ = Self::keyring_write_key(entry, &key);
        }

        Self {
            key: Some(key),
            decrypt_keys: vec![key],
        }
    }

    fn encrypt_value(&self, value: &str) -> Option<(Vec<u8>, [u8; 12])> {
        let key = self.key?;
        let cipher = Aes256Gcm::new_from_slice(&key).ok()?;

        let mut nonce = [0_u8; 12];
        let mut rng = rand::rng();
        rng.fill_bytes(&mut nonce);

        let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), value.as_bytes()).ok()?;
        Some((ciphertext, nonce))
    }

    fn decrypt_value(&self, ciphertext: &[u8], nonce: &[u8]) -> Option<String> {
        if nonce.len() != 12 {
            return None;
        }

        for key in &self.decrypt_keys {
            let cipher = match Aes256Gcm::new_from_slice(key) {
                Ok(cipher) => cipher,
                Err(_) => continue,
            };

            let Ok(plaintext) = cipher.decrypt(Nonce::from_slice(nonce), ciphertext) else {
                continue;
            };

            if let Ok(value) = String::from_utf8(plaintext) {
                return Some(value);
            }
        }

        None
    }

    fn encryption_enabled(&self) -> bool {
        self.key.is_some()
    }
}

#[derive(Debug)]
struct CookieStore {
    conn: Connection,
    crypto: CookieCrypto,
}

impl CookieStore {
    fn open() -> Option<Self> {
        let db_path = get_cookies_db_path();
        let conn = match Connection::open(&db_path) {
            Ok(conn) => conn,
            Err(err) => {
                eprintln!(
                    "[Cookies] Failed to open SQLite cookie DB {}: {}",
                    db_path.display(),
                    err
                );
                return None;
            }
        };

        if let Err(err) = conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA trusted_schema = OFF;
            PRAGMA secure_delete = ON;
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            ",
        ) {
            eprintln!("[Cookies] Failed to configure SQLite pragmas: {err}");
        }

        let mut store = Self {
            conn,
            crypto: CookieCrypto::load_or_create(),
        };

        if let Err(err) = store.initialize_schema() {
            eprintln!("[Cookies] Failed to initialize cookie DB schema: {err}");
            return None;
        }

        if let Err(err) = store.import_legacy_json_once() {
            eprintln!("[Cookies] Legacy cookie import failed: {err}");
        }

        Some(store)
    }

    fn initialize_schema(&mut self) -> rusqlite::Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;

        tx.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS cookies (
                name TEXT NOT NULL,
                domain TEXT NOT NULL,
                host_only INTEGER NOT NULL,
                path TEXT NOT NULL,
                secure INTEGER NOT NULL,
                http_only INTEGER NOT NULL,
                same_site TEXT,
                creation_time INTEGER NOT NULL,
                last_access_time INTEGER NOT NULL,
                expires_at INTEGER,
                persistent INTEGER NOT NULL,
                value_plaintext TEXT,
                value_encrypted BLOB,
                value_nonce BLOB,
                is_encrypted INTEGER NOT NULL DEFAULT 0,
                PRIMARY KEY (name, domain, path)
            );

            CREATE INDEX IF NOT EXISTS idx_cookies_domain ON cookies(domain);
            CREATE INDEX IF NOT EXISTS idx_cookies_expires ON cookies(expires_at);
            ",
        )?;

        tx.execute(
            "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![SQLITE_SCHEMA_VERSION.to_string()],
        )?;

        tx.commit()?;
        Ok(())
    }

    fn meta_value(&self, key: &str) -> rusqlite::Result<Option<String>> {
        self.conn
            .query_row("SELECT value FROM meta WHERE key = ?1", params![key], |row| {
                row.get::<_, String>(0)
            })
            .optional()
    }

    fn set_meta_value(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    fn import_legacy_json_once(&mut self) -> rusqlite::Result<()> {
        let already_imported = self
            .meta_value("legacy_json_imported")?
            .is_some_and(|value| value == "1");

        if already_imported {
            return Ok(());
        }

        let path = get_legacy_cookies_file_path();
        if !path.exists() {
            self.set_meta_value("legacy_json_imported", "1")?;
            return Ok(());
        }

        let contents = match std::fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(err) => {
                eprintln!(
                    "[Cookies] Failed to read legacy cookie file {}: {}",
                    path.display(),
                    err
                );
                return Ok(());
            }
        };

        let now = now_millis();
        let parsed: Option<Vec<Cookie>> = if let Ok(persisted) = serde_json::from_str::<PersistedCookieJar>(&contents)
        {
            Some(
                persisted
                    .cookies
                    .into_iter()
                    .filter_map(CookieJar::normalize_for_store)
                    .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
                    .collect(),
            )
        } else if let Ok(legacy) = serde_json::from_str::<LegacyCookieJar>(&contents) {
            Some(
                legacy
                    .cookies
                    .into_iter()
                    .filter_map(|legacy_cookie| legacy_cookie.into_modern(now))
                    .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
                    .collect(),
            )
        } else {
            None
        };

        if let Some(cookies) = parsed {
            let deduped = CookieJar::dedupe_overwrite(cookies);
            self.replace_all(&deduped)?;
            self.set_meta_value("legacy_json_imported", "1")?;
            return Ok(());
        }

        eprintln!("[Cookies] Failed to parse legacy cookie storage {}", path.display());
        Ok(())
    }

    fn delete_cookies_by_rowid(&self, rowids: &[i64]) {
        if rowids.is_empty() {
            return;
        }

        let mut stmt = match self.conn.prepare("DELETE FROM cookies WHERE rowid = ?1") {
            Ok(stmt) => stmt,
            Err(err) => {
                eprintln!("[Cookies] Failed to prepare invalid-cookie cleanup statement: {err}");
                return;
            }
        };

        for rowid in rowids {
            if let Err(err) = stmt.execute(params![rowid]) {
                eprintln!("[Cookies] Failed to delete invalid cookie row {rowid}: {err}");
            }
        }
    }

    fn load_cookies(&self) -> rusqlite::Result<Vec<Cookie>> {
        let mut stmt = self.conn.prepare(
            "
            SELECT
                rowid,
                name,
                domain,
                host_only,
                path,
                secure,
                http_only,
                same_site,
                creation_time,
                last_access_time,
                expires_at,
                persistent,
                value_plaintext,
                value_encrypted,
                value_nonce,
                is_encrypted
            FROM cookies
            ",
        )?;

        let now = now_millis();
        let mut rows = stmt.query([])?;
        let mut cookies = Vec::new();
        let mut invalid_rowids: Vec<i64> = Vec::new();

        while let Some(row) = rows.next()? {
            let rowid = row.get::<_, i64>(0)?;
            let is_encrypted = row.get::<_, i64>(15)? != 0;
            let value_plaintext = row.get::<_, Option<String>>(12)?;
            let value_encrypted = row.get::<_, Option<Vec<u8>>>(13)?;
            let value_nonce = row.get::<_, Option<Vec<u8>>>(14)?;

            let value = if is_encrypted {
                match (value_encrypted.as_deref(), value_nonce.as_deref()) {
                    (Some(ciphertext), Some(nonce)) => match self.crypto.decrypt_value(ciphertext, nonce) {
                        Some(value) => value,
                        None => {
                            eprintln!("[Cookies] Skipping encrypted cookie that could not be decrypted");
                            invalid_rowids.push(rowid);
                            continue;
                        }
                    },
                    _ => {
                        invalid_rowids.push(rowid);
                        continue;
                    }
                }
            } else {
                value_plaintext.unwrap_or_default()
            };

            let same_site = row
                .get::<_, Option<String>>(7)?
                .as_deref()
                .and_then(SameSite::parse);

            let mut cookie = Cookie {
                name: row.get(1)?,
                value,
                domain: row.get(2)?,
                host_only: row.get::<_, i64>(3)? != 0,
                path: row.get(4)?,
                secure: row.get::<_, i64>(5)? != 0,
                http_only: row.get::<_, i64>(6)? != 0,
                same_site,
                creation_time: row.get::<_, u64>(8)?,
                last_access_time: row.get::<_, u64>(9)?,
                expires_at: row.get::<_, Option<u64>>(10)?,
                persistent: row.get::<_, i64>(11)? != 0,
            };

            if cookie.last_access_time < cookie.creation_time {
                cookie.last_access_time = cookie.creation_time;
            }

            if cookie.persistent && !cookie.is_expired_at(now) {
                cookies.push(cookie);
            }
        }

        self.delete_cookies_by_rowid(&invalid_rowids);
        Ok(cookies)
    }

    fn replace_all(&mut self, cookies: &[Cookie]) -> rusqlite::Result<()> {
        if !self.crypto.encryption_enabled() {
            return Ok(());
        }

        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute("DELETE FROM cookies", [])?;

        for cookie in cookies {
            if !cookie.persistent {
                continue;
            }

            let Some((ciphertext, nonce)) = self.crypto.encrypt_value(&cookie.value) else {
                return Ok(());
            };

            let value_plaintext: Option<String> = None;
            let value_encrypted: Option<Vec<u8>> = Some(ciphertext);
            let value_nonce: Option<Vec<u8>> = Some(nonce.to_vec());
            let is_encrypted = 1_i64;

            tx.execute(
                "
                INSERT INTO cookies (
                    name,
                    domain,
                    host_only,
                    path,
                    secure,
                    http_only,
                    same_site,
                    creation_time,
                    last_access_time,
                    expires_at,
                    persistent,
                    value_plaintext,
                    value_encrypted,
                    value_nonce,
                    is_encrypted
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                ",
                params![
                    &cookie.name,
                    &cookie.domain,
                    if cookie.host_only { 1_i64 } else { 0_i64 },
                    &cookie.path,
                    if cookie.secure { 1_i64 } else { 0_i64 },
                    if cookie.http_only { 1_i64 } else { 0_i64 },
                    cookie
                        .same_site
                        .as_ref()
                        .map(|same_site| format!("{same_site:?}").to_ascii_lowercase()),
                    cookie.creation_time,
                    cookie.last_access_time,
                    cookie.expires_at,
                    if cookie.persistent { 1_i64 } else { 0_i64 },
                    value_plaintext,
                    value_encrypted,
                    value_nonce,
                    is_encrypted
                ],
            )?;
        }

        tx.commit()?;
        Ok(())
    }

    fn clear(&mut self) -> rusqlite::Result<()> {
        self.conn.execute("DELETE FROM cookies", [])?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct CookieJar {
    cookies: Vec<Cookie>,
    store: Option<CookieStore>,
}

impl Default for CookieJar {
    fn default() -> Self {
        Self {
            cookies: Vec::new(),
            store: None,
        }
    }
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
        let store = match CookieStore::open() {
            Some(store) => store,
            None => return Self::new(),
        };

        let cookies = match store.load_cookies() {
            Ok(cookies) => Self::dedupe_overwrite(
                cookies
                    .into_iter()
                    .filter_map(Self::normalize_for_store)
                    .collect(),
            ),
            Err(err) => {
                eprintln!("[Cookies] Failed to load cookies from SQLite: {err}");
                Vec::new()
            }
        };

        let mut jar = Self {
            cookies,
            store: Some(store),
        };

        jar.remove_expired();
        jar.save_to_disk();
        jar
    }

    pub fn save_to_disk(&mut self) {
        let now = now_millis();
        let persisted: Vec<Cookie> = self
            .cookies
            .iter()
            .filter(|cookie| cookie.persistent && !cookie.is_expired_at(now))
            .cloned()
            .collect();

        let Some(store) = self.store.as_mut() else {
            return;
        };

        if !store.crypto.encryption_enabled() {
            return;
        }

        if let Err(err) = store.replace_all(&persisted) {
            eprintln!("[Cookies] Failed to persist cookies to SQLite: {err}");
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
        if let Some(store) = self.store.as_mut() {
            if let Err(err) = store.clear() {
                eprintln!("[Cookies] Failed to clear SQLite cookie storage: {err}");
            }
        }
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
        let parsed = Cookie::parse_with_context("a=1; SameSite=None", "example.com", "/", true);
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
