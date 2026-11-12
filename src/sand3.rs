use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

use crate::geometry::{Emit, Solid};
use crate::layout::{BINDINGS, PARAMS, STATE};
use crate::rule3::{FIRST_PARAM, Oct, Rule};
use crate::sand::{Grid, WALL};

pub use crate::rule3::CORNERS;
pub use crate::rule3::sand::{
    HELD, MINUS_X, MINUS_Z, PLUS_X, PLUS_Z, RUNNING_MINUS_X, RUNNING_MINUS_Z, RUNNING_PLUS_X,
    RUNNING_PLUS_Z, Sand, TIRED, held_of, hold_of, material_of, running_of, settle_oct, tired_of,
    x_first,
};

/// A torus of `wide` columns (x), `deep` rows (y) and `tall` layers (z),
/// each a power of two. Rows are the 2D grid's rows: a grain falls from row
/// `y` to row `y + 1`. Cell `(x, y, z)` lies at `(z * deep + y) * wide + x`,
/// so layer zero is laid out as the 2D grid of the same width and depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid3 {
    pub wide: u32,
    pub deep: u32,
    pub tall: u32,
}

impl Grid3 {
    /// The grid with each side rounded up to a power of two, refused when a
    /// side rounds past `2^31`, rounds below two (no whole oct), or the cells
    /// pass a `u32`.
    #[must_use]
    pub const fn new(wide: u32, deep: u32, tall: u32) -> Option<Self> {
        let Some(wide) = wide.checked_next_power_of_two() else {
            return None;
        };
        let Some(deep) = deep.checked_next_power_of_two() else {
            return None;
        };
        let Some(tall) = tall.checked_next_power_of_two() else {
            return None;
        };
        if wide < 2 || deep < 2 || tall < 2 {
            return None;
        }
        let Some(plane) = wide.checked_mul(deep) else {
            return None;
        };
        if plane.checked_mul(tall).is_none() {
            return None;
        }
        Some(Self { wide, deep, tall })
    }

    #[must_use]
    pub const fn cells(self) -> usize {
        self.wide as usize * self.deep as usize * self.tall as usize
    }

    #[must_use]
    pub const fn octs(self) -> u32 {
        (self.wide / 2) * (self.deep / 2) * (self.tall / 2)
    }

    #[must_use]
    pub const fn index(self, x: u32, y: u32, z: u32) -> usize {
        let x = x & (self.wide - 1);
        let y = y & (self.deep - 1);
        let z = z & (self.tall - 1);
        ((z as usize * self.deep as usize) + y as usize) * self.wide as usize + x as usize
    }

    /// The eight cells of oct `oct` in frame `frame`, as cell indices. Octs
    /// are numbered row-major, x fastest. The oct's least corner is twice its
    /// coordinates plus the phase `frame & 1` on all three axes, wrapped.
    /// Corner `k` sits at `dx = k & 1`, `dz = (k >> 1) & 1`, `dy = k >> 2`:
    /// corners 0 to 3 are the upper layer, 4 to 7 the lower one.
    #[must_use]
    pub const fn corners(self, frame: u32, oct: u32) -> [usize; CORNERS] {
        let phase = frame & 1;
        let across = self.wide / 2;
        let down = self.deep / 2;
        let x = (oct % across) * 2 + phase;
        let y = ((oct / across) % down) * 2 + phase;
        let z = (oct / across / down) * 2 + phase;
        [
            self.index(x, y, z),
            self.index(x + 1, y, z),
            self.index(x, y, z + 1),
            self.index(x + 1, y, z + 1),
            self.index(x, y + 1, z),
            self.index(x + 1, y + 1, z),
            self.index(x, y + 1, z + 1),
            self.index(x + 1, y + 1, z + 1),
        ]
    }
}

/// The lattice point of oct `oct` of `grid`, x, y and z, as
/// [`Grid3::corners`] numbers the octs.
#[must_use]
pub const fn lattice(grid: Grid3, oct: u32) -> [u32; 3] {
    let across = grid.wide / 2;
    let down = grid.deep / 2;
    [oct % across, (oct / across) % down, oct / across / down]
}

/// One frame of the whole torus on the CPU under `rule`: every oct of the
/// frame's phase steps once with [`Rule::step`], given `params` as its
/// parameters (a missing word reads as 0). An oct with a cell past `cells`
/// is not stepped, so the engine makes up no word for a rule.
#[must_use]
pub fn reference<R: Rule>(
    rule: &R,
    grid: Grid3,
    frame: u32,
    cells: &[u32],
    params: &[u32],
) -> Vec<u32> {
    let held: Vec<u32> = (0..rule.params())
        .map(|which| crate::rule3::param(params, which))
        .collect();
    let mut out = cells.to_vec();
    let planes = rule.planes();
    let stride = grid.cells();
    'octs: for oct in 0..grid.octs() {
        let places = grid.corners(frame, oct);
        let mut words = [0; CORNERS];
        for (slot, place) in words.iter_mut().zip(places) {
            let Some(&word) = out.get(place) else {
                continue 'octs;
            };
            *slot = word;
        }
        let oct_of = Oct {
            words,
            frame,
            at: lattice(grid, oct),
            params: &held,
        };
        if planes <= 1 {
            let ruled = rule.step(oct_of);
            for (place, value) in places.into_iter().zip(ruled.out) {
                if let Some(slot) = out.get_mut(place) {
                    *slot = value;
                }
            }
            continue;
        }
        let mut upper = vec![[0_u32; CORNERS]; planes - 1];
        for (plane, words) in (1..).zip(upper.iter_mut()) {
            for (slot, place) in words.iter_mut().zip(places) {
                let Some(&word) = out.get(plane * stride + place) else {
                    continue 'octs;
                };
                *slot = word;
            }
        }
        let planed = rule.step_planes(oct_of, &upper);
        for (place, value) in places.into_iter().zip(planed.ruled.out) {
            if let Some(slot) = out.get_mut(place) {
                *slot = value;
            }
        }
        for (plane, words) in (1..).zip(planed.planes.iter().take(planes - 1)) {
            for (place, &value) in places.into_iter().zip(words) {
                if let Some(slot) = out.get_mut(plane * stride + place) {
                    *slot = value;
                }
            }
        }
    }
    out
}

/// One frame of the whole torus on the CPU: every oct of the frame's phase
/// steps once with [`settle_oct`]. A cell index past `cells` reads as a wall
/// and is never written.
#[must_use]
pub fn settle_reference3(grid: Grid3, frame: u32, cells: &[u32]) -> Vec<u32> {
    let mut walled = cells.to_vec();
    walled.resize(walled.len().max(grid.cells()), WALL);
    let mut out = reference(&Sand, grid, frame, &walled, &[]);
    out.truncate(cells.len());
    out
}

/// Why [`settle`] or [`Spread::of`] refuses a grid that is not solid.
pub const FLAT: &str = "the grid is flat (tall 1), and the 3D kernel steps a solid grid";

/// Why a kernel under a rule is refused when the rule declares no tally
/// count or more than [`crate::rule3::MOST_COUNTS`], or its step gives
/// another number of counts than it declares.
pub const MISCOUNTED: &str = "the rule declares no tally count, more than rule3::MOST_COUNTS, or another number than its step gives";

pub const UNPLANED: &str = "the rule declares no plane, more than rule3::MOST_PLANES, or gives another number of planes than it declares";

pub const PLANES_PAST: &str =
    "the grid's cells times the rule's planes pass the u32 index a kernel addresses STATE with";

/// Why [`settle`] or [`Spread::of`] refuses a device that dispatches no group.
pub const NO_GROUPS: &str = "the device dispatches no workgroup at all";

/// Why [`settle`] or [`Spread::of`] refuses a grid whose sides are not
/// powers of two of at least two, or whose octs pass a `u32`.
pub const UNEVEN: &str = "a side is not a power of two of at least two, or the octs pass a u32";

const OPERATION: &str = "sand3::settle";

const fn refused(because: &'static str) -> LaneError {
    LaneError::NoSuchForm {
        operation: OPERATION,
        because,
    }
}

/// How [`settle`] covers every oct of a grid: `groups` workgroups of
/// `workgroup` invocations, each stepping `per` octs. Invocation `i` steps
/// octs `i + k * stride` for `k` below `per`, with `stride` the invocations
/// of the whole dispatch, so every oct is stepped once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spread {
    pub workgroup: u32,
    pub per: u32,
    pub groups: u32,
}

impl Spread {
    /// The spread of `grid` at a subgroup of `subgroup` on a device that
    /// dispatches at most `most` groups, as `Host::limits().groups` gives it.
    /// A workgroup is one subgroup, or every oct when there are fewer. Each
    /// invocation takes the fewest octs, a power of two, that keep the
    /// groups at or under `most`.
    pub fn of(grid: Grid, subgroup: u32, most: u32) -> Result<Self, LaneError> {
        if grid.tall < 2 {
            return Err(refused(FLAT));
        }
        let sides = [grid.wide, grid.deep, grid.tall];
        if sides
            .iter()
            .any(|side| *side < 2 || !side.is_power_of_two())
        {
            return Err(refused(UNEVEN));
        }
        let Some(octs) = (grid.wide / 2)
            .checked_mul(grid.deep / 2)
            .and_then(|plane| plane.checked_mul(grid.tall / 2))
        else {
            return Err(refused(UNEVEN));
        };
        if most == 0 {
            return Err(refused(NO_GROUPS));
        }
        if !subgroup.is_power_of_two() {
            return Err(LaneError::BadWidth { width: subgroup });
        }
        let workgroup = subgroup.min(octs);
        let invocations = octs / workgroup;
        let per = invocations.div_ceil(most).next_power_of_two();
        Ok(Self {
            workgroup,
            per,
            groups: invocations / per,
        })
    }

    /// The invocations of the whole dispatch.
    #[must_use]
    pub const fn stride(self) -> u32 {
        self.groups * self.workgroup
    }

    /// The dispatch the kernel is built for.
    #[must_use]
    pub const fn dispatch(self) -> crate::Groups {
        crate::Groups::Fixed(self.groups)
    }
}

/// A built [`settle`] kernel and the spread it was built for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whole {
    pub words: Vec<u32>,
    pub spread: Spread,
}

/// The kernel that steps every oct of a solid grid by one frame of
/// [`settle_oct`], the frame read from `PARAMS[0]`: its phase from bit 0 and
/// its axis order from [`x_first`]. Bit for bit it is [`settle_reference3`].
/// `most` is the most groups the device dispatches; the kernel is built so
/// that its [`Spread::dispatch`] stays at or under it.
pub fn settle(subgroup: u32, grid: Grid, most: u32) -> Result<Whole, LaneError> {
    settle_by(&Sand, subgroup, grid, most)
}

/// The kernel that steps every oct of a solid grid by one frame of `rule`,
/// the frame read from `PARAMS[0]` and the rule's parameters from
/// `PARAMS[FIRST_PARAM..]`. Bit for bit it is [`reference()`].
pub fn settle_by<R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    most: u32,
) -> Result<Whole, LaneError> {
    let spread = Spread::of(grid, subgroup, most)?;
    stride_of(rule, grid)?;
    let words = match subgroup {
        4 => whole::<4, R>(rule, subgroup, grid, spread),
        8 => whole::<8, R>(rule, subgroup, grid, spread),
        16 => whole::<16, R>(rule, subgroup, grid, spread),
        32 => whole::<32, R>(rule, subgroup, grid, spread),
        64 => whole::<64, R>(rule, subgroup, grid, spread),
        other => Err(LaneError::NoMapping {
            lanes: other,
            width: subgroup,
        }),
    }?;
    Ok(Whole { words, spread })
}

pub(crate) fn stride_of<R: Rule>(rule: &R, grid: Grid) -> Result<u32, LaneError> {
    let planes = rule.planes();
    if planes == 0 || planes > crate::rule3::MOST_PLANES {
        return Err(LaneError::NoSuchForm {
            operation: "sand3::planes",
            because: UNPLANED,
        });
    }
    let cells = u64::from(grid.wide) * u64::from(grid.deep) * u64::from(grid.tall);
    if planes > 1 && cells * planes as u64 > 1 << 32 {
        return Err(LaneError::NoSuchForm {
            operation: "sand3::planes",
            because: PLANES_PAST,
        });
    }
    Ok(u32::try_from(cells).unwrap_or(u32::MAX))
}

/// Loads the frame, `PARAMS[0]`, and the rule's parameters.
pub(crate) fn frame_and_params<R: Rule>(
    kernel: &mut Kernel<U32>,
    rule: &R,
) -> Result<(Id, Vec<Id>), LaneError> {
    let head = kernel.module().constant_u32(0)?;
    let frame = kernel.load_at(PARAMS, head)?;
    let mut params = Vec::with_capacity(rule.params());
    for which in 0..rule.params() {
        let at = u32::try_from(FIRST_PARAM + which).unwrap_or(u32::MAX);
        let at = kernel.module().constant_u32(at)?;
        params.push(kernel.load_at(PARAMS, at)?);
    }
    Ok((frame, params))
}

fn whole<const LANES: u32, R: Rule>(
    rule: &R,
    subgroup: u32,
    grid: Grid,
    spread: Spread,
) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, spread.workgroup, BINDINGS))?;
    let uint = kernel.index_type();

    let group = kernel.workgroup_index();
    let lane = kernel.local_index();
    let width = kernel.module().constant_u32(spread.workgroup)?;
    let base = kernel.module().i_mul(uint, group, width)?;
    let first = kernel.module().i_add(uint, base, lane)?;

    let (frame, params) = frame_and_params(&mut kernel, rule)?;
    let given = Given {
        frame,
        params: &params,
    };

    if spread.per == 1 {
        step::<LANES, R>(&mut kernel, rule, grid, first, given)?;
    } else {
        let stride = kernel.module().constant_u32(spread.stride())?;
        let _ = kernel.repeat_rolled(spread.per, uint, first, |kernel, oct, _| {
            step::<LANES, R>(kernel, rule, grid, oct, given)?;
            Ok(kernel.module().i_add(uint, oct, stride)?)
        })?;
    }
    kernel.finish()
}

/// The frame and the rule's parameters, as loaded once by a kernel.
#[derive(Clone, Copy)]
pub(crate) struct Given<'held> {
    pub frame: Id,
    pub params: &'held [Id],
}

/// Where each corner of [`Grid3::corners`] lies among the corners of
/// [`Solid::corners`], whose bits run x, y, z where the reference's run
/// x, z, y.
pub(crate) const REFERENCE: [usize; CORNERS] = [0, 1, 4, 5, 2, 3, 6, 7];

type Word<const LANES: u32> = Vector<U32, LANES>;

/// Steps oct `oct` of `grid` under `rule`.
fn step<const LANES: u32, R: Rule>(
    kernel: &mut Kernel<U32>,
    rule: &R,
    grid: Grid,
    oct: Id,
    given: Given<'_>,
) -> Result<(), LaneError> {
    let placing = {
        let mut lanes = kernel.lanes()?;
        placed::<LANES>(&mut lanes, grid, oct, given.frame)?
    };

    let planes = rule.planes();
    if planes > 1 {
        let stride = stride_of(rule, grid)?;
        let mut places = vec![placing.places];
        for plane in 1..planes {
            let lifted = {
                let mut lanes = kernel.lanes()?;
                let by = u32::try_from(plane).unwrap_or(0).wrapping_mul(stride);
                let by = lanes.splat_bits::<U32, LANES>(by)?;
                let mut lifted = placing.places;
                for place in &mut lifted {
                    *place = lanes.add(*place, by)?;
                }
                lifted
            };
            places.push(lifted);
        }
        let mut loaded = Vec::with_capacity(planes);
        for plane in &places {
            let mut words = [given.frame; CORNERS];
            for (word, place) in words.iter_mut().zip(plane) {
                *word = kernel.load_at(STATE, place.id())?;
            }
            loaded.push(words);
        }
        let out = {
            let mut lanes = kernel.lanes()?;
            ruled_planes::<LANES, R>(&mut lanes, rule, &placing, &loaded, given)?
        };
        let mut stored = vec![out.ruled.out];
        stored.extend(out.planes);
        for (plane, values) in places.iter().zip(&stored) {
            for (place, value) in plane.iter().zip(values) {
                kernel.store_at(STATE, place.id(), value.id())?;
            }
        }
        return Ok(());
    }

    let mut loaded = [given.frame; CORNERS];
    for (word, place) in loaded.iter_mut().zip(&placing.places) {
        *word = kernel.load_at(STATE, place.id())?;
    }

    let out = {
        let mut lanes = kernel.lanes()?;
        ruled::<LANES, R>(&mut lanes, rule, &placing, loaded, given)?.out
    };

    for (place, value) in placing.places.iter().zip(&out) {
        kernel.store_at(STATE, place.id(), value.id())?;
    }
    Ok(())
}

/// Where the cells of an oct lie, in the order of [`Grid3::corners`], and
/// the oct's point in the lattice of octs.
pub(crate) struct Placing<const LANES: u32> {
    pub places: [Word<LANES>; CORNERS],
    pub block: [Word<LANES>; 3],
}

/// The cells of oct `oct` of `grid` at frame `frame`, as [`Grid3::corners`]
/// orders them.
pub(crate) fn placed<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    grid: Grid,
    oct: Id,
    frame: Id,
) -> Result<Placing<LANES>, LaneError> {
    let which = lanes.splat_id::<U32, LANES>(oct)?;
    let now = lanes.splat_id::<U32, LANES>(frame)?;
    let solid = Solid::block(&mut Emit::<LANES>(lanes), grid, which, now)?;
    let mut places = [which; CORNERS];
    for (place, from) in places.iter_mut().zip(REFERENCE) {
        if let Some(found) = solid.places.get(from) {
            *place = *found;
        }
    }
    Ok(Placing {
        places,
        block: solid.block,
    })
}

/// The eight words `loaded`, from the places of `placing`, stepped by
/// `rule`.
pub(crate) fn ruled<const LANES: u32, R: Rule>(
    lanes: &mut Lanes<'_>,
    rule: &R,
    placing: &Placing<LANES>,
    loaded: [Id; CORNERS],
    given: Given<'_>,
) -> Result<crate::rule3::Ruled<Word<LANES>>, LaneError> {
    let frame = lanes.splat_id::<U32, LANES>(given.frame)?;
    let mut words = [frame; CORNERS];
    for (word, id) in words.iter_mut().zip(loaded) {
        *word = lanes.splat_id::<U32, LANES>(id)?;
    }
    let mut params = Vec::with_capacity(given.params.len());
    for &id in given.params {
        params.push(lanes.splat_id::<U32, LANES>(id)?);
    }
    let stepped = rule.emit::<LANES>(
        lanes,
        Oct {
            words,
            frame,
            at: placing.block,
            params: &params,
        },
    )?;
    let tally = rule.tally();
    if !tally.fits() || stepped.counts.len() != tally.counts {
        return Err(LaneError::NoSuchForm {
            operation: "sand3::ruled",
            because: MISCOUNTED,
        });
    }
    Ok(stepped)
}

pub(crate) fn ruled_planes<const LANES: u32, R: Rule>(
    lanes: &mut Lanes<'_>,
    rule: &R,
    placing: &Placing<LANES>,
    loaded: &[[Id; CORNERS]],
    given: Given<'_>,
) -> Result<crate::rule3::Planed<Word<LANES>>, LaneError> {
    let frame = lanes.splat_id::<U32, LANES>(given.frame)?;
    let mut splatted = Vec::with_capacity(loaded.len());
    for plane in loaded {
        let mut words = [frame; CORNERS];
        for (word, &id) in words.iter_mut().zip(plane) {
            *word = lanes.splat_id::<U32, LANES>(id)?;
        }
        splatted.push(words);
    }
    let mut params = Vec::with_capacity(given.params.len());
    for &id in given.params {
        params.push(lanes.splat_id::<U32, LANES>(id)?);
    }
    let (first, upper) = splatted.split_first().ok_or(LaneError::NoSuchForm {
        operation: "sand3::ruled_planes",
        because: UNPLANED,
    })?;
    let stepped = rule.emit_planes::<LANES>(
        lanes,
        Oct {
            words: *first,
            frame,
            at: placing.block,
            params: &params,
        },
        upper,
    )?;
    let tally = rule.tally();
    if !tally.fits() || stepped.ruled.counts.len() != tally.counts {
        return Err(LaneError::NoSuchForm {
            operation: "sand3::ruled",
            because: MISCOUNTED,
        });
    }
    if stepped.planes.len() + 1 != rule.planes() || loaded.len() != rule.planes() {
        return Err(LaneError::NoSuchForm {
            operation: "sand3::ruled_planes",
            because: UNPLANED,
        });
    }
    Ok(stepped)
}
