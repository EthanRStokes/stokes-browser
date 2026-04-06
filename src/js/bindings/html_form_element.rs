use mozjs::jsapi::{JS_DefineProperty, JS_NewPlainObject, JSObject, JSContext, JSPROP_ENUMERATE};
use mozjs::jsval::ObjectValue;
use mozjs::rooted;

pub(crate) unsafe fn setup_html_form_element_constructor_bindings(
    raw_cx: *mut JSContext,
    global: *mut JSObject,
) -> Result<(), String> {
    rooted!(in(raw_cx) let html_form_element = JS_NewPlainObject(raw_cx));
    if html_form_element.get().is_null() {
        return Err("Failed to create HTMLFormElement constructor".to_string());
    }

    rooted!(in(raw_cx) let html_form_element_val = ObjectValue(html_form_element.get()));
    rooted!(in(raw_cx) let global_rooted = global);
    let name = std::ffi::CString::new("HTMLFormElement").unwrap();
    JS_DefineProperty(
        raw_cx,
        global_rooted.handle().into(),
        name.as_ptr(),
        html_form_element_val.handle().into(),
        JSPROP_ENUMERATE as u32,
    );

    Ok(())
}

