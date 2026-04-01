// AbortSignal and AbortController implementation for JavaScript using mozjs
// Provides the global AbortController constructor and AbortSignal API

use crate::js::JsRuntime;
use mozjs::conversions::jsstr_to_string;
use mozjs::jsapi::CallArgs;
use mozjs::jsval::{Int32Value, StringValue, UndefinedValue, BooleanValue};
use mozjs::rooted;
use mozjs::rust::wrappers2::JS_NewUCStringCopyN;
use crate::js::helpers::ToSafeCx;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;

#[derive(Clone)]
struct AbortControllerState {
    id: u32,
    signal_id: u32,
    aborted: bool,
    reason: Option<String>,
}

#[derive(Clone)]
struct SignalState {
    id: u32,
    controller_id: u32,
    aborted: bool,
    reason: Option<String>,
}

thread_local! {
    static ABORT_STATE: RefCell<AbortState> = RefCell::new(AbortState::new());
}

struct AbortState {
    controllers: HashMap<u32, AbortControllerState>,
    signals: HashMap<u32, SignalState>,
    next_id: u32,
}

impl AbortState {
    fn new() -> Self {
        Self {
            controllers: HashMap::new(),
            signals: HashMap::new(),
            next_id: 1,
        }
    }

    fn create_controller(&mut self) -> u32 {
        let controller_id = self.next_id;
        self.next_id += 1;
        let signal_id = self.next_id;
        self.next_id += 1;

        self.controllers.insert(
            controller_id,
            AbortControllerState {
                id: controller_id,
                signal_id,
                aborted: false,
                reason: None,
            },
        );

        self.signals.insert(
            signal_id,
            SignalState {
                id: signal_id,
                controller_id,
                aborted: false,
                reason: None,
            },
        );

        controller_id
    }

    fn get_signal_id(&self, controller_id: u32) -> Option<u32> {
        self.controllers.get(&controller_id).map(|c| c.signal_id)
    }

    fn abort_controller(&mut self, controller_id: u32, reason: Option<String>) {
        if let Some(controller) = self.controllers.get_mut(&controller_id) {
            controller.aborted = true;
            controller.reason = reason.clone();
            let signal_id = controller.signal_id;
            if let Some(signal) = self.signals.get_mut(&signal_id) {
                signal.aborted = true;
                signal.reason = reason;
            }
        }
    }

    fn is_aborted(&self, signal_id: u32) -> bool {
        self.signals.get(&signal_id).map(|s| s.aborted).unwrap_or(false)
    }

    fn get_abort_reason(&self, signal_id: u32) -> Option<String> {
        self.signals.get(&signal_id).and_then(|s| s.reason.clone())
    }
}

pub fn setup_abort_signal(runtime: &mut JsRuntime) -> Result<(), String> {
    let script = r#"
(function () {
    'use strict';
    function AbortSignal(signalId) {
        this._signalId = signalId;
        this._listeners = [];
        this.onabort = null;
    }

    Object.defineProperty(AbortSignal.prototype, 'aborted', {
        get: function() { return _abort_signal_aborted(this._signalId); },
        enumerable: true,
        configurable: true
    });

    Object.defineProperty(AbortSignal.prototype, 'reason', {
        get: function() { return _abort_signal_reason(this._signalId); },
        enumerable: true,
        configurable: true
    });

    AbortSignal.prototype.addEventListener = function(type, listener, options) {
        if (type !== 'abort' || typeof listener !== 'function') return;
        if (!this._listeners) this._listeners = [];
        this._listeners.push({ listener: listener, options: options });
        if (this.aborted) {
            try { listener.call(this, { type: 'abort' }); } catch (e) {}
        }
    };

    AbortSignal.prototype.removeEventListener = function(type, listener, options) {
        if (type !== 'abort' || !this._listeners) return;
        var idx = -1;
        for (var i = 0; i < this._listeners.length; i++) {
            if (this._listeners[i].listener === listener) { idx = i; break; }
        }
        if (idx !== -1) this._listeners.splice(idx, 1);
    };

    AbortSignal.prototype.dispatchEvent = function(event) {
        if (event.type === 'abort' && this.onabort) {
            try { this.onabort.call(this, event); } catch (e) {}
        }
        if (this._listeners) {
            var copy = this._listeners.slice();
            for (var i = 0; i < copy.length; i++) {
                try { copy[i].listener.call(this, event); } catch (e) {}
            }
        }
        return true;
    };

    function AbortController() {
        var controllerId = _abort_controller_new();
        this._controllerId = controllerId;
        this._signal = new AbortSignal(_abort_controller_signal(controllerId));
    }

    Object.defineProperty(AbortController.prototype, 'signal', {
        get: function() { return this._signal; },
        enumerable: true,
        configurable: true
    });

    AbortController.prototype.abort = function(reason) {
        var finalReason = reason !== undefined ? reason : new Error('The operation was aborted');
        _abort_controller_abort(this._controllerId, finalReason);
        if (this._signal) {
            this._signal.dispatchEvent({ type: 'abort', target: this._signal });
        }
    };

    AbortSignal.abort = function(reason) {
        var signal = new AbortSignal(0);
        signal._aborted = true;
        signal._reason = reason;
        return signal;
    };

    AbortSignal.timeout = function(milliseconds) {
        var controller = new AbortController();
        setTimeout(function() {
            var err = new Error('AbortSignal timed out');
            err.name = 'TimeoutError';
            controller.abort(err);
        }, milliseconds);
        return controller.signal;
    };

    globalThis.AbortController = AbortController;
    globalThis.AbortSignal = AbortSignal;
})();
"#;

    runtime.execute(script, false).map_err(|e| {
        eprintln!("[JS] Warning: Failed to setup AbortSignal/AbortController: {}", e);
        e
    })?;

    runtime.add_global_function("_abort_controller_new", |_cx, args| {
        let id = ABORT_STATE.with(|s| s.borrow_mut().create_controller());
        args.rval().set(Int32Value(id as i32));
        true
    });

    runtime.add_global_function("_abort_controller_signal", |_cx, args| {
        let cid = get_u32_arg(&args, 0);
        let sid = ABORT_STATE.with(|s| s.borrow().get_signal_id(cid).unwrap_or(0));
        args.rval().set(Int32Value(sid as i32));
        true
    });

    runtime.add_global_function("_abort_controller_abort", |cx, args| {
        unsafe {
            let cid = get_u32_arg(&args, 0);
            let reason = if args.argc_ > 1 {
                let val = *args.get(1);
                if val.is_string() {
                    let js_str = val.to_string();
                    if !js_str.is_null() {
                        Some(jsstr_to_string(cx, NonNull::new(js_str).unwrap()))
                    } else { None }
                } else { None }
            } else { None };
            ABORT_STATE.with(|s| s.borrow_mut().abort_controller(cid, reason));
            args.rval().set(UndefinedValue());
            true
        }
    });

    runtime.add_global_function("_abort_signal_aborted", |_cx, args| {
        let sid = get_u32_arg(&args, 0);
        let aborted = ABORT_STATE.with(|s| s.borrow().is_aborted(sid));
        args.rval().set(BooleanValue(aborted));
        true
    });

    runtime.add_global_function("_abort_signal_reason", |cx, args| {
        unsafe {
            let sid = get_u32_arg(&args, 0);
            let reason = ABORT_STATE.with(|s| s.borrow().get_abort_reason(sid));
            if let Some(reason_str) = reason {
                let reason_utf16: Vec<u16> = reason_str.encode_utf16().collect();
                let safe_cx = &mut cx.to_safe_cx();
                rooted!(in(safe_cx.raw_cx()) let reason_js_str = JS_NewUCStringCopyN(safe_cx, reason_utf16.as_ptr(), reason_utf16.len()));
                rooted!(in(safe_cx.raw_cx()) let reason_val = StringValue(&*reason_js_str.get()));
                args.rval().set(*reason_val);
            } else {
                args.rval().set(UndefinedValue());
            }
            true
        }
    });

    Ok(())
}

fn get_u32_arg(args: &CallArgs, index: u32) -> u32 {
    if args.argc_ > index {
        let val = *args.get(index);
        if val.is_int32() {
            val.to_int32() as u32
        } else if val.is_double() {
            val.to_double() as u32
        } else {
            0
        }
    } else {
        0
    }
}











