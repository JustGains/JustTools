//! Line-based status messages that leave stdout available for command results.
use std::io::{self, IsTerminal};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::error::ToolResult;

/// Announce a phase and its outcome. Only terminals receive timed reminders,
/// so redirected diagnostics do not depend on how long an operation takes.
pub fn run<T>(tool: &str, phase: &str, work: impl FnOnce() -> ToolResult<T>) -> ToolResult<T> {
    eprintln!("{tool}: {phase} ...");
    let started = Instant::now();
    let interactive = io::stderr().is_terminal();
    let result = std::thread::scope(|scope| {
        let (stop, ticks) = mpsc::channel::<()>();
        if interactive {
            scope.spawn(move || {
                while matches!(
                    ticks.recv_timeout(Duration::from_secs(5)),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    eprintln!(
                        "{tool}: {phase} ({}s elapsed; still working) ...",
                        started.elapsed().as_secs()
                    );
                }
            });
        }
        let result = work();
        let _ = stop.send(());
        result
    });
    let outcome = if result.is_ok() { "done" } else { "failed" };
    if interactive {
        eprintln!(
            "{tool}: {phase} {outcome} ({:.2}s)",
            started.elapsed().as_secs_f64()
        );
    } else {
        eprintln!("{tool}: {phase} {outcome}");
    }
    result
}
