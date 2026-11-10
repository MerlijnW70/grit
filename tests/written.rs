#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

use grit::compact::{Over, Plan, Tier};
use grit::layout::{
    AWAKE, BINDINGS, DENSE, GRAND, HOT, HOT_DENSE, HOT_INDIRECT, INDIRECT, PARAMS, STATE, TALLY,
};
use grit::sand::Grid;
use grit::sweep;
use grit::written::{Written, written};
use simdr::kernel::{Kernel, Shape};
use simdr::lanes::U32;

const WIDTHS: [u32; 5] = [4, 8, 16, 32, 64];
const ROOM: Grid = Grid::of(256, 128);

const ACCESS_CHAIN: u32 = 65;
const TYPE_POINTER: u32 = 32;
const DECORATE: u32 = 71;
const FUNCTION: u32 = 54;
const FUNCTION_END: u32 = 56;
const STORE: u32 = 62;
const UNDEF: u32 = 1;
const COPY_OBJECT: u32 = 83;
const FUNCTION_CALL: u32 = 57;
const CAPABILITY: u32 = 17;
const BINDING: u32 = 33;
const STORAGE_BUFFER: u32 = 12;
const VARIABLE_POINTERS: u32 = 4442;
const PTR_EQUAL: u32 = 401;
const VARIABLE: u32 = 59;
const FUNCTION_CLASS: u32 = 7;

fn plan_for(width: u32, grid: Grid) -> Plan {
    Plan::over(width, grid.quads().div_ceil(sweep::cells(width))).unwrap()
}

fn only(bindings: &[u32]) -> Written {
    let mut held: Vec<usize> = bindings.iter().map(|&binding| binding as usize).collect();
    held.sort_unstable();
    Written::Only(held)
}

fn storing(width: u32, binding: u32) -> Vec<u32> {
    let mut kernel = Kernel::<U32>::new(Shape::new(width, 1, BINDINGS)).unwrap();
    let slot = kernel.module().constant_u32(3).unwrap();
    let word = kernel.module().constant_u32(9).unwrap();
    kernel.store_at(binding, slot, word).unwrap();
    kernel.finish().unwrap()
}

fn reading(width: u32, from: u32, into: u32) -> Vec<u32> {
    let mut kernel = Kernel::<U32>::new(Shape::new(width, 1, BINDINGS)).unwrap();
    let slot = kernel.module().constant_u32(0).unwrap();
    let value = kernel.load_at(from, slot).unwrap();
    kernel.store_at(into, slot, value).unwrap();
    kernel.finish().unwrap()
}

fn atomics(width: u32, added: u32, loaded: u32) -> Vec<u32> {
    let mut kernel = Kernel::<U32>::new(Shape::new(width, 1, BINDINGS)).unwrap();
    let slot = kernel.module().constant_u32(1).unwrap();
    let one = kernel.module().constant_u32(1).unwrap();
    let _ = kernel.atomic_add_at(added, slot, one).unwrap();
    let _ = kernel.atomic_load_at(loaded, slot).unwrap();
    kernel.finish().unwrap()
}

struct Op {
    at: usize,
    code: u32,
    operands: Vec<u32>,
}

fn ops(words: &[u32]) -> Vec<Op> {
    let mut out = Vec::new();
    let mut at = 5;
    while at < words.len() {
        let count = (words[at] >> 16) as usize;
        out.push(Op {
            at,
            code: words[at] & 0xFFFF,
            operands: words[at + 1..at + count].to_vec(),
        });
        at += count;
    }
    out
}

fn instruction(code: u32, operands: &[u32]) -> Vec<u32> {
    let mut out = vec![((operands.len() as u32 + 1) << 16) | code];
    out.extend_from_slice(operands);
    out
}

fn fresh(words: &mut [u32]) -> u32 {
    let id = words[3];
    words[3] += 1;
    id
}

fn before_end(words: &mut Vec<u32>, added: &[u32]) {
    let end = ops(words)
        .iter()
        .rev()
        .find(|op| op.code == FUNCTION_END)
        .unwrap()
        .at;
    words.splice(end..end, added.iter().copied());
}

fn first_chain(words: &[u32]) -> u32 {
    ops(words)
        .iter()
        .find(|op| op.code == ACCESS_CHAIN)
        .unwrap()
        .operands[1]
}

fn element_pointer(words: &[u32]) -> u32 {
    ops(words)
        .iter()
        .find(|op| op.code == TYPE_POINTER && op.operands[1] == STORAGE_BUFFER)
        .unwrap()
        .operands[0]
}

fn main_function(words: &[u32]) -> u32 {
    ops(words)
        .iter()
        .find(|op| op.code == FUNCTION)
        .unwrap()
        .operands[1]
}

#[test]
fn a_store_counts_for_its_binding_alone() {
    for width in WIDTHS {
        for binding in [0, DENSE, INDIRECT, BINDINGS - 1] {
            assert_eq!(
                written(&storing(width, binding)),
                only(&[binding]),
                "width {width} binding {binding}"
            );
        }
    }
}

#[test]
fn a_load_is_not_a_write() {
    let module = reading(16, DENSE, STATE);
    let found = written(&module);
    assert_eq!(found, only(&[STATE]));
    assert!(!found.may_write(DENSE as usize));
    assert!(found.may_write(STATE as usize));
}

#[test]
fn an_atomic_add_writes_and_an_atomic_load_does_not() {
    assert_eq!(written(&atomics(8, GRAND, HOT)), only(&[GRAND]));
    assert_eq!(written(&atomics(8, HOT, HOT)), only(&[HOT]));
}

#[test]
fn the_list_count_pass_stores_to_dense_and_settle_only_reads_it() {
    for width in WIDTHS {
        let plan = plan_for(width, ROOM);
        let unclaimed = written(&grit::live::unclaimed(width, &plan).unwrap());
        assert_eq!(unclaimed, only(&[DENSE]), "unclaimed width {width}");

        let settle = written(&grit::live::settle(width, ROOM).unwrap());
        assert!(!settle.may_write(DENSE as usize), "settle width {width}");
        assert!(settle.may_write(STATE as usize), "settle width {width}");
        assert_eq!(settle, only(&[STATE, AWAKE, TALLY, HOT]), "width {width}");
    }
}

#[test]
fn the_brush_writes_the_list_count_the_settle_dispatch_reads() {
    for width in WIDTHS {
        let brush = written(&grit::sand::brush(width, ROOM, true).unwrap());
        assert!(brush.may_write(INDIRECT as usize), "width {width}");
        assert!(brush.may_write(DENSE as usize), "width {width}");
        assert!(!brush.may_write(PARAMS as usize), "width {width}");
    }
}

#[test]
fn only_the_prepare_passes_write_the_dispatch_arguments() {
    for width in WIDTHS {
        let plan = plan_for(width, ROOM);
        let fine = written(&grit::compact::prepare(width, width, Tier::FINE).unwrap());
        let coarse = written(&grit::compact::prepare(width, 1, Tier::COARSE).unwrap());
        assert_eq!(fine, only(&[INDIRECT]), "width {width}");
        assert_eq!(coarse, only(&[HOT_INDIRECT]), "width {width}");

        let gather = grit::compact::gather(width, Tier::FINE, Over::Listed(HOT_DENSE), plan.dump());
        let gather = written(&gather.unwrap());
        assert_eq!(gather, only(&[DENSE]), "width {width}");
        let mark = grit::live::mark_below(width, Tier::FINE, Over::Listed(HOT_DENSE), plan.dump());
        let mark = written(&mark.unwrap());
        assert!(!mark.may_write(HOT_INDIRECT as usize), "width {width}");
    }
}

#[test]
fn words_that_are_not_a_module_write_everything() {
    let module = storing(4, 2);
    assert_eq!(written(&[]), Written::Every);
    assert_eq!(written(&module[..4]), Written::Every);
    let stored = ops(&module)
        .iter()
        .rev()
        .find(|op| op.code == STORE)
        .unwrap()
        .at;
    assert_eq!(written(&module[..stored + 2]), Written::Every);
    let mut swapped = module.clone();
    swapped[0] = swapped[0].swap_bytes();
    assert_eq!(written(&swapped), Written::Every);
    let mut zero = module;
    let at = ops(&zero)[2].at;
    zero[at] &= 0xFFFF;
    assert_eq!(written(&zero), Written::Every);
}

#[test]
fn a_module_with_variable_pointers_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    module.splice(5..5, instruction(CAPABILITY, &[VARIABLE_POINTERS]));
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn a_pointer_passed_to_a_function_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let chain = first_chain(&module);
    let callee = main_function(&module);
    let result = fresh(&mut module);
    let kind = element_pointer(&module);
    before_end(
        &mut module,
        &instruction(FUNCTION_CALL, &[kind, result, callee, chain]),
    );
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn an_unknown_instruction_on_a_pointer_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let chain = first_chain(&module);
    let result = fresh(&mut module);
    let kind = element_pointer(&module);
    before_end(
        &mut module,
        &instruction(PTR_EQUAL, &[kind, result, chain, chain]),
    );
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn a_chain_from_a_pointer_the_analysis_cannot_follow_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let kind = element_pointer(&module);
    let unknown = fresh(&mut module);
    let index = fresh(&mut module);
    let target = fresh(&mut module);
    let value = fresh(&mut module);
    let mut added = instruction(UNDEF, &[kind, unknown]);
    added.extend(instruction(ACCESS_CHAIN, &[kind, target, unknown, index]));
    added.extend(instruction(STORE, &[target, value]));
    before_end(&mut module, &added);
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn a_storage_variable_without_a_binding_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let at = ops(&module)
        .iter()
        .find(|op| op.code == DECORATE && op.operands[1] == BINDING)
        .unwrap()
        .at;
    module[at + 2] = 0;
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn a_store_through_a_copied_pointer_counts_for_its_binding() {
    let mut module = reading(4, DENSE, STATE);
    let chain = first_chain(&module);
    let kind = element_pointer(&module);
    let copy = fresh(&mut module);
    let value = fresh(&mut module);
    let mut added = instruction(COPY_OBJECT, &[kind, copy, chain]);
    added.extend(instruction(STORE, &[copy, value]));
    before_end(&mut module, &added);
    assert_eq!(written(&module), only(&[DENSE, STATE]));
}

fn local_variable(module: &mut Vec<u32>) -> u32 {
    let element = ops(module)
        .iter()
        .find(|op| op.code == TYPE_POINTER && op.operands[1] == STORAGE_BUFFER)
        .unwrap()
        .operands[2];
    let kind = fresh(module);
    let local = fresh(module);
    let mut added = instruction(TYPE_POINTER, &[kind, FUNCTION_CLASS, element]);
    added.extend(instruction(VARIABLE, &[kind, local, FUNCTION_CLASS]));
    before_end(module, &added);
    local
}

#[test]
fn a_store_to_a_function_variable_is_not_a_write_to_a_binding() {
    let mut module = reading(4, DENSE, STATE);
    let local = local_variable(&mut module);
    let value = fresh(&mut module);
    before_end(&mut module, &instruction(STORE, &[local, value]));
    assert_eq!(written(&module), only(&[STATE]));
}

#[test]
fn storing_a_pointer_as_a_value_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let chain = first_chain(&module);
    let local = local_variable(&mut module);
    before_end(&mut module, &instruction(STORE, &[local, chain]));
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn a_store_through_a_pointer_nothing_declares_writes_everything() {
    let mut module = reading(4, DENSE, STATE);
    let nowhere = fresh(&mut module);
    let value = fresh(&mut module);
    before_end(&mut module, &instruction(STORE, &[nowhere, value]));
    assert_eq!(written(&module), Written::Every);
}

#[test]
fn every_module_of_the_engine_is_followed() {
    for width in WIDTHS {
        for grid in [ROOM, Grid::of(4096, 4096)] {
            let Ok(plan) = Plan::over(width, grid.quads().div_ceil(sweep::cells(width))) else {
                continue;
            };
            for coarse in [false, true] {
                let (passes, _) = grit::live::arranged(width, grid, &plan, None, coarse).unwrap();
                for (pass, module) in passes.iter().enumerate() {
                    let found = written(module);
                    assert_ne!(found, Written::Every, "width {width} pass {pass}");
                    assert_ne!(
                        found,
                        Written::Only(Vec::new()),
                        "width {width} pass {pass}"
                    );
                }
            }
        }
    }
}
