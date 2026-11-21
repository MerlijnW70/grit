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
pub use fake::{Aside, Fake, Serving, aside, point, serving};

#[cfg(not(feature = "faults"))]
pub(crate) fn point(_: &dyn Time, _: &'static str) {}

#[cfg(feature = "faults")]
mod fake {
    use std::cell::Cell;
    use std::collections::VecDeque;
    use std::marker::PhantomData;
    use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
    use std::time::{Duration, Instant};

    use super::Time;

    thread_local! {
        static SERVING: Cell<u32> = const { Cell::new(0) };
        static ASIDE: Cell<u32> = const { Cell::new(0) };
    }

    type Armed = Vec<(usize, Weak<Mutex<Held>>)>;

    static ARMED: Mutex<Armed> = Mutex::new(Vec::new());

    fn armed() -> MutexGuard<'static, Armed> {
        ARMED.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[derive(Debug)]
    pub struct Serving {
        held: PhantomData<*const ()>,
    }

    #[must_use]
    pub fn serving() -> Serving {
        SERVING.set(SERVING.get().saturating_add(1));
        Serving { held: PhantomData }
    }

    impl Drop for Serving {
        fn drop(&mut self) {
            SERVING.set(SERVING.get().saturating_sub(1));
        }
    }

    fn served() -> bool {
        SERVING.get() > 0
    }

    #[derive(Debug)]
    pub struct Aside {
        held: PhantomData<*const ()>,
    }

    #[must_use]
    pub fn aside() -> Aside {
        ASIDE.set(ASIDE.get().saturating_add(1));
        Aside { held: PhantomData }
    }

    impl Drop for Aside {
        fn drop(&mut self) {
            ASIDE.set(ASIDE.get().saturating_sub(1));
        }
    }

    fn set_aside() -> bool {
        ASIDE.get() > 0
    }

    fn key(time: &dyn Time) -> usize {
        std::ptr::from_ref(time).cast::<()>().addr()
    }

    pub fn point(time: &dyn Time, name: &'static str) {
        if !served() {
            return;
        }
        let found = armed()
            .iter()
            .filter(|(at, _)| *at == key(time))
            .find_map(|(_, held)| held.upgrade());
        let Some(held) = found else {
            return;
        };
        let mut held = held.lock().unwrap_or_else(PoisonError::into_inner);
        held.passed.push(name);
        let step = held
            .points
            .iter_mut()
            .find(|(named, _)| *named == name)
            .and_then(|(_, steps)| steps.pop_front());
        if let Some(step) = step {
            held.at = held.at.saturating_add(step);
            held.steps = held.steps.saturating_add(1);
        }
    }

    #[derive(Debug, Default)]
    struct Held {
        at: Duration,
        script: VecDeque<Duration>,
        points: Vec<(&'static str, VecDeque<Duration>)>,
        passed: Vec<&'static str>,
        reads: u64,
        steps: u64,
        slept: Duration,
    }

    #[derive(Debug)]
    pub struct Fake {
        base: Instant,
        held: Arc<Mutex<Held>>,
    }

    impl Default for Fake {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for Fake {
        fn drop(&mut self) {
            let own = Arc::downgrade(&self.held);
            armed().retain(|(_, held)| held.strong_count() > 0 && !held.ptr_eq(&own));
        }
    }

    impl Fake {
        #[must_use]
        pub fn new() -> Self {
            Self {
                base: Instant::now(),
                held: Arc::default(),
            }
        }

        fn held(&self) -> MutexGuard<'_, Held> {
            self.held.lock().unwrap_or_else(PoisonError::into_inner)
        }

        pub fn advance(&self, ms: u64) {
            let mut held = self.held();
            held.at = held.at.saturating_add(Duration::from_millis(ms));
        }

        pub fn script(&self, steps: impl IntoIterator<Item = Duration>) {
            self.held().script = steps.into_iter().collect();
        }

        pub fn at(&self, point: &'static str, steps: impl IntoIterator<Item = Duration>) {
            let steps: VecDeque<Duration> = steps.into_iter().collect();
            {
                let mut held = self.held();
                held.points.retain(|(named, _)| *named != point);
                held.points.push((point, steps));
            }
            let own = Arc::downgrade(&self.held);
            let place = key(self);
            let mut armed = armed();
            armed.retain(|(_, held)| held.strong_count() > 0 && !held.ptr_eq(&own));
            armed.push((place, own));
        }

        #[must_use]
        pub fn scripted(&self) -> usize {
            self.held().script.len()
        }

        #[must_use]
        pub fn waiting(&self, point: &'static str) -> usize {
            self.held()
                .points
                .iter()
                .find(|(named, _)| *named == point)
                .map_or(0, |(_, steps)| steps.len())
        }

        #[must_use]
        pub fn passed(&self) -> Vec<&'static str> {
            self.held().passed.clone()
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
        pub fn steps(&self) -> u64 {
            self.held().steps
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
            if served()
                && let Some(step) = held.script.pop_front()
            {
                held.at = held.at.saturating_add(step);
                held.steps = held.steps.saturating_add(1);
            }
            self.base.checked_add(held.at).unwrap_or(self.base)
        }

        fn sleep(&self, time: Duration) {
            let mut held = self.held();
            if !set_aside() {
                held.at = held.at.saturating_add(time);
            }
            held.slept = held.slept.saturating_add(time);
        }
    }
}
