#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::sand::{self, Grid, WALL};
use grit::sand3::{self, Grid3, running_of, settle_reference3};

const SIZES: [(u32, u32); 6] = [(2, 2), (8, 8), (16, 8), (8, 32), (32, 32), (64, 16)];

const SEEDS: [u64; 4] = [3, 11, 0xF1A7, 0x0DD5_EED5];

const FRAMES: u32 = 40;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn flat(grid: Grid, seed: u64) -> Vec<u32> {
    let mut state = seed;
    (0..grid.cells())
        .map(|_| (next(&mut state) % 256) as u32)
        .collect()
}

fn widened(word: u32) -> u32 {
    assert!(word < 256);
    word % 16 + ((word >> 4) << 5)
}

fn narrowed(word: u32) -> u32 {
    assert!(running_of(word) < 4, "a z runner in a flat world: {word}");
    assert!(word < 512);
    word % 16 + ((word >> 5) << 4)
}

fn layer(space: Grid3, cells: &[u32], z: u32) -> Vec<u32> {
    let start = space.index(0, 0, z);
    cells[start..start + (space.wide * space.deep) as usize].to_vec()
}

fn agree(beside: impl Fn(u32) -> u32, both: bool) {
    for (wide, deep) in SIZES {
        let grid = Grid::of(wide, deep);
        let space = Grid3::new(wide, deep, 2).unwrap();
        for seed in SEEDS {
            let mut plane = flat(grid, seed);
            let mut cells: Vec<u32> = plane.iter().map(|&word| widened(word)).collect();
            cells.extend(plane.iter().map(|&word| beside(widened(word))));

            for frame in 0..FRAMES {
                plane = sand::settle_reference(grid, frame, &plane);
                cells = settle_reference3(space, frame, &cells);
                let front: Vec<u32> = layer(space, &cells, 0).into_iter().map(narrowed).collect();
                assert_eq!(front, plane, "{wide}x{deep} seed {seed} frame {frame}");
                let back = layer(space, &cells, 1);
                let want: Vec<u32> = plane.iter().map(|&word| beside(widened(word))).collect();
                if both {
                    assert_eq!(back, want, "{wide}x{deep} seed {seed} frame {frame} back");
                } else {
                    assert!(back.iter().all(|&word| word == WALL));
                }
            }
        }
    }
}

#[test]
fn one_layer_between_walls_steps_as_the_2d_reference_frame_by_frame() {
    agree(|_| WALL, false);
}

#[test]
fn two_equal_layers_step_as_the_2d_reference_frame_by_frame() {
    agree(|word| word, true);
}

#[test]
fn the_flat_word_maps_into_the_3d_word_field_by_field() {
    for word in 0..256 {
        let wide = widened(word);
        assert_eq!(sand3::material_of(wide), sand::material_of(word));
        assert_eq!(sand3::running_of(wide), sand::running_of(word));
        assert_eq!(sand3::held_of(wide), sand::held_of(word));
        assert_eq!(sand3::tired_of(wide), sand::tired_of(word));
        assert_eq!(narrowed(wide), word);
        for under in 0..256 {
            assert_eq!(
                sand3::hold_of(wide, widened(under)),
                widened(sand::hold_of(word, under))
            );
        }
    }
    assert_eq!(widened(sand::RUNNING_LEFT), sand3::RUNNING_MINUS_X);
    assert_eq!(widened(sand::RUNNING_RIGHT), sand3::RUNNING_PLUS_X);
}
