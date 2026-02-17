use mozjs::jsapi::{JSContext, JS_ClearPendingException, JS_GetPendingException, JS_IsExceptionPending};
use mozjs::jsval::UndefinedValue;
use mozjs::rooted;
use tracing::debug;
use crate::js::jsapi::objects::{get_obj_prop_val_as_i32, get_obj_prop_val_as_string};

#[derive(Debug)]
pub struct JsError {
    pub message: String,
    pub filename: String,
    pub lineno: i32,
    pub column: i32,
}

impl JsError {
    pub fn err_msg(&self) -> String {
        format!(
            "{} at {}:{}:{}",
            self.message, self.filename, self.lineno, self.column
        )
    }
}

impl Clone for JsError {
    fn clone(&self) -> Self {
        Self {
            message: self.message.clone(),
            filename: self.filename.clone(),
            lineno: self.lineno,
            column: self.column,
        }
    }
}

/// see if there is a pending exception and return it as a JsError
#[allow(dead_code)]
pub fn get_pending_exception(context: *mut JSContext) -> Option<JsError> {
    if unsafe { JS_IsExceptionPending(context) } {
        rooted!(in(context) let mut error_value = UndefinedValue());
        if unsafe { JS_GetPendingException(context, error_value.handle_mut().into()) } {
            let js_error_obj: *mut mozjs::jsapi::JSObject = error_value.to_object();
            rooted!(in(context) let mut js_error_obj_root = js_error_obj);

            let message =
                get_obj_prop_val_as_string(context, js_error_obj_root.handle(), "message")
                    .ok()
                    .unwrap();
            let filename =
                get_obj_prop_val_as_string(context, js_error_obj_root.handle(), "fileName")
                    .ok()
                    .unwrap();
            let lineno =
                get_obj_prop_val_as_i32(context, js_error_obj_root.handle(), "lineNumber");
            let column =
                get_obj_prop_val_as_i32(context, js_error_obj_root.handle(), "columnNumber");

            let error_info: JsError = JsError {
                message,
                filename,
                lineno,
                column,
            };

            debug!(
                "ex = {} in {} at {}:{}",
                error_info.message, error_info.filename, error_info.lineno, error_info.column
            );

            unsafe { JS_ClearPendingException(context) };
            Some(error_info)
        } else {
            None
        }
    } else {
        None
    }
}