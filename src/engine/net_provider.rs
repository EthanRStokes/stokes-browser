use std::sync::Arc;
use blitz_traits::net::{NetHandler, NetProvider, Request};
use log::warn;

pub struct StokesNetProvider {
}

impl StokesNetProvider {
    pub fn new() -> Self {
        Self {}
    }
}

impl NetProvider for StokesNetProvider {
    fn fetch(&self, doc_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        println!("STOKES NET PROVIDER: fetching url {}", request.url.to_string());
        if request.url.scheme() == "stokes" {
            match dioxus_asset_resolver::native::serve_asset(request.url.path()) {
                Ok(res) => {
                    handler.bytes(request.url.to_string(), res.into_body().into())
                }
                Err(_) => {
                    warn!("fetching asset from file system error {request:#?}");
                }
            }
        } else {
            warn!("unsupported URL scheme: {}", request.url.scheme());
        }
    }
}