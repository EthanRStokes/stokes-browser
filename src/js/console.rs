// Console API implementation for JavaScript
use boa_engine::{Context, JsResult as BoaResult, JsValue, NativeFunction, object::builtins::JsArray, JsString};
use boa_gc::{Finalize, Trace};

/// Console object for JavaScript
#[derive(Debug, Clone, Trace, Finalize)]
pub struct Console;

impl Console {
    /// Format arguments for console output
    fn format_args(args: &[JsValue], context: &mut Context) -> String {
        args.iter()
            .map(|arg| {
                if arg.is_string() {
                    arg.to_string(context)
                        .map(|s| s.to_std_string_escaped())
                        .unwrap_or_else(|_| String::from("[Error converting to string]"))
                } else if arg.is_object() {
                    // Try to stringify objects
                    format!("{:?}", arg)
                } else {
                    arg.display().to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// console.log implementation
    fn log(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = Self::format_args(args, context);
        println!("[JS] {}", message);
        Ok(JsValue::undefined())
    }

    /// console.error implementation
    fn error(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = Self::format_args(args, context);
        eprintln!("[JS Error] {}", message);
        Ok(JsValue::undefined())
    }

    /// console.warn implementation
    fn warn(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = Self::format_args(args, context);
        println!("[JS Warning] {}", message);
        Ok(JsValue::undefined())
    }

    /// console.info implementation
    fn info(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = Self::format_args(args, context);
        println!("[JS Info] {}", message);
        Ok(JsValue::undefined())
    }

    /// console.debug implementation
    fn debug(_this: &JsValue, args: &[JsValue], context: &mut Context) -> BoaResult<JsValue> {
        let message = Self::format_args(args, context);
        println!("[JS Debug] {}", message);
        Ok(JsValue::undefined())
    }
}

/// Set up the console object in the JavaScript context
pub fn setup_console(context: &mut Context) -> Result<(), String> {
    use boa_engine::object::ObjectInitializer;

    let console = ObjectInitializer::new(context)
        .function(NativeFunction::from_fn_ptr(Console::log), JsString::from("log"), 0)
        .function(NativeFunction::from_fn_ptr(Console::error), JsString::from("error"), 0)
        .function(NativeFunction::from_fn_ptr(Console::warn), JsString::from("warn"), 0)
        .function(NativeFunction::from_fn_ptr(Console::info), JsString::from("info"), 0)
        .function(NativeFunction::from_fn_ptr(Console::debug), JsString::from("debug"), 0)
        .build();

    context.register_global_property(JsString::from("console"), console, boa_engine::property::Attribute::all())
        .map_err(|e| format!("Failed to register console: {}", e))?;

    Ok(())
}

