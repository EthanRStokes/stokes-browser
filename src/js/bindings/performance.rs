// Performance API implementation for JavaScript using mozjs
use crate::js::JsRuntime;
use mozjs::jsapi::{CallArgs, JSContext, JSNative, JSObject, JSPROP_ENUMERATE, JSPROP_READONLY};
use mozjs::context::JSContext as SafeJSContext;
use mozjs::jsval::{JSVal, UndefinedValue, DoubleValue, ObjectValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_uint;
use std::ptr::NonNull;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use mozjs::conversions::jsstr_to_string;
use mozjs::rust::wrappers2::{JS_DefineFunction, JS_DefineProperty, JS_NewPlainObject};
use crate::js::helpers::create_empty_array;
use crate::js::helpers::set_string_property;
use crate::js::helpers::ToSafeCx;
use crate::js::bindings::warnings::{warn_stubbed_binding, warn_unexpected_nullish_return};

/// Performance mark entry
#[derive(Debug, Clone)]
struct PerformanceMark {
    name: String,
    start_time: f64,
}

/// Performance measure entry
#[derive(Debug, Clone)]
struct PerformanceMeasure {
    name: String,
    start_time: f64,
    duration: f64,
}

#[derive(Debug, Clone)]
struct NavigationTiming {
    navigation_start: f64,
    domain_lookup_start: f64,
    domain_lookup_end: f64,
    connect_start: f64,
    connect_end: f64,
    request_start: f64,
    response_start: f64,
    response_end: f64,
    dom_loading: f64,
    dom_interactive: f64,
    dom_content_loaded_event_start: f64,
    dom_content_loaded_event_end: f64,
    dom_complete: f64,
    load_event_start: f64,
    load_event_end: f64,
}

/// Performance entry types
#[derive(Debug, Clone)]
enum PerformanceEntry {
    Mark(PerformanceMark),
    Measure(PerformanceMeasure),
}

/// Performance manager that tracks marks and measures
#[derive(Clone)]
pub struct PerformanceManager {
    start_instant: Instant,
    start_time: f64,
    entries: RefCell<HashMap<String, PerformanceEntry>>,
}

impl PerformanceManager {
    pub fn new() -> Self {
        let start_instant = Instant::now();
        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs_f64() * 1000.0;

        Self {
            start_instant,
            start_time,
            entries: RefCell::new(HashMap::new()),
        }
    }

    /// Get the current time in milliseconds since performance timing began
    pub fn now(&self) -> f64 {
        self.start_instant.elapsed().as_secs_f64() * 1000.0
    }

    /// Create a performance mark with the given name
    pub fn mark(&self, name: String) -> f64 {
        let start_time = self.now();
        let mark = PerformanceMark {
            name: name.clone(),
            start_time,
        };
        self.entries.borrow_mut().insert(name, PerformanceEntry::Mark(mark));
        start_time
    }

    /// Create a performance measure between two marks
    pub fn measure(&self, name: String, start_mark: Option<String>, end_mark: Option<String>) -> Result<f64, String> {
        let start_time = if let Some(ref start_name) = start_mark {
            let entries = self.entries.borrow();
            match entries.get(start_name) {
                Some(PerformanceEntry::Mark(mark)) => mark.start_time,
                _ => return Err(format!("Mark '{}' not found", start_name)),
            }
        } else {
            0.0
        };

        let end_time = if let Some(ref end_name) = end_mark {
            let entries = self.entries.borrow();
            match entries.get(end_name) {
                Some(PerformanceEntry::Mark(mark)) => mark.start_time,
                _ => return Err(format!("Mark '{}' not found", end_name)),
            }
        } else {
            self.now()
        };

        let duration = end_time - start_time;
        let measure = PerformanceMeasure {
            name: name.clone(),
            start_time,
            duration,
        };
        self.entries.borrow_mut().insert(name, PerformanceEntry::Measure(measure));
        Ok(duration)
    }

    /// Clear marks by name (or all if name is None)
    pub fn clear_marks(&self, name: Option<String>) {
        if let Some(name) = name {
            let mut entries = self.entries.borrow_mut();
            if let Some(PerformanceEntry::Mark(_)) = entries.get(&name) {
                entries.remove(&name);
            }
        } else {
            // Clear all marks
            self.entries.borrow_mut().retain(|_, entry| {
                !matches!(entry, PerformanceEntry::Mark(_))
            });
        }
    }

    /// Clear measures by name (or all if name is None)
    pub fn clear_measures(&self, name: Option<String>) {
        if let Some(name) = name {
            let mut entries = self.entries.borrow_mut();
            if let Some(PerformanceEntry::Measure(_)) = entries.get(&name) {
                entries.remove(&name);
            }
        } else {
            // Clear all measures
            self.entries.borrow_mut().retain(|_, entry| {
                !matches!(entry, PerformanceEntry::Measure(_))
            });
        }
    }

    /// Get all entries by type
    pub fn get_entries_by_type(&self, entry_type: &str) -> Vec<(String, f64, Option<f64>)> {
        let entries = self.entries.borrow();
        let mut result = Vec::new();

        match entry_type {
            "mark" => {
                for (name, entry) in entries.iter() {
                    if let PerformanceEntry::Mark(mark) = entry {
                        result.push((name.clone(), mark.start_time, None));
                    }
                }
            }
            "measure" => {
                for (name, entry) in entries.iter() {
                    if let PerformanceEntry::Measure(measure) = entry {
                        result.push((name.clone(), measure.start_time, Some(measure.duration)));
                    }
                }
            }
            _ => {}
        }

        result
    }

    /// Get entries by name
    pub fn get_entries_by_name(&self, name: &str) -> Vec<(String, f64, Option<f64>)> {
        let entries = self.entries.borrow();
        let mut result = Vec::new();

        if let Some(entry) = entries.get(name) {
            match entry {
                PerformanceEntry::Mark(mark) => {
                    result.push((mark.name.clone(), mark.start_time, None));
                }
                PerformanceEntry::Measure(measure) => {
                    result.push((measure.name.clone(), measure.start_time, Some(measure.duration)));
                }
            }
        }

        result
    }

    fn navigation_timing(&self) -> NavigationTiming {
        // Use a coherent timing snapshot so every field exists for legacy beacons.
        let navigation_start = self.start_time;
        let now = self.start_time + self.now();

        NavigationTiming {
            navigation_start,
            domain_lookup_start: navigation_start,
            domain_lookup_end: navigation_start,
            connect_start: navigation_start,
            connect_end: navigation_start,
            request_start: navigation_start,
            response_start: now,
            response_end: now,
            dom_loading: now,
            dom_interactive: now,
            dom_content_loaded_event_start: now,
            dom_content_loaded_event_end: now,
            dom_complete: now,
            load_event_start: now,
            load_event_end: now,
        }
    }

    fn navigation_entry_relative_fields(&self) -> (f64, f64) {
        let now = self.now();
        (now, now)
    }
}

// Thread-local storage for the performance manager pointer
thread_local! {
    static PERFORMANCE_MANAGER: RefCell<Option<PerformanceManager>> = RefCell::new(None);
}

/// Set up the performance object in the JavaScript context
pub fn setup_performance(runtime: &mut JsRuntime) -> Result<(), String> {
    // Store performance manager in thread-local storage
    let perf_manager = PerformanceManager::new();
    PERFORMANCE_MANAGER.with(|pm| {
        *pm.borrow_mut() = Some(perf_manager);
    });

    runtime.do_with_jsapi(|cx, global| unsafe {
        let raw_cx = cx.raw_cx();
        // Create performance object
        rooted!(in(raw_cx) let performance = JS_NewPlainObject(cx));
        if performance.get().is_null() {
            return Err("Failed to create performance object".to_string());
        }

        // Define performance.now()
        define_performance_method(cx, performance.handle().get(), "now", Some(performance_now))?;

        // Define performance.mark()
        define_performance_method(cx, performance.handle().get(), "mark", Some(performance_mark))?;

        // Define performance.measure()
        define_performance_method(cx, performance.handle().get(), "measure", Some(performance_measure))?;

        // Define performance.clearMarks()
        define_performance_method(cx, performance.handle().get(), "clearMarks", Some(performance_clear_marks))?;

        // Define performance.clearMeasures()
        define_performance_method(cx, performance.handle().get(), "clearMeasures", Some(performance_clear_measures))?;

        // Define performance.getEntriesByType()
        define_performance_method(cx, performance.handle().get(), "getEntriesByType", Some(performance_get_entries_by_type))?;

        // Define performance.getEntriesByName()
        define_performance_method(cx, performance.handle().get(), "getEntriesByName", Some(performance_get_entries_by_name))?;

        // Define performance.getEntries()
        define_performance_method(cx, performance.handle().get(), "getEntries", Some(performance_get_entries))?;

        // Set performance.timeOrigin as a readonly property
        PERFORMANCE_MANAGER.with(|pm| {
            if let Some(ref manager) = *pm.borrow() {
                rooted!(in(raw_cx) let time_origin = DoubleValue(manager.start_time));
                let name = std::ffi::CString::new("timeOrigin").unwrap();
                if !JS_DefineProperty(
                    cx,
                    performance.handle().into(),
                    name.as_ptr(),
                    time_origin.handle().into(),
                    (JSPROP_ENUMERATE | JSPROP_READONLY) as u32,
                ) {
                    return Err("Failed to define timeOrigin property".to_string());
                }

                // Expose legacy performance.timing for third-party scripts that still read it.
                rooted!(in(raw_cx) let timing = JS_NewPlainObject(cx));
                if timing.get().is_null() {
                    return Err("Failed to create performance.timing object".to_string());
                }
                define_timing_object(cx, timing.get(), &manager.navigation_timing())?;

                rooted!(in(raw_cx) let timing_val = ObjectValue(timing.get()));
                let timing_name = std::ffi::CString::new("timing").unwrap();
                if !JS_DefineProperty(
                    cx,
                    performance.handle().into(),
                    timing_name.as_ptr(),
                    timing_val.handle().into(),
                    JSPROP_ENUMERATE as u32,
                ) {
                    return Err("Failed to define performance.timing property".to_string());
                }

                // Provide an object for code that iterates performance.memory fields.
                rooted!(in(raw_cx) let memory = JS_NewPlainObject(cx));
                if memory.get().is_null() {
                    return Err("Failed to create performance.memory object".to_string());
                }
                rooted!(in(raw_cx) let memory_val = ObjectValue(memory.get()));
                let memory_name = std::ffi::CString::new("memory").unwrap();
                if !JS_DefineProperty(
                    cx,
                    performance.handle().into(),
                    memory_name.as_ptr(),
                    memory_val.handle().into(),
                    JSPROP_ENUMERATE as u32,
                ) {
                    return Err("Failed to define performance.memory property".to_string());
                }
            }
            Ok(())
        })?;

        // Set performance on global object
        rooted!(in(raw_cx) let performance_val = ObjectValue(performance.get()));
        let name = std::ffi::CString::new("performance").unwrap();
        if !JS_DefineProperty(
            cx,
            global.into(),
            name.as_ptr(),
            performance_val.handle().into(),
            JSPROP_ENUMERATE as u32,
        ) {
            return Err("Failed to define performance property".to_string());
        }

        Ok(())
    })
}

unsafe fn define_performance_method(
    cx: &mut SafeJSContext,
    performance: *mut JSObject,
    name: &str,
    func: JSNative,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let performance_rooted = performance);

    if !JS_DefineFunction(
        cx,
        performance_rooted.handle().into(),
        cname.as_ptr(),
        func,
        0,
        JSPROP_ENUMERATE as u32,
    ).is_null() {
        Ok(())
    } else {
        Err(format!("Failed to define performance.{}", name))
    }
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string_perf(raw_cx: *mut JSContext, val: JSVal) -> Option<String> {
    if !val.is_string() {
        return None;
    }

    let jsstr = val.to_string();
    Some(jsstr_to_string(raw_cx, NonNull::new(jsstr).unwrap()))
}

unsafe fn set_empty_array_result(raw_cx: *mut JSContext, args: &CallArgs) {
    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
    } else {
        args.rval().set(ObjectValue(array.get()));
    }
}

unsafe fn define_number_property(
    cx: &mut SafeJSContext,
    obj: *mut JSObject,
    name: &str,
    value: f64,
) -> Result<(), String> {
    let raw_cx = cx.raw_cx();
    rooted!(in(raw_cx) let obj_rooted = obj);
    rooted!(in(raw_cx) let val = DoubleValue(value));
    let cname = std::ffi::CString::new(name).unwrap();
    if !JS_DefineProperty(
        cx,
        obj_rooted.handle().into(),
        cname.as_ptr(),
        val.handle().into(),
        JSPROP_ENUMERATE as u32,
    ) {
        Err(format!("Failed to set property {}", name))
    } else {
        Ok(())
    }
}

unsafe fn define_timing_object(
    cx: &mut SafeJSContext,
    timing_obj: *mut JSObject,
    timing: &NavigationTiming,
) -> Result<(), String> {
    define_number_property(cx, timing_obj, "navigationStart", timing.navigation_start)?;
    define_number_property(cx, timing_obj, "domainLookupStart", timing.domain_lookup_start)?;
    define_number_property(cx, timing_obj, "domainLookupEnd", timing.domain_lookup_end)?;
    define_number_property(cx, timing_obj, "connectStart", timing.connect_start)?;
    define_number_property(cx, timing_obj, "connectEnd", timing.connect_end)?;
    define_number_property(cx, timing_obj, "requestStart", timing.request_start)?;
    define_number_property(cx, timing_obj, "responseStart", timing.response_start)?;
    define_number_property(cx, timing_obj, "responseEnd", timing.response_end)?;
    define_number_property(cx, timing_obj, "domLoading", timing.dom_loading)?;
    define_number_property(cx, timing_obj, "domInteractive", timing.dom_interactive)?;
    define_number_property(
        cx,
        timing_obj,
        "domContentLoadedEventStart",
        timing.dom_content_loaded_event_start,
    )?;
    define_number_property(
        cx,
        timing_obj,
        "domContentLoadedEventEnd",
        timing.dom_content_loaded_event_end,
    )?;
    define_number_property(cx, timing_obj, "domComplete", timing.dom_complete)?;
    define_number_property(cx, timing_obj, "loadEventStart", timing.load_event_start)?;
    define_number_property(cx, timing_obj, "loadEventEnd", timing.load_event_end)?;
    Ok(())
}

unsafe fn define_navigation_entry(
    cx: &mut SafeJSContext,
    nav_obj: *mut JSObject,
    now: f64,
    response_start: f64,
) -> Result<(), String> {
    define_number_property(cx, nav_obj, "startTime", 0.0)?;
    define_number_property(cx, nav_obj, "duration", now)?;
    define_number_property(cx, nav_obj, "activationStart", 0.0)?;
    define_number_property(cx, nav_obj, "domainLookupStart", 0.0)?;
    define_number_property(cx, nav_obj, "domainLookupEnd", 0.0)?;
    define_number_property(cx, nav_obj, "connectStart", 0.0)?;
    define_number_property(cx, nav_obj, "connectEnd", 0.0)?;
    define_number_property(cx, nav_obj, "requestStart", 0.0)?;
    define_number_property(cx, nav_obj, "responseStart", response_start)?;
    define_number_property(cx, nav_obj, "responseEnd", now)?;
    define_number_property(cx, nav_obj, "domInteractive", now)?;
    define_number_property(cx, nav_obj, "domContentLoadedEventStart", now)?;
    define_number_property(cx, nav_obj, "domComplete", now)?;
    define_number_property(cx, nav_obj, "loadEventStart", now)?;
    define_number_property(cx, nav_obj, "loadEventEnd", now)?;
    define_number_property(cx, nav_obj, "transferSize", 0.0)?;
    define_number_property(cx, nav_obj, "decodedBodySize", 0.0)?;
    set_string_property(cx, nav_obj, "entryType", "navigation")?;
    set_string_property(cx, nav_obj, "name", "document")?;
    set_string_property(cx, nav_obj, "type", "navigate")?;
    set_string_property(cx, nav_obj, "deliveryType", "")?;
    set_string_property(cx, nav_obj, "nextHopProtocol", "")?;
    Ok(())
}

unsafe fn add_entry_object_to_array(
    raw_cx: *mut JSContext,
    array_obj: *mut JSObject,
    index: u32,
    name: &str,
    entry_type: &str,
    start_time: f64,
    duration: f64,
) {
    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let entry_obj = JS_NewPlainObject(safe_cx));
    if entry_obj.get().is_null() {
        return;
    }
    if set_string_property(safe_cx, entry_obj.get(), "name", name).is_err() {
        return;
    }
    if set_string_property(safe_cx, entry_obj.get(), "entryType", entry_type).is_err() {
        return;
    }
    if define_number_property(safe_cx, entry_obj.get(), "startTime", start_time).is_err() {
        return;
    }
    if define_number_property(safe_cx, entry_obj.get(), "duration", duration).is_err() {
        return;
    }
    rooted!(in(raw_cx) let entry_val = ObjectValue(entry_obj.get()));
    rooted!(in(raw_cx) let array_rooted = array_obj);
    mozjs::rust::wrappers::JS_SetElement(
        raw_cx,
        array_rooted.handle().into(),
        index,
        entry_val.handle().into(),
    );
}

/// performance.now() implementation
unsafe extern "C" fn performance_now(_raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let result = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.now()
        } else {
            0.0
        }
    });

    let args = CallArgs::from_vp(vp, argc);
    args.rval().set(DoubleValue(result));
    true
}

/// performance.mark() implementation
unsafe extern "C" fn performance_mark(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc < 1 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let name_val = *args.get(0);
    let name = match js_value_to_string_perf(raw_cx, name_val) {
        Some(s) => s,
        None => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.mark(name);
        }
    });

    // FIXME: Should return a PerformanceMark object (with name, startTime, entryType, duration
    // properties) per the Web Performance API spec, not undefined.
    warn_stubbed_binding("performance.mark", "returns undefined instead of PerformanceMark");
    warn_unexpected_nullish_return(
        "performance.mark",
        "undefined",
        "PerformanceMark object",
        "this partial implementation only records the entry internally",
    );
    args.rval().set(UndefinedValue());
    true
}

/// performance.measure() implementation
unsafe extern "C" fn performance_measure(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc < 1 {
        args.rval().set(UndefinedValue());
        return true;
    }

    let name_val = *args.get(0);
    let name = match js_value_to_string_perf(raw_cx, name_val) {
        Some(s) => s,
        None => {
            args.rval().set(UndefinedValue());
            return true;
        }
    };

    let start_mark = if argc > 1 {
        js_value_to_string_perf(raw_cx, *args.get(1))
    } else {
        None
    };

    let end_mark = if argc > 2 {
        js_value_to_string_perf(raw_cx, *args.get(2))
    } else {
        None
    };

    PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            if let Err(e) = manager.measure(name, start_mark, end_mark) {
                eprintln!("Performance measure error: {}", e);
            }
        }
    });

    // FIXME: Should return a PerformanceMeasure object (with name, startTime, duration,
    // entryType properties) per the Web Performance API spec, not undefined.
    warn_stubbed_binding("performance.measure", "returns undefined instead of PerformanceMeasure");
    warn_unexpected_nullish_return(
        "performance.measure",
        "undefined",
        "PerformanceMeasure object",
        "this partial implementation only records the entry internally",
    );
    args.rval().set(UndefinedValue());
    true
}

/// performance.clearMarks() implementation
unsafe extern "C" fn performance_clear_marks(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let name = if argc > 0 {
        js_value_to_string_perf(raw_cx, *args.get(0))
    } else {
        None
    };

    PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.clear_marks(name);
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// performance.clearMeasures() implementation
unsafe extern "C" fn performance_clear_measures(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let name = if argc > 0 {
        js_value_to_string_perf(raw_cx, *args.get(0))
    } else {
        None
    };

    PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.clear_measures(name);
        }
    });

    args.rval().set(UndefinedValue());
    true
}

/// performance.getEntriesByType() implementation
unsafe extern "C" fn performance_get_entries_by_type(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc < 1 {
        set_empty_array_result(raw_cx, &args);
        return true;
    }

    let entry_type = match js_value_to_string_perf(raw_cx, *args.get(0)) {
        Some(s) => s,
        None => {
            set_empty_array_result(raw_cx, &args);
            return true;
        }
    };

    let entries = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.get_entries_by_type(&entry_type)
        } else {
            Vec::new()
        }
    });

    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    let mut index = 0_u32;
    if entry_type == "navigation" {
        PERFORMANCE_MANAGER.with(|pm| {
            if let Some(ref manager) = *pm.borrow() {
                let (now, response_start) = manager.navigation_entry_relative_fields();
                add_entry_object_to_array(
                    raw_cx,
                    array.get(),
                    index,
                    "document",
                    "navigation",
                    0.0,
                    now,
                );

                // Add navigation-specific properties on the created object.
                let raw_cx_inner = raw_cx;
                let safe_cx_inner = &mut raw_cx_inner.to_safe_cx();
                rooted!(in(raw_cx_inner) let nav_obj = JS_NewPlainObject(safe_cx_inner));
                if !nav_obj.get().is_null()
                    && define_navigation_entry(safe_cx_inner, nav_obj.get(), now, response_start).is_ok()
                {
                    rooted!(in(raw_cx_inner) let nav_val = ObjectValue(nav_obj.get()));
                    rooted!(in(raw_cx_inner) let array_rooted = array.get());
                    mozjs::rust::wrappers::JS_SetElement(
                        raw_cx_inner,
                        array_rooted.handle().into(),
                        index,
                        nav_val.handle().into(),
                    );
                }
                index += 1;
            }
        });
    } else {
        for (name, start_time, duration) in entries {
            let entry_duration = duration.unwrap_or(0.0);
            add_entry_object_to_array(
                raw_cx,
                array.get(),
                index,
                &name,
                &entry_type,
                start_time,
                entry_duration,
            );
            index += 1;
        }
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// performance.getEntriesByName() implementation
unsafe extern "C" fn performance_get_entries_by_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc < 1 {
        set_empty_array_result(raw_cx, &args);
        return true;
    }

    let name = match js_value_to_string_perf(raw_cx, *args.get(0)) {
        Some(s) => s,
        None => {
            set_empty_array_result(raw_cx, &args);
            return true;
        }
    };

    let entries = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.get_entries_by_name(&name)
        } else {
            Vec::new()
        }
    });

    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    for (index, (entry_name, start_time, duration)) in entries.into_iter().enumerate() {
        let entry_type = if duration.is_some() { "measure" } else { "mark" };
        add_entry_object_to_array(
            raw_cx,
            array.get(),
            index as u32,
            &entry_name,
            entry_type,
            start_time,
            duration.unwrap_or(0.0),
        );
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// performance.getEntries() implementation
unsafe extern "C" fn performance_get_entries(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    let all_entries = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            let mut marks = manager.get_entries_by_type("mark");
            let mut measures = manager.get_entries_by_type("measure");
            marks.append(&mut measures);
            marks
        } else {
            Vec::new()
        }
    });

    let safe_cx = &mut raw_cx.to_safe_cx();
    rooted!(in(raw_cx) let array = create_empty_array(safe_cx));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    for (index, (name, start_time, duration)) in all_entries.into_iter().enumerate() {
        let entry_type = if duration.is_some() { "measure" } else { "mark" };
        add_entry_object_to_array(
            raw_cx,
            array.get(),
            index as u32,
            &name,
            entry_type,
            start_time,
            duration.unwrap_or(0.0),
        );
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

#[cfg(test)]
mod tests {
    use super::PerformanceManager;

    #[test]
    fn navigation_timing_contains_navigation_start() {
        let manager = PerformanceManager::new();
        let timing = manager.navigation_timing();
        assert!(timing.navigation_start > 0.0);
        assert!(timing.response_start >= timing.navigation_start);
    }

    #[test]
    fn navigation_entry_relative_times_are_non_negative() {
        let manager = PerformanceManager::new();
        let (now, response_start) = manager.navigation_entry_relative_fields();
        assert!(now >= 0.0);
        assert!(response_start >= 0.0);
    }
}

