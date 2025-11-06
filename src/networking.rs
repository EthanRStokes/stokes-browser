use std::io::Cursor;
// Networking module for handling HTTP requests
use curl::easy::{Easy, List};
use std::path::Path;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use blitz_traits::net::{NetHandler, Request, SharedCallback, SharedProvider};
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

#[derive(Debug)]
pub enum NetworkError {
    Curl(String),
    Utf8(String),
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
    Image(usize, ImageType, u32, u32, Arc<Vec<u8>>),
    Svg(usize, ImageType, Box<usvg::Tree>),
    Css(usize, DocumentStyleSheet),
    Font(Bytes),
    None,
}

pub struct CssHandler {
    pub node: usize,
    pub url: Url,
    pub lock: SharedRwLock,
    pub provider: SharedProvider<Resource>,
}

#[derive(Clone)]
pub(crate) struct StylesheetLoader(pub(crate) usize, pub(crate) SharedProvider<Resource>);

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

        self.1.fetch(
            self.0,
            Request::get(url.as_ref().clone()),
            Box::new(StylesheetLoaderInner {
                loader: self.clone(),
                lock: lock.clone(),
                url: url.clone(),
                media: media,
                import_rule: import.clone(),
                provider: self.1.clone(),
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
    provider: SharedProvider<Resource>,
}

impl NetHandler<Resource> for StylesheetLoaderInner {
    fn bytes(self: Box<Self>, doc_id: usize, bytes: Bytes, callback: SharedCallback<Resource>) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            callback.call(doc_id, Err(Some(String::from("Invalid UTF8"))));
            return;
        };

        let sheet = ServoArc::new(Stylesheet::from_str(
            css,
            UrlExtraData(self.url),
            Origin::Author,
            self.media.clone(),
            self.lock.clone(),
            Some(&self.loader),
            None, // error_reporter
            QuirksMode::NoQuirks,
            AllowImportRules::Yes,
        ));

        // Fetch @font-face fonts
        fetch_font_face(doc_id, &sheet, &self.provider, &self.lock.read());

        let mut guard = self.lock.write();
        self.import_rule.write_with(&mut guard).stylesheet = ImportSheet::Sheet(sheet);

        callback.call(doc_id, Ok(Resource::None))
    }
}

impl NetHandler<Resource> for CssHandler {
    fn bytes(self: Box<Self>, doc_id: usize, bytes: Bytes, callback: blitz_traits::net::SharedCallback<Resource>) {
        let Ok(css) = std::str::from_utf8(&bytes) else {
            callback.call(doc_id, Err(Some(String::from("Invalid UTF-8 in CSS resource"))));
            return;
        };

        let stylesheet = Stylesheet::from_str(
            css,
            self.url.into(),
            Origin::Author,
            ServoArc::new(self.lock.wrap(MediaList::empty())),
            self.lock.clone(),
            Some(&StylesheetLoader(doc_id, self.provider.clone())),
            None,
            QuirksMode::NoQuirks,
            AllowImportRules::Yes
        );

        fetch_font_face(doc_id, &stylesheet, &self.provider, &self.lock.read());

        callback.call(doc_id, Ok(Resource::Css(self.node, DocumentStyleSheet(ServoArc::new(stylesheet)))));
    }
}

struct FontFaceHandler(FontFaceSourceFormatKeyword);
impl NetHandler<Resource> for FontFaceHandler {
    fn bytes(mut self: Box<Self>, doc_id: usize, bytes: Bytes, callback: SharedCallback<Resource>) {
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

        // Satisfy rustc's mutability linting with woff feature both enabled/disabled
        let mut bytes = bytes;

        match self.0 {
            FontFaceSourceFormatKeyword::Woff => {
                tracing::info!("Decompressing woff1 font");

                // Use woff crate to decompress font
                let decompressed = woff::version1::decompress(&bytes);

                // Use wuff crate to decompress font
                let decompressed = wuff::decompress_woff1(&bytes).ok();

                if let Some(decompressed) = decompressed {
                    bytes = Bytes::from(decompressed);
                } else {
                    tracing::warn!("Failed to decompress woff1 font");
                }
            }
            FontFaceSourceFormatKeyword::Woff2 => {
                tracing::info!("Decompressing woff2 font");

                // Use woff crate to decompress font
                let decompressed = woff::version2::decompress(&bytes);

                // Use wuff crate to decompress font
                let decompressed = wuff::decompress_woff2(&bytes).ok();

                if let Some(decompressed) = decompressed {
                    bytes = Bytes::from(decompressed);
                } else {
                    tracing::warn!("Failed to decompress woff2 font");
                }
            }
            FontFaceSourceFormatKeyword::None => {
                return;
            }
            _ => {}
        }

        callback.call(doc_id, Ok(Resource::Font(bytes)))
    }
}

fn fetch_font_face(
    doc_id: usize,
    sheet: &Stylesheet,
    network_provider: &SharedProvider<Resource>,
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
                #[cfg(feature = "tracing")]
                tracing::warn!("Skipping unsupported font of type {:?}", _font_format);
                return;
            }
            let url = url_source.url.url().unwrap().as_ref().clone();
            network_provider.fetch(doc_id, Request::get(url), Box::new(FontFaceHandler(format)))
        });
}

pub struct ImageHandler(usize, ImageType);
impl ImageHandler {
    pub fn new(node_id: usize, kind: ImageType) -> Self {
        Self(node_id, kind)
    }
}
impl NetHandler<Resource> for ImageHandler {
    fn bytes(self: Box<Self>, doc_id: usize, bytes: Bytes, callback: SharedCallback<Resource>) {
        // Try parse image
        if let Ok(image) = image::ImageReader::new(Cursor::new(&bytes))
            .with_guessed_format()
            .expect("IO errors impossible with Cursor")
            .decode()
        {
            let raw_rgba8_data = image.clone().into_rgba8().into_raw();
            callback.call(
                doc_id,
                Ok(Resource::Image(
                    self.0,
                    self.1,
                    image.width(),
                    image.height(),
                    Arc::new(raw_rgba8_data),
                )),
            );
            return;
        };

        if let Ok(tree) = parse_svg(&bytes) {
            callback.call(doc_id, Ok(Resource::Svg(self.0, self.1, Box::new(tree))));
            return;
        }

        callback.call(doc_id, Err(Some(String::from("Could not parse image"))))
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
    async fn read_local_file(path: &str) -> Result<String, NetworkError> {
        println!("Reading local file: {}", path);

        let path = path.to_string();
        tokio::task::spawn_blocking(move || {
            // Check if file exists
            let file_path = Path::new(&path);
            if !file_path.exists() {
                return Err(NetworkError::FileNotFound(path.clone()));
            }

            // Read the file
            std::fs::read_to_string(file_path)
                .map_err(|e| NetworkError::FileRead(e.to_string()))
        })
        .await
        .map_err(|e| NetworkError::FileRead(e.to_string()))?
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
    pub async fn fetch(&self, url: &str) -> Result<String, NetworkError> {
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
            return Self::read_local_file(&file_path).await;
        }

        // Normalize the URL: if it lacks a scheme, default to https://

        // Run curl operation in a blocking task since curl is synchronous
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();
            let mut data = Vec::new();
            let mut headers = Vec::new();

            // Configure curl
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
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

            Ok::<String, NetworkError>(html)
        }).await.map_err(|e| NetworkError::Curl(e.to_string()))?;

        result
    }

    /// Fetch an image or other resource
    pub async fn fetch_resource(&self, url: &str) -> Result<Vec<u8>, NetworkError> {
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
        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();
            let mut data = Vec::new();
            let mut headers = Vec::new();

            // Configure curl
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
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
    pub async fn head(&self, url: &str) -> Result<bool, NetworkError> {
        let url = match Url::parse(url) {
            Ok(u) => u,
            Err(e) => return Err(NetworkError::Curl(e.to_string())),
        };

        let result = tokio::task::spawn_blocking(move || {
            let mut easy = Easy::new();

            // Configure curl for HEAD request
            easy.url(&url.as_str()).map_err(|e| NetworkError::Curl(e.to_string()))?;
            easy.useragent("Stokes-Browser/1.0").map_err(|e| NetworkError::Curl(e.to_string()))?;
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
