// Timer implementation for setTimeout and setInterval
use boa_engine::{object::JsObject, Context, JsString, JsValue, NativeFunction};
use boa_gc::Finalize;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// A pending timer that will execute a callback after a delay
#[derive(Debug, Clone)]
struct Timer {
    id: u32,
    start_time: Instant,
    duration: Duration,
    callback: JsObject,
    repeating: bool,
}

impl Finalize for Timer {}

/// Timer manager that tracks all active timers
#[derive(Clone)]
pub struct TimerManager {
    #[allow(clippy::type_complexity)]
    timers: Rc<RefCell<HashMap<u32, Timer>>>,
    next_id: Rc<RefCell<u32>>,
}

impl Finalize for TimerManager {}

impl TimerManager {
    pub fn new() -> Self {
        Self {
            timers: Rc::new(RefCell::new(HashMap::new())),
            next_id: Rc::new(RefCell::new(1)),
        }
    }

    /// Register a new timeout
    pub fn set_timeout(&self, callback: JsObject, delay: u32) -> u32 {
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
            callback,
            repeating: false,
        };

        self.timers.borrow_mut().insert(id, timer);
        id
    }

    /// Register a new interval
    pub fn set_interval(&self, callback: JsObject, delay: u32) -> u32 {
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
            callback,
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
    pub fn process_timers(&self, context: &mut Context) -> bool {
        let now = Instant::now();
        let mut ready_timers = Vec::new();
        let mut timers_to_reschedule = Vec::new();

        // Find all timers that are ready to execute
        {
            let timers = self.timers.borrow();
            for (id, timer) in timers.iter() {
                if now.duration_since(timer.start_time) >= timer.duration {
                    ready_timers.push((*id, timer.callback.clone(), timer.repeating));
                }
            }
        }

        let had_timers = !ready_timers.is_empty();

        // Execute callbacks for ready timers
        for (id, callback, repeating) in ready_timers {
            // Execute the callback
            if let Err(e) = callback.call(&JsValue::undefined(), &[], context) {
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

/// Set up timer functions in the JavaScript context
pub fn setup_timers(context: &mut Context, timer_manager: Rc<TimerManager>) -> Result<(), String> {
    // setTimeout function
    let timer_manager_1 = timer_manager.clone();
    context.register_global_callable(
        JsString::from("setTimeout"),
        1,
        unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                // Get callback function
                let callback = args.get(0)
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        boa_engine::JsNativeError::typ()
                            .with_message("setTimeout requires a function as first argument")
                    })?;

                // Get delay (default to 0 if not provided)
                let delay = args.get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .max(0.0) as u32;

                let id = timer_manager_1.set_timeout(callback.clone(), delay);
                Ok(JsValue::from(id))
            })
        }
    ).map_err(|e| format!("Failed to register setTimeout: {}", e))?;

    // clearTimeout function
    let timer_manager_2 = timer_manager.clone();
    context.register_global_callable(
        JsString::from("clearTimeout"),
        1,
        unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                if let Some(id) = args.get(0).and_then(|v| v.as_number()) {
                    timer_manager_2.clear_timer(id as u32);
                }
                Ok(JsValue::undefined())
            })
        }
    ).map_err(|e| format!("Failed to register clearTimeout: {}", e))?;

    // setInterval function
    let timer_manager_3 = timer_manager.clone();
    context.register_global_callable(
        JsString::from("setInterval"),
        1,
        unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                // Get callback function
                let callback = args.get(0)
                    .and_then(|v| v.as_object())
                    .ok_or_else(|| {
                        boa_engine::JsNativeError::typ()
                            .with_message("setInterval requires a function as first argument")
                    })?;

                // Get delay (default to 0 if not provided)
                let delay = args.get(1)
                    .and_then(|v| v.as_number())
                    .unwrap_or(0.0)
                    .max(0.0) as u32;

                let id = timer_manager_3.set_interval(callback.clone(), delay);
                Ok(JsValue::from(id))
            })
        }
    ).map_err(|e| format!("Failed to register setInterval: {}", e))?;

    // clearInterval function
    let timer_manager_4 = timer_manager.clone();
    context.register_global_callable(
        JsString::from("clearInterval"),
        1,
        unsafe {
            NativeFunction::from_closure(move |_this: &JsValue, args: &[JsValue], _context: &mut Context| {
                if let Some(id) = args.get(0).and_then(|v| v.as_number()) {
                    timer_manager_4.clear_timer(id as u32);
                }
                Ok(JsValue::undefined())
            })
        }
    ).map_err(|e| format!("Failed to register clearInterval: {}", e))?;

    Ok(())
}
