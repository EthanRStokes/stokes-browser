use blitz_net::ProviderError;
use blitz_traits::net::{AbortSignal, Body, Entry, NetHandler, NetProvider, Request};
use bytes::Bytes;
use curl::easy::{Easy2, Handler, List, WriteError};
use data_url::DataUrl;
use log::warn;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::Poll;
use tokio::runtime::Handle;

pub struct StokesNetProvider {
    rt: Handle,
    user_agent: String,
}

impl StokesNetProvider {
    pub fn new(user_agent: String) -> Self {
        Self {
            rt: Handle::current(),
            user_agent,
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
                        println!("Success {url}");
                    }
                    Err(e) => {
                        eprintln!("Error fetching {url}: {e:?}")
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
                easy.url(request.url.as_str()).unwrap();

                let mut headers = List::new();
                for (name, value) in &request.headers {
                    headers.append(&format!("{}: {}", name.as_str(), value.to_str().unwrap())).unwrap();
                }
                easy.http_headers(headers);

                easy.follow_location(true).unwrap();
                easy.useragent(user_agent).unwrap();
                Self::apply_request_method(&mut easy, &request);
                easy.perform().unwrap();

                (request.url.to_string(), Bytes::from(easy.get_ref().0.clone()))
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