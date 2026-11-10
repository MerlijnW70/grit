#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::compact::Plan;
use grit::layout::{BINDINGS, Dimension, PARAMS, PIXELS, STATE};
use grit::live;
use grit::rule3::code::{self, Code};
use grit::rule3::heat::{self, Heat};
use grit::rule3::sand::{self, Sand};
use grit::rule3::{FIRST_PARAM, Rule};
use grit::sand::{Grid, View};
use grit::sand3::{self, Grid3};
use grit::serve3;
use grit::sweep;
use grit::view3::{
    self, Axis, Kinds, Look, PALETTE, Picture, SHOWN, cell_under, choosing, extent, palette_at,
    pictures, plane, shaded,
};
use grit::{Groups, Host, Resident};
use simdr::lanes::LaneError;

type KindOf = fn(u32) -> u32;

const RULES: [&str; 3] = ["sand", "heat", "code"];

const VIEWS: [(u32, u32); 4] = [(64, 64), (16, 4), (4, 2), (2, 1)];

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    next(state) as u32
}

fn kinds(rule: &str) -> Kinds {
    Kinds::named(rule).unwrap_or_else(|| panic!("no kinds for {rule}"))
}

/// Random words of which about three in four have kind 0, so a depth view
/// walks past several cells before it hits.
fn sparse(cells: usize, kinds: Kinds, seed: u64) -> Vec<u32> {
    let mut state = seed;
    let field = kinds.mask() << kinds.shift();
    (0..cells)
        .map(|_| {
            let held = word(&mut state);
            if next(&mut state).is_multiple_of(4) {
                held
            } else {
                held & !field
            }
        })
        .collect()
}

fn palette() -> [u32; PALETTE] {
    let mut state = 0xC0_10E5;
    std::array::from_fn(|_| word(&mut state) | 0xFF00_0000)
}

#[test]
fn every_rules_kinds_are_its_mask_and_shift_on_random_words() {
    let named: [(&str, KindOf); 3] = [
        ("sand", sand::material_of),
        ("heat", heat::kind_of),
        ("code", code::instruction_of),
    ];
    let mut state = 0x5EED;
    for (rule, kind_of) in named {
        let held = kinds(rule);
        for _ in 0..200_000 {
            let one = word(&mut state);
            assert_eq!(held.of(one), kind_of(one), "{rule} word {one:#010x}");
            assert!((held.of(one) as usize) < PALETTE, "{rule} word {one:#010x}");
        }
        for one in [0, u32::MAX, 0x8000_0000, 0x0F00_0000, 0xF] {
            assert_eq!(held.of(one), kind_of(one), "{rule} word {one:#010x}");
        }
    }
    let listed = serve3::rules();
    assert!(!listed.is_empty());
    for rule in &listed {
        let held = Kinds::named(rule.name())
            .unwrap_or_else(|| panic!("the world tools list {}, which has no kinds", rule.name()));
        for _ in 0..200_000 {
            let one = word(&mut state);
            assert_eq!(held.of(one), rule.kind(one), "{} {one:#010x}", rule.name());
        }
    }
    assert_eq!(Kinds::named("pond"), None);
}

#[test]
fn a_mapping_past_the_palette_or_the_word_is_refused() {
    assert_eq!(Kinds::new(16, 0), None);
    assert_eq!(Kinds::new(u32::MAX, 0), None);
    assert_eq!(Kinds::new(0xF, 32), None);
    let edge = Kinds::new(0xF, 31).unwrap();
    assert_eq!(edge.of(u32::MAX), 1);
    assert_eq!(Kinds::new(0, 0).unwrap().of(u32::MAX), 0);
}

#[test]
fn a_picture_refuses_a_flat_grid_and_an_unknown_width() {
    let flat = Grid::of(16, 16);
    let solid = Grid::solid(16, 8, 4).unwrap();
    let view = View::of(8, 8);
    let held = kinds("heat");
    for look in Look::ALL {
        match view3::picture(32, flat, view, Axis::Z, look, held) {
            Err(LaneError::NoSuchForm { because, .. }) => assert_eq!(because, view3::FLAT),
            other => panic!("a flat grid gave {other:?}"),
        }
        assert!(matches!(
            view3::picture(12, solid, view, Axis::Z, look, held),
            Err(LaneError::NoMapping { lanes: 12, .. })
        ));
    }
    match view3::picture(
        64,
        solid,
        View::of(1 << 16, 1 << 16),
        Axis::Z,
        Look::Slice,
        held,
    ) {
        Err(LaneError::NoSuchForm { because, .. }) => assert_eq!(because, view3::TOO_LARGE),
        other => panic!("a view past a u32 gave {other:?}"),
    }
}

#[test]
fn the_cpu_picture_scales_by_the_nearest_cell_and_wraps_the_layer() {
    let grid = Grid::solid(32, 16, 8).unwrap();
    assert_eq!(plane(grid, Axis::X), [8, 16]);
    assert_eq!(plane(grid, Axis::Y), [32, 8]);
    assert_eq!(plane(grid, Axis::Z), [32, 16]);
    let big = View::of(64, 64);
    assert_eq!(cell_under(grid, big, Axis::Z, 5, (63, 63)), [31, 15, 5]);
    assert_eq!(cell_under(grid, big, Axis::Z, 5, (2, 4)), [1, 1, 5]);
    assert_eq!(cell_under(grid, big, Axis::X, 7, (63, 0)), [7, 0, 7]);
    assert_eq!(cell_under(grid, big, Axis::Y, 3, (0, 63)), [0, 3, 7]);
    let small = View::of(4, 2);
    assert_eq!(cell_under(grid, small, Axis::Z, 0, (3, 1)), [24, 8, 0]);
    assert_eq!(cell_under(grid, small, Axis::Z, 8 + 2, (0, 0)), [0, 0, 2]);
    assert_eq!(
        cell_under(grid, small, Axis::Z, u32::MAX, (0, 0)),
        [0, 0, 7]
    );
    assert_eq!(shaded(0xFF12_3456, 0, 4), 0x0012_3456);
    assert_eq!(shaded(0x00FF_FFFF, 15, 4), 0x0087_8787);
    assert_eq!(shaded(0x00FF_FFFF, 8, 4), 0x00BF_BFBF);
}

fn bare_sizes(grid: Grid, width: u32, view: View) -> [usize; BINDINGS as usize] {
    let mut sizes = [1_usize; BINDINGS as usize];
    sizes[PARAMS as usize] = Dimension::Three.params_words();
    sizes[STATE as usize] = grid.cells();
    view3::showing(sizes, width, view)
}

/// Runs the picture of `axis` and `look` at `layer` and returns what it
/// painted and what it left of the palette.
fn shown(
    held: &mut Resident<'_>,
    width: u32,
    view: View,
    (axis, look): (Axis, Look),
    layer: u32,
    before: &[Groups],
) -> (Vec<u32>, Vec<u32>) {
    held.write_at(PARAMS as usize, axis.layer_word(), &[layer])
        .unwrap();
    let mut stages: Vec<&[Groups]> = Vec::new();
    let chosen = choosing(width, view, axis, look);
    if !before.is_empty() {
        stages.push(before);
    }
    stages.push(&chosen);
    let (_, all) = held
        .staged_reading(&stages, PIXELS as usize, palette_at(view) + PALETTE)
        .unwrap();
    let (picture, kept) = all.split_at(view.pixels());
    (picture.to_vec(), kept.to_vec())
}

fn first_difference(got: &[u32], want: &[u32]) -> Option<String> {
    if got.len() != want.len() {
        return Some(format!("{} pixels, {} wanted", got.len(), want.len()));
    }
    let at = got.iter().zip(want).position(|(a, b)| a != b)?;
    let wrong = got.iter().zip(want).filter(|(a, b)| a != b).count();
    Some(format!(
        "pixel {at}: {:#010x}, {:#010x} wanted; {wrong} pixels differ",
        got[at], want[at]
    ))
}

#[test]
fn every_slice_and_depth_view_equals_the_cpu_picture() {
    let Some(host) = device("view3 pictures") else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(32, 16, 8).unwrap();
    let colours = palette();
    let mut wrong = Vec::new();
    let mut checked = 0;
    for (seed, rule) in RULES.into_iter().enumerate() {
        let held_kinds = kinds(rule);
        let cells = sparse(grid.cells(), held_kinds, 0xFACE + seed as u64);
        for (wide, deep) in VIEWS {
            let view = View::of(wide, deep);
            let passes = pictures(width, grid, view, held_kinds).unwrap();
            let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
            let sizes = bare_sizes(grid, width, view);
            let mut held = host.resident(&sizes, &modules).unwrap();
            for (binding, &words) in sizes.iter().enumerate() {
                held.write(binding, &vec![0; words]).unwrap();
            }
            held.write(STATE as usize, &cells).unwrap();
            held.write_at(PIXELS as usize, palette_at(view), &colours)
                .unwrap();
            for (axis, look) in SHOWN {
                let length = extent(grid, axis);
                for layer in [0, length / 2, length - 1, length + 3, u32::MAX] {
                    let (got, kept) = shown(&mut held, width, view, (axis, look), layer, &[]);
                    let picture = Picture {
                        grid,
                        view,
                        axis,
                        look,
                        kinds: held_kinds,
                    };
                    let want = picture.painted(layer, &colours, &cells);
                    let label =
                        format!("{rule} axis {axis:?} {look:?} view {wide}x{deep} layer {layer}");
                    if let Some(why) = first_difference(&got, &want) {
                        wrong.push(format!("{label}: {why}"));
                    }
                    if kept != colours {
                        wrong.push(format!("{label}: the palette was overwritten"));
                    }
                    checked += 1;
                }
            }
        }
    }
    println!(
        "width {width}: {checked} pictures, {} differ from the CPU",
        wrong.len()
    );
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_lone_cell_at_the_far_end_of_every_axis_is_found_and_shaded() {
    let Some(host) = device("view3 lone cell") else {
        return;
    };
    let width = host.width();
    let grid = Grid::solid(16, 8, 4).unwrap();
    let reference = Grid3::new(16, 8, 4).unwrap();
    let held_kinds = kinds("code");
    let colours = palette();
    let view = View::of(16, 16);
    let passes = pictures(width, grid, view, held_kinds).unwrap();
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let sizes = bare_sizes(grid, width, view);
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0; words]).unwrap();
    }
    held.write_at(PIXELS as usize, palette_at(view), &colours)
        .unwrap();
    let lone = code::word(code::MOVE, code::PLUS_X, 0x00AB_CDEF);
    let far = reference.index(15, 7, 3);
    let mut cells = vec![code::word(code::NOP, code::MINUS_Z, 0x0012_3456); reference.cells()];
    cells[far] = lone;
    held.write(STATE as usize, &cells).unwrap();
    let background = colours[0] & 0x00FF_FFFF;
    for axis in Axis::ALL {
        let length = extent(grid, axis);
        for layer in [0, length - 1] {
            let (got, _) = shown(&mut held, width, view, (axis, Look::Depth), layer, &[]);
            let hits: Vec<usize> = (0..got.len()).filter(|&at| got[at] != background).collect();
            let [across, down] = plane(grid, axis);
            let pixels = (view.wide / across.min(view.wide)) * (view.deep / down.min(view.deep));
            assert_eq!(hits.len() as u32, pixels, "{axis:?} layer {layer}");
            let depth = length - 1 - layer;
            let want = shaded(colours[code::MOVE as usize], depth, length.trailing_zeros());
            for at in hits {
                assert_eq!(got[at], want, "{axis:?} layer {layer} pixel {at}");
            }
            let (slice, _) = shown(&mut held, width, view, (axis, Look::Slice), layer, &[]);
            let lit = slice
                .iter()
                .filter(|&&one| one == colours[code::MOVE as usize])
                .count() as u32;
            assert_eq!(lit, if layer == length - 1 { pixels } else { 0 });
        }
    }
}

struct Ran {
    name: &'static str,
    params: Vec<u32>,
}

fn plan_of(width: u32, grid: Grid) -> Plan {
    Plan::over_in(
        width,
        grid.blocks().div_ceil(sweep::cells(width)),
        Dimension::Three,
    )
    .unwrap()
}

fn running<R: Rule>(host: &Host, rule: &R, ran: &Ran) -> Vec<String> {
    let width = host.width();
    let (wide, deep, tall) = (16, 16, 16);
    let grid = Grid::solid(wide, deep, tall).unwrap();
    let reference = Grid3::new(wide, deep, tall).unwrap();
    let plan = plan_of(width, grid);
    let held_kinds = kinds(ran.name);
    let view = View::of(32, 64);
    let (mut passes, stages) = live::wired3_by(rule, width, grid, &plan).unwrap();
    passes.extend(pictures(width, grid, view, held_kinds).unwrap());
    let sizes = view3::showing(live::sizes3(rule, &plan), width, view);
    let modules: Vec<&[u32]> = passes.iter().map(Vec::as_slice).collect();
    let mut held = host.resident(&sizes, &modules).unwrap();
    for (binding, &words) in sizes.iter().enumerate() {
        held.write(binding, &vec![0; words]).unwrap();
    }
    let mut cells = sparse(reference.cells(), held_kinds, 0xBEE5);
    held.write(STATE as usize, &cells).unwrap();
    if !ran.params.is_empty() {
        held.write_at(PARAMS as usize, FIRST_PARAM, &ran.params)
            .unwrap();
    }
    held.write(PARAMS as usize, &[u32::MAX, plan.dump()])
        .unwrap();
    live::seed(&mut held, grid, &plan).unwrap();
    let colours = palette();
    held.write_at(PIXELS as usize, palette_at(view), &colours)
        .unwrap();

    let mut wrong = Vec::new();
    for frame in 0..12_u32 {
        let (axis, look) = SHOWN[frame as usize % SHOWN.len()];
        let layer = [3, 8, 15][frame as usize % 3];
        held.write_at(PARAMS as usize, axis.layer_word(), &[layer])
            .unwrap();
        let mut groups: Vec<&[Groups]> = stages.iter().map(Vec::as_slice).collect();
        let chosen = choosing(width, view, axis, look);
        groups.push(&chosen);
        let (_, got) = held
            .staged_reading(&groups, PIXELS as usize, view.pixels())
            .unwrap();
        cells = sand3::reference(rule, reference, frame, &cells, &ran.params);
        let picture = Picture {
            grid,
            view,
            axis,
            look,
            kinds: held_kinds,
        };
        let want = picture.painted(layer, &colours, &cells);
        if let Some(why) = first_difference(&got, &want) {
            wrong.push(format!(
                "{} frame {frame} axis {axis:?} {look:?} layer {layer}: {why}",
                ran.name
            ));
        }
    }
    let state = held.read(STATE as usize, reference.cells()).unwrap();
    if state != cells {
        wrong.push(format!("{}: the world left the reference", ran.name));
    }
    wrong
}

#[test]
fn the_picture_after_each_frame_of_a_running_world_equals_the_cpu_picture_under_every_rule() {
    let Some(host) = device("view3 running world") else {
        return;
    };
    let mut wrong = running(
        &host,
        &Sand,
        &Ran {
            name: "sand",
            params: Vec::new(),
        },
    );
    wrong.extend(running(
        &host,
        &Heat,
        &Ran {
            name: "heat",
            params: heat::TUNED.to_vec(),
        },
    ));
    wrong.extend(running(
        &host,
        &Code,
        &Ran {
            name: "code",
            params: Vec::new(),
        },
    ));
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
