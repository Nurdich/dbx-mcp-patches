//! Optional connect-time progress messages (proxy failover, tunnel setup).
//!
//! MCP/CLI can install a hook to mirror these into their `[dbx]` stderr/tool streams.
//! Without a hook, messages go to `log::info`.

use std::cell::RefCell;
use std::sync::Arc;

thread_local! {
    static HOOK: RefCell<Option<Arc<dyn Fn(&str) + Send + Sync>>> = RefCell::new(None);
}

/// Install a process-wide (thread-local) progress sink for the current async task/thread.
pub fn set_hook(hook: Option<Arc<dyn Fn(&str) + Send + Sync>>) {
    HOOK.with(|slot| *slot.borrow_mut() = hook);
}

/// Emit a connect-progress line (without the `[dbx]` prefix; callers may add it).
pub fn emit(message: impl AsRef<str>) {
    let message = message.as_ref();
    let mut handled = false;
    HOOK.with(|slot| {
        if let Some(hook) = slot.borrow().as_ref() {
            hook(message);
            handled = true;
        }
    });
    if !handled {
        log::info!("[dbx] {message}");
    }
}
