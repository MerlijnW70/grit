use crate::rule3::MOST_COUNTS;

pub const VERSION: u32 = 1;

pub const BODY: usize = 1 << 20;

pub const NODES: usize = 16_384;

pub const PARAMS: usize = 32;

pub const COUNTS: usize = MOST_COUNTS;

pub const CHANNELS: u32 = 4;

pub const HALO: u32 = 1;

pub const WORK: u64 = 1_000_000;

pub const DEPTH: usize = 128;

pub const NAME: usize = 64;

pub const SHOWN: usize = 64;

pub const ALL: [(&str, u64); 9] = [
    ("body_bytes", BODY as u64),
    ("nodes", NODES as u64),
    ("params", PARAMS as u64),
    ("counts", COUNTS as u64),
    ("channels", CHANNELS as u64),
    ("halo", HALO as u64),
    ("work", WORK),
    ("depth", DEPTH as u64),
    ("name_bytes", NAME as u64),
];
