#![cfg(feature = "faults")]
#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod budget;
mod common;

use ash::vk;
use common::device;
use grit::resident::faults::{self, Step};
use grit::{Error, Host, Resident};

const SIZES: [usize; 4] = [64, 1, 256, 16];

const LIMIT: u32 = 64;

fn alive() -> Vec<String> {
    Step::ALL
        .iter()
        .map(|&step| (step, faults::count(step)))
        .filter(|(_, count)| count.made != count.gone || count.outstanding != 0)
        .map(|(step, count)| {
            format!(
                "{step:?} made {} freed {}; the driver holds {} of its {} host allocations",
                count.made, count.gone, count.outstanding, count.allocations
            )
        })
        .collect()
}

fn nothing_alive(label: &str) {
    let left = alive();
    assert!(left.is_empty(), "{label}: {left:?}");
}

fn injected(made: &Result<Resident<'_>, Error>) -> bool {
    matches!(
        made,
        Err(Error::Vulkan(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY))
    )
}

#[derive(Debug, Clone, Copy)]
struct Tried {
    step: Step,
    reached: u32,
    failures: u32,
}

/// Fails every occurrence of every construction step in turn, one per
/// attempt, and checks that each attempt leaves no object alive once what it
/// returned is dropped. Returns how often each step was reached and how many
/// of those failures failed the construction.
fn every_step<'host>(
    label: &str,
    build: impl Fn() -> Result<Resident<'host>, Error>,
) -> Vec<Tried> {
    let mut tried = Vec::new();
    for step in Step::ALL {
        let mut failures = 0;
        let mut reached = 0;
        let mut ended = false;
        for nth in 0..LIMIT {
            faults::reset();
            faults::arm(step, nth);
            let made = build();
            let fired = faults::fired();
            faults::disarm();
            let failed = made.is_err();
            if failed {
                assert!(
                    injected(&made),
                    "{label}: {step:?} #{nth} failed with {:?}",
                    made.as_ref().err()
                );
            }
            drop(made);
            nothing_alive(&format!("{label}: {step:?} #{nth}"));
            if !fired {
                assert!(!failed, "{label}: an attempt with nothing injected failed");
                ended = true;
                break;
            }
            reached += 1;
            failures += u32::from(failed);
        }
        assert!(
            ended,
            "{label}: {step:?} is reached more than {LIMIT} times"
        );
        eprintln!("{label}: {step:?} reached {reached}, failed {failures} constructions");
        tried.push(Tried {
            step,
            reached,
            failures,
        });
    }
    faults::reset();
    tried
}

fn of(tried: &[Tried], step: Step) -> Tried {
    tried.iter().copied().find(|one| one.step == step).unwrap()
}

fn failed(tried: &[Tried], step: Step) -> bool {
    of(tried, step).failures > 0
}

/// Every step must fail a construction when it fails, except where the
/// constructor has a fallback: when `fallback` is set, every allocation has
/// one (the near window falls back to device-local memory, and cached staging
/// to uncached), so those steps must be reached and fail none.
fn every_step_fails(label: &str, tried: &[Tried], modules: bool, fallback: bool) {
    for step in [Step::Buffer, Step::Memory, Step::Bind] {
        let one = of(tried, step);
        assert!(one.reached > 0, "{label}: {step:?} never reached");
        assert_eq!(one.failures > 0, !fallback, "{label}: {one:?}");
    }
    let always = [
        Step::SetLayout,
        Step::Pool,
        Step::Set,
        Step::PipelineLayout,
        Step::Commands,
        Step::CommandBuffers,
        Step::Fence,
    ];
    for step in always {
        assert!(failed(tried, step), "{label}: {step:?} never failed one");
    }
    for step in [Step::Shader, Step::Pipeline] {
        assert_eq!(failed(tried, step), modules, "{label}: {step:?}");
    }
    assert!(
        !failed(tried, Step::Queries),
        "{label}: a query pool failed one"
    );
}

fn ticks(host: &Host) -> Vec<u32> {
    grit::live::tick(host.width()).unwrap()
}

#[test]
fn a_failure_at_every_step_of_resident_leaves_nothing_alive() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let tick = ticks(&host);
    let steps = every_step("resident", || host.resident(&SIZES, &[&tick, &tick]));
    every_step_fails("resident", &steps, true, false);
}

#[test]
fn a_failure_at_every_step_of_resident_direct_leaves_nothing_alive() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let tick = ticks(&host);
    let steps = every_step("direct", || host.resident_direct(&SIZES, &[&tick]));
    every_step_fails("direct", &steps, true, false);
}

#[test]
fn a_failure_at_every_step_of_resident_near_leaves_nothing_alive() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let tick = ticks(&host);
    let steps = every_step("near", || host.resident_near(&SIZES, &[&tick], 1 << 20));
    every_step_fails("near", &steps, true, true);
}

#[test]
fn a_failure_at_every_step_of_resident_vast_leaves_nothing_alive() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    if !host.sparse() {
        eprintln!(
            "vast: {} has no sparse residency, so resident_vast refuses before it makes anything",
            host.name()
        );
        faults::reset();
        assert!(matches!(
            host.resident_vast(&SIZES, &[], 1),
            Err(Error::NoSparse)
        ));
        nothing_alive("vast without sparse");
        return;
    }
    let tick = ticks(&host);
    let steps = every_step("vast", || host.resident_vast(&SIZES, &[&tick], 1));
    every_step_fails("vast", &steps, true, false);
}

#[test]
fn a_failure_at_every_step_with_no_binding_and_no_module_leaves_nothing_alive() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let steps = every_step("empty", || host.resident(&[], &[]));
    every_step_fails("empty", &steps, false, true);
}

#[test]
fn a_failed_bind_frees_its_memory_and_its_buffer() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    faults::reset();
    faults::arm(Step::Bind, 0);
    let made = host.resident(&SIZES, &[]);
    assert!(injected(&made));
    let buffer = faults::count(Step::Buffer);
    let memory = faults::count(Step::Memory);
    assert_eq!((buffer.made, buffer.gone), (1, 1));
    assert_eq!((memory.made, memory.gone), (1, 1));
    nothing_alive("bind");
}

#[test]
fn a_failed_memory_allocation_frees_its_buffer() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    faults::reset();
    faults::arm(Step::Memory, 2);
    let made = host.resident(&SIZES, &[]);
    assert!(injected(&made));
    let buffer = faults::count(Step::Buffer);
    let memory = faults::count(Step::Memory);
    assert_eq!((buffer.made, buffer.gone), (3, 3));
    assert_eq!((memory.made, memory.gone), (2, 2));
    nothing_alive("memory");
}

#[test]
fn a_failed_pipeline_frees_its_modules_its_layout_and_the_pipelines_before_it() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let tick = ticks(&host);
    faults::reset();
    faults::arm(Step::Pipeline, 1);
    let made = host.resident(&SIZES, &[&tick, &tick]);
    assert!(injected(&made));
    let shader = faults::count(Step::Shader);
    let layout = faults::count(Step::PipelineLayout);
    let pipeline = faults::count(Step::Pipeline);
    assert_eq!((shader.made, shader.gone), (2, 2));
    assert_eq!((layout.made, layout.gone), (1, 1));
    assert_eq!((pipeline.made, pipeline.gone), (1, 1));
    nothing_alive("pipeline");
}

#[test]
fn a_failed_near_allocation_falls_back_and_frees_the_near_buffer_at_once() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    faults::reset();
    faults::arm(Step::Memory, 0);
    let held = host.resident_near(&[64], &[], 1 << 20).unwrap();
    assert!(faults::fired());
    let buffer = faults::count(Step::Buffer);
    assert_eq!(buffer.gone, 1, "the near buffer whose memory failed");
    assert_eq!(faults::count(Step::Memory).gone, 0);
    assert_eq!(held.bindings(), 1);
    drop(held);
    nothing_alive("near fallback");
}

#[test]
fn a_failed_query_pool_still_builds_and_frees_everything() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    faults::reset();
    faults::arm(Step::Queries, 0);
    let held = host.resident(&SIZES, &[]).unwrap();
    eprintln!(
        "queries: armed failure reached {}, pools made {}",
        faults::fired(),
        faults::count(Step::Queries).made
    );
    drop(held);
    nothing_alive("queries");
}

#[test]
fn the_host_builds_and_runs_a_resident_after_failed_constructions() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    faults::reset();
    for (step, nth) in [(Step::Fence, 1), (Step::Set, 0), (Step::Buffer, 3)] {
        faults::arm(step, nth);
        assert!(injected(&host.resident(&SIZES, &[])));
    }
    faults::disarm();
    let mut held = host.resident(&SIZES, &[]).unwrap();
    let words: Vec<u32> = (0..256).collect();
    held.write(2, &words).unwrap();
    assert_eq!(held.read(2, 256).unwrap(), words);
    drop(held);
    nothing_alive("after failures");
}

#[test]
fn three_failed_constructions_leave_the_heap_where_it_was() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    const QUARTER: usize = 16 << 20;
    const LARGE: [usize; 4] = [QUARTER; 4];
    let bytes = (QUARTER * 4 * 4) as u64;
    faults::reset();
    budget::three_failures(
        &host,
        bytes,
        || host.resident(&LARGE, &[]),
        || {
            faults::arm(Step::Fence, 2);
            host.resident(&LARGE, &[])
        },
    );
    faults::disarm();
    nothing_alive("three failures");
}

#[test]
fn the_driver_reports_its_host_allocations_through_the_callbacks() {
    let _alone = budget::alone();
    let Some(host) = device("cleanup") else {
        return;
    };
    let tick = ticks(&host);
    faults::reset();
    let held = host.resident(&SIZES, &[&tick, &tick]).unwrap();
    let holding: Vec<(Step, i64)> = Step::ALL
        .iter()
        .map(|&step| (step, faults::count(step).outstanding))
        .filter(|&(_, outstanding)| outstanding > 0)
        .collect();
    eprintln!("callbacks {}: held while built {holding:?}", host.name());
    drop(held);
    nothing_alive("callbacks");
    assert!(
        !holding.is_empty(),
        "{} made no host allocation through the callbacks",
        host.name()
    );
}
