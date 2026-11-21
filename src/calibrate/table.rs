use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use crate::wire::{self, Json, map, text};

pub const FILE: &str = "calibration.json";

pub const VERSION: i64 = 2;

const FEMTOS: u128 = 1_000_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Device {
    pub name: String,
    pub driver: u32,
    pub width: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum By {
    Rule(String),
    Law(String),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Build { sparse: bool },
    Write { sparse: bool },
    Fork,
    Restore,
    Snapshot { file: bool },
    Save,
    File,
    Params,
    Frame { sparse: bool, by: By },
}

const fn layout(sparse: bool) -> &'static str {
    if sparse { "sparse" } else { "dense" }
}

fn laid(name: &str) -> Option<bool> {
    match name {
        "dense" => Some(false),
        "sparse" => Some(true),
        _ => None,
    }
}

#[must_use]
pub fn law_id(canonical: &str) -> String {
    crate::branch::format::sha256(canonical.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Key {
    #[must_use]
    pub fn rule(name: &str, sparse: bool) -> Self {
        Self::Frame {
            sparse,
            by: By::Rule(name.to_owned()),
        }
    }

    #[must_use]
    pub fn law(canonical: &str, sparse: bool) -> Self {
        Self::Frame {
            sparse,
            by: By::Law(law_id(canonical)),
        }
    }

    #[must_use]
    pub fn name(&self) -> String {
        match self {
            Self::Build { sparse } => format!("build/{}", layout(*sparse)),
            Self::Write { sparse } => format!("write/{}", layout(*sparse)),
            Self::Fork => "fork".to_owned(),
            Self::Restore => "restore".to_owned(),
            Self::Snapshot { file } => {
                format!("snapshot/{}", if *file { "file" } else { "memory" })
            }
            Self::Save => "save".to_owned(),
            Self::File => "file".to_owned(),
            Self::Params => "params".to_owned(),
            Self::Frame { sparse, by } => match by {
                By::Rule(name) => format!("frame/{}/rule/{name}", layout(*sparse)),
                By::Law(id) => format!("frame/{}/law/{id}", layout(*sparse)),
            },
        }
    }

    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        let parts: Vec<&str> = name.splitn(4, '/').collect();
        match parts.as_slice() {
            ["build", layout] => Some(Self::Build {
                sparse: laid(layout)?,
            }),
            ["write", layout] => Some(Self::Write {
                sparse: laid(layout)?,
            }),
            ["fork"] => Some(Self::Fork),
            ["restore"] => Some(Self::Restore),
            ["snapshot", "memory"] => Some(Self::Snapshot { file: false }),
            ["snapshot", "file"] => Some(Self::Snapshot { file: true }),
            ["save"] => Some(Self::Save),
            ["file"] => Some(Self::File),
            ["params"] => Some(Self::Params),
            ["frame", layout, "rule", rule] if !rule.is_empty() => Some(Self::Frame {
                sparse: laid(layout)?,
                by: By::Rule((*rule).to_owned()),
            }),
            ["frame", layout, "law", id]
                if id.len() == 16 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
            {
                Some(Self::Frame {
                    sparse: laid(layout)?,
                    by: By::Law((*id).to_owned()),
                })
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub units: u64,
    pub frames: u32,
    pub submits: u32,
    pub time: Duration,
}

impl Sample {
    #[must_use]
    pub const fn valid(&self) -> bool {
        !self.time.is_zero()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rate {
    pub per_unit_fs: u64,
    pub per_frame_ns: u64,
    pub per_submit_ns: u64,
}

impl Rate {
    #[must_use]
    pub fn nanos(&self, units: u64, frames: u32, submits: u32) -> u128 {
        let further = u128::from(frames.saturating_sub(submits));
        u128::from(units)
            .saturating_mul(u128::from(self.per_unit_fs))
            .div_euclid(FEMTOS)
            .saturating_add(u128::from(submits).saturating_mul(u128::from(self.per_submit_ns)))
            .saturating_add(further.saturating_mul(u128::from(self.per_frame_ns)))
    }

    #[must_use]
    pub fn predict(&self, units: u64, frames: u32, submits: u32) -> Duration {
        Duration::from_nanos(u64::try_from(self.nanos(units, frames, submits)).unwrap_or(u64::MAX))
    }

    #[must_use]
    pub fn framed(sizes: &[(Sample, Sample)]) -> Option<Self> {
        let mut first: Option<u128> = None;
        let mut further: Option<u128> = None;
        for (short, long) in sizes {
            if !short.valid()
                || !long.valid()
                || short.frames != 1
                || short.submits != 1
                || long.submits != 1
                || long.frames <= 1
            {
                return None;
            }
            let one = short.time.as_nanos();
            let each = long
                .time
                .as_nanos()
                .saturating_sub(one)
                .div_euclid(u128::from(long.frames - 1));
            first = Some(first.map_or(one, |held| held.min(one)));
            further = Some(further.map_or(each, |held| held.min(each)));
        }
        let rate = Self {
            per_unit_fs: 0,
            per_frame_ns: u64::try_from(further?).ok()?,
            per_submit_ns: u64::try_from(first?).ok()?,
        };
        (rate.per_submit_ns > 0).then_some(rate)
    }

    #[must_use]
    pub fn lowered(&self, sample: Sample) -> Option<Self> {
        if !sample.valid() {
            return None;
        }
        let predicted = self.nanos(sample.units, sample.frames, sample.submits);
        let taken = sample.time.as_nanos();
        if taken >= predicted || predicted == 0 {
            return None;
        }
        let scaled = |term: u64| {
            u64::try_from(u128::from(term).saturating_mul(taken).div_euclid(predicted))
                .unwrap_or(term)
                .min(term)
        };
        let lower = Self {
            per_unit_fs: scaled(self.per_unit_fs),
            per_frame_ns: scaled(self.per_frame_ns),
            per_submit_ns: scaled(self.per_submit_ns),
        };
        (lower != *self).then_some(lower)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    device: Device,
    rates: BTreeMap<Key, Rate>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Moved {
    Unchanged,
    Lowered,
}

impl Table {
    #[must_use]
    pub const fn new(device: Device) -> Self {
        Self {
            device,
            rates: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn device(&self) -> &Device {
        &self.device
    }

    #[must_use]
    pub fn rate(&self, key: &Key) -> Option<Rate> {
        self.rates.get(key).copied()
    }

    pub fn set(&mut self, key: Key, rate: Rate) {
        self.rates.insert(key, rate);
    }

    pub fn rates(&self) -> impl Iterator<Item = (&Key, &Rate)> {
        self.rates.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.rates.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rates.is_empty()
    }

    pub fn lower(&mut self, key: &Key, sample: Sample) -> Moved {
        let Some(lower) = self.rates.get(key).and_then(|rate| rate.lowered(sample)) else {
            return Moved::Unchanged;
        };
        self.rates.insert(key.clone(), lower);
        Moved::Lowered
    }

    fn body(&self) -> Json {
        map(vec![
            (
                "device",
                map(vec![
                    ("name", text(&self.device.name)),
                    ("driver", Json::Int(i64::from(self.device.driver))),
                    ("width", Json::Int(i64::from(self.device.width))),
                ]),
            ),
            (
                "rates",
                Json::List(
                    self.rates
                        .iter()
                        .map(|(key, rate)| {
                            map(vec![
                                ("key", text(&key.name())),
                                ("per_unit_fs", wide(rate.per_unit_fs)),
                                ("per_frame_ns", wide(rate.per_frame_ns)),
                                ("per_submit_ns", wide(rate.per_submit_ns)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ])
    }

    #[must_use]
    pub fn written(&self) -> String {
        let body = self.body();
        let sum = summed(&body);
        wire::written(&map(vec![
            ("calibration", Json::Int(VERSION)),
            ("body", body),
            ("sum", text(&sum)),
        ]))
    }

    pub fn read(held: &str) -> Result<Self, String> {
        let whole = wire::read(held).map_err(|_| "is not JSON".to_owned())?;
        let version = whole
            .get("calibration")
            .and_then(Json::int)
            .ok_or_else(|| "names no calibration version".to_owned())?;
        if version != VERSION {
            return Err(format!(
                "is calibration version {version}, and this server reads version {VERSION}"
            ));
        }
        let body = whole
            .get("body")
            .ok_or_else(|| "is damaged: it has no body".to_owned())?;
        let sum = whole.get("sum").and_then(Json::text);
        if sum != Some(summed(body).as_str()) {
            return Err("is damaged: its checksum does not match its content".to_owned());
        }
        let device = body
            .get("device")
            .ok_or_else(|| "is damaged: it names no device".to_owned())?;
        let device = Device {
            name: device
                .get("name")
                .and_then(Json::text)
                .ok_or_else(|| "is damaged: the device has no name".to_owned())?
                .to_owned(),
            driver: small(device, "driver")?,
            width: small(device, "width")?,
        };
        let mut table = Self::new(device);
        let listed = body
            .get("rates")
            .and_then(Json::list)
            .ok_or_else(|| "is damaged: it has no list of rates".to_owned())?;
        for one in listed {
            let named = one.get("key").and_then(Json::text).unwrap_or_default();
            let key = Key::parse(named)
                .ok_or_else(|| format!("is damaged: {named:?} is not a rate key"))?;
            let rate = Rate {
                per_unit_fs: large(one, "per_unit_fs")?,
                per_frame_ns: large(one, "per_frame_ns")?,
                per_submit_ns: large(one, "per_submit_ns")?,
            };
            if table.rates.insert(key, rate).is_some() {
                return Err(format!("is damaged: {named:?} is listed twice"));
            }
        }
        Ok(table)
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let path = dir.join(FILE);
        let staged = dir.join(format!("{FILE}.tmp"));
        std::fs::write(&staged, self.written())?;
        std::fs::rename(&staged, &path)
    }
}

fn summed(body: &Json) -> String {
    crate::branch::format::sha256(wire::written(body).as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn wide(value: u64) -> Json {
    Json::Int(i64::try_from(value).unwrap_or(i64::MAX))
}

fn small(held: &Json, key: &str) -> Result<u32, String> {
    held.get(key)
        .and_then(Json::int)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| format!("is damaged: the device's {key} is not a count"))
}

fn large(held: &Json, key: &str) -> Result<u64, String> {
    held.get(key)
        .and_then(Json::int)
        .and_then(|value| u64::try_from(value).ok())
        .ok_or_else(|| format!("is damaged: {key} is not a count"))
}
