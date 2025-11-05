use super::{initialize_bindings, JsResult, TimerManager};
use crate::dom::{Dom, DomNode};
// JavaScript runtime management
use boa_engine::{Context, JsValue, Source};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

// Stack size for growing when needed (16MB to handle very large scripts)
const STACK_SIZE: usize = 16 * 1024 * 1024;
// Red zone threshold (32KB)
const RED_ZONE: usize = 32 * 1024;

/// JavaScript runtime that manages execution context
pub struct JsRuntime {
    context: Context,
    //document_root: Rc<RefCell<Dom>>,
    timer_manager: Rc<TimerManager>,
    user_agent: String,
}

impl JsRuntime {
    /// Create a new JavaScript runtime
    pub fn new(/*TODO mut document_root: *mut Dom, */user_agent: String) -> JsResult<Self> {
        let mut context = Context::default();
        //let document_root = unsafe { *document_root };
        //let document_root = Rc::new(RefCell::new(document_root));

        // Initialize browser bindings
        // TODO reimplement javascript
        //initialize_bindings(&mut context, document_root.clone(), user_agent.clone())?;

        // Create and set up timer manager
        let timer_manager = Rc::new(TimerManager::new());
        super::timers::setup_timers(&mut context, timer_manager.clone())?;

        Ok(Self {
            context,
        //    document_root,
            timer_manager,
            user_agent,
        })
    }

    /// Execute JavaScript code
    pub fn execute(&mut self, code: &str) -> JsResult<JsValue> {
        // Use stacker::grow to ensure we have enough stack for parsing large scripts
        // This forces stack growth rather than checking, which is safer for boa's deep recursion
        stacker::grow(STACK_SIZE, || {
            let result = self.context
                .eval(Source::from_bytes(code))
                .map_err(|e| format!("JavaScript error: {}", e));

            // Process the job queue to execute Promise callbacks
            // This is crucial for .then() and .catch() to work
            if result.is_ok() {
                self.run_pending_jobs();
            }

            result
        })
    }

    /// Execute JavaScript code from a script tag
    pub fn execute_script(&mut self, code: &str) -> JsResult<()> {
        match self.execute(code) {
            Ok(result) => {
                // Process any remaining jobs after script execution
                self.run_pending_jobs();
                Ok({
                    println!("{}", result.display())
                })
            },
            Err(e) => {
                eprintln!("Script execution error: {}", e);
                Err(e)
            }
        }
    }

    /// Run all pending jobs in the job queue (for Promises)
    fn run_pending_jobs(&mut self) {
        // Run all pending jobs in the queue
        // This is necessary for Promise .then() and .catch() handlers to execute
        // Run jobs multiple times to handle chained promises
        for _ in 0..100 {
            match self.context.run_jobs() {
                Ok(()) => {
                    // Successfully ran jobs
                }
                Err(e) => {
                    eprintln!("Error running job queue: {}", e);
                    break;
                }
            }
        }
    }

    /// Process pending timers and execute callbacks that are ready
    /// Returns true if any timers were executed
    pub fn process_timers(&mut self) -> bool {
        self.timer_manager.process_timers(&mut self.context)
    }

    /// Check if there are any active timers
    pub fn has_active_timers(&self) -> bool {
        self.timer_manager.has_active_timers()
    }

    /// Get the time until the next timer should fire
    pub fn time_until_next_timer(&self) -> Option<Duration> {
        self.timer_manager.time_until_next_timer()
    }

    /// Get a reference to the context
    pub fn context_mut(&mut self) -> &mut Context {
        &mut self.context
    }
}
