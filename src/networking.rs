use std::io::Cursor;
// Networking module for handling HTTP requests
use curl::easy::{Easy, List};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::time::Duration;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use blitz_traits::shell::ShellProvider;
use bytes::Bytes;
use selectors::context::QuirksMode;
use style::font_face::{FontFaceSourceFormat, FontFaceSourceFormatKeyword, Source};
use style::media_queries::MediaList;
use style::shared_lock::{Locked, SharedRwLock, SharedRwLockReadGuard};
use style::stylesheets::{AllowImportRules, CssRule, DocumentStyleSheet, Origin, Stylesheet, StylesheetInDocument, UrlExtraData};
use url::Url;
use style::servo_arc::Arc as ServoArc;
use usvg::fontdb;
use style::parser::ParserContext;
use style::stylesheets::import_rule::{ImportLayer, ImportSheet, ImportSupportsCondition};
use style::stylesheets::{ImportRule, StylesheetLoader as StyloStylesheetLoader};
use style::values::{CssUrl, SourceLocation};
use crate::dom::DomEvent;

#[derive(Debug)]
pub enum NetworkError {
    Curl(String),
    Utf8(String),
    Engine(String),
    Http(u32),
    Empty,
    FileNotFound(String),
    FileRead(String),
}

impl std::fmt::Display for NetworkError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            NetworkError::Curl(msg) => write!(f, "Curl error: {}", msg),
            NetworkError::Utf8(msg) => write!(f, "UTF-8 error: {}", msg),
            NetworkError::Engine(msg) => write!(f, "Engine error: {}", msg),
            NetworkError::Http(code) => write!(f, "HTTP error: {}", code),
            NetworkError::Empty => write!(f, "Empty response body"),
            NetworkError::FileNotFound(path) => write!(f, "File not found: {}", path),
            NetworkError::FileRead(msg) => write!(f, "File read error: {}", msg),
        }
    }
}

impl std::error::Error for NetworkError {}

pub(crate) static FONT_DB: LazyLock<Arc<fontdb::Database>> = LazyLock::new(|| {
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    Arc::new(db)
});

pub(crate) fn parse_svg(source: &[u8]) -> Result<usvg::Tree, usvg::Error> {
    let options = usvg::Options {
        fontdb: Arc::clone(&*FONT_DB),
        ..Default::default()
    };

    let tree = usvg::Tree::from_data(source, &options)?;
    Ok(tree)
}

#[derive(Clone, Debug)]
pub enum ImageType {
    Image,
    Background(usize)
}

#[derive(Clone, Debug)]
pub enum Resource {
    Image(ImageType, u32, u32, Arc<Vec<u8>>),
    Svg(ImageType, Box<usvg::Tree>),
    Css(DocumentStyleSheet),
    Font(Bytes),
    None,
}

pub(crate) struct ResourceHandler<T: Send + Sync + 'static> {
    dom_id: usize,
    request_id: usize,
    node_id: Option<usize>,
    tx: Sender<DomEvent>,
    shell_provider: Arc<dyn ShellProvider>,
    data: T,
}

impl<T: Send + Sync + 'static> ResourceHandler<T> {
    pub(crate) fn new(
        tx: Sender<DomEvent>,
        doc_id: usize,
        node_id: Option<usize>,
        shell_provider: Arc<dyn ShellProvider>,
        data: T,
    ) -> Self {
        static REQUEST_ID_COUNTER: AtomicUsize = AtomicUsize::new(0);
        Self {
            request_id: REQUEST_ID_COUNTER.fetch_add(1, Ordering::Relaxed),
            dom_id: doc_id,
            node_id,
            tx,
            shell_provider,
            data,
        }
    }

    pub(crate) fn boxed(
        tx: Sender<DomEvent>,
        doc_id: usize,
        node_id: Option<usize>,
        shell_provider: Arc<dyn ShellProvider>,
        data: T,
    ) -> Box<dyn NetHandler>
    where
        ResourceHandler<T>: NetHandler,
    {
        Box::new(Self::new(tx, doc_id, node_id, shell_provider, data)) as _
    }

    fn respond(&self, resolved_url: String, result: Result<Resource, String>) {
        let response = ResourceLoadResponse {
            request_id: self.request_id,
            node_id: self.node_id,
            resolved_url: Some(resolved_url),
            result,
        };
        let _ = self.tx.send(DomEvent::ResourceLoad(response));
        self.shell_provider.request_redraw();
    }
}

#[allow(unused)]
pub struct ResourceLoadResponse {
    pub request_id: usize,
    pub node_id: Option<usize>,
    pub resolved_url: Option<String>,
    pub result: Result<Resource, String>,
}

pub struct StylesheetHandler {
    pub source_url: Url,
    pub guard: SharedRwLock,
    pub net_provider: Arc<dyn NetProvider>,
}

impl NetHandler for ResourceHandler<StylesheetHandler> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            return self.respond(resolved_url, Err(String::from("Invalid UTF8")));
        };

        // NOTE(Nico): I don't *think* external stylesheets should have HTML entities escaped
        // let escaped_css = html_escape::decode_html_entities(css);

        let sheet = Stylesheet::from_str(
            css,
            self.data.source_url.clone().into(),
            Origin::Author,
            ServoArc::new(self.data.guard.wrap(MediaList::empty())),
            self.data.guard.clone(),
            Some(&StylesheetLoader {
                tx: self.tx.clone(),
                dom_id: self.dom_id,
                net_provider: self.data.net_provider.clone(),
                shell_provider: self.shell_provider.clone(),
            }),
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        );

        // Fetch @font-face fonts
        fetch_font_face(
            self.tx.clone(),
            self.dom_id,
            self.node_id,
            &sheet,
            &self.data.net_provider,
            &self.shell_provider,
            &self.data.guard.read(),
        );

        self.respond(
            resolved_url,
            Ok(Resource::Css(DocumentStyleSheet(ServoArc::new(sheet)))),
        );
    }
}

#[derive(Clone)]
pub(crate) struct StylesheetLoader {
    pub(crate) tx: Sender<DomEvent>,
    pub(crate) dom_id: usize,
    pub(crate) net_provider: Arc<dyn NetProvider>,
    pub(crate) shell_provider: Arc<dyn ShellProvider>,
}

impl StyloStylesheetLoader for StylesheetLoader {
    fn request_stylesheet(
        &self,
        url: CssUrl,
        location: SourceLocation,
        lock: &SharedRwLock,
        media: ServoArc<Locked<MediaList>>,
        supports: Option<ImportSupportsCondition>,
        layer: ImportLayer
    ) -> ServoArc<Locked<ImportRule>> {
        if !supports.as_ref().is_none_or(|s| s.enabled) {
            return ServoArc::new(lock.wrap(ImportRule {
                url,
                stylesheet: ImportSheet::new_refused(),
                supports,
                layer,
                source_location: location,
            }))
        }

        let import = ImportRule {
            url,
            stylesheet: ImportSheet::new_pending(),
            supports,
            layer,
            source_location: Default::default(),
        };

        let url = import.url.url().unwrap().clone();
        let import = ServoArc::new(lock.wrap(import));

        self.net_provider.fetch(
            self.dom_id,
            Request::get(url.as_ref().clone()),
            ResourceHandler::boxed(
                self.tx.clone(),
                self.dom_id,
                None, // node_id
                self.shell_provider.clone(),
                StylesheetLoaderInner {
                    loader: self.clone(),
                    lock: lock.clone(),
                    url: url.clone(),
                    media,
                    import_rule: import.clone(),
                    provider: self.net_provider.clone(),
            }),
        );

        import
    }
}

struct StylesheetLoaderInner {
    loader: StylesheetLoader,
    lock: SharedRwLock,
    url: ServoArc<Url>,
    media: ServoArc<Locked<MediaList>>,
    import_rule: ServoArc<Locked<ImportRule>>,
    provider: Arc<dyn NetProvider>,
}

impl NetHandler for ResourceHandler<StylesheetLoaderInner> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            return self.respond(resolved_url, Err(String::from("Invalid UTF8")));
        };

        // NOTE(Nico): I don't *think* external stylesheets should have HTML entities escaped
        // let escaped_css = html_escape::decode_html_entities(css);

        let sheet = ServoArc::new(Stylesheet::from_str(
            css,
            UrlExtraData(self.data.url.clone()),
            Origin::Author,
            self.data.media.clone(),
            self.data.lock.clone(),
            Some(&self.data.loader),
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        ));

        // Fetch @font-face fonts
        fetch_font_face(
            self.tx.clone(),
            self.dom_id,
            self.node_id,
            &sheet,
            &self.data.provider,
            &self.shell_provider,
            &self.data.lock.read(),
        );

        let mut guard = self.data.lock.write();
        self.data.import_rule.write_with(&mut guard).stylesheet = ImportSheet::Sheet(sheet);
        drop(guard);

        self.respond(resolved_url, Ok(Resource::None))
    }
}

struct FontFaceHandler(FontFaceSourceFormatKeyword);
impl NetHandler for ResourceHandler<FontFaceHandler> {
    fn bytes(mut self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let result = self.data.parse(bytes);
        self.respond(resolved_url, result)
    }
}
impl FontFaceHandler {
    fn parse(&mut self, bytes: Bytes) -> Result<Resource, String> {
        if self.0 == FontFaceSourceFormatKeyword::None && bytes.len() >= 4 {
            self.0 = match &bytes.as_ref()[0..4] {
                // WOFF (v1) files begin with 0x774F4646 ('wOFF' in ascii)
                // See: <https://w3c.github.io/woff/woff1/spec/Overview.html#WOFFHeader>
                b"wOFF" => FontFaceSourceFormatKeyword::Woff,
                // WOFF2 files begin with 0x774F4632 ('wOF2' in ascii)
                // See: <https://w3c.github.io/woff/woff2/#woff20Header>
                b"wOF2" => FontFaceSourceFormatKeyword::Woff2,
                // Opentype fonts with CFF data begin with 0x4F54544F ('OTTO' in ascii)
                // See: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff#organization-of-an-opentype-font>
                b"OTTO" => FontFaceSourceFormatKeyword::Opentype,
                // Opentype fonts truetype outlines begin with 0x00010000
                // See: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff#organization-of-an-opentype-font>
                &[0x00, 0x01, 0x00, 0x00] => FontFaceSourceFormatKeyword::Truetype,
                // Truetype fonts begin with 0x74727565 ('true' in ascii)
                // See: <https://developer.apple.com/fonts/TrueType-Reference-Manual/RM06/Chap6.html#ScalerTypeNote>
                b"true" => FontFaceSourceFormatKeyword::Truetype,
                _ => FontFaceSourceFormatKeyword::None,
            }
        }

        let mut bytes = bytes;

        match self.0 {
            FontFaceSourceFormatKeyword::Woff => {
                tracing::info!("Decompressing woff1 font");

                // Use woff crate to decompress font
                let decompressed = wuff::decompress_woff1(&bytes);

                if let Some(decompressed) = decompressed.ok() {
                    bytes = Bytes::from(decompressed);
                } else {
                    tracing::warn!("Failed to decompress woff1 font");
                }
            }
            FontFaceSourceFormatKeyword::Woff2 => {
                tracing::info!("Decompressing woff2 font");

                // Use woff crate to decompress font
                let decompressed = wuff::decompress_woff2(&bytes);

                if let Some(decompressed) = decompressed.ok() {
                    bytes = Bytes::from(decompressed);
                } else {
                    tracing::warn!("Failed to decompress woff2 font");
                }
            }
            FontFaceSourceFormatKeyword::None => {
                // Should this be an error?
                return Ok(Resource::None);
            }
            _ => {}
        }

        Ok(Resource::Font(bytes))
    }
}

fn fetch_font_face(
    tx: Sender<DomEvent>,
    doc_id: usize,
    node_id: Option<usize>,
    sheet: &Stylesheet,
    network_provider: &Arc<dyn NetProvider>,
    shell_provider: &Arc<dyn ShellProvider>,
    read_guard: &SharedRwLockReadGuard,
) {
    sheet
        .contents(read_guard)
        .rules(read_guard)
        .iter()
        .filter_map(|rule| match rule {
            CssRule::FontFace(font_face) => font_face.read_with(read_guard).sources.as_ref(),
            _ => None,
        })
        .flat_map(|source_list| &source_list.0)
        .filter_map(|source| match source {
            Source::Url(url_source) => Some(url_source),
            _ => None,
        })
        .for_each(|url_source| {
            let mut format = match &url_source.format_hint {
                Some(FontFaceSourceFormat::Keyword(fmt)) => *fmt,
                Some(FontFaceSourceFormat::String(str)) => match str.as_str() {
                    "woff2" => FontFaceSourceFormatKeyword::Woff2,
                    "ttf" => FontFaceSourceFormatKeyword::Truetype,
                    "otf" => FontFaceSourceFormatKeyword::Opentype,
                    _ => FontFaceSourceFormatKeyword::None,
                },
                _ => FontFaceSourceFormatKeyword::None,
            };
            if format == FontFaceSourceFormatKeyword::None {
                let Some((_, end)) = url_source.url.as_str().rsplit_once('.') else {
                    return;
                };
                format = match end {
                    "woff2" => FontFaceSourceFormatKeyword::Woff2,
                    "woff" => FontFaceSourceFormatKeyword::Woff,
                    "ttf" => FontFaceSourceFormatKeyword::Truetype,
                    "otf" => FontFaceSourceFormatKeyword::Opentype,
                    "svg" => FontFaceSourceFormatKeyword::Svg,
                    "eot" => FontFaceSourceFormatKeyword::EmbeddedOpentype,
                    _ => FontFaceSourceFormatKeyword::None,
                }
            }
            if let _font_format @ (FontFaceSourceFormatKeyword::Svg
            | FontFaceSourceFormatKeyword::EmbeddedOpentype
            | FontFaceSourceFormatKeyword::Woff) = format
            {
                tracing::warn!("Skipping unsupported font of type {:?}", _font_format);
                return;
            }
            let url = url_source.url.url().unwrap().as_ref().clone();
            network_provider.fetch(
                doc_id,
                Request::get(url),
                ResourceHandler::boxed(
                    tx.clone(),
                    doc_id,
                    node_id,
                    shell_provider.clone(),
                    FontFaceHandler(format),
                ),
            );
        });
}

pub struct ImageHandler {
    kind: ImageType,
}
impl ImageHandler {
    pub fn new(kind: ImageType) -> Self {
        Self { kind }
    }
}

impl NetHandler for ResourceHandler<ImageHandler> {
    fn bytes(self: Box<Self>, resolved_url: String, bytes: Bytes) {
        let result = self.data.parse(bytes);
        self.respond(resolved_url, result)
    }
}

impl ImageHandler {
    fn parse(&self, bytes: Bytes) -> Result<Resource, String> {
        // Try parse image
        if let Ok(image) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .expect("IO errors impossible with Cursor")
            .decode()
        {
            let raw_rgba8_data = image.clone().into_rgba8().into_raw();
            return Ok(Resource::Image(
                self.kind.clone(),
                image.width(),
                image.height(),
                Arc::new(raw_rgba8_data),
            ));
        };

        if let Ok(tree) = parse_svg(&bytes) {
            return Ok(Resource::Svg(self.kind.clone(), Box::new(tree)));
        }

        Err(String::from("Could not parse image"))
    }
}


/// A client for making HTTP requests and fetching web resources
pub struct HttpClient {
    // curl::Easy is not Send/Sync, so we'll create it fresh for each request
}

impl HttpClient {
    pub fn new() -> Self {
        Self {}
    }


    /// Convert an input (which may be a file:// URL or a plain filesystem path)
    /// to a local file system path string.
    fn url_to_file_path(input: &str) -> String {
        if input.starts_with("file://") {
            // Url crate can normalize file URLs properly
            match Url::parse(input) {
                Ok(url) => {
                    if let Ok(path) = url.to_file_path() {
                        return path.to_string_lossy().into_owned();
                    }
                }
                Err(_) => {
                    // Fall through to manual stripping
                }
            }
            // Manual fallback: strip file:// prefix
            let path = &input[7..];
            if cfg!(windows) && path.starts_with('/') && path.len() > 2 && path.chars().nth(2) == Some(':') {
                return path[1..].to_string();
            }
            return path.to_string();
        }

        // If the input isn't a file:// URL but looks like a filesystem path,
        // return it as-is.
        input.to_string()
    }

    /// Read a local HTML file
    fn read_local_file(path: &str) -> Result<String, NetworkError> {
        println!("Reading local file: {}", path);

        let path = path.to_string();
        // Check if file exists
        let file_path = Path::new(&path);
        if !file_path.exists() {
            return Err(NetworkError::FileNotFound(path.clone()));
        }

        // Read the file
        std::fs::read_to_string(file_path)
            .map_err(|e| NetworkError::FileRead(e.to_string()))
        .map_err(|e| NetworkError::FileRead(e.to_string()))
    }

    /// Read a local resource file (for images, etc.)
    async fn read_local_resource(path: &str) -> Result<Vec<u8>, NetworkError> {
        println!("Reading local resource: {}", path);

        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            // Check if file exists
            let file_path = Path::new(&path);
            if !file_path.exists() {
                return Err(NetworkError::FileNotFound(path.clone()));
            }

            // Read the file as bytes
            std::fs::read(file_path)
                .map_err(|e| NetworkError::FileRead(e.to_string()))
        })
        .await
        .map_err(|e| NetworkError::FileRead(e.to_string()))?
    }

    /// Fetch HTML content from a URL or local file
    pub fn fetch(&self, url: &str, user_agent: &str) -> Result<String, NetworkError> {
        println!("Fetching: {}", url);

        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(NetworkError::Curl(err.to_string()))
            }
        };

        // Check if it's a local file
        if url.scheme() == "file" {
            let file_path = Self::url_to_file_path(url.as_str());
            return Self::read_local_file(&file_path);
        }

        // Normalize the URL: if it lacks a scheme, default to https://

        // Run curl operation in a blocking task since curl is synchronous
        let user_agent = user_agent.to_string();

        let mut easy = Easy::new();
        let mut data = Vec::new();
        let mut headers = Vec::new();

        // Configure curl
        easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
        easy.useragent(&user_agent).map_err(|e| NetworkError::Curl(e.to_string()))?;
        easy.timeout(Duration::from_secs(30)).map_err(|e| NetworkError::Curl(e.to_string()))?;
        easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
        easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

        // Set up data collection
        {
            let mut transfer = easy.transfer();
            transfer.write_function(|new_data| {
                data.extend_from_slice(new_data);
                Ok(new_data.len())
            }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
            transfer.header_function(|header| {
                headers.push(String::from_utf8_lossy(header).to_string());
                true
            }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
            transfer.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;
        }

        // Check response code
        let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
        if response_code >= 400 {
            return Err(NetworkError::Http(response_code));
        }

        // Check content type
        let content_type = headers.iter()
            .find(|h| h.to_lowercase().starts_with("content-type:"))
            .and_then(|h| h.split(':').nth(1))
            .map(|s| s.trim())
            .unwrap_or("text/html");

        if !content_type.contains("text/html") {
            println!("Warning: Content type is {}, not HTML", content_type);
        }

        // Convert to string
        let html = String::from_utf8(data)
            .map_err(|_| NetworkError::Utf8("Response contains invalid UTF-8".to_string()))?;

        Ok::<String, NetworkError>(html).map_err(|e| NetworkError::Curl(e.to_string()))
    }

    /// Fetch an image or other resource
    pub async fn fetch_resource(&self, url: &str, user_agent: &str) -> Result<Vec<u8>, NetworkError> {
        println!("Fetching resource: {}", url);

        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(err) => {
                return Err(NetworkError::Curl(err.to_string()))
            }
        };

        // Check if it's a local file
        if url.scheme() == "file" {
            let file_path = Self::url_to_file_path(url.as_str());
            return Self::read_local_resource(&file_path).await;
        }

        // Run curl operation in a blocking task
        let user_agent = user_agent.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();
            let mut data = Vec::new();
            let mut headers = Vec::new();

            // Configure curl
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent(&user_agent).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.timeout(Duration::from_secs(30)).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Set Accept header for resources
            let mut header_list = List::new();
            header_list.append("Accept: image/*, */*").map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.http_headers(header_list).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Set up data collection
            {
                let mut transfer = easy.transfer();
                transfer.write_function(|new_data| {
                    data.extend_from_slice(new_data);
                    Ok(new_data.len())
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.header_function(|header| {
                    headers.push(String::from_utf8_lossy(header).to_string());
                    true
                }).map_err(|e| NetworkError::Curl(e.to_string()))?;
                
                transfer.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;
            }

            // Check response code
            let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
            if response_code >= 400 {
                return Err(NetworkError::Http(response_code));
            }

            // Check content type
            if let Some(content_type_header) = headers.iter()
                .find(|h| h.to_lowercase().starts_with("content-type:")) {
                if let Some(content_type) = content_type_header.split(':').nth(1) {
                    let content_type = content_type.trim();
                    println!("Resource content type: {}", content_type);

                    if !content_type.starts_with("image/") && !content_type.contains("svg") {
                        println!("Warning: Expected image content type, got: {}", content_type);
                    }
                }
            }

            // Validate we got some data
            if data.is_empty() {
                return Err(NetworkError::Empty);
            }

            println!("Successfully fetched resource: {} bytes", data.len());
            Ok::<Vec<u8>, NetworkError>(data)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }

    /// Check if a URL is valid and reachable (for validation)
    pub async fn head(&self, url: &str, user_agent: &str) -> Result<bool, NetworkError> {
        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(e) => return Err(NetworkError::Curl(e.to_string())),
        };

        let user_agent = user_agent.to_string();
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();

            // Configure curl for HEAD request
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent(&user_agent).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.timeout(Duration::from_secs(10)).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.nobody(true).map_err(|e| NetworkError::Curl(e.to_string()))?; // This makes it a HEAD request
            easy.follow_location(true).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.max_redirections(5).map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Perform the request
            easy.perform().map_err(|e| NetworkError::Curl(e.to_string()))?;

            // Check response code
            let response_code = easy.response_code().map_err(|e| NetworkError::Curl(e.to_string()))?;
            Ok::<bool, NetworkError>(response_code < 400)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }
}
