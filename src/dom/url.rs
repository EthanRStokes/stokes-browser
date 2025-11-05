use std::ops::Deref;
use std::str::FromStr;
use style::stylesheets::UrlExtraData;
use url::Url;

#[derive(Clone)]
pub(crate) struct DocUrl {
    base_url: style::servo_arc::Arc<Url>,
}

impl DocUrl {
    pub(crate) fn url_extra_data(&self) -> UrlExtraData {
        UrlExtraData(style::servo_arc::Arc::clone(&self.base_url))
    }

    pub(crate) fn resolve_relative(&self, raw: &str) -> Option<Url> {
        self.base_url.join(raw).ok()
    }
}

impl Default for DocUrl {
    fn default() -> Self {
        Self::from_str("data:text/css;charset=utf-8;base64,").unwrap()
    }
}

impl FromStr for DocUrl {
    type Err = <Url as FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let base_url = style::servo_arc::Arc::new(Url::parse(s)?);
        Ok(Self { base_url })
    }
}

impl From<Url> for DocUrl {
    fn from(base_url: Url) -> Self {
        Self {
            base_url: style::servo_arc::Arc::new(base_url),
        }
    }
}

impl From<style::servo_arc::Arc<Url>> for DocUrl {
    fn from(base_url: style::servo_arc::Arc<Url>) -> Self {
        Self { base_url }
    }
}

impl Deref for DocUrl {
    type Target = Url;

    fn deref(&self) -> &Self::Target {
        &self.base_url
    }
}