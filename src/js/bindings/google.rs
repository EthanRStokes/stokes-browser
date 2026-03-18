/// Native bindings for the `window.google` namespace.
///
/// Several Google scripts (XJS modules, inline snippets) assume that
/// `window.google.*` helpers are already present before they execute.
/// The canonical implementations live in Google-hosted XJS bundles that may
/// not finish loading in this browser.  This module creates the `google`
/// object and every helper as a real native function so that Google's scripts
/// can run without throwing.
///
/// Each function is fully implemented:
///
/// | Function | Behaviour |
/// |---|---|
/// | `google.cv` | Viewport intersection check using real DOM layout |
/// | `google.rll` | Register lazy-load — fires callback immediately when the element is in the viewport (or always, since we have no IntersectionObserver) |
/// | `google.ml` | Error logger — formats and prints to stderr |
/// | `google.fce` | Failure-callback-executor — logs the failure |
/// | `google.drty` | Mark element dirty — invalidates layout via the DOM |
/// | `google.c.e / .r / .b` | CSI timing recorders — store marks in a thread-local map |
/// | `google.c.maft` | Mark Above-Fold Time — records the ATF timestamp |
/// | `google.c.miml` | Mark Initial Markup Loaded — records the IML timestamp |
/// | `google.c.u` | Update timing mark — stores a mark with the current time |
/// | `google.c.q` | Queue callback — fires the callback immediately (no deferred queue needed) |
/// | `google.timers.load.tick` | Load-timer recorder — stores marks in a thread-local map |
/// | `google.tick` | Top-level tick recorder — delegates to the load-timer store |

use crate::js::bindings::dom_bindings::DOM_REF;
use crate::js::helpers::{define_function, get_node_id_from_value, js_value_to_string, set_bool_property, set_int_property, ToSafeCx};
use crate::js::JsRuntime;
use mozjs::jsapi::{
    CallArgs, HandleValueArray, JSContext, JSObject,
    JSPROP_ENUMERATE, JSPROP_PERMANENT, JSPROP_READONLY,
};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsval::{DoubleValue, Int32Value, JSVal, ObjectValue, UndefinedValue};
use mozjs::rooted;
use mozjs::rust::ValueArray;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::CString;
use std::os::raw::c_uint;
use std::time::{SystemTime, UNIX_EPOCH};
use mozjs::realm::AutoRealm;
use mozjs::rust::wrappers2::{CurrentGlobalOrNull, JS_CallFunctionValue, JS_ClearPendingException, JS_DefineProperty, JS_GetProperty, JS_NewPlainObject};
// ============================================================================
// Thread-local state
// ============================================================================

thread_local! {
    /// CSI timing marks: (timer_name, key) → timestamp_ms
    static CSI_MARKS: RefCell<HashMap<(String, String), f64>> =
        RefCell::new(HashMap::new());

    /// Load-timer marks from google.timers.load.tick(key, time)
    static LOAD_MARKS: RefCell<HashMap<String, f64>> =
        RefCell::new(HashMap::new());

    /// Millisecond timestamp captured when this page's JS runtime was
    /// initialized (approximates the navigation start time).
    static NAV_START_MS: RefCell<f64> = const { RefCell::new(0.0) };
}

fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
        * 1000.0
}

// ============================================================================
// Public setup
// ============================================================================

/// Install the `window.google` object and all required helpers.
///
/// Must be called after DOM bindings exist (so `window` / `document` are
/// available) but before any page scripts run.
pub fn setup_google_polyfill(runtime: &mut JsRuntime) -> Result<(), String> {
    // Snapshot the navigation start time once per page load.
    NAV_START_MS.with(|ns| *ns.borrow_mut() = epoch_ms());

    runtime.do_with_jsapi(|cx, global| unsafe {
        setup_google(cx, global.get())
    })
}

// ============================================================================
// Internal setup helpers
// ============================================================================

unsafe fn define_prop(
    cx: &mut SafeJSContext,
    parent: *mut JSObject,
    name: &str,
    val: JSVal,
) -> Result<(), String> {
    define_prop_with_flags(cx, parent, name, val, JSPROP_ENUMERATE as u32)
}

/// Define a property that cannot be deleted or overwritten via assignment.
///
/// This is used for sub-objects such as `google.c` that Google XJS scripts
/// may try to replace entirely (e.g. `google.c = {…}` without `maft`).
/// Making the property non-writable and non-configurable means the assignment
/// silently fails in sloppy mode and our polyfill object is preserved.
/// Individual named properties *on* the protected object are still writable,
/// so Google's XJS modules can still upgrade individual functions.
unsafe fn define_prop_sealed(
    cx: &mut SafeJSContext,
    parent: *mut JSObject,
    name: &str,
    val: JSVal,
) -> Result<(), String> {
    define_prop_with_flags(
        cx,
        parent,
        name,
        val,
        (JSPROP_ENUMERATE | JSPROP_READONLY | JSPROP_PERMANENT) as u32,
    )
}

unsafe fn define_prop_with_flags(
    cx: &mut SafeJSContext,
    parent: *mut JSObject,
    name: &str,
    val: JSVal,
    flags: u32,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let parent_root = parent);
    rooted!(in(raw_cx) let val_root = val);
    let cname = CString::new(name).unwrap();
    if !JS_DefineProperty(
        cx,
        parent_root.handle().into(),
        cname.as_ptr(),
        val_root.handle().into(),
        flags,
    ) {
        Err(format!("Failed to define property '{}'", name))
    } else {
        Ok(())
    }
}

unsafe fn new_plain_object(cx: &mut SafeJSContext, name: &str) -> Result<*mut JSObject, String> {
    let obj = JS_NewPlainObject(cx);
    if obj.is_null() {
        Err(format!("Failed to create object for '{}'", name))
    } else {
        Ok(obj)
    }
}

unsafe fn setup_google(cx: &mut mozjs::context::JSContext, global: *mut JSObject) -> Result<(), String> {
    // ------------------------------------------------------------------
    // Retrieve or create the top-level `google` object.
    // Google's own inline HTML scripts may have already set `window.google`
    // to a plain object — we extend it rather than replace it.
    // ------------------------------------------------------------------
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let global_root = global);
    rooted!(in(raw_cx) let mut existing = UndefinedValue());
    let google_cname = CString::new("google").unwrap();
    JS_GetProperty(
        cx,
        global_root.handle().into(),
        google_cname.as_ptr(),
        existing.handle_mut().into(),
    );

    rooted!(in(raw_cx) let google = if existing.get().is_object() && !existing.get().is_null() {
        existing.get().to_object()
    } else {
        let obj = new_plain_object(cx, "google")?;
        // Seal window.google so Google's own page-init script
        // (`var google = {kEI:…, c:{e:…}}`) cannot replace this object and
        // lose our polyfill functions.  In sloppy mode the assignment silently
        // fails; individual property writes (google.kEI = '…') still work
        // because only the container reference is sealed, not its members.
        define_prop_sealed(cx, global, "google", ObjectValue(obj))?;
        obj
    });

    // ------------------------------------------------------------------
    // google.cv — Check Viewport
    // ------------------------------------------------------------------
    // Only install if not already provided by a Google XJS bundle.
    rooted!(in(raw_cx) let mut cv_val = UndefinedValue());
    let cv_cname = CString::new("cv").unwrap();
    JS_GetProperty(cx, google.handle().into(), cv_cname.as_ptr(), cv_val.handle_mut().into());
    if !cv_val.get().is_object() {
        define_function(cx, google.get(), "cv", Some(google_cv), 3)?;
    }

    // google.rll — Register Lazy Load
    define_function(cx, google.get(), "rll", Some(google_rll), 3)?;

    // google.ml — Error logger
    define_function(cx, google.get(), "ml", Some(google_ml), 3)?;

    // google.fce — Failure / error callback executor
    define_function(cx, google.get(), "fce", Some(google_fce), 3)?;

    // google.drty — Mark element dirty / request re-layout
    define_function(cx, google.get(), "drty", Some(google_drty), 1)?;

    // google.tick — top-level timing tick (google.tick(timer, key, value, label))
    define_function(cx, google.get(), "tick", Some(google_tick), 4)?;

    // ------------------------------------------------------------------
    // google.c — CSI / timing sub-object
    // Sealed (non-writable, non-configurable) so that Google XJS scripts
    // that do `google.c = {…}` cannot replace it with an object that lacks
    // our polyfill functions (e.g. maft, miml, u, q).
    // ------------------------------------------------------------------
    rooted!(in(raw_cx) let google_c = setup_google_c(cx)?);
    define_prop_sealed(cx, google.get(), "c", ObjectValue(google_c.get()))?;

    // ------------------------------------------------------------------
    // google.timers — timer tracking sub-object (also sealed)
    // ------------------------------------------------------------------
    rooted!(in(raw_cx) let google_timers = setup_google_timers(cx)?);
    define_prop_sealed(cx, google.get(), "timers", ObjectValue(google_timers.get()))?;

    // ------------------------------------------------------------------
    // google.stvsc — navigation-start / scroll checkpoint (also sealed)
    // ------------------------------------------------------------------
    rooted!(in(raw_cx) let google_stvsc = new_plain_object(cx, "stvsc")?);
    let ns = NAV_START_MS.with(|v| *v.borrow());
    define_prop(cx, google_stvsc.get(), "ns", DoubleValue(ns))?;
    define_prop_sealed(cx, google.get(), "stvsc", ObjectValue(google_stvsc.get()))?;

    // ------------------------------------------------------------------
    // google.xsrf — XSRF token map (populated later by server responses)
    // ------------------------------------------------------------------
    rooted!(in(raw_cx) let google_xsrf = new_plain_object(cx, "xsrf")?);
    define_prop(cx, google.get(), "xsrf", ObjectValue(google_xsrf.get()))?;

    Ok(())
}

unsafe fn invoke_callback_with_global_this(cx: &mut SafeJSContext, callback_val: JSVal) {
    let raw_cx = cx.raw_cx();
    if !callback_val.is_object() || callback_val.is_null() {
        return;
    }

    rooted!(in(raw_cx) let callback_obj = callback_val.to_object());
    let mut cx = AutoRealm::new_from_handle(cx, callback_obj.handle());
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let this = CurrentGlobalOrNull(&cx));
    if this.get().is_null() {
        return;
    }

    rooted!(in(raw_cx) let mut rval = UndefinedValue());
    rooted!(in(raw_cx) let callable = callback_val);
    rooted!(in(raw_cx) let zero_args = ValueArray::<0usize>::new([]));

    JS_CallFunctionValue(
        &mut cx,
        this.handle().into(),
        callable.handle().into(),
        &HandleValueArray::from(&zero_args),
        rval.handle_mut().into(),
    );

    // Ignore any JS exception thrown by the callback — these hooks are fire-and-forget.
    JS_ClearPendingException(&cx);
}

unsafe fn setup_google_c(cx: &mut SafeJSContext) -> Result<*mut JSObject, String> {
    let raw_cx = cx.raw_cx();
    let c = new_plain_object(cx, "google.c")?;

    // Timing recorders
    define_function(cx, c, "e", Some(google_c_e), 3)?;
    define_function(cx, c, "r", Some(google_c_r), 2)?;
    define_function(cx, c, "b", Some(google_c_b), 2)?;

    // Mark Above-Fold Time — called as google.c.maft(timestamp, label)
    define_function(cx, c, "maft", Some(google_c_maft), 2)?;

    // Mark Initial Markup Loaded — called as google.c.miml(timestamp)
    define_function(cx, c, "miml", Some(google_c_miml), 1)?;

    // Update timing mark — called as google.c.u(key)
    define_function(cx, c, "u", Some(google_c_u), 1)?;

    // Queue callback for a timing mark — called as google.c.q(key, callback)
    define_function(cx, c, "q", Some(google_c_q), 2)?;

    // Boolean flags read by inline Google scripts.
    // ubf  — "update before first": if true, google.c.u("frt") is called before maft
    set_bool_property(cx, c, "ubf", false)?;
    // cae  — "content already evaluated": if true, maft is skipped
    set_bool_property(cx, c, "cae", false)?;
    // wpr  — "wait for page ready": if true, ATF is gated through google.c.q
    set_bool_property(cx, c, "wpr", false)?;
    // uchfv — flag used to select the wh threshold in the polling loop
    set_bool_property(cx, c, "uchfv", false)?;
    // wh   — wait-hint frame count used by the polling loop when wpr is true
    set_int_property(cx, c, "wh", 0)?;

    // iim  — "immediately-invoke metrics" map, populated by other inline scripts
    rooted!(in(raw_cx) let iim = new_plain_object(cx, "google.c.iim")?);
    define_prop(cx, c, "iim", ObjectValue(iim.get()))?;

    Ok(c)
}

unsafe fn setup_google_timers(cx: &mut SafeJSContext) -> Result<*mut JSObject, String> {
    // google.timers = { load: { e: {}, t: {}, tick: fn } }
    let timers = new_plain_object(cx, "google.timers")?;
    let load = new_plain_object(cx, "google.timers.load")?;
    let e_obj = new_plain_object(cx, "google.timers.load.e")?;
    let t_obj = new_plain_object(cx, "google.timers.load.t")?;
    define_prop(cx, load, "e", ObjectValue(e_obj))?;
    define_prop(cx, load, "t", ObjectValue(t_obj))?;
    define_function(cx, load, "tick", Some(google_timers_load_tick), 2)?;
    define_prop(cx, timers, "load", ObjectValue(load))?;
    Ok(timers)
}

// ============================================================================
// Viewport check helper (shared by google.cv and google.rll)
// ============================================================================

/// Compute the viewport-intersection bitmask for the DOM node with the given
/// `node_id`.
///
/// Return value (bitmask):
///   bit 0 (1) — element intersects the current viewport
///   bit 2 (4) — "completion" hint; always set so the ATF finalisation path
///               in Google's async-image framework can proceed
fn viewport_bitmask(node_id: usize) -> i32 {
    let in_viewport = DOM_REF.with(|dom_ref| {
        let borrow = dom_ref.borrow();
        let dom_ptr = match *borrow {
            Some(p) => p,
            None => return false,
        };
        let dom = unsafe { &*dom_ptr };
        let node = match dom.get_node(node_id) {
            Some(n) => n,
            None => return false,
        };

        // Absolute position in document/CSS-pixel space
        let abs = node.absolute_position(0.0, 0.0);
        let size = node.final_layout.size;

        // Viewport dimensions in CSS pixels
        let scale = dom.viewport.scale() as f32;
        let vp_w = dom.viewport.window_size.0 as f32 / scale;
        let vp_h = dom.viewport.window_size.1 as f32 / scale;

        // Convert to viewport-relative coordinates by subtracting the page
        // scroll offset (which is also stored in CSS pixels).
        let scroll_x = dom.viewport_scroll.x as f32;
        let scroll_y = dom.viewport_scroll.y as f32;
        let rel_left = abs.x - scroll_x;
        let rel_top = abs.y - scroll_y;
        let rel_right = rel_left + size.width;
        let rel_bottom = rel_top + size.height;

        // An element is "in viewport" when its box overlaps [0, vp_w] × [0, vp_h]
        rel_top < vp_h && rel_bottom > 0.0 && rel_left < vp_w && rel_right > 0.0
    });

    let mut result = 0i32;
    if in_viewport {
        result |= 1; // above-the-fold bit
    }
    result |= 4; // completion bit — always set
    result
}

// ============================================================================
// google.cv
// ============================================================================

/// `google.cv(element, strict, container) → number`
///
/// Checks whether `element` is currently in the viewport and returns a
/// bitmask describing the result:
///   - bit 0 (1): element is above the fold (intersects the viewport)
///   - bit 2 (4): completion hint — the ATF framework uses this to know
///                that it no longer needs to wait for this element
unsafe extern "C" fn google_cv(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // Extract the node_id from the element passed as the first argument.
    let bitmask = if argc >= 1 {
        match get_node_id_from_value(safe_cx, *args.get(0)) {
            Some(node_id) => viewport_bitmask(node_id),
            // Non-DOM argument (e.g. a plain object): not in viewport, but
            // still set the completion bit so ATF finalisation can proceed.
            None => 4,
        }
    } else {
        4 // no argument → completion bit only
    };

    args.rval().set(Int32Value(bitmask));
    true
}

// ============================================================================
// google.rll
// ============================================================================

/// `google.rll(element, eager, callback)`
///
/// "Register Lazy Load" — in a real browser this schedules `callback` to fire
/// when `element` enters the viewport.  Because this browser has no
/// IntersectionObserver infrastructure we fire the callback immediately
/// whenever the element is either already in the viewport or `eager` is true.
/// For off-screen elements the callback is still invoked so that lazy-loaded
/// resources do not stall page rendering indefinitely.
unsafe extern "C" fn google_rll(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc < 3 {
        args.rval().set(UndefinedValue());
        return true;
    }

    // Determine whether the element is in the viewport.
    let in_viewport = if let Some(node_id) = get_node_id_from_value(safe_cx, *args.get(0)) {
        viewport_bitmask(node_id) & 1 != 0
    } else {
        false
    };

    let eager = argc >= 2 && (*args.get(1)).is_boolean() && (*args.get(1)).to_boolean();

    // We always fire the callback: either because the element is visible now,
    // because eager=true was requested, or as a safe fallback so off-screen
    // images are not permanently blocked.
    let callback_val = *args.get(2);
    let _ = in_viewport; // used above; keep variable for clarity
    let _ = eager;

    invoke_callback_with_global_this(safe_cx, callback_val);

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.ml
// ============================================================================

/// `google.ml(error, fatal, extraData)`
///
/// Google's machine-learning / error-reporting sink.  Formats the error and
/// writes it to the browser's console output.
unsafe extern "C" fn google_ml(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let error_str = if argc >= 1 {
        js_value_to_string(safe_cx, *args.get(0))
    } else {
        "unknown error".to_string()
    };

    let fatal = argc >= 2 && (*args.get(1)).is_boolean() && (*args.get(1)).to_boolean();

    if fatal {
        eprintln!("[google.ml] FATAL: {}", error_str);
    } else {
        eprintln!("[google.ml] {}", error_str);
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.fce
// ============================================================================

/// `google.fce(container, callbackId, error)`
///
/// "Fire Callback on Error" — invoked when an async request fails.  Logs the
/// failure so it appears in the browser's debug output.
unsafe extern "C" fn google_fce(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let callback_id = if argc >= 2 {
        js_value_to_string(safe_cx, *args.get(1))
    } else {
        "(unknown)".to_string()
    };

    let error_str = if argc >= 3 {
        js_value_to_string(safe_cx, *args.get(2))
    } else {
        "(no error details)".to_string()
    };

    eprintln!("[google.fce] async request failed — id='{}' error={}", callback_id, error_str);

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.drty
// ============================================================================

/// `google.drty(element?)`
///
/// Marks an element (or the whole document) as dirty so it will be
/// re-rendered.  Triggers layout invalidation on the identified DOM node.
unsafe extern "C" fn google_drty(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 1 {
        if let Some(node_id) = get_node_id_from_value(safe_cx, *args.get(0)) {
            DOM_REF.with(|dom_ref| {
                let borrow = dom_ref.borrow();
                if let Some(dom_ptr) = *borrow {
                    let dom = unsafe { &*dom_ptr };
                    if let Some(node) = dom.get_node(node_id) {
                        // Signal that this node's subtree needs re-layout.
                        node.dirty_descendants
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                        if let Some(cb) = &node.layout_invalidation_callback {
                            cb();
                        }
                    }
                }
            });
        }
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.c.e / .r / .b  — CSI timing
// ============================================================================

/// `google.c.e(timerName, key, value)`
///
/// Records a CSI (Client-Side Instrumentation) timing mark.
unsafe extern "C" fn google_c_e(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 3 {
        let timer = js_value_to_string(safe_cx, *args.get(0));
        let key = js_value_to_string(safe_cx, *args.get(1));
        let value = if (*args.get(2)).is_number() {
            (*args.get(2)).to_number()
        } else {
            epoch_ms()
        };
        CSI_MARKS.with(|m| {
            m.borrow_mut().insert((timer, key), value);
        });
    }

    args.rval().set(UndefinedValue());
    true
}

/// `google.c.r(timerName, key)` — read a previously stored CSI mark.
unsafe extern "C" fn google_c_r(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    let result = if argc >= 2 {
        let timer = js_value_to_string(safe_cx, *args.get(0));
        let key = js_value_to_string(safe_cx, *args.get(1));
        CSI_MARKS.with(|m| m.borrow().get(&(timer, key)).copied())
    } else {
        None
    };

    match result {
        Some(v) => args.rval().set(DoubleValue(v)),
        None => args.rval().set(UndefinedValue()),
    }
    true
}

/// `google.c.b(key, time)` — record a "begin" mark (alias of .e with a
/// synthetic timer name).
unsafe extern "C" fn google_c_b(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 1 {
        let key = js_value_to_string(safe_cx, *args.get(0));
        let time = if argc >= 2 && (*args.get(1)).is_number() {
            (*args.get(1)).to_number()
        } else {
            epoch_ms()
        };
        CSI_MARKS.with(|m| {
            m.borrow_mut().insert(("begin".to_string(), key), time);
        });
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.c.maft
// ============================================================================

/// `google.c.maft(timestamp, label)`
///
/// "Mark Above-Fold Time" — records the time at which the above-fold portion
/// of the page was fully painted.  The timestamp is in milliseconds since
/// the Unix epoch (as returned by `Date.now()`).
unsafe extern "C" fn google_c_maft(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let ts = if argc >= 1 && (*args.get(0)).is_number() {
        (*args.get(0)).to_number()
    } else {
        epoch_ms()
    };

    CSI_MARKS.with(|m| m.borrow_mut().insert(("load".to_string(), "aft".to_string()), ts));

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.c.miml
// ============================================================================

/// `google.c.miml(timestamp)`
///
/// "Mark Initial Markup Loaded" — records the time at which Google's server-
/// rendered initial markup was inserted into the DOM.
unsafe extern "C" fn google_c_miml(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let ts = if argc >= 1 && (*args.get(0)).is_number() {
        (*args.get(0)).to_number()
    } else {
        epoch_ms()
    };

    CSI_MARKS.with(|m| m.borrow_mut().insert(("load".to_string(), "iml".to_string()), ts));

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.c.u
// ============================================================================

/// `google.c.u(key)`
///
/// "Update" — records a CSI timing mark for the given key using the current
/// wall-clock time.
unsafe extern "C" fn google_c_u(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 1 {
        let key = js_value_to_string(safe_cx, *args.get(0));
        let ts = epoch_ms();
        CSI_MARKS.with(|m| m.borrow_mut().insert(("load".to_string(), key), ts));
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.c.q
// ============================================================================

/// `google.c.q(key, callback)`
///
/// "Queue" — in Google's real XJS framework this defers `callback` until the
/// timing mark `key` has been recorded.  Because this browser processes
/// everything synchronously and has no deferred-mark infrastructure, we fire
/// the callback immediately.  `google.c.wpr` is set to `false` by our setup,
/// so inline scripts that guard on `wpr` will not call this function at all;
/// it is provided here as a fallback in case any script calls it directly.
unsafe extern "C" fn google_c_q(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    // arg 0: key (string, ignored — we fire immediately)
    // arg 1: callback (function)
    if argc >= 2 {
        let callback_val = *args.get(1);
        invoke_callback_with_global_this(safe_cx, callback_val);
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.tick
// ============================================================================

/// `google.tick(timerName, key, value, label)`
///
/// Top-level timing recorder.  Stores the mark in both the Rust-side
/// LOAD_MARKS store and — when `timerName` is "load" — the CSI_MARKS store
/// so that later `google.c.r("load", key)` calls can retrieve it.
unsafe extern "C" fn google_tick(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 2 {
        let timer = js_value_to_string(safe_cx, *args.get(0));
        let key = js_value_to_string(safe_cx, *args.get(1));
        let ts = if argc >= 3 && (*args.get(2)).is_number() {
            (*args.get(2)).to_number()
        } else {
            epoch_ms()
        };

        LOAD_MARKS.with(|m| m.borrow_mut().insert(key.clone(), ts));
        CSI_MARKS.with(|m| m.borrow_mut().insert((timer, key), ts));
    }

    args.rval().set(UndefinedValue());
    true
}

// ============================================================================
// google.timers.load.tick
// ============================================================================

/// `google.timers.load.tick(key, optionalTime)`
///
/// Records a load-phase timing mark.  Entries are stored in a thread-local
/// map and can be read back for diagnostics.
unsafe extern "C" fn google_timers_load_tick(
    raw_cx: *mut JSContext,
    argc: c_uint,
    vp: *mut JSVal,
) -> bool {
    let args = CallArgs::from_vp(vp, argc);
    let safe_cx = &mut raw_cx.to_safe_cx();

    if argc >= 1 {
        let key = js_value_to_string(safe_cx, *args.get(0));
        let time = if argc >= 2 && (*args.get(1)).is_number() {
            (*args.get(1)).to_number()
        } else {
            epoch_ms()
        };
        LOAD_MARKS.with(|m| {
            m.borrow_mut().insert(key, time);
        });
    }

    args.rval().set(UndefinedValue());
    true
}
