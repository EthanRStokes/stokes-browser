// Alert callback system for JavaScript alert() function
use std::cell::RefCell;
use std::rc::Rc;

/// Callback function type for alert
pub type AlertCallback = Box<dyn Fn(String)>;

/// Global alert callback storage
thread_local! {
    static ALERT_CALLBACK: RefCell<Option<Rc<AlertCallback>>> = RefCell::new(None);
}

/// Set the alert callback function
pub fn set_alert_callback<F>(callback: F)
where
    F: Fn(String) + 'static,
{
    ALERT_CALLBACK.set(Some(Rc::new(Box::new(callback))));
}

/// Trigger the alert callback with a message
pub fn trigger_alert(message: String) {
    ALERT_CALLBACK.with(|cb| {
        if let Some(callback) = cb.borrow().as_ref() {
            callback(message);
        } else {
            // Fallback to console if no callback is set
            println!("[JS Alert] {}", message);
        }
    });
}

/// Clear the alert callback
pub fn clear_alert_callback() {
    ALERT_CALLBACK.set(None);
}

