use std::sync::Arc;
use std::time::{Duration, Instant};

pub trait Time: Send + Sync + std::fmt::Debug {
    fn now(&self) -> Instant;

    fn sleep(&self, time: Duration);

    fn since(&self, earlier: Instant) -> Duration {
        self.now().saturating_duration_since(earlier)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Real;

impl Time for Real {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn sleep(&self, time: Duration) {
        std::thread::sleep(time);
    }
}

#[must_use]
pub fn real() -> Arc<dyn Time> {
    Arc::new(Real)
}

#[cfg(feature = "faults")]
pub use fake::Fake;

#[cfg(feature = "faults")]
mod fake {
    use std::collections::VecDeque;
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::Time;

    #[derive(Debug, Default)]
    struct Held {
        at: Duration,
        script: VecDeque<Duration>,
        reads: u64,
        slept: Duration,
    }

    #[derive(Debug)]
    pub struct Fake {
        base: Instant,
        held: Mutex<Held>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Fake {
        #[must_use]
        pub fn new() -> Self {
            Self {
                base: Instant::now(),
                held: Mutex::new(Held::default()),
            }
        }

        fn held(&self) -> std::sync::MutexGuard<'_, Held> {
            self.held.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub fn advance(&self, ms: u64) {
            let mut held = self.held();
            held.at = held.at.saturating_add(Duration::from_millis(ms));
        }

        pub fn script(&self, steps: impl IntoIterator<Item = Duration>) {
            self.held().script = steps.into_iter().collect();
        }

        #[must_use]
        pub fn scripted(&self) -> usize {
            self.held().script.len()
        }

        #[must_use]
        pub fn elapsed(&self) -> Duration {
            self.held().at
        }

        #[must_use]
        pub fn reads(&self) -> u64 {
            self.held().reads
        }

        #[must_use]
        pub fn slept(&self) -> Duration {
            self.held().slept
        }
    }

    impl Time for Fake {
        fn now(&self) -> Instant {
            let mut held = self.held();
            held.reads = held.reads.saturating_add(1);
            if let Some(step) = held.script.pop_front() {
                held.at = held.at.saturating_add(step);
            }
            self.base.checked_add(held.at).unwrap_or(self.base)
        }

        fn sleep(&self, time: Duration) {
            let mut held = self.held();
            held.at = held.at.saturating_add(time);
            held.slept = held.slept.saturating_add(time);
        }
    }
}
