// Performance API implementation for JavaScript using mozjs
use crate::js::JsRuntime;
use mozjs::jsapi::{CallArgs, JSContext, JSNative, JSObject, JS_DefineFunction, JS_DefineProperty, JS_NewPlainObject, JSPROP_ENUMERATE, JSPROP_READONLY, HandleValueArray};
use mozjs::jsval::{JSVal, UndefinedValue, DoubleValue, ObjectValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::HashMap;
use std::os::raw::c_uint;
use std::ptr::NonNull;
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use mozjs::conversions::jsstr_to_string;

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

    runtime.do_with_jsapi(|_rt, cx, global| unsafe {
        // Create performance object
        rooted!(in(cx) let performance = JS_NewPlainObject(cx));
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
                rooted!(in(cx) let time_origin = DoubleValue(manager.start_time));
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
            }
            Ok(())
        })?;

        // Set performance on global object
        rooted!(in(cx) let performance_val = ObjectValue(performance.get()));
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
    raw_cx: *mut JSContext,
    performance: *mut JSObject,
    name: &str,
    func: JSNative,
) -> Result<(), String> {
    let cname = std::ffi::CString::new(name).unwrap();
    rooted!(in(raw_cx) let performance_rooted = performance);

    if !JS_DefineFunction(
        raw_cx,
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
        // Return empty array
        rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
        args.rval().set(ObjectValue(array.get()));
        return true;
    }

    let entry_type = match js_value_to_string_perf(raw_cx, *args.get(0)) {
        Some(s) => s,
        None => {
            rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
            args.rval().set(ObjectValue(array.get()));
            return true;
        }
    };

    let _entries = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.get_entries_by_type(&entry_type)
        } else {
            Vec::new()
        }
    });

    // Create an array to return
    rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    // Add entries to array (simplified - just returns empty array for now)
    args.rval().set(ObjectValue(array.get()));
    true
}

/// performance.getEntriesByName() implementation
unsafe extern "C" fn performance_get_entries_by_name(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    if argc < 1 {
        rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
        args.rval().set(ObjectValue(array.get()));
        return true;
    }

    let name = match js_value_to_string_perf(raw_cx, *args.get(0)) {
        Some(s) => s,
        None => {
            rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
            args.rval().set(ObjectValue(array.get()));
            return true;
        }
    };

    let _entries = PERFORMANCE_MANAGER.with(|pm| {
        if let Some(ref manager) = *pm.borrow() {
            manager.get_entries_by_name(&name)
        } else {
            Vec::new()
        }
    });

    // Create an array to return
    rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

/// performance.getEntries() implementation
unsafe extern "C" fn performance_get_entries(raw_cx: *mut JSContext, argc: c_uint, vp: *mut JSVal) -> bool {
    let args = CallArgs::from_vp(vp, argc);

    // Create an array to return (returns empty array for now)
    rooted!(in(raw_cx) let array = mozjs::jsapi::NewArrayObject(raw_cx, &HandleValueArray::empty()));
    if array.get().is_null() {
        args.rval().set(UndefinedValue());
        return true;
    }

    args.rval().set(ObjectValue(array.get()));
    true
}

