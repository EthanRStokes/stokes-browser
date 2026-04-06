use crate::dom::{Dom, NodeData};
use crate::engine::js_provider::ScriptKind;
use crate::engine::js_provider::StokesJsProvider;
use crate::engine::net_provider::{ProviderError, StokesNetProvider};
use crate::engine::script_type::executable_script_kind;
use crate::networking::HttpClient;
use blitz_traits::net::Request;
use markup5ever::local_name;
use std::sync::Arc;

/// Parsed script work item discovered in document order.
pub(crate) struct PendingScript {
    pub(crate) node_id: usize,
    pub(crate) kind: ScriptKind,
    pub(crate) inline_script: Option<String>,
    pub(crate) external_url: Option<url::Url>,
    pub(crate) source_url: Option<String>,
}

pub(crate) struct ScriptFetchContext {
    net_provider: Arc<StokesNetProvider>,
}

/// Collect executable script tasks from `<script>` tags without mutating runtime state.
pub(crate) fn collect_pending_scripts(dom: &Dom) -> Vec<PendingScript> {
    let script_elements = dom.query_selector("script");
    let mut pending_scripts = Vec::new();

    for script_element in script_elements {
        if let NodeData::Element(element_data) = &script_element.data {
            let Some(script_kind) = executable_script_kind(element_data.attr(local_name!("type"))) else {
                // Non-JS types are data blocks by spec and are not executed.
                continue;
            };

            let node_id = script_element.id;
            if let Some(src) = element_data.attr(local_name!("src")) {
                let resolved_url = dom.resolve_url(src);
                let source_url = (script_kind == ScriptKind::Module).then(|| resolved_url.to_string());
                pending_scripts.push(PendingScript {
                    node_id,
                    kind: script_kind,
                    inline_script: None,
                    external_url: Some(resolved_url),
                    source_url,
                });
            } else {
                let script_content = script_element.text_content();
                if !script_content.trim().is_empty() {
                    pending_scripts.push(PendingScript {
                        node_id,
                        kind: script_kind,
                        inline_script: Some(script_content),
                        external_url: None,
                        source_url: (script_kind == ScriptKind::Module).then(|| dom.url.to_string()),
                    });
                }
            }
        }
    }

    pending_scripts
}

pub(crate) fn dispatch_script(
    js_provider: &StokesJsProvider,
    script: String,
    node_id: usize,
    kind: ScriptKind,
    source_url: Option<String>,
) {
    if kind == ScriptKind::Module {
        js_provider.execute_module_script_with_node_id(script, node_id, source_url);
    } else {
        js_provider.execute_script_with_node_id(script, node_id);
    }
}

pub(crate) fn resolve_script_fetch_context(
    new_http_client: Option<&HttpClient>,
    dom: Option<&Dom>,
) -> Option<ScriptFetchContext> {
    let net_provider = new_http_client
        .map(|client| client.net_provider.clone())
        .or_else(|| dom.map(|dom| dom.net_provider.clone()))?;

    Some(ScriptFetchContext { net_provider })
}

impl ScriptFetchContext {
    pub(crate) async fn fetch_external_script(&self, request: Request) -> Result<String, String> {
        fetch_external_script(self.net_provider.clone(), request).await
    }
}

pub(crate) async fn fetch_external_script(
    net_provider: Arc<StokesNetProvider>,
    request: Request,
) -> Result<String, String> {
    let request_url = request.url.to_string();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Result<String, String>>();

    net_provider.fetch_with_callback(
        request,
        Box::new(move |result| {
            let response = match result {
                Ok((_, bytes)) => String::from_utf8(bytes.to_vec()).map_err(|error| {
                    format!("External script at '{}' is not valid UTF-8: {}", request_url, error)
                }),
                Err(error) => Err(match error {
                    ProviderError::Blocked => {
                        format!("Blocked by content filtering: {}", request_url)
                    }
                    _ => format!("{:?}", error),
                }),
            };

            let _ = tx.send(response);
        }),
    );

    rx.recv()
        .await
        .ok_or_else(|| "Script fetch callback dropped before script delivery".to_string())?
}



