use adblock::engine::Engine;
use adblock::lists::{FilterSet, ParseOptions};
use adblock::request::Request;
use std::cell::RefCell;

const DEFAULT_FILTER_LIST: &str = r#"
! Small default list; will make this system better in the future lol
||doubleclick.net^
||googlesyndication.com^
||googletagmanager.com^
||googleadservices.com^
||adservice.google.com^
||adsystem.com^
||taboola.com^
||outbrain.com^
||scorecardresearch.com^
||zedo.com^
"#;

thread_local! {
    static ADBLOCK_ENGINE: RefCell<Option<Engine>> = const { RefCell::new(None) };
}

fn build_engine() -> Engine {
    let mut filter_set = FilterSet::new(true);
    let _ = filter_set.add_filter_list(DEFAULT_FILTER_LIST, ParseOptions::default());
    Engine::from_filter_set(filter_set, true)
}

pub fn should_block(request_url: &str, source_url: Option<&str>, request_type: &str) -> bool {
    let source = source_url.unwrap_or(request_url);

    ADBLOCK_ENGINE.with(|slot| {
        if slot.borrow().is_none() {
            *slot.borrow_mut() = Some(build_engine());
        }

        let borrow = slot.borrow();
        let Some(engine) = borrow.as_ref() else {
            return false;
        };

        let Ok(request) = Request::new(request_url, source, request_type) else {
            return false;
        };

        engine.check_network_request(&request).matched
    })
}

#[cfg(test)]
mod tests {
    use super::should_block;

    #[test]
    fn blocks_known_ad_domain() {
        assert!(should_block(
            "https://doubleclick.net/ads.js",
            Some("https://example.com"),
            "script"
        ));
    }

    #[test]
    fn allows_regular_content() {
        assert!(!should_block(
            "https://example.com/app.js",
            Some("https://example.com"),
            "script"
        ));
    }
}


