use crate::js::JsRuntime;
// Timer implementation for setTimeout and setInterval using mozjs
use mozjs::context::RawJSContext;
use mozjs::conversions::jsstr_to_string;
use mozjs::jsval::{Int32Value, JSVal, UndefinedValue};
use mozjs::rooted;
use std::cell::RefCell;
use std::collections::HashMap;
use std::ptr::NonNull;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A pending timer that will execute a callback after a delay
#[derive(Debug)]
struct Timer {
    id: u32,
    start_time: Instant,
    duration: Duration,
    callback_code: String, // Store the code/function as string for now
    repeating: bool,
}

/// Timer manager that tracks all active timers
#[derive(Clone)]
pub struct TimerManager {
    timers: Rc<RefCell<HashMap<u32, Timer>>>,
    next_id: Rc<RefCell<u32>>,
}

impl TimerManager {
    pub fn new() -> Self {
        Self {
            timers: Rc::new(RefCell::new(HashMap::new())),
            next_id: Rc::new(RefCell::new(1)),
        }
    }

    /// Register a new timeout
    pub fn set_timeout(&self, callback_code: String, delay: u32) -> u32 {
        let id = {
            let mut next_id = self.next_id.borrow_mut();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let timer = Timer {
            id,
            start_time: Instant::now(),
            duration: Duration::from_millis(delay as u64),
            callback_code,
            repeating: false,
        };

        self.timers.borrow_mut().insert(id, timer);
        id
    }

    /// Register a new interval
    pub fn set_interval(&self, callback_code: String, delay: u32) -> u32 {
        let id = {
            let mut next_id = self.next_id.borrow_mut();
            let id = *next_id;
            *next_id += 1;
            id
        };

        let timer = Timer {
            id,
            start_time: Instant::now(),
            duration: Duration::from_millis(delay as u64),
            callback_code,
            repeating: true,
        };

        self.timers.borrow_mut().insert(id, timer);
        id
    }

    /// Clear a timeout or interval
    pub fn clear_timer(&self, id: u32) {
        self.timers.borrow_mut().remove(&id);
    }

    /// Process all timers and execute callbacks that are ready
    /// Returns true if any timers were executed
    pub fn process_timers(&self, runtime: &mut JsRuntime) -> bool {
        let now = Instant::now();
        let mut ready_timers: Vec<(u32, String, bool)> = Vec::new();
        let mut timers_to_reschedule = Vec::new();

        // Find all timers that are ready to execute
        {
            let timers = self.timers.borrow();
            for (id, timer) in timers.iter() {
                if now.duration_since(timer.start_time) >= timer.duration {
                    ready_timers.push((*id, timer.callback_code.clone(), timer.repeating));
                }
            }
        }

        let had_timers = !ready_timers.is_empty();

        // Execute callbacks for ready timers
        for (id, callback_code, repeating) in ready_timers {
            // Execute the callback code
            if let Err(e) = runtime.execute(&callback_code) {
                eprintln!("Timer callback error: {}", e);
            }

            // Remove the timer if it's not repeating
            if !repeating {
                self.timers.borrow_mut().remove(&id);
            } else {
                // Reschedule repeating timers
                timers_to_reschedule.push(id);
            }
        }

        // Reschedule repeating timers
        for id in timers_to_reschedule {
            if let Some(timer) = self.timers.borrow_mut().get_mut(&id) {
                timer.start_time = Instant::now();
            }
        }

        had_timers
    }

    /// Check if there are any active timers
    pub fn has_active_timers(&self) -> bool {
        !self.timers.borrow().is_empty()
    }

    /// Get the time until the next timer should fire
    pub fn time_until_next_timer(&self) -> Option<Duration> {
        let now = Instant::now();
        let timers = self.timers.borrow();

        timers.values()
            .map(|timer| {
                let elapsed = now.duration_since(timer.start_time);
                if elapsed >= timer.duration {
                    Duration::from_millis(0)
                } else {
                    timer.duration - elapsed
                }
            })
            .min()
    }
}

// Thread-local storage for the timer manager pointer
thread_local! {
    static TIMER_MANAGER: RefCell<Option<Rc<TimerManager>>> = RefCell::new(None);
}

/// Set up timer functions in the JavaScript context
pub fn setup_timers(runtime: &mut JsRuntime, timer_manager: Rc<TimerManager>) -> Result<(), String> {
    // Store timer manager in thread-local storage
    TIMER_MANAGER.with(|tm| {
        *tm.borrow_mut() = Some(timer_manager);
    });

    // setTimeout
    runtime.add_global_function("setTimeout", |cx, args| {
        unsafe {
            let argc = args.argc_;
            // Get callback (first argument)
            let callback_code = if argc > 0 {
                let callback_val = *args.get(0);
                if callback_val.is_string() {
                    js_value_to_string(cx, callback_val)
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            };

            // Get delay (second argument)
            let delay = if argc > 1 {
                let delay_val = *args.get(1);
                if delay_val.is_int32() {
                    delay_val.to_int32().max(0) as u32
                } else if delay_val.is_double() {
                    delay_val.to_double().max(0.0) as u32
                } else {
                    0
                }
            } else {
                0
            };

            let id = TIMER_MANAGER.with(|tm| {
                if let Some(ref manager) = *tm.borrow() {
                    manager.set_timeout(callback_code, delay)
                } else {
                    0
                }
            });

            args.rval().set(Int32Value(id as i32));
            true
        }
    });

    // clearTimeout
    runtime.add_global_function("clearTimeout", |_cx, args| {
        let argc = args.argc_;
        if argc > 0 {
            let id_val = *args.get(0);
            let id = if id_val.is_int32() {
                id_val.to_int32() as u32
            } else if id_val.is_double() {
                id_val.to_double() as u32
            } else {
                0
            };

            TIMER_MANAGER.with(|tm| {
                if let Some(ref manager) = *tm.borrow() {
                    manager.clear_timer(id);
                }
            });
        }

        args.rval().set(UndefinedValue());
        true
    });

    // setInterval
    runtime.add_global_function("setInterval", |cx, args| {
        unsafe {
            let argc = args.argc_;
            // Get callback (first argument)
            let callback_code = if argc > 0 {
                let callback_val = *args.get(0);
                if callback_val.is_string() {
                    js_value_to_string(cx, callback_val)
                } else {
                    "".to_string()
                }
            } else {
                "".to_string()
            };

            // Get delay (second argument)
            let delay = if argc > 1 {
                let delay_val = *args.get(1);
                if delay_val.is_int32() {
                    delay_val.to_int32().max(0) as u32
                } else if delay_val.is_double() {
                    delay_val.to_double().max(0.0) as u32
                } else {
                    0
                }
            } else {
                0
            };

            let id = TIMER_MANAGER.with(|tm| {
                if let Some(ref manager) = *tm.borrow() {
                    manager.set_interval(callback_code, delay)
                } else {
                    0
                }
            });

            args.rval().set(Int32Value(id as i32));
            true
        }
    });

    // clearInterval
    runtime.add_global_function("clearInterval", |_cx, args| {
        let argc = args.argc_;
        if argc > 0 {
            let id_val = *args.get(0);
            let id = if id_val.is_int32() {
                id_val.to_int32() as u32
            } else if id_val.is_double() {
                id_val.to_double() as u32
            } else {
                0
            };

            TIMER_MANAGER.with(|tm| {
                if let Some(ref manager) = *tm.borrow() {
                    manager.clear_timer(id);
                }
            });
        }

        args.rval().set(UndefinedValue());
        true
    });

    Ok(())
}

/// Convert a JS value to a Rust string
unsafe fn js_value_to_string(cx: *mut RawJSContext, val: JSVal) -> String {
    if val.is_string() {
        rooted!(in(cx) let str_val = val.to_string());
        if str_val.get().is_null() {
            return String::new();
        }

        jsstr_to_string(cx, NonNull::new(str_val.get()).unwrap())
    } else if val.is_object() && !val.is_null() {
        // For function objects, we'll just return a placeholder
        // In a real implementation, we'd store the function object
        "[function]".to_string()
    } else {
        String::new()
    }
}
