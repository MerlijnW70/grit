use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Driver {
    serial: u32,
    pool: Vec<u32>,
    lanes: u32,
}

#[derive(Debug, Clone, Default)]
pub struct Seen {
    threads: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    own: u64,
    serial: u64,
    pool: u64,
}

const RASTER: &str = "llvmpipe-";

#[cfg(target_os = "linux")]
fn listed() -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir("/proc/self/task") else {
        return Vec::new();
    };
    let mut threads: Vec<(u32, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let tid = entry.file_name().to_str()?.parse::<u32>().ok()?;
            let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            Some((tid, name.trim_end().to_owned()))
        })
        .collect();
    threads.sort_unstable();
    threads
}

#[cfg(not(target_os = "linux"))]
const fn listed() -> Vec<(u32, String)> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn ran(path: &str) -> Option<u64> {
    let held = std::fs::read_to_string(path).ok()?;
    held.split_whitespace().next()?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
const fn ran(_: &str) -> Option<u64> {
    None
}

#[must_use]
pub fn seen() -> Seen {
    Seen { threads: listed() }
}

#[must_use]
pub fn own_name() -> Option<String> {
    if cfg!(target_os = "linux") {
        std::fs::read_to_string("/proc/thread-self/comm")
            .ok()
            .map(|name| name.trim_end().to_owned())
    } else {
        None
    }
}

#[must_use]
pub fn own() -> Option<u64> {
    ran("/proc/thread-self/schedstat")
}

#[must_use]
pub fn of(tid: u32) -> Option<u64> {
    ran(&format!("/proc/self/task/{tid}/schedstat"))
}

#[must_use]
pub fn lanes() -> u32 {
    std::thread::available_parallelism()
        .map_or(1, |count| u32::try_from(count.get()).unwrap_or(u32::MAX))
}

impl Driver {
    #[must_use]
    pub fn found(before: &Seen, after: &Seen, creator: &str, lanes: u32) -> Option<Self> {
        let fresh: Vec<&(u32, String)> = after
            .threads
            .iter()
            .filter(|one| !before.threads.iter().any(|old| old.0 == one.0))
            .collect();
        let raster: Vec<u32> = fresh
            .iter()
            .filter(|one| one.1.starts_with(RASTER))
            .map(|one| one.0)
            .collect();
        let mine: Vec<u32> = fresh
            .iter()
            .filter(|one| one.1 == creator)
            .map(|one| one.0)
            .collect();
        let (&first, rest) = mine.split_first()?;
        let low = raster.iter().copied().min()?;
        let high = raster.iter().copied().max()?;
        if rest.len() != raster.len() || first >= low || rest.iter().any(|tid| *tid <= high) {
            return None;
        }
        let workers = u32::try_from(rest.len()).ok()?;
        let mut pool = rest.to_vec();
        pool.extend(raster);
        Some(Self {
            serial: first,
            pool,
            lanes: workers.min(lanes).max(1),
        })
    }

    #[must_use]
    pub const fn lanes(&self) -> u32 {
        self.lanes
    }

    fn pooled(&self) -> Option<u64> {
        let mut pool = 0_u64;
        for tid in &self.pool {
            pool = pool.saturating_add(of(*tid)?);
        }
        Some(pool)
    }

    #[must_use]
    pub fn mark(&self) -> Option<Mark> {
        let pool = self.pooled()?;
        let serial = of(self.serial)?;
        Some(Mark {
            own: own()?,
            serial,
            pool,
        })
    }

    #[must_use]
    pub fn closing(&self) -> Option<Mark> {
        let own = own()?;
        Some(Mark {
            own,
            serial: of(self.serial)?,
            pool: self.pooled()?,
        })
    }

    #[must_use]
    pub fn between(&self, from: Mark, to: Mark) -> Duration {
        let own = to.own.saturating_sub(from.own);
        let serial = to.serial.saturating_sub(from.serial);
        let pool = to.pool.saturating_sub(from.pool) / u64::from(self.lanes.max(1));
        Duration::from_nanos(own.saturating_add(serial).saturating_add(pool))
    }
}
