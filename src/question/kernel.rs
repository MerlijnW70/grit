use simdr::kernel::{Kernel, Shape as Laid};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;
use simdr::spec::{LoopControl, MemorySemantics, Scope};

use crate::bricks::{ENTRY, UNIFORM};
use crate::live::{lane_if, state_load};
use crate::ops::Device;

use super::reduce::{COPIES, COUNTER, RESULT};
use super::{Layout, Map, Measure, Reduce, Region, Seen};

pub const FROM: usize = 0;
pub const TO: usize = 3;
pub const UNITS: usize = 6;
pub const PER: usize = 7;
pub const GROUPS: usize = 8;
pub const INNER: usize = 9;
pub const FIRST: usize = 10;
pub const SPAN: usize = 13;
pub const SIDES: usize = 16;

const ATOMIC_U_MIN: u16 = 237;
const ATOMIC_U_MAX: u16 = 239;

pub const NO_FORM: &str =
    "a brick of the layout holds 2^16 cells or more, or not a whole number of subgroups";

#[derive(Debug, Clone, Copy)]
pub struct Shape {
    pub layout: Layout,
    pub subgroup: u32,
    pub ask: u32,
    pub target: u32,
    pub reduce: Reduce,
    pub counting: bool,
    pub placed: bool,
}

impl Shape {
    const fn closed(&self) -> bool {
        !self.placed && !matches!(self.reduce, Reduce::Differ(_))
    }

    const fn bindings(&self) -> u32 {
        let past = self.target + 1;
        let laid = match self.layout {
            Layout::Dense { windows, .. } => windows.bindings(),
            Layout::Sparse(bricks) => bricks.bindings(),
        };
        if past > laid { past } else { laid }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Walk {
    pub units: u32,
    pub groups: u32,
    pub per: u32,
    pub inner: u32,
    pub first: [u32; 3],
    pub span: [u32; 3],
}

impl Walk {
    #[must_use]
    pub fn of(layout: &Layout, subgroup: u32, region: Region, most: u32) -> Self {
        let sides = region.sides();
        let (units, inner, first, span) = match layout {
            Layout::Dense { .. } => {
                let [wide, deep, tall] = sides;
                (
                    u64::from(deep) * u64::from(tall),
                    wide.div_ceil(subgroup),
                    [0; 3],
                    sides,
                )
            }
            Layout::Sparse(bricks) => {
                let brick = bricks.sides();
                let mut first = [0; 3];
                let mut span = [0; 3];
                for axis in 0..3 {
                    let side = brick.get(axis).copied().unwrap_or(1).max(1);
                    let from = region.from.get(axis).copied().unwrap_or(0);
                    let to = region.to.get(axis).copied().unwrap_or(1);
                    let low = from / side;
                    let high = to.saturating_sub(1) / side;
                    if let Some(slot) = first.get_mut(axis) {
                        *slot = low;
                    }
                    if let Some(slot) = span.get_mut(axis) {
                        *slot = high - low + 1;
                    }
                }
                let units = span.iter().map(|&one| u64::from(one)).product::<u64>();
                (units, bricks.cells() as u32 / subgroup, first, span)
            }
        };
        let units = u32::try_from(units).unwrap_or(u32::MAX);
        let groups = units.min(most).max(1);
        Self {
            units,
            groups,
            per: units.div_ceil(groups),
            inner,
            first,
            span,
        }
    }

    pub fn write(&self, words: &mut [u32], region: Region, _: Reduce) {
        let mut put = |at: usize, value: u32| {
            if let Some(slot) = words.get_mut(at) {
                *slot = value;
            }
        };
        for axis in 0..3 {
            put(FROM + axis, region.from.get(axis).copied().unwrap_or(0));
            put(TO + axis, region.to.get(axis).copied().unwrap_or(0));
            put(FIRST + axis, self.first.get(axis).copied().unwrap_or(0));
            put(SPAN + axis, self.span.get(axis).copied().unwrap_or(0));
            put(SIDES + axis, region.sides().get(axis).copied().unwrap_or(0));
        }
        put(UNITS, self.units);
        put(PER, self.per);
        put(GROUPS, self.groups);
        put(INNER, self.inner);
    }
}

pub fn built<M: Map>(map: &M, shape: &Shape) -> Result<Vec<u32>, LaneError> {
    if let Layout::Sparse(bricks) = shape.layout {
        let cells = bricks.cells() as u64;
        if cells >= 1 << 16 || !cells.is_multiple_of(u64::from(shape.subgroup)) {
            return Err(LaneError::NoSuchForm {
                operation: "question::kernel::built",
                because: NO_FORM,
            });
        }
    }
    match shape.subgroup {
        4 => emitted::<4, M>(map, shape),
        8 => emitted::<8, M>(map, shape),
        16 => emitted::<16, M>(map, shape),
        32 => emitted::<32, M>(map, shape),
        64 => emitted::<64, M>(map, shape),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: shape.subgroup,
        }),
    }
}

type V<const L: u32> = Vector<U32, L>;

struct E<'held, 'module, const L: u32>(&'held mut Lanes<'module>);

impl<const L: u32> E<'_, '_, L> {
    fn k(&mut self, value: u32) -> Result<V<L>, LaneError> {
        self.0.splat_bits::<U32, L>(value)
    }

    fn v(&self, id: Id) -> Result<V<L>, LaneError> {
        self.0.splat_id::<U32, L>(id)
    }

    fn add(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.add(left, right)
    }

    fn sub(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.sub(left, right)
    }

    fn mul(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.mul(left, right)
    }

    fn div(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.div(left, right)
    }

    fn and(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.and(left, right)
    }

    fn masked(&mut self, value: V<L>, mask: u32) -> Result<V<L>, LaneError> {
        let mask = self.k(mask)?;
        self.0.and(value, mask)
    }

    fn shr(&mut self, value: V<L>, by: u32) -> Result<V<L>, LaneError> {
        if by == 0 {
            return Ok(value);
        }
        let by = self.k(by)?;
        self.0.shift_right_logical(value, by)
    }

    fn shl(&mut self, value: V<L>, by: u32) -> Result<V<L>, LaneError> {
        if by == 0 {
            return Ok(value);
        }
        let by = self.k(by)?;
        self.0.shift_left(value, by)
    }

    fn min(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.min(left, right)
    }

    fn max(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        self.0.max(left, right)
    }

    fn one_if(&mut self, truth: simdr::lanes::Predicate<L>) -> Result<V<L>, LaneError> {
        let one = self.k(1)?;
        let none = self.k(0)?;
        self.0.select(truth, one, none)
    }

    fn lt(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        let truth = self.0.less_than(left, right)?;
        self.one_if(truth)
    }

    fn eq(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        let truth = self.0.equal(left, right)?;
        self.one_if(truth)
    }

    fn ne(&mut self, left: V<L>, right: V<L>) -> Result<V<L>, LaneError> {
        let truth = self.0.not_equal(left, right)?;
        self.one_if(truth)
    }

    fn not(&mut self, flag: V<L>) -> Result<V<L>, LaneError> {
        let one = self.k(1)?;
        self.0.xor(flag, one)
    }

    fn sel(&mut self, flag: V<L>, then: V<L>, otherwise: V<L>) -> Result<V<L>, LaneError> {
        let none = self.k(0)?;
        let truth = self.0.not_equal(flag, none)?;
        self.0.select(truth, then, otherwise)
    }

    fn truth(&mut self, flag: V<L>) -> Result<Id, LaneError> {
        let none = self.k(0)?;
        let truth = self.0.not_equal(flag, none)?;
        truth
            .strips()
            .first()
            .copied()
            .ok_or(LaneError::NoOpenBlock { arm: "question" })
    }

    fn wide_add(&mut self, sum: [V<L>; 2], add: [V<L>; 2]) -> Result<[V<L>; 2], LaneError> {
        let [low, high] = sum;
        let [add_low, add_high] = add;
        let lower = self.add(low, add_low)?;
        let carry = self.lt(lower, add_low)?;
        let higher = self.add(high, add_high)?;
        Ok([lower, self.add(higher, carry)?])
    }

    fn mul_small(&mut self, value: V<L>, small: V<L>) -> Result<[V<L>; 2], LaneError> {
        let below = self.masked(value, 0xFFFF)?;
        let above = self.shr(value, 16)?;
        let first = self.mul(below, small)?;
        let second = self.mul(above, small)?;
        let lifted = self.shl(second, 16)?;
        let low = self.add(first, lifted)?;
        let carry = self.lt(low, first)?;
        let high = self.shr(second, 16)?;
        Ok([low, self.add(high, carry)?])
    }

    fn wide_mul_small(&mut self, wide: [V<L>; 2], small: V<L>) -> Result<[V<L>; 2], LaneError> {
        let [low, high] = wide;
        let [product, carried] = self.mul_small(low, small)?;
        let lifted = self.mul(high, small)?;
        Ok([product, self.add(carried, lifted)?])
    }

    fn masked_pair(&mut self, flag: V<L>, pair: [V<L>; 2]) -> Result<[V<L>; 2], LaneError> {
        let none = self.k(0)?;
        let [low, high] = pair;
        Ok([self.sel(flag, low, none)?, self.sel(flag, high, none)?])
    }
}

#[derive(Clone, Copy)]
struct Given {
    shape: Shape,
    lane: Id,
    from: [Id; 3],
    to: [Id; 3],
    sides: [Id; 3],
    bins: u32,
    copy: Id,
}

fn accumulators(reduce: Reduce) -> usize {
    match reduce {
        Reduce::Count | Reduce::Sum | Reduce::Differ(_) => 2,
        Reduce::Min | Reduce::Max => 1,
        Reduce::Histogram { .. } => 3,
        Reduce::Moments => 8,
    }
}

fn start(kernel: &mut Kernel<U32>, shape: &Shape) -> Result<Vec<Id>, LaneError> {
    let mut state = Vec::new();
    let initial = if matches!(shape.reduce, Reduce::Min) {
        u32::MAX
    } else {
        0
    };
    for _ in 0..accumulators(shape.reduce) {
        state.push(kernel.module().constant_u32(initial)?);
    }
    if shape.counting {
        for _ in 0..COUNTER {
            state.push(kernel.module().constant_u32(0)?);
        }
    }
    Ok(state)
}

fn looped<F>(
    kernel: &mut Kernel<U32>,
    limit: Id,
    initial: &[Id],
    body: F,
) -> Result<Vec<Id>, LaneError>
where
    F: FnOnce(&mut Kernel<U32>, &[Id], Id) -> Result<Vec<Id>, LaneError>,
{
    use simdr::module::op;

    let uint = kernel.index_type();
    let boolean = kernel.module().type_bool()?;
    let zero = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let header = kernel.module().alloc_id()?;
    let body_block = kernel.module().alloc_id()?;
    let continue_block = kernel.module().alloc_id()?;
    let merge_block = kernel.module().alloc_id()?;
    let entry = kernel.module().alloc_id()?;
    kernel.module().branch(entry)?;
    kernel.module().label_at(entry)?;
    kernel.module().branch(header)?;
    kernel.module().label_at(header)?;
    let counter = kernel.module().alloc_id()?;
    let stepped = kernel.module().alloc_id()?;
    let mut carried = Vec::with_capacity(initial.len());
    let mut produced = Vec::with_capacity(initial.len());
    for _ in initial {
        carried.push(kernel.module().alloc_id()?);
        produced.push(kernel.module().alloc_id()?);
    }
    kernel
        .module()
        .phi_at(counter, uint, &[(zero, entry), (stepped, continue_block)])?;
    for ((&name, &was), &will) in carried.iter().zip(initial).zip(&produced) {
        kernel
            .module()
            .phi_at(name, uint, &[(was, entry), (will, continue_block)])?;
    }
    let carry_on = kernel
        .module()
        .binary(op::U_LESS_THAN, boolean, counter, limit)?;
    kernel
        .module()
        .loop_merge(merge_block, continue_block, LoopControl::None)?;
    kernel
        .module()
        .branch_conditional(carry_on, body_block, merge_block)?;
    kernel.module().label_at(body_block)?;
    let result = body(kernel, &carried, counter)?;
    if result.len() != produced.len() {
        return Err(LaneError::BadCarry {
            given: result.len(),
            wanted: produced.len(),
        });
    }
    for (&will, &one_of) in produced.iter().zip(&result) {
        kernel.module().copy_object_at(will, uint, one_of)?;
    }
    kernel.module().branch(continue_block)?;
    kernel.module().label_at(continue_block)?;
    kernel.module().i_add_at(stepped, uint, counter, one)?;
    kernel.module().branch(header)?;
    kernel.module().label_at(merge_block)?;
    Ok(carried)
}

fn mapped<const L: u32, M: Map>(
    kernel: &mut Kernel<U32>,
    map: &M,
    word: V<L>,
    at: [V<L>; 3],
) -> Result<V<L>, LaneError> {
    let mut lanes = kernel.lanes()?;
    let mut device = Device::<L>::new(&mut lanes)?;
    let planes = [word];
    let out = map.map(
        &mut device,
        Seen {
            planes: &planes,
            at,
        },
    );
    device.finish(out)
}

fn atomic_wide<const L: u32>(
    kernel: &mut Kernel<U32>,
    ask: u32,
    at: V<L>,
    add: [V<L>; 2],
) -> Result<(), LaneError> {
    let [low, high] = add;
    let old = kernel.atomic_add_at(ask, at.id(), low.id())?;
    let (next, high) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let old = e.v(old)?;
        let sum = e.add(old, low)?;
        let carry = e.lt(sum, old)?;
        let one = e.k(1)?;
        (e.add(at, one)?, e.add(high, carry)?)
    };
    let _ = kernel.atomic_add_at(ask, next.id(), high.id())?;
    Ok(())
}

fn atomic_extreme(
    kernel: &mut Kernel<U32>,
    ask: u32,
    at: u32,
    opcode: u16,
    value: Id,
) -> Result<(), LaneError> {
    let at = kernel.module().constant_u32(at)?;
    let pointer = kernel.element_pointer_to(ask, at)?;
    let element = kernel.element();
    let scope = kernel.module().scope(Scope::Device)?;
    let semantics = kernel.module().memory_semantics(MemorySemantics::None)?;
    let _ = kernel
        .module()
        .atomic(opcode, element, pointer, scope, semantics, value)?;
    Ok(())
}

fn pair<const L: u32>(e: &E<'_, '_, L>, state: &[Id], at: usize) -> Result<[V<L>; 2], LaneError> {
    let low = state.get(at).copied().ok_or(LaneError::BadCarry {
        given: state.len(),
        wanted: at + 1,
    })?;
    let high = state.get(at + 1).copied().ok_or(LaneError::BadCarry {
        given: state.len(),
        wanted: at + 2,
    })?;
    Ok([e.v(low)?, e.v(high)?])
}

fn put_pair<const L: u32>(state: &mut [Id], at: usize, value: [V<L>; 2]) {
    let [low, high] = value;
    if let Some(slot) = state.get_mut(at) {
        *slot = low.id();
    }
    if let Some(slot) = state.get_mut(at + 1) {
        *slot = high.id();
    }
}

fn binned<const L: u32>(
    kernel: &mut Kernel<U32>,
    given: &Given,
    state: &[Id],
    bin: V<L>,
    weight: [V<L>; 2],
    active: V<L>,
) -> Result<Vec<Id>, LaneError> {
    let ask = given.shape.ask;
    let (flush, at, run, kept) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let cur = e.v(state.first().copied().ok_or(LaneError::BadCarry {
            given: state.len(),
            wanted: 3,
        })?)?;
        let run = pair(&e, state, 1)?;
        let same = e.eq(bin, cur)?;
        let differs = e.not(same)?;
        let moving = e.and(active, differs)?;
        let none = e.k(0)?;
        let [run_low, run_high] = run;
        let either = e.0.or(run_low, run_high)?;
        let held = e.ne(either, none)?;
        let flush = e.and(moving, held)?;
        let two = e.k(2)?;
        let doubled = e.mul(cur, two)?;
        let base = e.v(given.copy)?;
        let at = e.add(base, doubled)?;
        let next = e.sel(moving, bin, cur)?;
        let grown = e.wide_add(run, weight)?;
        let [grown_low, grown_high] = grown;
        let [weight_low, weight_high] = weight;
        let kept_low = e.sel(same, grown_low, weight_low)?;
        let kept_high = e.sel(same, grown_high, weight_high)?;
        let new_low = e.sel(active, kept_low, run_low)?;
        let new_high = e.sel(active, kept_high, run_high)?;
        (
            e.truth(flush)?,
            at,
            run,
            vec![next.id(), new_low.id(), new_high.id()],
        )
    };
    lane_if(kernel, flush, |kernel| {
        atomic_wide::<L>(kernel, ask, at, run)
    })?;
    let mut out = state.to_vec();
    for (slot, value) in out.iter_mut().zip(kept) {
        *slot = value;
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn contributed<const L: u32, M: Map>(
    kernel: &mut Kernel<U32>,
    map: &M,
    given: &Given,
    state: &[Id],
    word: Id,
    at: [V<L>; 3],
    inside: V<L>,
    aimed: Option<Id>,
) -> Result<Vec<Id>, LaneError> {
    let word = kernel.lanes()?.splat_id::<U32, L>(word)?;
    let value = mapped::<L, M>(kernel, map, word, at)?;
    let reduce = given.shape.reduce;
    if let Reduce::Histogram { .. } = reduce {
        let (bin, weight) = {
            let mut lanes = kernel.lanes()?;
            let mut e = E::<L>(&mut lanes);
            let past = e.k(given.bins)?;
            let none = e.k(0)?;
            (e.min(value, past)?, [inside, none])
        };
        return binned::<L>(kernel, given, state, bin, weight, inside);
    }
    let mut lanes = kernel.lanes()?;
    let mut e = E::<L>(&mut lanes);
    let mut out = state.to_vec();
    let none = e.k(0)?;
    match reduce {
        Reduce::Count => {
            let set = e.ne(value, none)?;
            let add = e.and(set, inside)?;
            let sum = pair(&e, state, 0)?;
            let sum = e.wide_add(sum, [add, none])?;
            put_pair(&mut out, 0, sum);
        }
        Reduce::Sum => {
            let add = e.sel(inside, value, none)?;
            let sum = pair(&e, state, 0)?;
            let sum = e.wide_add(sum, [add, none])?;
            put_pair(&mut out, 0, sum);
        }
        Reduce::Min | Reduce::Max => {
            let best = e.v(state.first().copied().ok_or(LaneError::BadCarry {
                given: state.len(),
                wanted: 1,
            })?)?;
            let next = if matches!(reduce, Reduce::Min) {
                let top = e.k(u32::MAX)?;
                let seen = e.sel(inside, value, top)?;
                e.min(best, seen)?
            } else {
                let seen = e.sel(inside, value, none)?;
                e.max(best, seen)?
            };
            if let Some(slot) = out.first_mut() {
                *slot = next.id();
            }
        }
        Reduce::Moments => {
            let set = e.ne(value, none)?;
            let set = e.and(set, inside)?;
            let count = pair(&e, state, 0)?;
            let count = e.wide_add(count, [set, none])?;
            put_pair(&mut out, 0, count);
            for (axis, &along) in at.iter().enumerate() {
                let add = e.sel(set, along, none)?;
                let sum = pair(&e, state, 2 + 2 * axis)?;
                let sum = e.wide_add(sum, [add, none])?;
                put_pair(&mut out, 2 + 2 * axis, sum);
            }
        }
        Reduce::Differ(measure) => {
            let aimed = e.v(aimed.ok_or(LaneError::NoOpenBlock { arm: "target" })?)?;
            let add = match measure {
                Measure::Count => {
                    let differs = e.ne(value, aimed)?;
                    e.and(differs, inside)?
                }
                Measure::Sum => {
                    let above = e.lt(aimed, value)?;
                    let up = e.sub(value, aimed)?;
                    let down = e.sub(aimed, value)?;
                    let apart = e.sel(above, up, down)?;
                    e.sel(inside, apart, none)?
                }
            };
            let sum = pair(&e, state, 0)?;
            let sum = e.wide_add(sum, [add, none])?;
            put_pair(&mut out, 0, sum);
        }
        Reduce::Histogram { .. } => {}
    }
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn closed<const L: u32, M: Map>(
    kernel: &mut Kernel<U32>,
    map: &M,
    given: &Given,
    state: &[Id],
    value: Id,
    low: [V<L>; 3],
    many: [V<L>; 3],
    lead: V<L>,
) -> Result<Vec<Id>, LaneError> {
    let word = kernel.lanes()?.splat_id::<U32, L>(value)?;
    let value = mapped::<L, M>(kernel, map, word, low)?;
    let reduce = given.shape.reduce;
    let (cells, plane) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let [wide, deep, tall] = many;
        let plane = e.mul(wide, deep)?;
        (
            e.mul(plane, tall)?,
            [e.mul(deep, tall)?, e.mul(wide, tall)?, plane],
        )
    };
    if let Reduce::Histogram { .. } = reduce {
        let (bin, weight) = {
            let mut lanes = kernel.lanes()?;
            let mut e = E::<L>(&mut lanes);
            let past = e.k(given.bins)?;
            let none = e.k(0)?;
            (e.min(value, past)?, [cells, none])
        };
        return binned::<L>(kernel, given, state, bin, weight, lead);
    }
    let mut lanes = kernel.lanes()?;
    let mut e = E::<L>(&mut lanes);
    let mut out = state.to_vec();
    let none = e.k(0)?;
    match reduce {
        Reduce::Count => {
            let set = e.ne(value, none)?;
            let set = e.and(set, lead)?;
            let add = e.sel(set, cells, none)?;
            let sum = pair(&e, state, 0)?;
            let sum = e.wide_add(sum, [add, none])?;
            put_pair(&mut out, 0, sum);
        }
        Reduce::Sum => {
            let product = e.mul_small(value, cells)?;
            let add = e.masked_pair(lead, product)?;
            let sum = pair(&e, state, 0)?;
            let sum = e.wide_add(sum, add)?;
            put_pair(&mut out, 0, sum);
        }
        Reduce::Min | Reduce::Max => {
            let best = e.v(state.first().copied().ok_or(LaneError::BadCarry {
                given: state.len(),
                wanted: 1,
            })?)?;
            let next = if matches!(reduce, Reduce::Min) {
                let top = e.k(u32::MAX)?;
                let seen = e.sel(lead, value, top)?;
                e.min(best, seen)?
            } else {
                let seen = e.sel(lead, value, none)?;
                e.max(best, seen)?
            };
            if let Some(slot) = out.first_mut() {
                *slot = next.id();
            }
        }
        Reduce::Moments => {
            let set = e.ne(value, none)?;
            let set = e.and(set, lead)?;
            let add = e.sel(set, cells, none)?;
            let count = pair(&e, state, 0)?;
            let count = e.wide_add(count, [add, none])?;
            put_pair(&mut out, 0, count);
            for axis in 0..3 {
                let start = low
                    .get(axis)
                    .copied()
                    .ok_or(LaneError::NoOpenBlock { arm: "axis" })?;
                let along = many
                    .get(axis)
                    .copied()
                    .ok_or(LaneError::NoOpenBlock { arm: "axis" })?;
                let across = plane
                    .get(axis)
                    .copied()
                    .ok_or(LaneError::NoOpenBlock { arm: "axis" })?;
                let first = e.mul_small(start, along)?;
                let one = e.k(1)?;
                let less = e.sub(along, one)?;
                let steps = e.mul(along, less)?;
                let steps = e.shr(steps, 1)?;
                let line = e.wide_add(first, [steps, none])?;
                let total = e.wide_mul_small(line, across)?;
                let add = e.masked_pair(set, total)?;
                let sum = pair(&e, state, 2 + 2 * axis)?;
                let sum = e.wide_add(sum, add)?;
                put_pair(&mut out, 2 + 2 * axis, sum);
            }
        }
        Reduce::Differ(_) | Reduce::Histogram { .. } => {}
    }
    Ok(out)
}

fn counted<const L: u32>(
    kernel: &mut Kernel<U32>,
    given: &Given,
    state: &[Id],
    which: usize,
    add: V<L>,
) -> Result<Vec<Id>, LaneError> {
    let mut out = state.to_vec();
    if !given.shape.counting {
        return Ok(out);
    }
    let at = accumulators(given.shape.reduce) + 2 * which;
    let mut lanes = kernel.lanes()?;
    let mut e = E::<L>(&mut lanes);
    let none = e.k(0)?;
    let sum = pair(&e, state, at)?;
    let sum = e.wide_add(sum, [add, none])?;
    put_pair(&mut out, at, sum);
    Ok(out)
}

fn target_at<const L: u32>(
    e: &mut E<'_, '_, L>,
    given: &Given,
    point: [V<L>; 3],
    inside: V<L>,
) -> Result<V<L>, LaneError> {
    let [x, y, z] = point;
    let [from_x, from_y, from_z] = given.from;
    let [wide, deep, _] = given.sides;
    let (from_x, from_y, from_z) = (e.v(from_x)?, e.v(from_y)?, e.v(from_z)?);
    let (wide, deep) = (e.v(wide)?, e.v(deep)?);
    let dz = e.sub(z, from_z)?;
    let dy = e.sub(y, from_y)?;
    let dx = e.sub(x, from_x)?;
    let rows = e.mul(dz, deep)?;
    let rows = e.add(rows, dy)?;
    let cells = e.mul(rows, wide)?;
    let at = e.add(cells, dx)?;
    let none = e.k(0)?;
    e.sel(inside, at, none)
}

fn aim<const L: u32>(
    kernel: &mut Kernel<U32>,
    given: &Given,
    point: [V<L>; 3],
    inside: V<L>,
) -> Result<Option<Id>, LaneError> {
    if !matches!(given.shape.reduce, Reduce::Differ(_)) {
        return Ok(None);
    }
    let at = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        target_at(&mut e, given, point, inside)?
    };
    Ok(Some(kernel.load_at(given.shape.target, at.id())?))
}

fn within<const L: u32>(
    e: &mut E<'_, '_, L>,
    given: &Given,
    point: [V<L>; 3],
) -> Result<V<L>, LaneError> {
    let mut inside = e.k(1)?;
    for ((&at, &from), &to) in point.iter().zip(&given.from).zip(&given.to) {
        let from = e.v(from)?;
        let to = e.v(to)?;
        let below = e.lt(at, from)?;
        let above = e.lt(at, to)?;
        let low = e.not(below)?;
        inside = e.and(inside, low)?;
        inside = e.and(inside, above)?;
    }
    Ok(inside)
}

fn sparse_unit<const L: u32, M: Map>(
    kernel: &mut Kernel<U32>,
    map: &M,
    given: &Given,
    params: &Params,
    state: &[Id],
    unit: V<L>,
) -> Result<Vec<Id>, LaneError> {
    let Layout::Sparse(bricks) = given.shape.layout else {
        return Ok(state.to_vec());
    };
    let shape = given.shape;
    let sides = bricks.sides();
    let [side_x, side_y, _] = sides;
    let side_bits = sides.map(u32::trailing_zeros);
    let [bits_x, bits_y, _] = side_bits;
    let [lattice_x, lattice_y, _] = bricks.lattice().map(u32::trailing_zeros);
    let [span_x, span_y, _] = params.span;
    let cells = bricks.cells() as u32;
    let table = bricks.table();
    let (inside, entry, origin) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let units = e.v(params.units)?;
        let inside = e.lt(unit, units)?;
        let none = e.k(0)?;
        let unit = e.sel(inside, unit, none)?;
        let span_x = e.v(span_x)?;
        let span_y = e.v(span_y)?;
        let across = e.div(unit, span_x)?;
        let back = e.mul(across, span_x)?;
        let ux = e.sub(unit, back)?;
        let uz = e.div(across, span_y)?;
        let back = e.mul(uz, span_y)?;
        let uy = e.sub(across, back)?;
        let mut lattice = [ux, uy, uz];
        for (value, &first) in lattice.iter_mut().zip(&params.first) {
            let first = e.v(first)?;
            *value = e.add(*value, first)?;
        }
        let [bx, by, bz] = lattice;
        let y_part = e.shl(by, lattice_x)?;
        let z_part = e.shl(bz, lattice_x + lattice_y)?;
        let brick = e.add(bx, y_part)?;
        let brick = e.add(brick, z_part)?;
        let four = e.k(ENTRY as u32)?;
        let entry = e.mul(brick, four)?;
        let mut origin = lattice;
        for (value, bits) in origin.iter_mut().zip(side_bits) {
            *value = e.shl(*value, bits)?;
        }
        (inside, entry, origin)
    };
    let slot = kernel.load_at(table, entry.id())?;
    let value_at = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let one = e.k(1)?;
        e.add(entry, one)?
    };
    let value = kernel.load_at(table, value_at.id())?;
    let (held, uniform, low, many, count_held, count_uniform, lead, first_lane) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let slot = e.v(slot)?;
        let marker = e.k(UNIFORM)?;
        let held = e.ne(slot, marker)?;
        let held = e.and(held, inside)?;
        let free = e.not(held)?;
        let uniform = e.and(free, inside)?;
        let mut low = origin;
        let mut many = origin;
        for ((((slot, count), &start), &side), (&from, &to)) in low
            .iter_mut()
            .zip(many.iter_mut())
            .zip(&origin)
            .zip(&sides)
            .zip(given.from.iter().zip(&given.to))
        {
            let side = e.k(side)?;
            let end = e.add(start, side)?;
            let from = e.v(from)?;
            let to = e.v(to)?;
            let first = e.max(start, from)?;
            let last = e.min(end, to)?;
            *slot = first;
            *count = e.sub(last, first)?;
        }
        let per = e.k(cells / shape.subgroup)?;
        let none = e.k(0)?;
        let count_held = e.sel(held, per, none)?;
        let walked = if shape.closed() { none } else { uniform };
        let count_uniform = e.sel(walked, per, none)?;
        let lane = e.v(given.lane)?;
        let first_lane = e.eq(lane, none)?;
        let lead = if shape.closed() {
            e.and(uniform, first_lane)?
        } else {
            none
        };
        (
            held,
            uniform,
            low,
            many,
            count_held,
            count_uniform,
            lead,
            first_lane,
        )
    };
    let _ = (held, uniform);
    let table_read = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        e.and(first_lane, inside)?
    };
    let mut out = counted::<L>(kernel, given, state, 1, table_read)?;
    for pass in [true, false] {
        let limit = if pass { count_held } else { count_uniform };
        out = looped(kernel, limit.id(), &out, |kernel, carried, step| {
            let (point, inside, address) = {
                let mut lanes = kernel.lanes()?;
                let mut e = E::<L>(&mut lanes);
                let step = e.v(step)?;
                let width = e.k(shape.subgroup)?;
                let lane = e.v(given.lane)?;
                let offset = e.mul(step, width)?;
                let offset = e.add(offset, lane)?;
                let px = e.masked(offset, side_x - 1)?;
                let py = e.shr(offset, bits_x)?;
                let py = e.masked(py, side_y - 1)?;
                let pz = e.shr(offset, bits_x + bits_y)?;
                let mut point = origin;
                for (value, part) in point.iter_mut().zip([px, py, pz]) {
                    *value = e.add(*value, part)?;
                }
                let inside = within(&mut e, given, point)?;
                let slot = e.v(slot)?;
                let per = e.k(cells)?;
                let base = e.mul(slot, per)?;
                (point, inside, e.add(base, offset)?)
            };
            let word = if pass {
                state_load(kernel, shape.layout.windows(), address.id())?
            } else {
                value
            };
            let aimed = aim::<L>(kernel, given, point, inside)?;
            let next =
                contributed::<L, M>(kernel, map, given, carried, word, point, inside, aimed)?;
            if pass {
                let one = kernel.lanes()?.splat_bits::<U32, L>(1)?;
                counted::<L>(kernel, given, &next, 0, one)
            } else {
                Ok(next)
            }
        })?;
    }
    if shape.closed() {
        out = closed::<L, M>(kernel, map, given, &out, value, low, many, lead)?;
    }
    Ok(out)
}

fn dense_unit<const L: u32, M: Map>(
    kernel: &mut Kernel<U32>,
    map: &M,
    given: &Given,
    params: &Params,
    state: &[Id],
    unit: V<L>,
) -> Result<Vec<Id>, LaneError> {
    let Layout::Dense { grid, windows } = given.shape.layout else {
        return Ok(state.to_vec());
    };
    let shape = given.shape;
    let wide_bits = grid.wide.trailing_zeros();
    let deep_bits = grid.deep.trailing_zeros();
    let (limit, y, z) = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let units = e.v(params.units)?;
        let inside = e.lt(unit, units)?;
        let none = e.k(0)?;
        let unit = e.sel(inside, unit, none)?;
        let [_, deep, _] = given.sides;
        let deep = e.v(deep)?;
        let rz = e.div(unit, deep)?;
        let back = e.mul(rz, deep)?;
        let ry = e.sub(unit, back)?;
        let [_, from_y, from_z] = given.from;
        let from_y = e.v(from_y)?;
        let from_z = e.v(from_z)?;
        let inner = e.v(params.inner)?;
        (
            e.sel(inside, inner, none)?,
            e.add(ry, from_y)?,
            e.add(rz, from_z)?,
        )
    };
    looped(kernel, limit.id(), state, |kernel, carried, step| {
        let (point, inside, address) = {
            let mut lanes = kernel.lanes()?;
            let mut e = E::<L>(&mut lanes);
            let step = e.v(step)?;
            let width = e.k(shape.subgroup)?;
            let lane = e.v(given.lane)?;
            let [from_x, _, _] = given.from;
            let [to_x, _, _] = given.to;
            let from_x = e.v(from_x)?;
            let to_x = e.v(to_x)?;
            let x = e.mul(step, width)?;
            let x = e.add(x, lane)?;
            let x = e.add(x, from_x)?;
            let inside = e.lt(x, to_x)?;
            let x = e.sel(inside, x, from_x)?;
            let z_part = e.shl(z, wide_bits + deep_bits)?;
            let y_part = e.shl(y, wide_bits)?;
            let address = e.add(z_part, y_part)?;
            let address = e.add(address, x)?;
            ([x, y, z], inside, address)
        };
        let word = state_load(kernel, windows, address.id())?;
        let aimed = aim::<L>(kernel, given, point, inside)?;
        let next = contributed::<L, M>(kernel, map, given, carried, word, point, inside, aimed)?;
        let one = kernel.lanes()?.splat_bits::<U32, L>(1)?;
        counted::<L>(kernel, given, &next, 0, one)
    })
}

struct Params {
    units: Id,
    per: Id,
    groups: Id,
    inner: Id,
    first: [Id; 3],
    span: [Id; 3],
}

fn loaded(kernel: &mut Kernel<U32>, ask: u32, at: usize) -> Result<Id, LaneError> {
    let index = kernel.module().constant_u32(at as u32)?;
    kernel.load_at(ask, index)
}

fn finished<const L: u32>(
    kernel: &mut Kernel<U32>,
    given: &Given,
    state: &[Id],
) -> Result<(), LaneError> {
    let shape = given.shape;
    let ask = shape.ask;
    let reduce = shape.reduce;
    let first_lane = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let lane = e.v(given.lane)?;
        let none = e.k(0)?;
        let first = e.eq(lane, none)?;
        e.truth(first)?
    };
    let mut pairs = Vec::new();
    match reduce {
        Reduce::Count | Reduce::Sum | Reduce::Differ(_) => pairs.push((0, RESULT)),
        Reduce::Moments => {
            for at in 0..4 {
                pairs.push((2 * at, RESULT + 2 * at));
            }
        }
        Reduce::Min | Reduce::Max => {
            let best = state.first().copied().ok_or(LaneError::BadCarry {
                given: state.len(),
                wanted: 1,
            })?;
            let total = {
                let mut lanes = kernel.lanes()?;
                let best = lanes.splat_id::<U32, L>(best)?;
                if matches!(reduce, Reduce::Min) {
                    lanes.reduce_min::<U32, L>(best)?
                } else {
                    lanes.reduce_max::<U32, L>(best)?
                }
            };
            let opcode = if matches!(reduce, Reduce::Min) {
                ATOMIC_U_MIN
            } else {
                ATOMIC_U_MAX
            };
            lane_if(kernel, first_lane, |kernel| {
                atomic_extreme(kernel, ask, RESULT as u32, opcode, total)
            })?;
        }
        Reduce::Histogram { .. } => {
            let (flush, at, run) = {
                let mut lanes = kernel.lanes()?;
                let mut e = E::<L>(&mut lanes);
                let cur = e.v(state.first().copied().ok_or(LaneError::BadCarry {
                    given: state.len(),
                    wanted: 3,
                })?)?;
                let run = pair(&e, state, 1)?;
                let [low, high] = run;
                let either = e.0.or(low, high)?;
                let none = e.k(0)?;
                let held = e.ne(either, none)?;
                let two = e.k(2)?;
                let doubled = e.mul(cur, two)?;
                let base = e.v(given.copy)?;
                (e.truth(held)?, e.add(base, doubled)?, run)
            };
            lane_if(kernel, flush, |kernel| {
                atomic_wide::<L>(kernel, ask, at, run)
            })?;
        }
    }
    if shape.counting {
        let past = accumulators(reduce);
        let result = RESULT + reduce.words();
        pairs.push((past, result));
        pairs.push((past + 2, result + 2));
    }
    for (from, to) in pairs {
        let (low, high, at) = {
            let mut lanes = kernel.lanes()?;
            let [low, high] = {
                let e = E::<L>(&mut lanes);
                pair(&e, state, from)?
            };
            let mut e = E::<L>(&mut lanes);
            let bottom = e.masked(low, 0xFFFF)?;
            let top = e.shr(low, 16)?;
            let bottom = e.0.reduce_sum::<U32, L>(bottom)?;
            let top = e.0.reduce_sum::<U32, L>(top)?;
            let high = e.0.reduce_sum::<U32, L>(high)?;
            let bottom = e.v(bottom)?;
            let top = e.v(top)?;
            let high = e.v(high)?;
            let lifted = e.shl(top, 16)?;
            let low = e.add(bottom, lifted)?;
            let carry = e.lt(low, bottom)?;
            let spilled = e.shr(top, 16)?;
            let high = e.add(high, spilled)?;
            let high = e.add(high, carry)?;
            (low, high, e.k(to as u32)?)
        };
        lane_if(kernel, first_lane, |kernel| {
            atomic_wide::<L>(kernel, ask, at, [low, high])
        })?;
    }
    Ok(())
}

fn emitted<const L: u32, M: Map>(map: &M, shape: &Shape) -> Result<Vec<u32>, LaneError> {
    let mut kernel =
        Kernel::<U32>::new(Laid::new(shape.subgroup, shape.subgroup, shape.bindings()))?;
    let ask = shape.ask;
    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let mut ids = [lane; SIDES + 3];
    for (at, slot) in ids.iter_mut().enumerate() {
        *slot = loaded(&mut kernel, ask, at)?;
    }
    let pick = |at: usize| ids.get(at).copied().unwrap_or(lane);
    let params = Params {
        units: pick(UNITS),
        per: pick(PER),
        groups: pick(GROUPS),
        inner: pick(INNER),
        first: [pick(FIRST), pick(FIRST + 1), pick(FIRST + 2)],
        span: [pick(SPAN), pick(SPAN + 1), pick(SPAN + 2)],
    };
    let bins = match shape.reduce {
        Reduce::Histogram { bins } => bins,
        _ => 0,
    };
    let copy = {
        let mut lanes = kernel.lanes()?;
        let mut e = E::<L>(&mut lanes);
        let group = e.v(group)?;
        let which = e.masked(group, COPIES as u32 - 1)?;
        let stride = e.k(2 * (bins + 1))?;
        let offset = e.mul(which, stride)?;
        let base = e.k(RESULT as u32)?;
        e.add(base, offset)?.id()
    };
    let given = Given {
        shape: *shape,
        lane,
        from: [pick(FROM), pick(FROM + 1), pick(FROM + 2)],
        to: [pick(TO), pick(TO + 1), pick(TO + 2)],
        sides: [pick(SIDES), pick(SIDES + 1), pick(SIDES + 2)],
        bins,
        copy,
    };
    let state = start(&mut kernel, shape)?;
    let state = looped(&mut kernel, params.per, &state, |kernel, carried, step| {
        let unit = {
            let mut lanes = kernel.lanes()?;
            let mut e = E::<L>(&mut lanes);
            let step = e.v(step)?;
            let groups = e.v(params.groups)?;
            let group = e.v(group)?;
            let along = e.mul(step, groups)?;
            e.add(group, along)?
        };
        match shape.layout {
            Layout::Dense { .. } => dense_unit::<L, M>(kernel, map, &given, &params, carried, unit),
            Layout::Sparse(_) => sparse_unit::<L, M>(kernel, map, &given, &params, carried, unit),
        }
    })?;
    finished::<L>(&mut kernel, &given, &state)?;
    kernel.finish()
}
