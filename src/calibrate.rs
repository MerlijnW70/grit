pub mod cpu;
pub mod table;
pub mod take;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::host::Host;

pub use table::{By, Device, FILE, Key, Moved, Rate, Sample, Table, VERSION};

#[derive(Debug)]
pub struct Held {
    table: Table,
    dir: Option<PathBuf>,
}

impl Held {
    #[must_use]
    pub const fn table(&self) -> &Table {
        &self.table
    }

    pub fn lower(&mut self, key: &Key, sample: Sample, sound: bool) -> Moved {
        let moved = self.table.lower(key, sample, sound);
        if moved != Moved::Unchanged
            && let Some(dir) = &self.dir
        {
            let _ = self.table.save(dir);
        }
        moved
    }
}

pub type Shared = Arc<Mutex<Held>>;

static TABLES: Mutex<BTreeMap<Device, Shared>> = Mutex::new(BTreeMap::new());

fn tables() -> std::sync::MutexGuard<'static, BTreeMap<Device, Shared>> {
    TABLES.lock().unwrap_or_else(PoisonError::into_inner)
}

#[must_use]
pub fn held(table: Table, dir: Option<&Path>) -> Shared {
    Arc::new(Mutex::new(Held {
        table,
        dir: dir.map(Path::to_path_buf),
    }))
}

#[must_use]
pub fn inject(table: Table) -> Shared {
    let device = table.device().clone();
    let shared = held(table, None);
    tables().insert(device, Arc::clone(&shared));
    shared
}

#[must_use]
pub fn shared(device: &Device) -> Option<Shared> {
    tables().get(device).cloned()
}

pub fn forget(device: &Device) {
    tables().remove(device);
}

#[must_use]
pub fn read(shared: &Shared) -> Table {
    shared
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .table
        .clone()
}

#[must_use]
pub fn device(host: &Host) -> Device {
    Device {
        name: host.name().to_owned(),
        driver: host.driver_version(),
        width: host.width(),
        threads: host.driver_threads().map_or(0, cpu::Driver::lanes),
    }
}

pub type Meter<'host> = &'host cpu::Driver;

#[must_use]
pub fn meter(host: &Host) -> Option<Meter<'_>> {
    if host.on_cpu() {
        host.driver_threads()
    } else {
        None
    }
}

#[derive(Debug)]
pub struct Taken {
    pub table: Shared,
    pub refused: Option<String>,
    pub calibrated: bool,
}

pub fn load(dir: &Path, device: &Device) -> Result<Option<Table>, String> {
    let path = dir.join(FILE);
    let held = match std::fs::read(&path) {
        Ok(held) => String::from_utf8(held).unwrap_or_default(),
        Err(why) if why.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(why) => {
            return Err(format!(
                "{FILE} could not be read ({why}), so it was ignored and the device calibrated again"
            ));
        }
    };
    let table = Table::read(&held).map_err(|why| {
        format!("{FILE} {why}, so it was ignored and the device calibrated again")
    })?;
    if table.device() != device {
        let other = table.device();
        return Err(format!(
            "{FILE} holds the rates of another device ({}, driver {}, width {}, threads {}), so it was ignored and the device calibrated again",
            other.name, other.driver, other.width, other.threads
        ));
    }
    Ok(Some(table))
}

pub fn taken(host: &Host, dir: Option<&Path>) -> Result<Taken, crate::branch::Failed> {
    let device = device(host);
    if let Some(table) = shared(&device) {
        return Ok(Taken {
            table,
            refused: None,
            calibrated: false,
        });
    }
    let mut refused = None;
    if let Some(dir) = dir {
        match load(dir, &device) {
            Ok(Some(table)) => {
                let shared = held(table, Some(dir));
                tables().insert(device, Arc::clone(&shared));
                return Ok(Taken {
                    table: shared,
                    refused: None,
                    calibrated: false,
                });
            }
            Ok(None) => {}
            Err(why) => refused = Some(why),
        }
    }
    let table = take::calibrated(host)?;
    if let Some(dir) = dir {
        table.save(dir)?;
    }
    let shared = held(table, dir);
    tables().insert(device, Arc::clone(&shared));
    Ok(Taken {
        table: shared,
        refused,
        calibrated: true,
    })
}

#[cfg(feature = "faults")]
pub mod faults {
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

    pub type Hold = Arc<dyn Fn() + Send + Sync>;

    static HOLD: Mutex<Option<Hold>> = Mutex::new(None);

    static SOUND: Mutex<Vec<Duration>> = Mutex::new(Vec::new());

    pub fn hold(held: Option<Hold>) {
        *HOLD.lock().unwrap_or_else(PoisonError::into_inner) = held;
    }

    pub(crate) fn held() {
        let held = HOLD.lock().unwrap_or_else(PoisonError::into_inner).clone();
        if let Some(held) = held {
            held();
        }
    }

    pub fn script(times: impl IntoIterator<Item = Duration>) {
        let mut held = SOUND.lock().unwrap_or_else(PoisonError::into_inner);
        *held = times.into_iter().collect();
        held.reverse();
    }

    pub(crate) fn scripted() -> Option<Option<Duration>> {
        let mut held = SOUND.lock().unwrap_or_else(PoisonError::into_inner);
        if held.is_empty() {
            return None;
        }
        held.pop().map(|time| (!time.is_zero()).then_some(time))
    }
}

#[cfg(not(feature = "faults"))]
pub(crate) mod faults {
    use std::time::Duration;

    pub(crate) const fn held() {}

    pub(crate) const fn scripted() -> Option<Option<Duration>> {
        None
    }
}

#[must_use]
pub(crate) fn scripted_sound() -> Option<Option<Duration>> {
    faults::scripted()
}
