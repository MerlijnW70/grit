//! A scalar view of a kernel: one value per invocation, written one SPIR-V
//! instruction at a time, with the loops and branches the renderer needs.

use simdr::kernel::Kernel;
use simdr::lanes::{LaneError, U32};
use simdr::module::{Id, op};
use simdr::spec::{Glsl, LoopControl, SelectionControl};

use crate::bricks::UNIFORM;
use crate::layout::Windows;

use super::{Build, Sparse};

const LOGICAL_NOT: u16 = 168;

/// A branch [`Em::open`] opened: the block before it and the block it
/// merges into.
#[derive(Debug, Clone, Copy)]
pub struct Opened {
    before: Id,
    merge: Id,
}

/// The kernel being written, with the scalar types it uses.
pub struct Em<'held> {
    pub kernel: &'held mut Kernel<U32>,
    pub uint: Id,
    pub float: Id,
    pub truth: Id,
    glsl: Id,
    state: Windows,
    seen: Option<Seen>,
}

#[derive(Debug, Clone, Copy)]
struct Seen {
    sparse: Sparse,
    window: [u32; 3],
    origin: [Id; 3],
}

#[derive(Debug, Clone, Copy)]
pub struct Entry {
    pub held: Id,
    pub entry: Id,
    pub value: Id,
    pub offset: Id,
}

impl<'held> Em<'held> {
    pub fn new(kernel: &'held mut Kernel<U32>) -> Result<Self, LaneError> {
        let uint = kernel.index_type();
        let float = kernel.module().type_float(32)?;
        let truth = kernel.module().type_bool()?;
        let glsl = kernel.module().ext_inst_import(Glsl::SET_NAME)?;
        Ok(Self {
            kernel,
            uint,
            float,
            truth,
            glsl,
            state: Windows::of(0),
            seen: None,
        })
    }

    pub fn read_by(kernel: &'held mut Kernel<U32>, build: &Build) -> Result<Self, LaneError> {
        let mut em = Self::reading(kernel, build.windows)?;
        if let Some(sparse) = build.sparse {
            let mut origin = [em.u(0)?; 3];
            for (axis, slot) in origin.iter_mut().enumerate() {
                *slot = em.load_u(sparse.table, sparse.origin + axis as u32)?;
            }
            let grid = build.grid;
            em.seen = Some(Seen {
                sparse,
                window: [grid.wide, grid.deep, grid.tall].map(u32::trailing_zeros),
                origin,
            });
        }
        Ok(em)
    }

    #[must_use]
    pub const fn sparse(&self) -> Option<Sparse> {
        match self.seen {
            Some(seen) => Some(seen.sparse),
            None => None,
        }
    }

    #[must_use]
    pub fn origin(&self) -> Option<[Id; 3]> {
        self.seen.map(|seen| seen.origin)
    }

    fn point(&mut self, seen: Seen, at: Id) -> Result<[Id; 3], LaneError> {
        let [wide, deep, _] = seen.window;
        let x = self.and_u(at, (1 << wide) - 1)?;
        let y = self.shr(at, wide)?;
        let y = self.and_u(y, (1 << deep) - 1)?;
        let z = self.shr(at, wide + deep)?;
        self.placed(seen, [x, y, z])
    }

    fn placed(&mut self, seen: Seen, local: [Id; 3]) -> Result<[Id; 3], LaneError> {
        let sides = seen.sparse.sides();
        let mut out = local;
        for ((slot, origin), side) in out.iter_mut().zip(seen.origin).zip(sides) {
            let moved = self.add(*slot, origin)?;
            *slot = self.and_u(moved, side - 1)?;
        }
        Ok(out)
    }

    pub fn world_point(&mut self, local: [Id; 3]) -> Result<Option<[Id; 3]>, LaneError> {
        match self.seen {
            Some(seen) => Ok(Some(self.placed(seen, local)?)),
            None => Ok(None),
        }
    }

    pub fn entry(&mut self, point: [Id; 3]) -> Result<Option<Entry>, LaneError> {
        let Some(seen) = self.seen else {
            return Ok(None);
        };
        let sparse = seen.sparse;
        let lattice = sparse.lattice.map(u32::trailing_zeros);
        let mut brick = self.u(0)?;
        let mut offset = self.u(0)?;
        let mut brick_shift = 0;
        let mut cell_shift = 0;
        for ((at, bits), across) in point.into_iter().zip(sparse.bits).zip(lattice) {
            let high = self.shr(at, bits + 1)?;
            let high = self.shl(high, brick_shift)?;
            brick = self.or(brick, high)?;
            let low = self.and_u(at, (2 << bits) - 1)?;
            let low = self.shl(low, cell_shift)?;
            offset = self.or(offset, low)?;
            brick_shift += across;
            cell_shift += bits + 1;
        }
        let at = self.shl(brick, 2)?;
        let entry = self.load(sparse.table, at)?;
        let one = self.u(1)?;
        let at = self.add(at, one)?;
        let value = self.load(sparse.table, at)?;
        let held = self.ne_u(entry, UNIFORM)?;
        Ok(Some(Entry {
            held,
            entry,
            value,
            offset,
        }))
    }

    pub fn held_cell(&mut self, entry: Entry) -> Result<Id, LaneError> {
        let Some(seen) = self.seen else {
            return Ok(entry.value);
        };
        let bits = seen.sparse.cell_bits();
        let state = self.state;
        let out = self.when(entry.held, &[entry.value], |em| {
            let slot = em.shl(entry.entry, bits)?;
            let at = em.or(slot, entry.offset)?;
            Ok(vec![crate::live::state_load(em.kernel, state, at)?])
        })?;
        Ok(out.first().copied().unwrap_or(entry.value))
    }

    /// [`Em::new`] for a kernel that reads `STATE` laid out as `state`.
    pub fn reading(kernel: &'held mut Kernel<U32>, state: Windows) -> Result<Self, LaneError> {
        let mut em = Self::new(kernel)?;
        em.state = state;
        Ok(em)
    }

    /// Word `at` of `STATE`, from the window that holds it.
    pub fn cell(&mut self, at: Id) -> Result<Id, LaneError> {
        let Some(seen) = self.seen else {
            return crate::live::state_load(self.kernel, self.state, at);
        };
        let point = self.point(seen, at)?;
        match self.entry(point)? {
            Some(entry) => self.held_cell(entry),
            None => crate::live::state_load(self.kernel, self.state, at),
        }
    }

    pub fn u(&mut self, value: u32) -> Result<Id, LaneError> {
        Ok(self.kernel.module().constant_u32(value)?)
    }

    pub fn f(&mut self, value: f32) -> Result<Id, LaneError> {
        Ok(self.kernel.module().constant_f32(value)?)
    }

    fn two(&mut self, opcode: u16, kind: Id, left: Id, right: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().binary(opcode, kind, left, right)?)
    }

    fn one(&mut self, opcode: u16, kind: Id, value: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().unary(opcode, kind, value)?)
    }

    pub fn add(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::I_ADD, self.uint, left, right)
    }

    pub fn sub(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::I_SUB, self.uint, left, right)
    }

    pub fn mul(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::I_MUL, self.uint, left, right)
    }

    pub fn div(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::U_DIV, self.uint, left, right)
    }

    pub fn shl(&mut self, value: Id, by: u32) -> Result<Id, LaneError> {
        let by = self.u(by)?;
        self.two(op::SHIFT_LEFT_LOGICAL, self.uint, value, by)
    }

    pub fn shr(&mut self, value: Id, by: u32) -> Result<Id, LaneError> {
        let by = self.u(by)?;
        self.two(op::SHIFT_RIGHT_LOGICAL, self.uint, value, by)
    }

    pub fn shr_by(&mut self, value: Id, by: Id) -> Result<Id, LaneError> {
        self.two(op::SHIFT_RIGHT_LOGICAL, self.uint, value, by)
    }

    pub fn and(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::BITWISE_AND, self.uint, left, right)
    }

    pub fn and_u(&mut self, left: Id, right: u32) -> Result<Id, LaneError> {
        let right = self.u(right)?;
        self.and(left, right)
    }

    pub fn or(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::BITWISE_OR, self.uint, left, right)
    }

    pub fn xor(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::BITWISE_XOR, self.uint, left, right)
    }

    pub fn lt(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::U_LESS_THAN, self.truth, left, right)
    }

    pub fn le(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::U_LESS_THAN_EQUAL, self.truth, left, right)
    }

    pub fn ge(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::U_GREATER_THAN_EQUAL, self.truth, left, right)
    }

    pub fn eq(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::I_EQUAL, self.truth, left, right)
    }

    pub fn ne(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::I_NOT_EQUAL, self.truth, left, right)
    }

    pub fn ne_u(&mut self, left: Id, right: u32) -> Result<Id, LaneError> {
        let right = self.u(right)?;
        self.ne(left, right)
    }

    pub fn both(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().logical_and(self.truth, left, right)?)
    }

    pub fn either(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().logical_or(self.truth, left, right)?)
    }

    pub fn not(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(LOGICAL_NOT, self.truth, value)
    }

    pub fn pick(&mut self, when: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().select(self.uint, when, yes, no)?)
    }

    pub fn pick_f(&mut self, when: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().select(self.float, when, yes, no)?)
    }

    pub fn pick_t(&mut self, when: Id, yes: Id, no: Id) -> Result<Id, LaneError> {
        Ok(self.kernel.module().select(self.truth, when, yes, no)?)
    }

    /// 1 when `value` holds, else 0.
    pub fn flag(&mut self, value: Id) -> Result<Id, LaneError> {
        let one = self.u(1)?;
        let none = self.u(0)?;
        self.pick(value, one, none)
    }

    /// Whether bit `bit` of `mask` is set.
    pub fn bit(&mut self, mask: Id, bit: Id) -> Result<Id, LaneError> {
        let moved = self.shr_by(mask, bit)?;
        let low = self.and_u(moved, 1)?;
        self.ne_u(low, 0)
    }

    pub fn fadd(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_ADD, self.float, left, right)
    }

    pub fn fsub(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_SUB, self.float, left, right)
    }

    pub fn fmul(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_MUL, self.float, left, right)
    }

    pub fn fdiv(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_DIV, self.float, left, right)
    }

    pub fn flt(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_ORD_LESS_THAN, self.truth, left, right)
    }

    pub fn fgt(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.two(op::F_ORD_GREATER_THAN, self.truth, left, right)
    }

    fn glsl(&mut self, which: Glsl, operands: &[Id]) -> Result<Id, LaneError> {
        let (float, set) = (self.float, self.glsl);
        Ok(self
            .kernel
            .module()
            .ext_inst(float, set, which.word(), operands)?)
    }

    pub fn fmin(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::FMin, &[left, right])
    }

    pub fn fmax(&mut self, left: Id, right: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::FMax, &[left, right])
    }

    pub fn clamp01(&mut self, value: Id) -> Result<Id, LaneError> {
        let low = self.f(0.0)?;
        let high = self.f(1.0)?;
        self.glsl(Glsl::FClamp, &[value, low, high])
    }

    pub fn exp(&mut self, value: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::Exp, &[value])
    }

    pub fn pow(&mut self, value: Id, power: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::Pow, &[value, power])
    }

    pub fn sqrt(&mut self, value: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::Sqrt, &[value])
    }

    pub fn log(&mut self, value: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::Log, &[value])
    }

    /// `value` as a float, read as unsigned.
    pub fn to_float(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::CONVERT_U_TO_F, self.float, value)
    }

    /// `value` as a float, read as a two's complement signed word.
    pub fn signed_float(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::CONVERT_S_TO_F, self.float, value)
    }

    /// `value`, from 0 up, as a word, truncated.
    pub fn to_word(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::CONVERT_F_TO_U, self.uint, value)
    }

    /// `value` as a two's complement word, truncated.
    pub fn to_signed_word(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::CONVERT_F_TO_S, self.uint, value)
    }

    /// The bits of the float `value`, as a word.
    pub fn float_bits(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::BITCAST, self.uint, value)
    }

    pub fn floor(&mut self, value: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::Floor, &[value])
    }

    pub fn fabs(&mut self, value: Id) -> Result<Id, LaneError> {
        self.glsl(Glsl::FAbs, &[value])
    }

    /// The float whose bits `value` holds.
    pub fn bits_float(&mut self, value: Id) -> Result<Id, LaneError> {
        self.one(op::BITCAST, self.float, value)
    }

    pub fn load(&mut self, binding: u32, index: Id) -> Result<Id, LaneError> {
        self.kernel.load_at(binding, index)
    }

    pub fn load_u(&mut self, binding: u32, index: u32) -> Result<Id, LaneError> {
        let index = self.u(index)?;
        self.kernel.load_at(binding, index)
    }

    pub fn load_f(&mut self, binding: u32, index: u32) -> Result<Id, LaneError> {
        let bits = self.load_u(binding, index)?;
        self.bits_float(bits)
    }

    pub fn store(&mut self, binding: u32, index: Id, value: Id) -> Result<(), LaneError> {
        self.kernel.store_at(binding, index, value)
    }

    /// The invocation's index across the dispatch.
    pub fn invocation(&mut self, subgroup: u32) -> Result<Id, LaneError> {
        let group = self.kernel.workgroup_index();
        let lane = self.kernel.local_index();
        let width = self.u(subgroup)?;
        let base = self.mul(group, width)?;
        self.add(base, lane)
    }

    /// Runs `body` while the first carried word is not 0, at most `limit`
    /// times, and gives the carried words at the end. `body` is handed the
    /// carried words and the count of runs before this one.
    pub fn looped<F>(&mut self, limit: u32, initial: &[Id], body: F) -> Result<Vec<Id>, LaneError>
    where
        F: FnOnce(&mut Self, &[Id], Id) -> Result<Vec<Id>, LaneError>,
    {
        let (uint, truth) = (self.uint, self.truth);
        let header = self.kernel.module().alloc_id()?;
        let inside = self.kernel.module().alloc_id()?;
        let next = self.kernel.module().alloc_id()?;
        let after = self.kernel.module().alloc_id()?;
        let entry = self.kernel.module().alloc_id()?;
        let none = self.u(0)?;
        let one = self.u(1)?;
        let most = self.u(limit)?;

        self.kernel.module().branch(entry)?;
        self.kernel.module().label_at(entry)?;
        self.kernel.module().branch(header)?;

        self.kernel.module().label_at(header)?;
        let counter = self.kernel.module().alloc_id()?;
        let stepped = self.kernel.module().alloc_id()?;
        let mut carried = Vec::with_capacity(initial.len());
        let mut produced = Vec::with_capacity(initial.len());
        for _ in initial {
            carried.push(self.kernel.module().alloc_id()?);
            produced.push(self.kernel.module().alloc_id()?);
        }
        self.kernel
            .module()
            .phi_at(counter, uint, &[(none, entry), (stepped, next)])?;
        for ((&name, &was), &will) in carried.iter().zip(initial).zip(&produced) {
            self.kernel
                .module()
                .phi_at(name, uint, &[(was, entry), (will, next)])?;
        }
        let alive = carried.first().copied().ok_or(LaneError::BadCarry {
            given: 0,
            wanted: 1,
        })?;
        let under = self
            .kernel
            .module()
            .binary(op::U_LESS_THAN, truth, counter, most)?;
        let going = self
            .kernel
            .module()
            .binary(op::I_NOT_EQUAL, truth, alive, none)?;
        let going = self.kernel.module().logical_and(truth, under, going)?;
        self.kernel
            .module()
            .loop_merge(after, next, LoopControl::None)?;
        self.kernel
            .module()
            .branch_conditional(going, inside, after)?;

        self.kernel.module().label_at(inside)?;
        let result = body(self, &carried, counter)?;
        if result.len() != produced.len() {
            return Err(LaneError::BadCarry {
                given: result.len(),
                wanted: produced.len(),
            });
        }
        for (&will, &value) in produced.iter().zip(&result) {
            self.kernel.module().copy_object_at(will, uint, value)?;
        }
        self.kernel.module().branch(next)?;

        self.kernel.module().label_at(next)?;
        self.kernel.module().i_add_at(stepped, uint, counter, one)?;
        self.kernel.module().branch(header)?;

        self.kernel.module().label_at(after)?;
        Ok(carried)
    }

    /// The words `then` gives when `when` holds, else `otherwise`; `then`
    /// runs only on the invocations where `when` holds.
    pub fn when<F>(&mut self, when: Id, otherwise: &[Id], then: F) -> Result<Vec<Id>, LaneError>
    where
        F: FnOnce(&mut Self) -> Result<Vec<Id>, LaneError>,
    {
        let uint = self.uint;
        let before = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "before" })?;
        let taken = self.kernel.module().alloc_id()?;
        let merge = self.kernel.module().alloc_id()?;
        self.kernel
            .module()
            .selection_merge(merge, SelectionControl::None)?;
        self.kernel
            .module()
            .branch_conditional(when, taken, merge)?;
        self.kernel.module().label_at(taken)?;
        let given = then(self)?;
        if given.len() != otherwise.len() {
            return Err(LaneError::BadCarry {
                given: given.len(),
                wanted: otherwise.len(),
            });
        }
        let finished = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "then" })?;
        self.kernel.module().branch(merge)?;
        self.kernel.module().label_at(merge)?;
        let mut out = Vec::with_capacity(given.len());
        for (&yes, &no) in given.iter().zip(otherwise) {
            out.push(
                self.kernel
                    .module()
                    .phi(uint, &[(yes, finished), (no, before)])?,
            );
        }
        Ok(out)
    }

    /// Opens a branch that runs only on the invocations where `when` holds,
    /// until [`Em::close`].
    pub fn open(&mut self, when: Id) -> Result<Opened, LaneError> {
        let before = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "before" })?;
        let taken = self.kernel.module().alloc_id()?;
        let merge = self.kernel.module().alloc_id()?;
        self.kernel
            .module()
            .selection_merge(merge, SelectionControl::DontFlatten)?;
        self.kernel
            .module()
            .branch_conditional(when, taken, merge)?;
        self.kernel.module().label_at(taken)?;
        Ok(Opened { before, merge })
    }

    /// Closes the branch `opened`: the floats `given` where it ran, else
    /// `otherwise`, which must be made before it was opened.
    pub fn close(
        &mut self,
        opened: Opened,
        given: &[Id],
        otherwise: &[Id],
    ) -> Result<Vec<Id>, LaneError> {
        let float = self.float;
        self.closed(opened, (given, otherwise), float)
    }

    /// [`Em::close`] for words.
    pub fn close_words(
        &mut self,
        opened: Opened,
        given: &[Id],
        otherwise: &[Id],
    ) -> Result<Vec<Id>, LaneError> {
        let uint = self.uint;
        self.closed(opened, (given, otherwise), uint)
    }

    fn closed(
        &mut self,
        opened: Opened,
        (given, otherwise): (&[Id], &[Id]),
        float: Id,
    ) -> Result<Vec<Id>, LaneError> {
        if given.len() != otherwise.len() {
            return Err(LaneError::BadCarry {
                given: given.len(),
                wanted: otherwise.len(),
            });
        }
        let finished = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "then" })?;
        self.kernel.module().branch(opened.merge)?;
        self.kernel.module().label_at(opened.merge)?;
        let mut out = Vec::with_capacity(given.len());
        for (&yes, &no) in given.iter().zip(otherwise) {
            out.push(
                self.kernel
                    .module()
                    .phi(float, &[(yes, finished), (no, opened.before)])?,
            );
        }
        Ok(out)
    }

    /// The words `then` gives where `when` holds and `otherwise` gives
    /// elsewhere; each runs only on its own invocations.
    pub fn branch<F, G>(&mut self, when: Id, then: F, otherwise: G) -> Result<Vec<Id>, LaneError>
    where
        F: FnOnce(&mut Self) -> Result<Vec<Id>, LaneError>,
        G: FnOnce(&mut Self) -> Result<Vec<Id>, LaneError>,
    {
        let uint = self.uint;
        let taken = self.kernel.module().alloc_id()?;
        let other = self.kernel.module().alloc_id()?;
        let merge = self.kernel.module().alloc_id()?;
        self.kernel
            .module()
            .selection_merge(merge, SelectionControl::None)?;
        self.kernel
            .module()
            .branch_conditional(when, taken, other)?;
        self.kernel.module().label_at(taken)?;
        let yes = then(self)?;
        let from_yes = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "then" })?;
        self.kernel.module().branch(merge)?;
        self.kernel.module().label_at(other)?;
        let no = otherwise(self)?;
        let from_no = self
            .kernel
            .module()
            .current_block()
            .ok_or(LaneError::NoOpenBlock { arm: "else" })?;
        self.kernel.module().branch(merge)?;
        if yes.len() != no.len() {
            return Err(LaneError::BadCarry {
                given: yes.len(),
                wanted: no.len(),
            });
        }
        self.kernel.module().label_at(merge)?;
        let mut out = Vec::with_capacity(yes.len());
        for (&a, &b) in yes.iter().zip(&no) {
            out.push(
                self.kernel
                    .module()
                    .phi(uint, &[(a, from_yes), (b, from_no)])?,
            );
        }
        Ok(out)
    }
}
