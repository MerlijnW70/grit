#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::panic::{self, AssertUnwindSafe};
use std::time::Instant;

use grit::branch::format::{self, Image, Law, State};
use grit::branch::{Refused, sha256};

const DENSE_FIXTURE: &[u8] = include_bytes!("branch/dense.world");
const TWO_PLANES_FIXTURE: &[u8] = include_bytes!("branch/two-planes.world");

const CASES: u64 = 100_000;

const UNPACK_MOST: u64 = 1 << 16;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn below(state: &mut u64, most: u64) -> u64 {
    next(state) % most.max(1)
}

fn edges() -> Vec<u32> {
    let mut edges = vec![0, 1, 2];
    edges.extend((2..32).map(|power| 1_u32 << power));
    edges.push(u32::MAX);
    edges
}

fn wide_edges() -> Vec<u64> {
    let mut edges = vec![0, 1, 2, 3, 7, 8, 9];
    edges.extend((4..64).step_by(4).map(|power| 1_u64 << power));
    edges.extend([
        1 << 32,
        (1 << 32) - 1,
        1 << 62,
        1 << 63,
        u64::MAX,
        u64::MAX - 1,
    ]);
    edges
}

#[derive(Clone)]
struct Parsed {
    header: Vec<u8>,
    sections: Vec<([u8; 4], u64, Vec<u8>)>,
}

fn word_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn parsed(bytes: &[u8]) -> Parsed {
    let params = word_at(bytes, 44) as usize;
    let end = 88 + 4 * params;
    let mut sections = Vec::new();
    let mut at = end;
    loop {
        let tag: [u8; 4] = bytes[at..at + 4].try_into().unwrap();
        if tag == *b"SUM " {
            break;
        }
        let words = u64::from_le_bytes(bytes[at + 4..at + 12].try_into().unwrap());
        let length = words as usize * 4;
        sections.push((tag, words, bytes[at + 12..at + 12 + length].to_vec()));
        at += 12 + length;
    }
    Parsed {
        header: bytes[..end].to_vec(),
        sections,
    }
}

fn sealed(file: &Parsed) -> Vec<u8> {
    let mut bytes = file.header.clone();
    for (tag, words, payload) in &file.sections {
        bytes.extend_from_slice(tag);
        bytes.extend_from_slice(&words.to_le_bytes());
        bytes.extend_from_slice(payload);
    }
    let digest = sha256(&bytes);
    bytes.extend_from_slice(b"SUM ");
    bytes.extend_from_slice(&8_u64.to_le_bytes());
    bytes.extend_from_slice(&digest);
    bytes
}

fn heat(size: [u32; 3], state: State) -> Image {
    Image {
        law: Law::Rule {
            name: "heat".into(),
        },
        params: vec![1, 2, 3],
        frame: 9,
        size,
        state,
    }
}

fn two_planes() -> Image {
    format::decoded(TWO_PLANES_FIXTURE).unwrap()
}

fn seeds() -> Vec<Vec<u8>> {
    let mut state = 0xF022_5EED;
    let mut cells = |count: usize, kinds: u64| -> Vec<u32> {
        (0..count)
            .map(|at| {
                if at % 3 == 0 {
                    (next(&mut state) % kinds) as u32
                } else {
                    0
                }
            })
            .collect()
    };
    let dense = heat([4, 4, 2], State::Dense(cells(32, 5)));
    let mut sparse_cells = cells(512, 4);
    for word in sparse_cells.iter_mut().skip(256) {
        *word = 6;
    }
    let sparse = heat(
        [8, 8, 8],
        format::bricked(&sparse_cells, [8, 8, 8], [4, 4, 2], Some(5)),
    );
    let fixture = two_planes();
    let mut plane_cells = cells(128, 3);
    plane_cells.extend(cells(128, 7));
    for word in plane_cells.iter_mut().skip(128).take(16) {
        *word = 2;
    }
    let planes_sparse = Image {
        size: [8, 4, 4],
        state: format::bricked_planes(&plane_cells, 2, [8, 4, 4], [2, 2, 2], Some(12)),
        ..fixture.clone()
    };
    let planes_dense = Image {
        size: [4, 4, 2],
        state: State::Dense(cells(64, 9)),
        ..fixture
    };
    let images = [dense, sparse, planes_sparse, planes_dense];
    let mut seeds: Vec<Vec<u8>> = images
        .iter()
        .map(|image| {
            let bytes = format::encoded(image);
            assert_eq!(format::decoded(&bytes).as_ref(), Ok(image));
            bytes
        })
        .collect();
    seeds.push(DENSE_FIXTURE.to_vec());
    seeds
}

fn header_fields(params: usize) -> Vec<usize> {
    let mut fields = vec![8, 44];
    fields.extend((0..10).map(|word| 48 + 4 * params + 4 * word));
    fields
}

fn mutated(seed: &[u8], state: &mut u64) -> (Vec<u8>, &'static str) {
    let mut file = parsed(seed);
    let params = word_at(&file.header, 44) as usize;
    let edges = edges();
    let wide = wide_edges();
    let rounds = 1 + below(state, 3);
    let mut kind = "";
    for _ in 0..rounds {
        let choice = below(state, 14);
        match choice {
            0 => {
                kind = "bit flip in a section";
                if let Some(section) = pick(&mut file.sections, state)
                    && !section.2.is_empty()
                {
                    let at = below(state, section.2.len() as u64) as usize;
                    section.2[at] ^= 1 << below(state, 8);
                }
            }
            1 => {
                kind = "bit flip in the header";
                let at = below(state, file.header.len() as u64) as usize;
                file.header[at] ^= 1 << below(state, 8);
            }
            2 | 3 => {
                kind = "header word at an edge";
                let fields = header_fields(params);
                let at = fields[below(state, fields.len() as u64) as usize];
                let edge = edges[below(state, edges.len() as u64) as usize];
                file.header[at..at + 4].copy_from_slice(&edge.to_le_bytes());
            }
            4 => {
                kind = "size and sides at one edge";
                let edge = edges[below(state, edges.len() as u64) as usize];
                let size = 52 + 4 * params;
                for axis in 0..3 {
                    let word = if below(state, 3) == 0 {
                        edges[below(state, edges.len() as u64) as usize]
                    } else {
                        edge
                    };
                    let at = size + 4 * axis;
                    file.header[at..at + 4].copy_from_slice(&word.to_le_bytes());
                    if word_at(&file.header, 68 + 4 * params) == 1 {
                        let side = 76 + 4 * params + 4 * axis;
                        let brick = if below(state, 2) == 0 { word } else { 2 };
                        file.header[side..side + 4].copy_from_slice(&brick.to_le_bytes());
                    }
                }
            }
            5 => {
                kind = "section length at an edge";
                let edge = wide[below(state, wide.len() as u64) as usize];
                if let Some(section) = pick(&mut file.sections, state) {
                    section.1 = edge;
                }
            }
            6 => {
                kind = "section length off by one";
                if let Some(section) = pick(&mut file.sections, state) {
                    section.1 = if below(state, 2) == 0 {
                        section.1.wrapping_add(1)
                    } else {
                        section.1.wrapping_sub(1)
                    };
                }
            }
            7 => {
                kind = "section payload cut";
                if let Some(section) = pick(&mut file.sections, state) {
                    let keep = below(state, section.2.len() as u64 + 1) as usize;
                    section.2.truncate(keep & !3);
                    if below(state, 2) == 0 {
                        section.1 = (section.2.len() / 4) as u64;
                    }
                }
            }
            8 => {
                kind = "section duplicated";
                if !file.sections.is_empty() {
                    let at = below(state, file.sections.len() as u64) as usize;
                    let copy = file.sections[at].clone();
                    let into = below(state, file.sections.len() as u64 + 1) as usize;
                    file.sections.insert(into, copy);
                }
            }
            9 => {
                kind = "sections reordered";
                if file.sections.len() > 1 {
                    let a = below(state, file.sections.len() as u64) as usize;
                    let b = below(state, file.sections.len() as u64) as usize;
                    file.sections.swap(a, b);
                }
            }
            10 => {
                kind = "section dropped or retagged";
                if !file.sections.is_empty() {
                    let at = below(state, file.sections.len() as u64) as usize;
                    if below(state, 2) == 0 {
                        file.sections.remove(at);
                    } else {
                        let tags = [
                            *b"RULE", *b"LAW ", *b"TABL", *b"POOL", *b"DENS", *b"SUM ", *b"XYZW",
                        ];
                        file.sections[at].0 = tags[below(state, tags.len() as u64) as usize];
                    }
                }
            }
            11 => {
                kind = "table or pool word at an edge";
                let tags = [*b"TABL", *b"POOL", *b"DENS"];
                let tag = tags[below(state, 3) as usize];
                if let Some(section) = file.sections.iter_mut().find(|section| section.0 == tag)
                    && section.2.len() >= 4
                {
                    let at = below(state, (section.2.len() / 4) as u64) as usize * 4;
                    let word = match below(state, 4) {
                        0 => format::UNIFORM,
                        1 => 0,
                        2 => below(state, 8) as u32,
                        _ => edges[below(state, edges.len() as u64) as usize],
                    };
                    section.2[at..at + 4].copy_from_slice(&word.to_le_bytes());
                }
            }
            12 => {
                kind = "pool made uniform";
                if let Some(section) = file
                    .sections
                    .iter_mut()
                    .find(|section| section.0 == *b"POOL")
                {
                    let value = below(state, 3) as u8;
                    let from = below(state, section.2.len() as u64 + 1) as usize;
                    let run = below(state, 2048) as usize;
                    let end = (from + run).min(section.2.len());
                    for byte in &mut section.2[from..end] {
                        *byte = value;
                    }
                }
            }
            _ => {
                kind = "capacity at a neighbour";
                let at = 72 + 4 * params;
                let capacity = word_at(&file.header, at);
                let moved = match below(state, 3) {
                    0 => capacity.wrapping_sub(1),
                    1 => capacity.wrapping_add(1),
                    _ => 0,
                };
                file.header[at..at + 4].copy_from_slice(&moved.to_le_bytes());
            }
        }
    }
    let mut bytes = sealed(&file);
    match below(state, 20) {
        0 => {
            kind = "cut anywhere";
            let keep = below(state, bytes.len() as u64) as usize;
            bytes.truncate(keep);
        }
        1 => {
            kind = "flipped after the seal";
            let at = below(state, bytes.len() as u64) as usize;
            bytes[at] ^= 1 << below(state, 8);
        }
        _ => {}
    }
    (bytes, kind)
}

fn pick<'a, T>(items: &'a mut [T], state: &mut u64) -> Option<&'a mut T> {
    if items.is_empty() {
        return None;
    }
    let at = below(state, items.len() as u64) as usize;
    items.get_mut(at)
}

fn judged(bytes: &[u8], case: &str) -> Result<(), &'static str> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| format::decoded(bytes)));
    let image = match result {
        Err(_) => panic!("{case}: the reader panicked on {} bytes", bytes.len()),
        Ok(Err(why)) => {
            assert!(!why.name().is_empty(), "{case}");
            assert!(!why.to_string().is_empty(), "{case}");
            return Err(why.name());
        }
        Ok(Ok(image)) => image,
    };
    assert_eq!(
        format::encoded(&image),
        bytes,
        "{case}: accepted a file that writes back otherwise"
    );
    let cells = image
        .cells()
        .unwrap_or_else(|| panic!("{case}: accepted cells past 64 bits"));
    assert!(
        cells.checked_mul(u64::from(image.channels())).is_some(),
        "{case}"
    );
    if cells <= UNPACK_MOST
        && let State::Sparse {
            capacity, sides, ..
        } = &image.state
    {
        let unpacked = image.try_unpacked().unwrap();
        assert_eq!(unpacked.len() as u64, cells * u64::from(image.channels()));
        let canonical = format::bricked_planes(
            &unpacked,
            image.channels(),
            image.size,
            *sides,
            Some(*capacity),
        );
        assert_eq!(
            canonical, image.state,
            "{case}: a second file for one world"
        );
    }
    Ok(())
}

#[test]
fn a_hundred_thousand_mutated_files_give_a_world_or_a_named_refusal() {
    let seeds = seeds();
    let started = Instant::now();
    let mut state = 0x00B1_F0F0_22ED;
    let mut names: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut kinds: BTreeMap<&'static str, u64> = BTreeMap::new();
    for case in 0..CASES {
        let seed = &seeds[(case % seeds.len() as u64) as usize];
        let (bytes, kind) = mutated(seed, &mut state);
        *kinds.entry(kind).or_insert(0) += 1;
        let name = match judged(&bytes, &format!("case {case} ({kind})")) {
            Ok(()) => "accepted",
            Err(name) => name,
        };
        *names.entry(name).or_insert(0) += 1;
    }
    let seconds = started.elapsed().as_secs_f64();
    println!("{CASES} mutated files in {seconds:.1} s: {names:?}");
    println!("last mutation of each case: {kinds:?}");
    assert!(names.len() > 5, "{names:?}");
    assert_eq!(names.values().sum::<u64>(), CASES);
}

struct Header {
    size: [u32; 3],
    channels: u32,
    sparse: u32,
    capacity: u32,
    sides: [u32; 3],
}

fn crafted(header: &Header, sections: &[([u8; 4], Vec<u32>)], law: &Image) -> Vec<u8> {
    let text = law.law.canonical(law.params.len());
    let mut bytes = b"GRITWLD1".to_vec();
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&sha256(text.as_bytes()));
    bytes.extend_from_slice(&(law.params.len() as u32).to_le_bytes());
    for word in &law.params {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes.extend_from_slice(&law.frame.to_le_bytes());
    for word in header
        .size
        .iter()
        .chain([header.channels, header.sparse, header.capacity].iter())
        .chain(header.sides.iter())
    {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    let mut file = Parsed {
        header: bytes,
        sections: Vec::new(),
    };
    let mut law_words = vec![text.len() as u32];
    for chunk in text.as_bytes().chunks(4) {
        let mut four = [0_u8; 4];
        four[..chunk.len()].copy_from_slice(chunk);
        law_words.push(u32::from_le_bytes(four));
    }
    file.sections
        .push((law.law.tag(), law_words.len() as u64, to_bytes(&law_words)));
    for (tag, words) in sections {
        file.sections
            .push((*tag, words.len() as u64, to_bytes(words)));
    }
    sealed(&file)
}

fn to_bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|word| word.to_le_bytes()).collect()
}

const MOST_WORDS: u64 = 1 << 12;

fn sized_like(state: &mut u64, wanted: u64) -> usize {
    match below(state, 4) {
        0 => below(state, 9) as usize,
        _ if wanted <= MOST_WORDS => wanted as usize,
        _ => (wanted % MOST_WORDS) as usize,
    }
}

fn header_case(state: &mut u64, header: Header, law: &Image) -> Vec<u8> {
    let cells = header
        .size
        .iter()
        .fold(1_u64, |total, &side| total.wrapping_mul(u64::from(side)));
    let per = header
        .sides
        .iter()
        .fold(1_u64, |total, &side| total.wrapping_mul(u64::from(side)));
    let planes = u64::from(header.channels);
    if header.sparse != 1 {
        let words = sized_like(state, cells.wrapping_mul(planes));
        let dense: Vec<u32> = (0..words).map(|_| below(state, 4) as u32).collect();
        return crafted(&header, &[(*b"DENS", dense)], law);
    }
    let bricks = cells.checked_div(per).unwrap_or(0);
    let entries = sized_like(state, bricks.wrapping_mul(planes.wrapping_add(1)));
    let heads = (bricks.wrapping_mul(2) as usize).min(entries);
    let mut held = 0_u32;
    let mut table = Vec::with_capacity(entries);
    for at in 0..entries {
        if at < heads && at % 2 == 0 && below(state, 3) == 0 {
            table.push(held);
            held += 1;
        } else if at < heads && at % 2 == 0 {
            table.push(format::UNIFORM);
        } else if at < heads {
            table.push(if table.last() == Some(&format::UNIFORM) {
                below(state, 4) as u32
            } else {
                0
            });
        } else {
            table.push(0);
        }
    }
    let pooled = sized_like(
        state,
        u64::from(held).wrapping_mul(per).wrapping_mul(planes),
    );
    let pool: Vec<u32> = (0..pooled).map(|at| (at % 3) as u32).collect();
    crafted(&header, &[(*b"TABL", table), (*b"POOL", pool)], law)
}

fn headers_at_the_edges(cases: u64, seed: u64) {
    let edges = edges();
    let laws = [heat([2, 2, 2], State::Dense(Vec::new())), two_planes()];
    let mut state = seed;
    let started = Instant::now();
    let mut names: BTreeMap<&'static str, u64> = BTreeMap::new();
    let mut count = 0_u64;
    let mut run = |header: Header, state: &mut u64, names: &mut BTreeMap<&'static str, u64>| {
        let law = &laws[usize::from(header.channels > 1)];
        let bytes = header_case(state, header, law);
        let name = match judged(&bytes, &format!("header case {count}")) {
            Ok(()) => "accepted",
            Err(name) => name,
        };
        *names.entry(name).or_insert(0) += 1;
        count += 1;
    };
    for &side in &edges {
        for &brick in &edges {
            for &capacity in &edges {
                let header = Header {
                    size: [side; 3],
                    channels: 1,
                    sparse: 1,
                    capacity,
                    sides: [brick; 3],
                };
                run(header, &mut state, &mut names);
            }
        }
    }
    let channels = [0, 1, 2, 3, 4, 5, u32::MAX];
    for _ in 0..cases {
        let pick = |state: &mut u64| edges[below(state, edges.len() as u64) as usize];
        let header = Header {
            size: [pick(&mut state), pick(&mut state), pick(&mut state)],
            channels: channels[below(&mut state, channels.len() as u64) as usize],
            sparse: [0, 1, 1, 2][below(&mut state, 4) as usize],
            capacity: pick(&mut state),
            sides: [pick(&mut state), pick(&mut state), pick(&mut state)],
        };
        run(header, &mut state, &mut names);
    }
    println!(
        "{count} headers at the edges in {:.1} s: {names:?}",
        started.elapsed().as_secs_f64()
    );
    assert!(names.contains_key("accepted"), "{names:?}");
    assert!(names.contains_key("malformed"), "{names:?}");
    assert!(names.contains_key("sizes"), "{names:?}");
}

#[test]
fn every_header_drawn_from_the_edges_gives_a_world_or_a_named_refusal() {
    headers_at_the_edges(CASES, 0xED6E_5EED);
}

#[test]
fn the_five_reviewed_files_are_refused_or_held_canonical() {
    let big = 1_u32 << 31;
    let heat_law = heat([2, 2, 2], State::Dense(Vec::new()));
    let sparse = crafted(
        &Header {
            size: [big; 3],
            channels: 1,
            sparse: 1,
            capacity: 1,
            sides: [big; 3],
        },
        &[(*b"TABL", vec![format::UNIFORM, 7]), (*b"POOL", Vec::new())],
        &heat_law,
    );
    assert_eq!(sparse.len(), 208 + 12);
    let dense = crafted(
        &Header {
            size: [big, big, 4],
            channels: 1,
            sparse: 0,
            capacity: 0,
            sides: [0; 3],
        },
        &[(*b"DENS", Vec::new())],
        &heat_law,
    );
    for bytes in [&sparse, &dense] {
        assert!(matches!(
            format::decoded(bytes),
            Err(Refused::Malformed { .. })
        ));
    }
    let mut table = vec![0, 0];
    table.extend([format::UNIFORM, 0].repeat(7));
    let held_uniform = crafted(
        &Header {
            size: [4, 4, 4],
            channels: 1,
            sparse: 1,
            capacity: 1,
            sides: [2, 2, 2],
        },
        &[(*b"TABL", table), (*b"POOL", vec![0; 8])],
        &heat_law,
    );
    assert_eq!(judged(&held_uniform, "held uniform"), Err("malformed"));
    let empty = crafted(
        &Header {
            size: [4, 4, 4],
            channels: 1,
            sparse: 1,
            capacity: 0,
            sides: [2, 2, 2],
        },
        &[
            (*b"TABL", [format::UNIFORM, 5].repeat(8)),
            (*b"POOL", Vec::new()),
        ],
        &heat_law,
    );
    assert_eq!(judged(&empty, "capacity 0"), Err("sizes"));
}
