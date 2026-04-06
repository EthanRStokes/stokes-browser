use crate::dom::Dom;
use crate::js::JsResult;
use blitz_traits::net::Request;
use mozjs::context::{JSContext, RawJSContext};
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::Heap;
use mozjs::jsapi::{SetModulePrivate, JSObject};
use mozjs::jsval::{StringValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::{CompileModule1, JS_ClearPendingException, JS_GetPendingException, JS_IsExceptionPending, JS_NewUCStringCopyN, JS_ValueToSource};
use mozjs::rust::CompileOptionsWrapper;
use mozjs::rust::MutableHandleValue;
use mozjs::rust::transform_str_to_source_text;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::sync::mpsc;
use url::Url;

pub(crate) trait ModuleLoader {
    fn effective_module_source_url(&self, source_url: Option<&str>, dom: *mut Dom) -> String;
    fn module_cache_key(&self, source_url: Option<&str>, code: &str, dom: *mut Dom) -> Option<String>;

    unsafe fn prepare_root_module(
        &mut self,
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        code: &str,
        source_name: &str,
        cache_key: Option<&String>,
        print_eval_error: bool,
    ) -> JsResult<*mut JSObject>;

    unsafe fn load_or_compile_module(
        &mut self,
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        specifier: &str,
        referencing_url: Option<&str>,
        dom: *mut Dom,
    ) -> JsResult<*mut JSObject>;
}

pub(crate) struct DefaultModuleLoader {
    module_cache: HashMap<String, Heap<*mut JSObject>>,
}

impl DefaultModuleLoader {
    pub(crate) fn new() -> Self {
        Self {
            module_cache: HashMap::new(),
        }
    }

    pub(crate) fn clear(&mut self) {
        self.module_cache.clear();
    }

    fn compile_module_script(
        context: &mut JSContext,
        text: &str,
        filename: &str,
        line_number: u32,
    ) -> *mut JSObject {
        let options = unsafe { CompileOptionsWrapper::new(&context, filename.parse().unwrap(), line_number) };
        unsafe { CompileModule1(context, options.ptr, &mut transform_str_to_source_text(text)) }
    }

    fn resolve_module_url(specifier: &str, referencing_url: Option<&str>, dom: *mut Dom) -> JsResult<Url> {
        if let Ok(url) = Url::parse(specifier) {
            return Ok(url);
        }

        let base = referencing_url
            .and_then(|raw| Url::parse(raw).ok())
            .or_else(|| unsafe { Url::parse((&*dom).url.to_string().as_str()).ok() })
            .ok_or_else(|| "Unable to resolve module base URL".to_string())?;

        base.join(specifier)
            .map_err(|e| format!("Failed to resolve module specifier '{specifier}': {e}"))
    }

    fn fetch_module_source(url: &Url, dom: *mut Dom) -> JsResult<String> {
        let net_provider = unsafe { (&*dom).net_provider.clone() };
        let (tx, rx) = mpsc::channel();
        net_provider.fetch_with_callback(
            Request::get(url.clone()),
            Box::new(move |result| {
                let _ = tx.send(result);
            }),
        );

        let result = rx
            .recv()
            .map_err(|e| format!("Module fetch callback dropped for '{}': {e}", url))?;

        let (_final_url, bytes) = result
            .map_err(|e| format!("Failed to fetch module '{}': {e:?}", url))?;

        String::from_utf8(bytes.to_vec())
            .map_err(|e| format!("Module '{}' is not valid UTF-8: {e}", url))
    }

    unsafe fn set_module_private_url(
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        module: *mut JSObject,
        source_name: &str,
    ) {
        let source_url_utf16: Vec<u16> = source_name.encode_utf16().collect();
        rooted!(in(raw_cx) let source_url_js = JS_NewUCStringCopyN(context, source_url_utf16.as_ptr(), source_url_utf16.len()));
        rooted!(in(raw_cx) let module_private = StringValue(&*source_url_js.get()));
        let module_private_value = module_private.get();
        SetModulePrivate(module, &module_private_value);
    }

    unsafe fn extract_js_exception(
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        prefix: &str,
        code: &str,
        print_error: bool,
    ) -> String {
        if JS_IsExceptionPending(context) {
            rooted!(in(raw_cx) let mut exception = UndefinedValue());
            if JS_GetPendingException(context, MutableHandleValue::from(exception.handle_mut())) {
                JS_ClearPendingException(context);
                rooted!(in(raw_cx) let exc_str = JS_ValueToSource(context, exception.handle()));
                if !exc_str.get().is_null() {
                    let msg = jsstr_to_string(raw_cx, NonNull::new(exc_str.handle().get()).unwrap());
                    if print_error {
                        return format!("{prefix}: {msg}\n{code}");
                    }
                    return format!("{prefix}: {msg}");
                }
            }
        }

        prefix.to_string()
    }
}

impl ModuleLoader for DefaultModuleLoader {
    fn effective_module_source_url(&self, source_url: Option<&str>, dom: *mut Dom) -> String {
        source_url
            .map(str::to_string)
            .unwrap_or_else(|| unsafe { (&*dom).url.to_string() })
    }

    fn module_cache_key(&self, source_url: Option<&str>, code: &str, dom: *mut Dom) -> Option<String> {
        let module_url = source_url?;
        let doc_url = unsafe { (&*dom).url.to_string() };
        if module_url == doc_url {
            return None;
        }

        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        code.hash(&mut hasher);
        Some(format!("{}#{}", module_url, hasher.finish()))
    }

    unsafe fn prepare_root_module(
        &mut self,
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        code: &str,
        source_name: &str,
        cache_key: Option<&String>,
        print_eval_error: bool,
    ) -> JsResult<*mut JSObject> {
        if let Some(cache_key) = cache_key {
            if let Some(cached) = self.module_cache.get(cache_key) {
                return Ok(cached.get());
            }
        }

        let module = Self::compile_module_script(context, code, source_name, 1);
        if module.is_null() {
            let msg = Self::extract_js_exception(
                context,
                raw_cx,
                "JavaScript MODULE COMPILE error",
                code,
                print_eval_error,
            );
            return Err(msg);
        }

        Self::set_module_private_url(context, raw_cx, module, source_name);

        if let Some(cache_key) = cache_key {
            let mut rooted_module = Heap::default();
            rooted_module.set(module);
            self.module_cache.insert(cache_key.clone(), rooted_module);
        }

        Ok(module)
    }

    unsafe fn load_or_compile_module(
        &mut self,
        context: &mut JSContext,
        raw_cx: *mut RawJSContext,
        specifier: &str,
        referencing_url: Option<&str>,
        dom: *mut Dom,
    ) -> JsResult<*mut JSObject> {
        let resolved_url = Self::resolve_module_url(specifier, referencing_url, dom)?;
        let resolved_url_str = resolved_url.to_string();

        if let Some(module) = self.module_cache.get(&resolved_url_str) {
            return Ok(module.get());
        }

        let source = Self::fetch_module_source(&resolved_url, dom)?;
        let module = Self::compile_module_script(context, &source, &resolved_url_str, 1);
        if module.is_null() {
            return Err(Self::extract_js_exception(
                context,
                raw_cx,
                "JavaScript MODULE COMPILE error",
                &source,
                true,
            ));
        }

        Self::set_module_private_url(context, raw_cx, module, &resolved_url_str);

        let mut rooted_module = Heap::default();
        rooted_module.set(module);
        self.module_cache.insert(resolved_url_str, rooted_module);

        Ok(module)
    }
}


