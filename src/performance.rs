//! Opt-in timings for reproducing editor latency with synthetic notes.
use std::{sync::OnceLock, time::Instant};
pub(crate) struct Sample {
    name: &'static str,
    started: Instant,
}
impl Sample {
    pub(crate) fn start(name: &'static str) -> Option<Self> {
        static ENABLED: OnceLock<bool> = OnceLock::new();
        ENABLED
            .get_or_init(|| std::env::var_os("POLAR_PROFILE").is_some())
            .then(|| Self {
                name,
                started: Instant::now(),
            })
    }
}
impl Drop for Sample {
    fn drop(&mut self) {
        eprintln!(
            "POLAR_PROFILE {} {:.3}",
            self.name,
            self.started.elapsed().as_secs_f64() * 1000.
        );
    }
}
