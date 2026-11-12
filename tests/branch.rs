#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::PathBuf;

use common::device;

use grit::Host;
use grit::branch::format::{self, Image, Law, State};
use grit::branch::{Made, Refused, Sha256, World, sha256};
use grit::rule3::code::{self, ADD, DATA, MOVE};
use grit::rule3::heat;
use grit::sand3::Grid3;
use grit::serve3::rules;

const SIZE: [u32; 3] = [32, 32, 32];

const DENSE_FIXTURE: &[u8] = include_bytes!("branch/dense.world");
const SPARSE_FIXTURE: &[u8] = include_bytes!("branch/sparse.world");

const DENSE_FIXTURE_SHA: &str = "eae31f21a837d40b7873531256e699b61569d28af8564a3a8274d766eaec8035";
const DENSE_FIXTURE_CELLS: &str =
    "30ecd46a3d1a216c3e13ae26f5b9c756d4e2eebdbcb2c0720c154cefc6c143d6";
const SPARSE_FIXTURE_SHA: &str = "138cdd1d13ca10887ba29310b20e4a14585e0a46a42fa10db89178d3cfdb0d32";
const SPARSE_FIXTURE_CELLS: &str =
    "eae3e59aac78076f01e6bcfef7bca327f3a8b03a703967c2785c04b19e7eca6e";

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn words_hex(words: &[u32]) -> String {
    let bytes: Vec<u8> = words.iter().flat_map(|word| word.to_le_bytes()).collect();
    hex(&sha256(&bytes))
}

fn heat_cell(state: &mut u64) -> u32 {
    let roll = next(state);
    let kind = match roll % 32 {
        0..10 => heat::AIR,
        10..12 => heat::WALL,
        12..15 => heat::ICE,
        15..19 => heat::WATER,
        19..22 => heat::STEAM,
        22..26 => heat::FUEL,
        26..28 => heat::FIRE,
        28..31 => heat::ASH,
        _ => heat::KINDS + ((roll >> 40) % 8) as u32,
    };
    let temperature = ((roll >> 8) % 1200) as u32;
    let life = ((roll >> 20) % 16) as u32;
    let kept = ((roll >> 24) & 0xFF) as u32;
    heat::cell(kind, temperature) | (life << 4) | (kept << 8)
}

fn wild(state: &mut u64, rule: &str) -> u32 {
    match rule {
        "heat" => heat_cell(state),
        "code" => {
            let roll = next(state);
            let does = [code::COPY, MOVE, ADD, DATA, code::TURN][(roll % 5) as usize];
            let way = ((roll >> 8) % 8) as u32;
            code::word(does, way, (roll >> 16) as u32) | (((roll >> 63) as u32) << 31)
        }
        "sand" => (next(state) % 4) as u32,
        _ => (next(state) & 0xFFFF_FFFF) as u32,
    }
}

fn params_of(rule: &str, wanted: usize) -> Vec<u32> {
    match rule {
        "nature" => vec![0; wanted],
        _ => (0..wanted)
            .map(|at| heat::TUNED.get(at).copied().unwrap_or(1))
            .collect(),
    }
}

fn every_rule() -> Vec<(String, Vec<u32>)> {
    rules()
        .iter()
        .map(|one| (one.name().to_owned(), params_of(one.name(), one.params())))
        .collect()
}

fn scattered(rule: &str, seed: u64, many: usize, size: [u32; 3]) -> Vec<([u32; 3], u32)> {
    let mut state = seed;
    (0..many)
        .map(|_| {
            let at = next(&mut state);
            (
                [
                    (at % u64::from(size[0])) as u32,
                    ((at >> 16) % u64::from(size[1])) as u32,
                    ((at >> 32) % u64::from(size[2])) as u32,
                ],
                wild(&mut state, rule),
            )
        })
        .collect()
}

fn made<'made>(rule: &'made str, params: &'made [u32], sparse: bool) -> Made<'made> {
    Made {
        rule,
        params,
        size: SIZE,
        sparse,
        slots: None,
    }
}

fn filled<'host>(
    host: &'host Host,
    rule: &str,
    params: &[u32],
    sparse: bool,
    seed: u64,
) -> World<'host> {
    let mut world = World::create(host, &made(rule, params, sparse)).unwrap();
    let mut state = seed;
    let mut cells = Vec::new();
    for _ in 0..300 {
        let at = next(&mut state);
        cells.push((
            [
                4 + (at % 20) as u32,
                2 + ((at >> 8) % 12) as u32,
                6 + ((at >> 16) % 20) as u32,
            ],
            wild(&mut state, rule),
        ));
    }
    world.write(&cells).unwrap();
    world
}

fn folder(name: &str) -> PathBuf {
    let held = std::env::temp_dir().join(format!("grit-branch-{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&held);
    fs::create_dir_all(&held).unwrap();
    held
}

fn hashed(world: &mut World<'_>) -> [u8; 32] {
    world.state_hash().unwrap()
}

fn refused(why: &(dyn std::error::Error + 'static)) -> &'static str {
    why.downcast_ref::<Refused>()
        .map_or("not a refusal", Refused::name)
}

fn decoded(bytes: &[u8]) -> Result<Image, Refused> {
    format::decoded(bytes)
}

fn resealed(mut bytes: Vec<u8>) -> Vec<u8> {
    let body = bytes.len() - format::SUM_BYTES;
    let digest = sha256(&bytes[..body]);
    let at = bytes.len() - 32;
    bytes[at..].copy_from_slice(&digest);
    bytes
}

fn word_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

fn put_word(bytes: &mut [u8], at: usize, word: u32) {
    bytes[at..at + 4].copy_from_slice(&word.to_le_bytes());
}

fn header_end(bytes: &[u8]) -> usize {
    let params = word_at(bytes, 44) as usize;
    48 + params * 4 + 4 + 12 + 12 + 12
}

fn section_at(bytes: &[u8], tag: [u8; 4]) -> usize {
    let mut at = header_end(bytes);
    loop {
        if bytes[at..at + 4] == tag {
            return at;
        }
        let words = u64::from_le_bytes(bytes[at + 4..at + 12].try_into().unwrap());
        at += 12 + words as usize * 4;
    }
}

fn reference(
    rule: &str,
    size: [u32; 3],
    start: &[u32],
    params: &[u32],
    first: u32,
    frames: u32,
) -> Vec<u32> {
    let listed = rules().into_iter().find(|one| one.name() == rule).unwrap();
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    (0..frames).fold(start.to_vec(), |cells, done| {
        listed.reference(grid, first.wrapping_add(done), &cells, params)
    })
}

#[test]
fn sha256_gives_the_published_digests_whole_and_in_pieces() {
    assert_eq!(
        hex(&sha256(b"")),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        hex(&sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    let million = vec![b'a'; 1_000_000];
    assert_eq!(
        hex(&sha256(&million)),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );
    let mut state = 0x5A_1E_5E;
    let bytes: Vec<u8> = (0..4099).map(|_| next(&mut state) as u8).collect();
    let whole = sha256(&bytes);
    for _ in 0..200 {
        let mut hash = Sha256::new();
        let mut at = 0;
        while at < bytes.len() {
            let run = ((next(&mut state) % 150) as usize).min(bytes.len() - at);
            hash.update(&bytes[at..at + run]);
            at += run;
        }
        assert_eq!(hash.finish(), whole);
    }
}

fn fixture_checked(bytes: &[u8], sha: &str, cells: &str) -> Image {
    assert_eq!(hex(&sha256(bytes)), sha, "the checked-in file changed");
    let image = decoded(bytes).unwrap();
    assert_eq!(
        format::encoded(&image),
        bytes,
        "the writer drifted from the file"
    );
    assert_eq!(words_hex(&image.unpacked()), cells, "the reader drifted");
    image
}

#[test]
fn the_checked_in_dense_file_reads_as_version_1_and_writes_back_byte_for_byte() {
    let image = fixture_checked(DENSE_FIXTURE, DENSE_FIXTURE_SHA, DENSE_FIXTURE_CELLS);
    assert_eq!(&DENSE_FIXTURE[..8], b"GRITWLD1");
    assert_eq!(word_at(DENSE_FIXTURE, 8), 1);
    assert_eq!(
        image.law,
        Law::Rule {
            name: "sand".into()
        }
    );
    assert!(image.params.is_empty());
    assert_eq!(image.frame, 3);
    assert_eq!(image.size, [16, 8, 8]);
    assert!(matches!(image.state, State::Dense(ref cells) if cells.len() == 1024));
    assert_eq!(&DENSE_FIXTURE[12..44], &image.law_hash());
    assert_eq!(
        image.law.canonical(image.params.len()),
        "{\"params\":0,\"rule\":\"sand\"}"
    );
}

#[test]
fn the_checked_in_sparse_file_reads_as_version_1_and_writes_back_byte_for_byte() {
    let image = fixture_checked(SPARSE_FIXTURE, SPARSE_FIXTURE_SHA, SPARSE_FIXTURE_CELLS);
    assert_eq!(
        image.law,
        Law::Rule {
            name: "heat".into()
        }
    );
    assert_eq!(image.params, heat::TUNED);
    assert_eq!(image.frame, 3);
    assert_eq!(image.size, SIZE);
    let State::Sparse {
        capacity,
        sides,
        table,
        pool,
    } = &image.state
    else {
        panic!("the sparse file reads as dense");
    };
    assert_eq!(*sides, [16, 16, 8]);
    assert_eq!(table.len(), 2 * 2 * 2 * 4);
    let held = table
        .chunks(2)
        .filter(|entry| entry[0] != format::UNIFORM)
        .count();
    assert!((2..16).contains(&held), "{held}");
    assert_eq!(pool.len(), held * 16 * 16 * 8);
    assert_eq!(*capacity, 16);
}

#[test]
fn every_one_of_a_thousand_single_byte_corruptions_is_refused() {
    for (name, bytes) in [("dense", DENSE_FIXTURE), ("sparse", SPARSE_FIXTURE)] {
        let mut state = 0xC0_77_0B;
        let mut names = BTreeMap::new();
        for at in 0..1000 {
            let place = at * bytes.len() / 1000;
            let flip = 1 + (next(&mut state) % 255) as u8;
            let mut broken = bytes.to_vec();
            broken[place] ^= flip;
            let why = decoded(&broken)
                .err()
                .unwrap_or_else(|| panic!("{name}: byte {place} ^ {flip:#04x} was accepted"));
            *names.entry(why.name()).or_insert(0_u32) += 1;
        }
        println!("{name}: 1000 corruptions refused as {names:?}");
    }
}

#[test]
fn a_bad_magic_is_refused_by_name() {
    for at in 0..8 {
        let mut broken = DENSE_FIXTURE.to_vec();
        broken[at] ^= 0x01;
        assert_eq!(decoded(&broken), Err(Refused::Magic), "byte {at}");
    }
    assert_eq!(decoded(b"GRITWLD2xxxxxxxx"), Err(Refused::Magic));
    assert_eq!(decoded(b"PK\x03\x04"), Err(Refused::Magic));
}

#[test]
fn an_unknown_version_is_refused_by_name() {
    for version in [0, 2, 7, u32::MAX] {
        let mut broken = SPARSE_FIXTURE.to_vec();
        put_word(&mut broken, 8, version);
        assert_eq!(decoded(&broken), Err(Refused::Version { found: version }));
        assert_eq!(
            decoded(&resealed(broken)),
            Err(Refused::Version { found: version })
        );
    }
}

#[test]
fn a_bad_checksum_is_refused_by_name() {
    for bytes in [DENSE_FIXTURE, SPARSE_FIXTURE] {
        let middle = bytes.len() / 2;
        let last = bytes.len() - 1;
        for at in [header_end(bytes) - 4, middle, last, last - 31] {
            let mut broken = bytes.to_vec();
            broken[at] ^= 0x80;
            assert_eq!(decoded(&broken), Err(Refused::Checksum), "byte {at}");
        }
    }
}

#[test]
fn a_truncated_file_is_refused_by_name_at_every_length() {
    for (bytes, stride) in [(DENSE_FIXTURE, 1), (SPARSE_FIXTURE, 7)] {
        let lengths = (0..bytes.len())
            .step_by(stride)
            .chain(bytes.len() - 64..bytes.len());
        for length in lengths {
            match decoded(&bytes[..length]) {
                Err(Refused::Truncated { .. }) => {}
                other => panic!("cut at {length} of {}: {other:?}", bytes.len()),
            }
        }
    }
}

#[test]
fn sizes_that_disagree_with_the_header_are_refused_by_name() {
    let size_at = |bytes: &[u8]| header_end(bytes) - 36;
    let mut dense = DENSE_FIXTURE.to_vec();
    let at = size_at(&dense);
    assert_eq!(word_at(&dense, at), 16);
    put_word(&mut dense, at, 32);
    assert_eq!(
        decoded(&resealed(dense)),
        Err(Refused::Sizes {
            what: "dense",
            header: 32 * 8 * 8,
            found: 16 * 8 * 8
        })
    );

    let mut sparse = SPARSE_FIXTURE.to_vec();
    let sides = header_end(&sparse) - 12;
    assert_eq!(word_at(&sparse, sides), 16);
    put_word(&mut sparse, sides, 8);
    assert_eq!(
        decoded(&resealed(sparse)).map_err(|why| why.name()),
        Err("sizes")
    );

    let mut sparse = SPARSE_FIXTURE.to_vec();
    let capacity = header_end(&sparse) - 16;
    put_word(&mut sparse, capacity, 1);
    assert!(matches!(
        decoded(&resealed(sparse)),
        Err(Refused::Sizes {
            what: "capacity",
            header: 1,
            ..
        })
    ));

    let mut sparse = SPARSE_FIXTURE.to_vec();
    let capacity = header_end(&sparse) - 16;
    put_word(&mut sparse, capacity, 17);
    assert!(matches!(
        decoded(&resealed(sparse)),
        Err(Refused::Sizes {
            what: "capacity",
            header: 17,
            ..
        })
    ));

    let image = decoded(SPARSE_FIXTURE).unwrap();
    let mut shorter = image.clone();
    if let State::Sparse { pool, .. } = &mut shorter.state {
        pool.truncate(pool.len() - 1);
    }
    assert!(matches!(
        decoded(&format::encoded(&shorter)),
        Err(Refused::Sizes { what: "pool", .. })
    ));
    let mut fewer = image;
    fewer.params.pop();
    let mut bytes = format::encoded(&fewer);
    let law = section_at(&bytes, format::RULE);
    let text = fewer.law.canonical(3);
    let mut words = format::text_words(&text);
    words.resize(word_at(&bytes, law + 4) as usize, 0);
    for (at, word) in words.iter().enumerate() {
        put_word(&mut bytes, law + 12 + at * 4, *word);
    }
    bytes[12..44].copy_from_slice(&sha256(text.as_bytes()));
    assert_eq!(
        decoded(&resealed(bytes)),
        Err(Refused::Sizes {
            what: "params",
            header: 2,
            found: 3
        })
    );
}

#[test]
fn a_law_hash_that_does_not_match_the_law_section_is_refused_by_name() {
    let mut broken = DENSE_FIXTURE.to_vec();
    broken[20] ^= 0x10;
    assert_eq!(decoded(&resealed(broken)), Err(Refused::LawHash));

    let mut broken = SPARSE_FIXTURE.to_vec();
    let law = section_at(&broken, format::RULE);
    let quoted = law + 12 + 4 + "{\"params\":3,\"rule\":\"".len();
    assert_eq!(&broken[quoted..quoted + 4], b"heat");
    broken[quoted..quoted + 4].copy_from_slice(b"sand");
    assert_eq!(decoded(&resealed(broken)), Err(Refused::LawHash));
}

#[test]
fn an_unknown_section_and_bytes_past_the_checksum_are_refused_by_name() {
    let mut broken = DENSE_FIXTURE.to_vec();
    let at = section_at(&broken, format::DENSE);
    broken[at..at + 4].copy_from_slice(b"DENZ");
    assert_eq!(
        decoded(&resealed(broken)),
        Err(Refused::Section { tag: *b"DENZ" })
    );

    let mut longer = SPARSE_FIXTURE.to_vec();
    longer.push(0);
    assert_eq!(
        decoded(&longer),
        Err(Refused::Trailing {
            at: SPARSE_FIXTURE.len() as u64
        })
    );
}

#[test]
fn the_smallest_files_are_refused_as_truncated() {
    assert!(matches!(
        decoded(b""),
        Err(Refused::Truncated { at: 0, .. })
    ));
    assert!(matches!(
        decoded(b"GRITWLD1"),
        Err(Refused::Truncated { at: 8, .. })
    ));
    assert!(matches!(
        decoded(&DENSE_FIXTURE[..DENSE_FIXTURE.len() - 32]),
        Err(Refused::Truncated { .. })
    ));
}

#[test]
fn extreme_header_values_round_trip_through_the_format() {
    let image = Image {
        law: Law::Rule {
            name: "heat".into(),
        },
        params: vec![u32::MAX, 0, u32::MAX],
        frame: u32::MAX,
        size: [2, 2, 2],
        state: State::Dense(vec![u32::MAX, 0, 1, 2, 3, 4, 5, u32::MAX - 1]),
    };
    assert_eq!(decoded(&format::encoded(&image)), Ok(image));

    let cells: Vec<u32> = (0..64).map(|at| if at == 63 { 9 } else { 0 }).collect();
    let state = format::bricked(&cells, [4, 4, 4], [4, 4, 4], None);
    let image = Image {
        law: Law::Rule {
            name: "sa\"nd\n".into(),
        },
        params: Vec::new(),
        frame: 0,
        size: [4, 4, 4],
        state,
    };
    let back = decoded(&format::encoded(&image)).unwrap();
    assert_eq!(back.unpacked(), cells);
    assert_eq!(back, image);
}

struct Failing {
    inner: File,
    left: u64,
}

impl Write for Failing {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other("the write was cut off halfway"));
        }
        let run = bytes.len().min(self.left as usize);
        let written = self.inner.write(&bytes[..run])?;
        self.left -= written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

fn leftovers(folder: &PathBuf) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_save_cut_off_halfway_leaves_the_old_file_whole() {
    let folder = folder("cut");
    let path = folder.join("kept.world");
    let old = decoded(DENSE_FIXTURE).unwrap();
    format::store(&path, &old).unwrap();
    let new = decoded(SPARSE_FIXTURE).unwrap();
    let half = SPARSE_FIXTURE.len() as u64 / 2;
    let why = format::store_with(&path, &new, |inner| Failing { inner, left: half }).unwrap_err();
    assert_eq!(why.name(), "io");
    assert_eq!(format::fetch(&path).unwrap(), old);
    assert_eq!(fs::read(&path).unwrap(), DENSE_FIXTURE);
    assert_eq!(leftovers(&folder), ["kept.world"]);
    format::store(&path, &new).unwrap();
    assert_eq!(format::fetch(&path).unwrap(), new);
    assert_eq!(leftovers(&folder), ["kept.world"]);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_world_save_cut_off_halfway_leaves_the_old_world_loadable() {
    let Some(host) = device("branch cut") else {
        return;
    };
    let folder = folder("world-cut");
    for sparse in [false, true] {
        let path = folder.join(format!("{sparse}.world"));
        let params = params_of("heat", 3);
        let mut world = filled(&host, "heat", &params, sparse, 0xC07);
        world.step(4).unwrap();
        let size = world.save(&path).unwrap();
        let old = hashed(&mut world);
        world.step(9).unwrap();
        world
            .write(&[([1, 2, 3], heat::cell(heat::FIRE, 900))])
            .unwrap();
        let new = hashed(&mut world);
        assert_ne!(old, new);
        let why = world
            .save_with(&path, |inner| Failing {
                inner,
                left: size / 2,
            })
            .unwrap_err();
        assert_eq!(refused(why.as_ref()), "io");
        let mut loaded = World::load(&host, &path).unwrap();
        assert_eq!(hashed(&mut loaded), old);
        assert_eq!(loaded.frame(), 4);
        assert_eq!(leftovers(&folder).len(), usize::from(sparse) + 1);
        world.save(&path).unwrap();
        let mut loaded = World::load(&host, &path).unwrap();
        assert_eq!(hashed(&mut loaded), new);
    }
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_loaded_world_steps_as_the_saved_one() {
    let Some(host) = device("branch load") else {
        return;
    };
    let folder = folder("load");
    for (rule, params) in every_rule() {
        for sparse in [false, true] {
            let mut world = filled(&host, &rule, &params, sparse, 0x5A7E ^ rule.len() as u64);
            world.step(1).unwrap();
            let path = folder.join(format!("{rule}-{sparse}.world"));
            let bytes = world.save(&path).unwrap();
            let mut loaded = World::load(&host, &path).unwrap();
            assert_eq!(loaded.frame(), 1);
            assert_eq!(loaded.rule(), rule);
            assert_eq!(loaded.params(), params);
            let start = hashed(&mut world);
            assert_eq!(hashed(&mut loaded), start, "{rule} sparse {sparse}");
            for frames in [1, 1, 1, 5, 56] {
                world.step(frames).unwrap();
                loaded.step(frames).unwrap();
                assert_eq!(
                    hashed(&mut loaded),
                    hashed(&mut world),
                    "{rule} sparse {sparse} at frame {}",
                    loaded.frame()
                );
                if loaded.frame() > 3 {
                    assert_eq!(loaded.tally().unwrap(), world.tally().unwrap(), "{rule}");
                }
            }
            assert_eq!(loaded.frame(), 65);
            assert_ne!(hashed(&mut world), start, "{rule} moved in 64 frames");
            println!(
                "{rule} sparse {sparse}: saved in {bytes} bytes, loaded, and stepped 64 frames as the source"
            );
        }
    }
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_forked_world_steps_as_its_source() {
    let Some(host) = device("branch fork") else {
        return;
    };
    for (rule, params) in every_rule() {
        for sparse in [false, true] {
            let mut source = filled(&host, &rule, &params, sparse, 0xF0_4C ^ rule.len() as u64);
            source.step(5).unwrap();
            let start = hashed(&mut source);
            let mut fork = source.fork().unwrap();
            assert_eq!(fork.frame(), source.frame());
            assert_eq!(hashed(&mut fork), start);
            assert_eq!(fork.listed().unwrap(), source.listed().unwrap());
            for frames in [1, 7, 56] {
                source.step(frames).unwrap();
                fork.step(frames).unwrap();
                assert_eq!(fork.tally().unwrap(), source.tally().unwrap(), "{rule}");
                assert_eq!(fork.listed().unwrap(), source.listed().unwrap(), "{rule}");
                assert_eq!(
                    hashed(&mut fork),
                    hashed(&mut source),
                    "{rule} sparse {sparse}"
                );
            }
            assert_ne!(hashed(&mut source), start, "{rule} moved in 64 frames");
        }
    }
}

#[test]
fn a_fork_and_its_source_never_touch_each_other() {
    let Some(host) = device("branch apart") else {
        return;
    };
    for (rule, params) in [("sand", vec![]), ("heat", params_of("heat", 3))] {
        for sparse in [false, true] {
            let mut source = filled(&host, rule, &params, sparse, 0xA9A7);
            source.step(3).unwrap();
            let start = hashed(&mut source);
            let mut fork = source.fork().unwrap();
            let far = scattered(rule, 0x0FF, 40, SIZE);
            fork.step(16).unwrap();
            assert_eq!(
                hashed(&mut source),
                start,
                "{rule} {sparse}: stepping the fork"
            );
            fork.write(&far).unwrap();
            assert_eq!(
                hashed(&mut source),
                start,
                "{rule} {sparse}: writing the fork"
            );
            assert_eq!(source.frame(), 3);
            let forked = hashed(&mut fork);
            assert_ne!(forked, start);
            source.step(16).unwrap();
            assert_eq!(
                hashed(&mut fork),
                forked,
                "{rule} {sparse}: stepping the source"
            );
            source.write(&scattered(rule, 0xBEE, 40, SIZE)).unwrap();
            assert_eq!(
                hashed(&mut fork),
                forked,
                "{rule} {sparse}: writing the source"
            );
            assert_eq!(fork.frame(), 19);
            assert_ne!(hashed(&mut source), forked);
        }
    }
}

#[test]
fn a_restored_world_is_its_fork_again_and_steps_as_it() {
    let Some(host) = device("branch restore") else {
        return;
    };
    for (rule, params) in [("sand", vec![]), ("nature", params_of("nature", 1))] {
        for sparse in [false, true] {
            let mut world = filled(&host, rule, &params, sparse, 0x4E57);
            world.step(3).unwrap();
            let mut kept = world.fork().unwrap();
            let before = hashed(&mut kept);
            world.step(20).unwrap();
            world.write(&scattered(rule, 0x11, 30, SIZE)).unwrap();
            assert_ne!(hashed(&mut world), before);
            world.restore(&mut kept).unwrap();
            assert_eq!(hashed(&mut world), before);
            assert_eq!(world.frame(), 3);
            for frames in [5, 40] {
                world.step(frames).unwrap();
                kept.step(frames).unwrap();
                assert_eq!(world.tally().unwrap(), kept.tally().unwrap());
                assert_eq!(hashed(&mut world), hashed(&mut kept), "{rule} {sparse}");
            }
        }
    }
}

#[test]
fn a_restore_between_unlike_worlds_is_refused_by_name() {
    let Some(host) = device("branch unlike") else {
        return;
    };
    let mut sand = World::create(&host, &made("sand", &[], false)).unwrap();
    let params = params_of("heat", 3);
    let mut heat = World::create(&host, &made("heat", &params, false)).unwrap();
    let mut sparse = World::create(&host, &made("sand", &[], true)).unwrap();
    let mut wider = World::create(
        &host,
        &Made {
            size: [64, 32, 32],
            ..made("sand", &[], false)
        },
    )
    .unwrap();
    let before = hashed(&mut sand);
    for (from, what) in [
        (&mut heat, "rules"),
        (&mut sparse, "layouts"),
        (&mut wider, "sizes"),
    ] {
        let why = sand.restore(from).unwrap_err();
        assert_eq!(
            why.downcast_ref::<Refused>(),
            Some(&Refused::Unlike { what })
        );
    }
    assert_eq!(hashed(&mut sand), before);
}

#[test]
fn the_checked_in_files_open_on_the_device_and_step_as_the_cpu() {
    let Some(host) = device("branch fixtures") else {
        return;
    };
    for bytes in [DENSE_FIXTURE, SPARSE_FIXTURE] {
        let image = decoded(bytes).unwrap();
        let cells = image.unpacked();
        let mut world = World::open(&host, &image).unwrap();
        assert_eq!(world.cells().unwrap(), cells);
        let name = image.law.name().to_owned();
        world.step(12).unwrap();
        assert_eq!(
            world.cells().unwrap(),
            reference(&name, image.size, &cells, &image.params, image.frame, 12),
            "{name}"
        );
    }
}

#[test]
fn a_sparse_file_of_any_brick_opens_on_every_device_as_its_cells() {
    let Some(host) = device("branch bricks") else {
        return;
    };
    let image = decoded(SPARSE_FIXTURE).unwrap();
    let cells = image.unpacked();
    for sides in [
        [2, 2, 2],
        [4, 8, 2],
        [8, 8, 4],
        [16, 16, 8],
        [32, 32, 32],
        [32, 2, 16],
    ] {
        let bricks = format::lattice_of(image.size, sides)
            .iter()
            .product::<u32>();
        let state = format::bricked(&cells, image.size, sides, Some(bricks));
        let rebricked = Image {
            state,
            ..image.clone()
        };
        let back = decoded(&format::encoded(&rebricked)).unwrap();
        assert_eq!(back.unpacked(), cells, "{sides:?}");
        let mut world = World::open(&host, &back).unwrap();
        assert_eq!(world.cells().unwrap(), cells, "{sides:?}");
        world.step(6).unwrap();
        assert_eq!(
            world.cells().unwrap(),
            reference("heat", image.size, &cells, &image.params, image.frame, 6),
            "{sides:?}"
        );
        let used = world.pool_use().unwrap().unwrap();
        assert!(!used.overflow && !used.lost, "{sides:?}");
    }
}

#[test]
fn an_opened_world_writes_back_the_image_it_was_opened_from() {
    let Some(host) = device("branch round trip") else {
        return;
    };
    let dense = decoded(DENSE_FIXTURE).unwrap();
    let mut world = World::open(&host, &dense).unwrap();
    assert_eq!(world.image().unwrap(), dense);
    let sparse = decoded(SPARSE_FIXTURE).unwrap();
    let cells = sparse.unpacked();
    let mut world = World::open(&host, &sparse).unwrap();
    let back = world.image().unwrap();
    assert_eq!(back.unpacked(), cells);
    let bricks = world.pool().unwrap();
    if matches!(sparse.state, State::Sparse { sides, .. } if sides == bricks.sides()) {
        assert_eq!(back, sparse);
    } else {
        let canonical = Image {
            state: format::bricked(&cells, sparse.size, bricks.sides(), Some(bricks.capacity())),
            ..sparse
        };
        assert_eq!(back, canonical);
    }
}

#[test]
fn a_world_saved_before_its_first_frame_and_across_the_frame_wrap_steps_on() {
    let Some(host) = device("branch edges") else {
        return;
    };
    let folder = folder("edges");
    for sparse in [false, true] {
        let mut empty = World::create(&host, &made("sand", &[], sparse)).unwrap();
        let path = folder.join(format!("empty-{sparse}.world"));
        empty.save(&path).unwrap();
        let mut loaded = World::load(&host, &path).unwrap();
        assert_eq!(loaded.frame(), 0);
        assert_eq!(hashed(&mut loaded), hashed(&mut empty));
        let image = decoded(&fs::read(&path).unwrap()).unwrap();
        if let State::Sparse { pool, table, .. } = &image.state {
            assert!(pool.is_empty());
            assert!(table.chunks(2).all(|entry| entry == [format::UNIFORM, 0]));
        }
    }
    let mut image = decoded(DENSE_FIXTURE).unwrap();
    image.frame = u32::MAX - 1;
    let cells = image.unpacked();
    let mut world = World::open(&host, &image).unwrap();
    world.step(2).unwrap();
    assert_eq!(world.frame(), 0);
    let path = folder.join("wrapped.world");
    world.save(&path).unwrap();
    let mut loaded = World::load(&host, &path).unwrap();
    world.step(3).unwrap();
    loaded.step(3).unwrap();
    let want = reference("sand", image.size, &cells, &[], u32::MAX - 1, 5);
    assert_eq!(world.cells().unwrap(), want);
    assert_eq!(loaded.cells().unwrap(), want);
    assert_eq!(loaded.frame(), 3);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_non_cube_world_saves_loads_and_forks_in_both_tiers() {
    let Some(host) = device("branch shapes") else {
        return;
    };
    let folder = folder("shapes");
    for sparse in [false, true] {
        let size = [64, 16, 32];
        let mut world = World::create(
            &host,
            &Made {
                size,
                ..made("sand", &[], sparse)
            },
        )
        .unwrap();
        world.write(&scattered("sand", 0x5AD, 500, size)).unwrap();
        world.step(2).unwrap();
        let path = folder.join(format!("shape-{sparse}.world"));
        world.save(&path).unwrap();
        let mut loaded = World::load(&host, &path).unwrap();
        let mut fork = world.fork().unwrap();
        for one in [&mut world, &mut loaded, &mut fork] {
            one.step(30).unwrap();
        }
        let want = hashed(&mut world);
        assert_eq!(hashed(&mut loaded), want);
        assert_eq!(hashed(&mut fork), want);
    }
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_file_naming_an_unknown_rule_or_the_wrong_parameters_is_refused_on_open() {
    let Some(host) = device("branch names") else {
        return;
    };
    let mut image = decoded(SPARSE_FIXTURE).unwrap();
    image.law = Law::Rule {
        name: "lava".into(),
    };
    let why = World::open(&host, &image).err().unwrap();
    assert_eq!(
        why.downcast_ref::<Refused>(),
        Some(&Refused::Rule {
            name: "lava".into()
        })
    );
    let mut image = decoded(SPARSE_FIXTURE).unwrap();
    image.params.push(1);
    let back = decoded(&format::encoded(&image)).unwrap();
    let why = World::open(&host, &back).err().unwrap();
    assert_eq!(
        why.downcast_ref::<Refused>(),
        Some(&Refused::Params {
            rule: "heat".into(),
            given: 4,
            wanted: 3
        })
    );
}

#[test]
fn a_pool_that_does_not_fit_the_device_is_refused_by_name() {
    let Some(host) = device("branch unfit") else {
        return;
    };
    let size = [2048, 1024, 2048];
    let sides = [16, 16, 8];
    let bricks = format::lattice_of(size, sides).iter().product::<u32>();
    let image = Image {
        law: Law::Rule {
            name: "sand".into(),
        },
        params: Vec::new(),
        frame: 0,
        size,
        state: State::Sparse {
            capacity: bricks,
            sides,
            table: [format::UNIFORM, 0].repeat(bricks as usize),
            pool: Vec::new(),
        },
    };
    let back = decoded(&format::encoded(&image)).unwrap();
    let why = World::open(&host, &back).err().unwrap();
    assert_eq!(refused(why.as_ref()), "unfit", "{why}");
    println!("{why}");
}

const TWO_PLANES_FIXTURE: &[u8] = include_bytes!("branch/two-planes.world");

fn heat_image(size: [u32; 3], state: State) -> Image {
    Image {
        law: Law::Rule {
            name: "heat".into(),
        },
        params: params_of("heat", 3),
        frame: 0,
        size,
        state,
    }
}

fn two_plane_image(size: [u32; 3], state: State) -> Image {
    let fixture = decoded(TWO_PLANES_FIXTURE).unwrap();
    Image {
        size,
        state,
        frame: 0,
        ..fixture
    }
}

fn past_64_bits(file: &[u8]) {
    let why = decoded(file).unwrap_err();
    assert_eq!(why.name(), "malformed", "{why}");
    assert!(why.to_string().contains("64 bits"), "{why}");
}

#[test]
fn a_sparse_header_whose_cells_pass_64_bits_is_refused_by_name() {
    let big = 1_u32 << 31;
    let file = format::encoded(&heat_image(
        [big; 3],
        State::Sparse {
            capacity: 1,
            sides: [big; 3],
            table: vec![format::UNIFORM, 7],
            pool: Vec::new(),
        },
    ));
    past_64_bits(&file);
    for sides in [[2, 2, 2], [big, big, 2], [big, 2, big], [2, big, big]] {
        let image = heat_image(
            [big; 3],
            State::Sparse {
                capacity: 1,
                sides,
                table: vec![format::UNIFORM, 7],
                pool: Vec::new(),
            },
        );
        assert_eq!(image.cells(), None);
        past_64_bits(&format::encoded(&image));
    }
    let edge = [big, big, 2];
    let uniform = heat_image(
        edge,
        State::Sparse {
            capacity: 1,
            sides: edge,
            table: vec![format::UNIFORM, 7],
            pool: Vec::new(),
        },
    );
    assert_eq!(uniform.cells(), Some(1 << 63));
    assert_eq!(decoded(&format::encoded(&uniform)), Ok(uniform));
    let held = heat_image(
        edge,
        State::Sparse {
            capacity: 1,
            sides: edge,
            table: vec![0, 0],
            pool: Vec::new(),
        },
    );
    assert_eq!(
        decoded(&format::encoded(&held)),
        Err(Refused::Sizes {
            what: "pool",
            header: 1 << 63,
            found: 0
        })
    );
    let planes = two_plane_image(
        edge,
        State::Sparse {
            capacity: 1,
            sides: edge,
            table: vec![format::UNIFORM, 7, 9],
            pool: Vec::new(),
        },
    );
    assert_eq!(planes.channels(), 2);
    past_64_bits(&format::encoded(&planes));
}

#[test]
fn a_dense_header_of_two_to_the_64_cells_is_refused_by_name() {
    let big = 1_u32 << 31;
    let wrapping = heat_image([big, big, 4], State::Dense(Vec::new()));
    assert_eq!(wrapping.cells(), None);
    past_64_bits(&format::encoded(&wrapping));
    let edge = heat_image([big, big, 2], State::Dense(Vec::new()));
    assert_eq!(edge.cells(), Some(1 << 63));
    assert_eq!(
        decoded(&format::encoded(&edge)),
        Err(Refused::Sizes {
            what: "dense",
            header: 1 << 63,
            found: 0
        })
    );
    let planes = two_plane_image([big, big, 2], State::Dense(Vec::new()));
    let mut file = format::encoded(&planes);
    let channels = 64 + 4 * planes.params.len();
    assert_eq!(word_at(&file, channels), 1);
    put_word(&mut file, channels, 2);
    past_64_bits(&resealed(file));
}

#[test]
fn stale_temporary_files_never_block_a_save() {
    let folder = folder("stale");
    let path = folder.join("w.world");
    let probe = format::temporary(&path);
    let name = probe.file_name().unwrap().to_string_lossy().into_owned();
    let parts: Vec<&str> = name.split('.').collect();
    let pid = parts[parts.len() - 3];
    let count: u64 = parts[parts.len() - 2].parse().unwrap();
    assert_eq!(pid, std::process::id().to_string());
    let stale = 64;
    for at in count + 1..=count + stale {
        fs::write(
            folder.join(format!(".w.world.{pid}.{at}.tmp")),
            b"left by a crashed process with this pid",
        )
        .unwrap();
    }
    let image = decoded(DENSE_FIXTURE).unwrap();
    for _ in 0..5 {
        format::store(&path, &image).unwrap();
        assert_eq!(format::fetch(&path).unwrap(), image);
    }
    assert_eq!(leftovers(&folder).len(), stale as usize + 1);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_file_past_what_the_engine_holds_opens_as_unfit() {
    let Some(host) = device("branch past the engine") else {
        return;
    };
    let big = 1_u32 << 31;
    for size in [[big, big, 2], [1 << 16, 1 << 16, 1 << 16], [2, 2, big]] {
        let image = heat_image(
            size,
            State::Sparse {
                capacity: 1,
                sides: size,
                table: vec![format::UNIFORM, 7],
                pool: Vec::new(),
            },
        );
        let image = decoded(&format::encoded(&image)).unwrap();
        let why = World::open(&host, &image).err().unwrap();
        assert_eq!(refused(why.as_ref()), "unfit", "{size:?}: {why}");
    }
}
