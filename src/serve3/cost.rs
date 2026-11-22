#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Priced {
    Build,
    Write,
    Fork,
    Restore,
    Snapshot,
    Save,
    File,
    Frame,
    Params,
    Replay,
}

impl Priced {
    pub const ALL: [Self; 10] = [
        Self::Build,
        Self::Write,
        Self::Fork,
        Self::Restore,
        Self::Snapshot,
        Self::Save,
        Self::File,
        Self::Frame,
        Self::Params,
        Self::Replay,
    ];

    const fn index(self) -> usize {
        match self {
            Self::Build => 0,
            Self::Write => 1,
            Self::Fork => 2,
            Self::Restore => 3,
            Self::Snapshot => 4,
            Self::Save => 5,
            Self::File => 6,
            Self::Frame => 7,
            Self::Params => 8,
            Self::Replay => 9,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Write => "write",
            Self::Fork => "fork",
            Self::Restore => "restore",
            Self::Snapshot => "snapshot",
            Self::Save => "save",
            Self::File => "file",
            Self::Frame => "frame",
            Self::Params => "params",
            Self::Replay => "replay",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cost {
    pub kind: Priced,
    pub units: u64,
    pub frames: u32,
    pub submits: u32,
}

impl Cost {
    #[must_use]
    pub const fn none(kind: Priced) -> Self {
        Self {
            kind,
            units: 0,
            frames: 0,
            submits: 0,
        }
    }

    #[must_use]
    pub const fn plus(self, other: Self) -> Self {
        Self {
            kind: self.kind,
            units: self.units.saturating_add(other.units),
            frames: self.frames.saturating_add(other.frames),
            submits: self.submits.saturating_add(other.submits),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tally {
    totals: [Cost; Priced::ALL.len()],
}

impl Default for Tally {
    fn default() -> Self {
        Self {
            totals: Priced::ALL.map(Cost::none),
        }
    }
}

impl Tally {
    pub fn add(&mut self, cost: Cost) {
        if let Some(total) = self.totals.get_mut(cost.kind.index()) {
            *total = total.plus(cost);
        }
    }

    #[must_use]
    pub fn of(&self, kind: Priced) -> Cost {
        self.totals
            .get(kind.index())
            .copied()
            .unwrap_or(Cost::none(kind))
    }
}
