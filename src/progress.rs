//! Ephemeral progress for the current operation. No payloads are persisted.
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Snapshot {
    pub stage: String,
    pub detail: String,
    pub percent: Option<u32>,
    pub finished: Option<bool>,
    pub cancelled: bool,
    pub started: Instant,
}

#[derive(Clone)]
pub struct Reporter(Arc<Mutex<Snapshot>>);
thread_local! { static CURRENT: RefCell<Option<Reporter>> = const { RefCell::new(None) }; }

pub fn current() -> Option<Reporter> {
    CURRENT.with(|value| value.borrow().clone())
}
pub fn cancelled() -> bool {
    current().is_some_and(|r| r.0.lock().is_ok_and(|s| s.cancelled))
}
/// Runs `work` on this thread as part of `reporter`'s operation, so a worker
/// thread reports progress and sees cancellation like the thread that began it.
pub fn within<T>(reporter: Option<&Reporter>, work: impl FnOnce() -> T) -> T {
    let previous = CURRENT.with(|value| value.replace(reporter.cloned()));
    let result = work();
    CURRENT.with(|value| value.replace(previous));
    result
}
pub fn stage(stage: impl Into<String>) {
    report(stage, String::new(), None);
}
pub fn report(stage: impl Into<String>, detail: impl Into<String>, percent: Option<u32>) {
    if let Some(reporter) = current() {
        reporter.report(stage, detail, percent);
    }
}

impl Reporter {
    pub fn report(
        &self,
        stage: impl Into<String>,
        detail: impl Into<String>,
        percent: Option<u32>,
    ) {
        if let Ok(mut state) = self.0.lock() {
            if state.finished.is_some() || state.cancelled {
                return;
            }
            state.stage = stage.into();
            state.detail = detail.into();
            state.percent = percent.map(|p| p.min(100));
        }
    }
    #[cfg(windows)]
    pub fn snapshot(&self) -> Snapshot {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    #[cfg(windows)]
    pub fn cancel(&self) {
        if let Ok(mut state) = self.0.lock()
            && state.finished.is_none()
        {
            state.cancelled = true;
            state.stage = "Cancelling…".into();
            state.detail = "Completed files will be kept.".into();
            state.percent = None;
        }
    }
}

#[cfg(windows)]
pub struct Operation {
    pub reporter: Reporter,
    previous: Option<Reporter>,
}
#[cfg(windows)]
impl Operation {
    pub fn begin() -> Self {
        let reporter = Reporter(Arc::new(Mutex::new(Snapshot {
            stage: "Preparing your paste…".into(),
            detail: String::new(),
            percent: None,
            finished: None,
            cancelled: false,
            started: Instant::now(),
        })));
        let previous = CURRENT.with(|value| value.replace(Some(reporter.clone())));
        Self { reporter, previous }
    }
    pub fn finish(&self, result: &Result<String, String>) {
        let cancelled = crate::common::interrupted();
        let mut state = self.reporter.0.lock().unwrap_or_else(|e| e.into_inner());
        state.stage = if result.is_ok() {
            "Paste complete"
        } else if cancelled {
            "Paste cancelled"
        } else {
            "Couldn't finish this paste"
        }
        .into();
        state.detail = match result {
            Ok(text) | Err(text) => text.clone(),
        };
        state.percent = result.is_ok().then_some(100);
        state.finished = Some(result.is_ok());
    }
}
#[cfg(windows)]
impl Drop for Operation {
    fn drop(&mut self) {
        CURRENT.with(|value| {
            value.replace(self.previous.take());
        });
        let mut state = self.reporter.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.finished.is_none() {
            state.finished = Some(false);
            state.stage = "Paste stopped".into();
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_late_progress_never_cross_operations() {
        let first = Operation::begin();
        let old = first.reporter.clone();
        old.cancel();
        assert!(cancelled());
        first.finish(&Err("cancelled".into()));
        drop(first);
        assert!(!cancelled());
        let second = Operation::begin();
        old.cancel();
        old.report("late output", "", Some(55));
        assert!(!cancelled());
        assert_eq!(old.snapshot().stage, "Paste cancelled");
        assert_eq!(second.reporter.snapshot().percent, None);
    }
}
