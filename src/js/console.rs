// Console API implementation for JavaScript using V8
use std::rc::Rc;
use std::cell::RefCell;

/// Console object for JavaScript
pub struct Console;

impl Console {
    /// Format arguments for console output
    fn format_args(scope: &mut v8::HandleScope, args: &[v8::Local<v8::Value>]) -> String {
        args.iter()
            .map(|arg| {
                if let Some(s) = arg.to_string(scope) {
                    s.to_rust_string_lossy(scope)
                } else {
                    "[Error converting to string]".to_string()
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// console.log implementation
    fn log(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        _retval: v8::ReturnValue,
    ) {
        let args_vec: Vec<v8::Local<v8::Value>> = (0..args.length())
            .map(|i| args.get(i))
            .collect();
        let message = Self::format_args(scope, &args_vec);
        println!("[JS] {}", message);
    }

    /// console.error implementation
    fn error(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        _retval: v8::ReturnValue,
    ) {
        let args_vec: Vec<v8::Local<v8::Value>> = (0..args.length())
            .map(|i| args.get(i))
            .collect();
        let message = Self::format_args(scope, &args_vec);
        eprintln!("[JS Error] {}", message);
    }

    /// console.warn implementation
    fn warn(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        _retval: v8::ReturnValue,
    ) {
        let args_vec: Vec<v8::Local<v8::Value>> = (0..args.length())
            .map(|i| args.get(i))
            .collect();
        let message = Self::format_args(scope, &args_vec);
        println!("[JS Warning] {}", message);
    }

    /// console.info implementation
    fn info(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        _retval: v8::ReturnValue,
    ) {
        let args_vec: Vec<v8::Local<v8::Value>> = (0..args.length())
            .map(|i| args.get(i))
            .collect();
        let message = Self::format_args(scope, &args_vec);
        println!("[JS Info] {}", message);
    }

    /// console.debug implementation
    fn debug(
        scope: &mut v8::HandleScope,
        args: v8::FunctionCallbackArguments,
        _retval: v8::ReturnValue,
    ) {
        let args_vec: Vec<v8::Local<v8::Value>> = (0..args.length())
            .map(|i| args.get(i))
            .collect();
        let message = Self::format_args(scope, &args_vec);
        println!("[JS Debug] {}", message);
    }
}

/// Set up the console object in the JavaScript context
pub fn setup_console(
    scope: &mut v8::PinScope,
    global: v8::Local<v8::Object>,
) -> Result<(), String> {
    // Create console object
    let console_obj = v8::Object::new(scope);

    // Add console methods
    let log_name = v8::String::new(scope, "log").unwrap();
    let log_fn = v8::Function::new(scope, Console::log).unwrap();
    console_obj.set(scope, log_name.into(), log_fn.into());

    let error_name = v8::String::new(scope, "error").unwrap();
    let error_fn = v8::Function::new(scope, Console::error).unwrap();
    console_obj.set(scope, error_name.into(), error_fn.into());

    let warn_name = v8::String::new(scope, "warn").unwrap();
    let warn_fn = v8::Function::new(scope, Console::warn).unwrap();
    console_obj.set(scope, warn_name.into(), warn_fn.into());

    let info_name = v8::String::new(scope, "info").unwrap();
    let info_fn = v8::Function::new(scope, Console::info).unwrap();
    console_obj.set(scope, info_name.into(), info_fn.into());

    let debug_name = v8::String::new(scope, "debug").unwrap();
    let debug_fn = v8::Function::new(scope, Console::debug).unwrap();
    console_obj.set(scope, debug_name.into(), debug_fn.into());

    // Set console on global object
    let console_name = v8::String::new(scope, "console").unwrap();
    global.set(scope, console_name.into(), console_obj.into());

    Ok(())
}
