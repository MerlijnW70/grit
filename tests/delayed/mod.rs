use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use grit::clock::{Fake, Time};

#[derive(Debug)]
pub struct Waiting {
    fake: Fake,
    each: Duration,
    most: Duration,
    waited: Mutex<(u64, Duration)>,
}

impl Waiting {
    pub fn new(each: Duration, most: Duration) -> Self {
        Self {
            fake: Fake::new(),
            each,
            most,
            waited: Mutex::new((0, Duration::ZERO)),
        }
    }

    pub const fn fake(&self) -> &Fake {
        &self.fake
    }

    pub fn waited(&self) -> (u64, Duration) {
        *self.waited.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait(&self, time: Duration) {
        let began = Instant::now();
        std::thread::sleep(time);
        let mut waited = self.waited.lock().unwrap_or_else(PoisonError::into_inner);
        waited.0 += 1;
        waited.1 += began.elapsed();
    }
}

impl Time for Waiting {
    fn now(&self) -> Instant {
        self.wait(self.each);
        self.fake.now()
    }

    fn sleep(&self, time: Duration) {
        self.wait(time.min(self.most).max(self.each));
        self.fake.sleep(time);
    }
}
