#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::rule3::evo::{self, Evo};
use grit::rule3::{CORNERS, Corner, Declared, Oct, Rule};

const OCTS: u32 = 100_000;

fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut mixed = *state;
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    mixed ^ (mixed >> 31)
}

fn word(state: &mut u64) -> u32 {
    (next(state) >> 16) as u32
}

fn living(state: &mut u64) -> u32 {
    let roll = next(state);
    let genome = (roll >> 8) as u32 & evo::GENOME_MASK;
    let energy = match roll % 4 {
        0 => (roll >> 30) as u32 % 8,
        _ => (roll >> 30) as u32 & evo::FULL,
    };
    let age = match (roll >> 40) % 4 {
        0 => evo::lifespan_of(genome) - (roll >> 44) as u32 % 2,
        _ => (roll >> 44) as u32 & evo::OLDEST,
    };
    let spare = ((roll >> 52) as u32 & 1) << 15;
    evo::cell(genome, energy, age) | spare
}

fn lifeless(state: &mut u64) -> u32 {
    match next(state) % 3 {
        0 => 0,
        _ => word(state) & !evo::ALIVE,
    }
}

fn mixed_oct(state: &mut u64) -> [u32; CORNERS] {
    let share = next(state) % 9;
    core::array::from_fn(|_| {
        if next(state) % 8 < share {
            living(state)
        } else {
            lifeless(state)
        }
    })
}

fn params(state: &mut u64) -> Vec<u32> {
    let mut held = evo::DEFAULTS.to_vec();
    for slot in &mut held {
        *slot = match next(state) % 6 {
            0 => *slot,
            1 => 0,
            2 => u32::MAX,
            3 => word(state),
            _ => word(state) % 1024,
        };
    }
    held
}

fn stepped(
    words: [u32; CORNERS],
    frame: u32,
    at: [u32; 3],
    params: &[u32],
) -> (Vec<u32>, Vec<u32>) {
    let made = Evo.step(Oct {
        words,
        frame,
        at,
        params,
    });
    (made.out.to_vec(), made.counts)
}

fn alive_count(words: &[u32]) -> u32 {
    words.iter().filter(|&&one| evo::alive_of(one)).count() as u32
}

#[test]
fn the_cell_word_holds_its_fields() {
    let mut state = 0xCE11;
    for _ in 0..10_000 {
        let genome = word(&mut state) & evo::GENOME_MASK;
        let energy = word(&mut state) & evo::FULL;
        let age = word(&mut state) & evo::OLDEST;
        let made = evo::cell(genome, energy, age);
        assert!(evo::alive_of(made));
        assert_eq!(evo::kind_of(made), 1);
        assert_eq!(evo::genome_of(made), genome);
        assert_eq!(evo::energy_of(made), energy);
        assert_eq!(evo::age_of(made), age);
        assert_eq!(made & (1 << 15), 0);
    }
    let made = evo::genome(3, 2, 1, 0x3FF);
    assert_eq!(evo::gene(made, evo::THRESHOLD_GENE), 3);
    assert_eq!(evo::gene(made, evo::LIFESPAN_GENE), 2);
    assert_eq!(evo::gene(made, evo::GUARD_GENE), 1);
    assert_eq!(made >> evo::MARKER_GENE, 0x3FF);
    assert_eq!(evo::kind_of(0), 0);
    assert_eq!(evo::kind_of(0xFFFF_FFFE), 0);
}

#[test]
fn an_oct_without_life_is_unchanged_at_every_frame() {
    let mut state = 0x9_01E7;
    for _ in 0..OCTS {
        let words: [u32; CORNERS] = core::array::from_fn(|_| lifeless(&mut state));
        let held = params(&mut state);
        let at = [word(&mut state), word(&mut state), word(&mut state)];
        for frame in [word(&mut state), 0, 1, 2, 3, u32::MAX] {
            let (out, counts) = stepped(words, frame, at, &held);
            assert_eq!(out, words, "frame {frame} at {at:?} params {held:?}");
            assert_eq!(counts, vec![0; evo::COUNTS]);
        }
    }
}

#[test]
fn an_oct_with_life_changes_at_every_frame() {
    let mut state = 0x11FE;
    let mut tried = 0;
    while tried < OCTS {
        let words = mixed_oct(&mut state);
        if alive_count(&words) == 0 {
            continue;
        }
        tried += 1;
        let held = params(&mut state);
        let at = [word(&mut state), word(&mut state), word(&mut state)];
        for frame in [word(&mut state), word(&mut state)] {
            let (out, counts) = stepped(words, frame, at, &held);
            assert_ne!(out, words, "frame {frame} at {at:?} params {held:?}");
            assert!(counts[evo::CHANGED] > 0);
        }
    }
}

#[test]
fn the_tally_counts_what_the_step_did() {
    let mut state = 0x7A11;
    let mut totals = [0_u64; evo::COUNTS];
    for _ in 0..OCTS {
        let words = mixed_oct(&mut state);
        let held = params(&mut state);
        let at = [word(&mut state), word(&mut state), word(&mut state)];
        let (out, counts) = stepped(words, word(&mut state), at, &held);
        let changed = out.iter().zip(&words).filter(|(a, b)| a != b).count() as u32;
        let born = out
            .iter()
            .filter(|&&one| evo::alive_of(one) && evo::age_of(one) == 0)
            .count() as u32;
        assert_eq!(counts[evo::CHANGED], changed);
        assert_eq!(counts[evo::LIVING], alive_count(&out));
        assert_eq!(counts[evo::BIRTHS], born);
        assert_eq!(
            alive_count(&words) + counts[evo::BIRTHS],
            counts[evo::LIVING] + counts[evo::DEATHS],
            "{words:x?} -> {out:x?}"
        );
        assert!(counts[evo::MUTATIONS] <= evo::GENOME_BITS * counts[evo::BIRTHS]);
        for (total, &count) in totals.iter_mut().zip(&counts) {
            *total += u64::from(count);
        }
    }
    println!("totals over {OCTS} octs: {totals:?}");
    assert!(totals.iter().all(|&total| total > 0));
}

fn parents_of(words: &[u32; CORNERS], corner: Corner) -> Vec<u32> {
    [
        grit::rule3::Axis::X,
        grit::rule3::Axis::Y,
        grit::rule3::Axis::Z,
    ]
    .into_iter()
    .map(|axis| words[corner.neighbour(axis).index()])
    .filter(|&one| evo::alive_of(one))
    .map(evo::genome_of)
    .collect()
}

fn inherited(rate: u32, made: fn(u32) -> u32) -> u64 {
    let mut state = 0x1E1F ^ u64::from(rate);
    let mut births = 0;
    for _ in 0..OCTS {
        let words = mixed_oct(&mut state);
        let mut held = params(&mut state);
        held[evo::MUTATION] = rate;
        let at = [word(&mut state), word(&mut state), word(&mut state)];
        let (out, counts) = stepped(words, word(&mut state), at, &held);
        for corner in Corner::ALL {
            let one = out[corner.index()];
            if !evo::alive_of(one) || evo::age_of(one) != 0 {
                continue;
            }
            let parents = parents_of(&words, corner);
            let child = evo::genome_of(one);
            assert!(
                parents.iter().any(|&parent| made(parent) == child),
                "child {child:#x} at {corner:?} of {words:x?}, parents {parents:x?}"
            );
            assert!(evo::energy_of(one) >= evo::THRESHOLD_LEAST / 2);
            assert_eq!(evo::age_of(one), 0);
        }
        let flips = if rate == 0 { 0 } else { evo::GENOME_BITS };
        assert_eq!(counts[evo::MUTATIONS], flips * counts[evo::BIRTHS]);
        births += u64::from(counts[evo::BIRTHS]);
    }
    births
}

#[test]
fn with_mutation_rate_zero_every_child_carries_its_parents_genome() {
    let births = inherited(0, |genome| genome);
    println!("{births} births, every genome inherited whole");
    assert!(births > 10_000);
}

#[test]
fn with_every_bit_flipped_every_child_carries_its_parents_complement() {
    let births = inherited(evo::CHANCES, |genome| !genome & evo::GENOME_MASK);
    println!("{births} births, every genome complemented");
    assert!(births > 10_000);
}

#[test]
fn mutation_flips_each_bit_at_its_rate_within_the_binomial_interval() {
    let mut state = 0xB17;
    for rate in [1_u32, 64, 1024, 16_384, 49_152] {
        let draws: u64 = 200_000;
        let mut flipped = 0_u64;
        let mut per_bit = [0_u64; evo::GENOME_BITS as usize];
        for _ in 0..draws {
            let base = word(&mut state);
            let corner = (next(&mut state) % 8) as usize;
            let genome = word(&mut state) & evo::GENOME_MASK;
            let mask = evo::flips_of(base, corner, genome, rate);
            assert_eq!(mask & !evo::GENOME_MASK, 0);
            flipped += u64::from(mask.count_ones());
            for (bit, count) in per_bit.iter_mut().enumerate() {
                *count += u64::from((mask >> bit) & 1);
            }
        }
        let p = f64::from(rate) / f64::from(evo::CHANCES);
        let trials = (draws * u64::from(evo::GENOME_BITS)) as f64;
        let mean = trials * p;
        let spread = 5.0 * (trials * p * (1.0 - p)).sqrt() + 1.0;
        println!(
            "rate {rate}/65536: {flipped} flips in {trials} bit draws, expected {mean:.1} +- {spread:.1}"
        );
        assert!((flipped as f64 - mean).abs() <= spread, "rate {rate}");
        let bit_trials = draws as f64;
        let bit_spread = 5.0 * (bit_trials * p * (1.0 - p)).sqrt() + 1.0;
        for (bit, &count) in per_bit.iter().enumerate() {
            assert!(
                (count as f64 - bit_trials * p).abs() <= bit_spread,
                "rate {rate} bit {bit}: {count}"
            );
        }
    }
}

#[test]
fn mutation_flips_every_two_bits_independently_within_the_binomial_interval() {
    let bits = evo::GENOME_BITS as usize;
    let mut state = 0x9A12;
    for rate in [1024_u32, 16_384, 49_152] {
        let draws: u64 = 200_000;
        let mut both = vec![0_u64; bits * bits];
        for _ in 0..draws {
            let base = word(&mut state);
            let corner = (next(&mut state) % 8) as usize;
            let genome = word(&mut state) & evo::GENOME_MASK;
            let mask = evo::flips_of(base, corner, genome, rate);
            for one in 0..bits {
                if (mask >> one) & 1 == 0 {
                    continue;
                }
                for other in one + 1..bits {
                    both[one * bits + other] += u64::from((mask >> other) & 1);
                }
            }
        }
        let p = f64::from(rate) / f64::from(evo::CHANCES);
        let pp = p * p;
        let mean = draws as f64 * pp;
        let spread = 5.0 * (draws as f64 * pp * (1.0 - pp)).sqrt() + 1.0;
        let mut worst = 0.0_f64;
        for one in 0..bits {
            for other in one + 1..bits {
                let count = both[one * bits + other] as f64;
                worst = worst.max((count - mean).abs() / spread);
                assert!(
                    (count - mean).abs() <= spread,
                    "rate {rate} bits {one} and {other}: both flipped {count}, expected {mean:.1} +- {spread:.1}"
                );
            }
        }
        println!(
            "rate {rate}/65536: each of the 120 pairs of bits flipped together within {mean:.1} +- {spread:.1} in {draws} draws; worst pair at {worst:.2} of the interval"
        );
    }
}

#[test]
fn the_mutation_hash_reads_the_point_the_frame_and_the_genome() {
    let rate = evo::CHANCES / 2;
    let base = evo::base_of([3, 4, 5], 6, 0);
    let mask = evo::flips_of(base, 2, 0x1234, rate);
    assert_eq!(evo::flips_of(base, 2, 0x1234, rate), mask);
    assert_ne!(evo::flips_of(base, 3, 0x1234, rate), mask);
    assert_ne!(evo::flips_of(base, 2, 0x1235, rate), mask);
    assert_ne!(
        evo::flips_of(evo::base_of([3, 4, 5], 7, 0), 2, 0x1234, rate),
        mask
    );
    assert_ne!(
        evo::flips_of(evo::base_of([3, 4, 6], 6, 0), 2, 0x1234, rate),
        mask
    );
    assert_ne!(
        evo::flips_of(evo::base_of([3, 4, 5], 6, 1), 2, 0x1234, rate),
        mask
    );
    assert_eq!(evo::flips_of(base, 2, 0x1234, 0), 0);
    assert_eq!(
        evo::flips_of(base, 2, 0x1234, evo::CHANCES),
        evo::GENOME_MASK
    );
}

fn calm() -> Vec<u32> {
    let mut held = evo::DEFAULTS.to_vec();
    held[evo::HAZARD] = 0;
    held
}

fn alone(genome: u32, energy: u32, age: u32, params: &[u32]) -> Vec<u32> {
    let mut words = [0; CORNERS];
    words[0] = evo::cell(genome, energy, age);
    stepped(words, 0, [0, 0, 0], params).0
}

#[test]
fn a_lower_copy_threshold_copies_sooner_and_gives_a_smaller_child() {
    let held = calm();
    let gain = held[evo::FEED] * 7;
    for threshold in 0..4 {
        let genome = evo::genome(threshold, 0, 0, 0);
        let cost = evo::upkeep_of(genome, &held);
        let at = evo::threshold_of(genome) + cost - gain;
        let below = alone(genome, at - 1, 1, &held);
        assert_eq!(
            alive_count(&below),
            1,
            "threshold {threshold}: copied below"
        );
        let copied = alone(genome, at, 1, &held);
        assert_eq!(
            alive_count(&copied),
            2,
            "threshold {threshold}: did not copy"
        );
        let child = copied
            .iter()
            .copied()
            .find(|&one| evo::alive_of(one) && evo::age_of(one) == 0)
            .unwrap();
        assert_eq!(evo::energy_of(child), evo::threshold_of(genome) / 2);
        assert_eq!(evo::genome_of(child), genome);
        let parent = copied[0];
        assert_eq!(
            evo::energy_of(parent),
            evo::threshold_of(genome) - evo::threshold_of(genome) / 2 - held[evo::BIRTH_COST]
        );
    }
    assert!(
        evo::threshold_of(evo::genome(0, 0, 0, 0)) < evo::threshold_of(evo::genome(3, 0, 0, 0))
    );
}

#[test]
fn a_longer_life_costs_upkeep_every_frame() {
    let held = calm();
    let mut last_span = 0;
    let mut last_cost = 0;
    for lifespan in 0..4 {
        let genome = evo::genome(3, lifespan, 0, 0);
        let span = evo::lifespan_of(genome);
        let cost = evo::upkeep_of(genome, &held);
        assert!(span > last_span && cost > last_cost, "lifespan {lifespan}");
        last_span = span;
        last_cost = cost;
        let kept = alone(genome, 20, span - 1, &held);
        assert_eq!(evo::age_of(kept[0]), span, "lifespan {lifespan}");
        assert_eq!(evo::energy_of(kept[0]), 20 + held[evo::FEED] * 7 - cost);
        let gone = alone(genome, 20, span, &held);
        assert_eq!(gone, vec![0; CORNERS], "lifespan {lifespan}");
    }
    assert!(evo::lifespan_of(evo::genome(0, 3, 0, 0)) < evo::OLDEST);
}

#[test]
fn a_guard_against_the_hazard_costs_upkeep_every_frame() {
    let mut held = calm();
    held[evo::HAZARD] = evo::CHANCES / 2;
    let mut state = 0x6A2D;
    let mut last_cost = 0;
    let mut last_deaths = u32::MAX;
    for guard in 0..4 {
        let genome = evo::genome(3, 0, guard, 0);
        let cost = evo::upkeep_of(genome, &held);
        assert!(cost > last_cost, "guard {guard}");
        last_cost = cost;
        let trials = 20_000;
        let mut deaths = 0;
        for _ in 0..trials {
            let mut words = [0; CORNERS];
            words[0] = evo::cell(genome, 100, 1);
            let at = [word(&mut state), word(&mut state), word(&mut state)];
            let (out, _) = stepped(words, word(&mut state), at, &held);
            deaths += u32::from(!evo::alive_of(out[0]));
        }
        let p = f64::from(held[evo::HAZARD] >> guard) / f64::from(evo::CHANCES);
        let expected = p * f64::from(trials);
        let spread = 5.0 * (f64::from(trials) * p * (1.0 - p)).sqrt();
        println!(
            "guard {guard}: {deaths} of {trials} struck, expected {expected:.0} +- {spread:.0}, upkeep {cost}"
        );
        assert!(
            (f64::from(deaths) - expected).abs() <= spread,
            "guard {guard}"
        );
        assert!(deaths < last_deaths);
        last_deaths = deaths;
    }
}

#[test]
fn energy_comes_only_from_empty_cells_at_the_feeding_rate() {
    let held = calm();
    let genome = evo::genome(3, 0, 0, 0);
    let cost = evo::upkeep_of(genome, &held);
    for living in 1..=CORNERS {
        let mut words = [0; CORNERS];
        for slot in words.iter_mut().take(living) {
            *slot = evo::cell(genome, 40, 1);
        }
        let (out, _) = stepped(words, 0, [0, 0, 0], &held);
        let empties = (CORNERS - living) as u32;
        let total = 40 + held[evo::FEED] * empties;
        for one in out.iter().take(living) {
            if total <= cost {
                assert_eq!(*one, 0);
            } else {
                assert_eq!(evo::energy_of(*one), total - cost);
            }
        }
    }
    let mut starving = held.clone();
    starving[evo::FEED] = 0;
    let lasting = evo::genome(3, 3, 0, 0);
    let burn = evo::upkeep_of(lasting, &starving);
    let words = [evo::cell(lasting, evo::FULL, 0); CORNERS];
    let mut frames = 0;
    let mut out = words.to_vec();
    while evo::alive_of(out[0]) {
        out = stepped(
            out.clone().try_into().unwrap(),
            frames,
            [0, 0, 0],
            &starving,
        )
        .0;
        frames += 1;
    }
    println!("an oct of eight full cells with no food dies after {frames} frames");
    assert_eq!(out, vec![0; CORNERS]);
    assert!(evo::FULL.div_ceil(burn) < evo::lifespan_of(lasting));
    assert_eq!(frames, evo::FULL.div_ceil(burn));
}

#[test]
fn the_defaults_and_the_caps_are_as_named() {
    assert_eq!(evo::DEFAULTS.len(), evo::PARAMS);
    assert_eq!(Evo.params(), evo::PARAMS);
    assert_eq!(Evo.tally().counts, evo::COUNTS);
    let mut held = calm();
    held[evo::UPKEEP] = u32::MAX;
    held[evo::LIFE_COST] = u32::MAX;
    held[evo::GUARD_COST] = u32::MAX;
    assert_eq!(
        evo::upkeep_of(evo::genome(0, 3, 3, 0), &held),
        7 * evo::COST_MOST
    );
}
