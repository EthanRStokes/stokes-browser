use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use blitz_traits::net::{AbortSignal, Body, Entry, NetHandler, NetProvider, Request};
use bytes::Bytes;
use curl::easy::{Easy2, Handler, List, WriteError};
use curl::Error;
use data_url::DataUrl;
use log::warn;
use tokio::runtime::Handle;

#[derive(Debug)]
pub enum ProviderError {
    Abort,
    Io(std::io::Error),
    DataUrl(data_url::DataUrlError),
    DataUrlBase64(data_url::forgiving_base64::InvalidBase64),
    ReqwestError(Error),
    HttpError(u32),
    #[cfg(feature = "cache")]
    ReqwestMiddlewareError(reqwest_middleware::Error),
}

impl From<std::io::Error> for ProviderError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<data_url::DataUrlError> for ProviderError {
    fn from(value: data_url::DataUrlError) -> Self {
        Self::DataUrl(value)
    }
}

impl From<data_url::forgiving_base64::InvalidBase64> for ProviderError {
    fn from(value: data_url::forgiving_base64::InvalidBase64) -> Self {
        Self::DataUrlBase64(value)
    }
}

impl From<Error> for ProviderError {
    fn from(value: Error) -> Self {
        Self::ReqwestError(value)
    }
}

pub struct StokesNetProvider {
    rt: Handle,
    user_agent: String,
    debug_net: bool,
}

impl StokesNetProvider {
    pub fn new(user_agent: String, debug_net: bool,) -> Self {
        Self {
            rt: Handle::current(),
            user_agent,
            debug_net,
        }
    }
}

impl NetProvider for StokesNetProvider {
    fn fetch(&self, doc_id: usize, mut request: Request, handler: Box<dyn NetHandler>) {
        //println!("STOKES NET PROVIDER: fetching url {}", request.url.to_string());
        if request.url.scheme() == "stokes" {
            match dioxus_asset_resolver::native::serve_asset(request.url.path()) {
                Ok(res) => {
                    handler.bytes(request.url.to_string(), res.into_body().into());
                }
                Err(_) => {
                    warn!("fetching asset from file system error {request:#?}");
                }
            }
        } else {
            let user_agent = self.user_agent.clone();
            let debug_net = self.debug_net;
            self.rt.spawn(async move {
                let url = request.url.to_string();

                let signal = request.signal.take();
                let result = if let Some(signal) = signal {
                    AbortFetch::new(signal, Box::pin(async move { Self::fetch_inner(request, &user_agent).await })).await
                } else {
                    Self::fetch_inner(request, &user_agent).await
                };

                match result {
                    Ok((response_url, bytes)) => {
                        handler.bytes(response_url, bytes);
                        if debug_net {
                            println!("Success {url}");
                        }
                    }
                    Err(e) => {
                        if debug_net {
                            eprintln!("Error fetching {url}: {e:?}");
                        }
                    }
                }
            });
        }
    }
}

struct Collector(Vec<u8>);

impl Handler for Collector {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

impl StokesNetProvider {
    fn apply_request_method(easy: &mut Easy2<Collector>, request: &Request) {
        let body = Self::encode_request_body(request);

        match request.method.as_str() {
            "GET" => {
                easy.get(true).unwrap();
            }
            "POST" => {
                easy.post(true).unwrap();
                if let Some(body) = body.as_deref() {
                    easy.post_fields_copy(body).unwrap();
                }
            }
            "HEAD" => {
                easy.nobody(true).unwrap();
                easy.custom_request("HEAD").unwrap();
            }
            method => {
                easy.custom_request(method).unwrap();
                if let Some(body) = body.as_deref() {
                    easy.post_fields_copy(body).unwrap();
                }
            }
        }
    }

    fn encode_request_body(request: &Request) -> Option<Vec<u8>> {
        match &request.body {
            Body::Empty => None,
            Body::Form(form_data) => {
                let mut encoded = String::new();
                url::form_urlencoded::Serializer::new(&mut encoded).extend_pairs(
                    form_data
                        .iter()
                        .map(|Entry { name, value }| (name.as_str(), value.as_ref())),
                );
                Some(encoded.into_bytes())
            }
            _ => None,
        }
    }

    async fn fetch_inner(request: Request, user_agent: &str) -> Result<(String, Bytes), ProviderError> {
        Ok(match request.url.scheme() {
            "data" => {
                let data_url = DataUrl::process(request.url.as_str())?;
                let decoded = data_url.decode_to_vec()?;
                (request.url.to_string(), Bytes::from(decoded.0))
            },
            "file" => {
                let file_content = std::fs::read(request.url.path())?;
                (request.url.to_string(), Bytes::from(file_content))
            },
            _ => {
                let mut easy = Easy2::new(Collector(Vec::new()));
                easy.url(request.url.as_str())?;

                let mut headers = List::new();
                // Forward any request-level headers first.
                for (name, value) in &request.headers {
                    headers.append(&format!("{}: {}", name.as_str(), value.to_str().unwrap()))?;
                }
                // Add browser-like headers so servers such as Google do not
                // reject the request with a 4xx response.
                headers.append("Accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")?;
                headers.append("Accept-Language: en-US,en;q=0.5")?;
                easy.http_headers(headers)?;

                easy.follow_location(true)?;
                easy.useragent(user_agent)?;
                // Enable automatic decompression for gzip/deflate/br responses.
                easy.accept_encoding("")?;
                Self::apply_request_method(&mut easy, &request);
                match easy.perform() {
                    Ok(_) => {}
                    Err(err) => {
                        return Err(err.into());
                    }
                }

                let status_code = easy.response_code().unwrap_or(0);
                // Only treat a non-2xx response as a hard failure when the
                // body is empty.  If the server sent content (e.g. Google's
                // sorry/CAPTCHA page on 429), render it instead of falling
                // back to our own 404 page.
                let body = easy.get_ref().0.clone();
                if !(200..300).contains(&status_code) && body.is_empty() {
                    return Err(ProviderError::HttpError(status_code));
                }

                // Use the final URL after any redirects as the canonical URL
                let final_url = match easy.effective_url() {
                    Ok(Some(u)) if !u.is_empty() => u.to_string(),
                    _ => request.url.to_string(),
                };

                (final_url, Bytes::from(body))
            }
        })
    }

    pub fn fetch_with_callback(
        &self,
        request: Request,
        callback: Box<dyn FnOnce(Result<(String, Bytes), ProviderError>) + Send + Sync + 'static>,
    ) {
        let user_agent = self.user_agent.clone();

        self.rt.spawn(async move {
            let result = Self::fetch_inner(request, &user_agent).await;

            callback(result);
        });
    }
}

/// A future that is cancellable using an AbortSignal
struct AbortFetch<F, T> {
    signal: AbortSignal,
    future: F,
    _rt: PhantomData<T>,
}

impl<F, T> AbortFetch<F, T> {
    fn new(signal: AbortSignal, future: F) -> Self {
        Self {
            signal,
            future,
            _rt: PhantomData,
        }
    }
}

impl<F, T> Future for AbortFetch<F, T>
where
    F: Future + Unpin + Send + 'static,
    F::Output: Send + Into<Result<T, ProviderError>> + 'static,
    T: Unpin,
{
    type Output = Result<T, ProviderError>;

    fn poll(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        if self.signal.aborted() {
            return Poll::Ready(Err(ProviderError::Abort));
        }

        match Pin::new(&mut self.future).poll(cx) {
            Poll::Ready(output) => Poll::Ready(output.into()),
            Poll::Pending => Poll::Pending,
        }
    }
}