//! Pictures of a solid world, painted into `PIXELS` by a compute pass.
//!
//! A slice shows one layer across an axis; a depth view walks each pixel's
//! line along the axis from that layer on and shows the first cell whose
//! kind is not 0, darker the farther it lies. Both read one `STATE` word a
//! cell, turn it into a kind with a rule's [`Kinds`], and the kind into a
//! colour through the palette the caller writes into `PIXELS` after the
//! picture. The pictures know no material: what a kind looks like is the
//! palette's business.
//!
//! The layer of each axis is a word of `PARAMS`, [`Axis::layer_word`], so
//! moving the layer needs no new pass. The picture is scaled to the view by
//! the nearest cell: every side is a power of two, so a pixel's cell is its
//! coordinate shifted.

use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, Lanes, U32, Vector};
use simdr::module::Id;

use crate::Groups;
use crate::geometry::{Emit, Solid, low_bits};
use crate::layout::{BINDINGS, LOOKING, LOOKING_Z, PARAMS, PIXELS, STATE};
use crate::sand::{Grid, View};

type Word<const LANES: u32> = Vector<U32, LANES>;

/// The colours a palette holds, one a kind.
pub const PALETTE: usize = 16;

/// Why a picture refuses a flat grid.
pub const FLAT: &str = "the grid is flat (tall 1), and a picture of a solid world needs three axes";

/// Why a picture refuses a view too large to address.
pub const TOO_LARGE: &str = "the view's pixels, palette and spare lanes pass a u32";

/// An axis of a solid world: x, y or z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub const ALL: [Self; 3] = [Self::X, Self::Y, Self::Z];

    /// 0 for x, 1 for y, 2 for z.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::X => 0,
            Self::Y => 1,
            Self::Z => 2,
        }
    }

    /// The next axis, z wrapping to x.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::X => Self::Y,
            Self::Y => Self::Z,
            Self::Z => Self::X,
        }
    }

    /// The word of `PARAMS` that holds the layer shown across this axis.
    #[must_use]
    pub const fn layer_word(self) -> usize {
        match self {
            Self::X => LOOKING,
            Self::Y => LOOKING + 1,
            Self::Z => LOOKING_Z,
        }
    }

    /// The axis a row of pixels runs along: z for an x slice, x otherwise.
    #[must_use]
    pub const fn across(self) -> usize {
        match self {
            Self::X => 2,
            Self::Y | Self::Z => 0,
        }
    }

    /// The axis a column of pixels runs down: z for a y slice, y otherwise.
    #[must_use]
    pub const fn down(self) -> usize {
        match self {
            Self::Y => 2,
            Self::X | Self::Z => 1,
        }
    }
}

/// What a picture shows of its layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Look {
    /// The layer itself.
    Slice,
    /// The first cell of kind other than 0 from the layer on, along the
    /// axis, shaded by how far past the layer it lies.
    Depth,
}

impl Look {
    pub const ALL: [Self; 2] = [Self::Slice, Self::Depth];
}

/// How a rule's words map to kinds: `(word >> shift) & mask`, a kind below
/// [`PALETTE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Kinds {
    mask: u32,
    shift: u32,
}

impl Kinds {
    /// The mapping, or none when `mask` reaches [`PALETTE`] or `shift` is
    /// not below 32.
    #[must_use]
    pub const fn new(mask: u32, shift: u32) -> Option<Self> {
        if mask as usize >= PALETTE || shift >= u32::BITS {
            None
        } else {
            Some(Self { mask, shift })
        }
    }

    /// The mapping of the rule the world tools list under `rule`: `sand`
    /// shows its material, `heat` and `nature` their kind and `code` its
    /// instruction.
    #[must_use]
    pub fn named(rule: &str) -> Option<Self> {
        match rule {
            "sand" => Self::new(0x3, 0),
            "heat" => Self::new(0xF, 0),
            "code" => Self::new(0xF, 24),
            "nature" => Self::new(0xF, 0),
            _ => None,
        }
    }

    #[must_use]
    pub const fn mask(self) -> u32 {
        self.mask
    }

    #[must_use]
    pub const fn shift(self) -> u32 {
        self.shift
    }

    /// The kind of `word`.
    #[must_use]
    pub const fn of(self, word: u32) -> u32 {
        (word >> self.shift) & self.mask
    }
}

/// The cells a slice across `axis` shows: along a row of pixels, then down
/// a column.
#[must_use]
pub fn plane(grid: Grid, axis: Axis) -> [u32; 2] {
    let extents = [grid.wide, grid.deep, grid.tall];
    [read(extents, axis.across()), read(extents, axis.down())]
}

/// The cells along `axis`.
#[must_use]
pub fn extent(grid: Grid, axis: Axis) -> u32 {
    read([grid.wide, grid.deep, grid.tall], axis.index())
}

fn read(values: [u32; 3], which: usize) -> u32 {
    values.get(which).copied().unwrap_or(0)
}

fn written(mut values: [u32; 3], which: usize, value: u32) -> [u32; 3] {
    if let Some(slot) = values.get_mut(which) {
        *slot = value;
    }
    values
}

/// The word of `PIXELS` the palette starts at: right after the picture.
#[must_use]
pub const fn palette_at(view: View) -> usize {
    view.pixels()
}

/// The words `PIXELS` holds for a picture of `view`: the picture, the
/// palette, and one spare word for each lane of the last workgroup.
#[must_use]
pub const fn words(subgroup: u32, view: View) -> usize {
    view.pixels() + PALETTE + subgroup as usize
}

/// `sizes` with `PIXELS` grown to [`words`].
#[must_use]
pub fn showing(
    mut sizes: [usize; BINDINGS as usize],
    subgroup: u32,
    view: View,
) -> [usize; BINDINGS as usize] {
    if let Some(held) = sizes.get_mut(PIXELS as usize) {
        *held = words(subgroup, view);
    }
    sizes
}

/// The workgroups a picture of `view` dispatches.
#[must_use]
pub fn groups(subgroup: u32, view: View) -> Groups {
    let lanes = subgroup.max(1) as usize;
    Groups::Fixed(u32::try_from(view.pixels().div_ceil(lanes)).unwrap_or(u32::MAX))
}

/// Every picture, in the order [`pictures`] builds them.
pub const SHOWN: [(Axis, Look); 6] = [
    (Axis::X, Look::Slice),
    (Axis::Y, Look::Slice),
    (Axis::Z, Look::Slice),
    (Axis::X, Look::Depth),
    (Axis::Y, Look::Depth),
    (Axis::Z, Look::Depth),
];

/// Every picture of [`SHOWN`], built for one stage, so a frame picks one by
/// its groups ([`choosing`]) without building anything new.
pub fn pictures(
    subgroup: u32,
    grid: Grid,
    view: View,
    kinds: Kinds,
) -> Result<Vec<Vec<u32>>, LaneError> {
    SHOWN
        .iter()
        .map(|&(axis, look)| picture(subgroup, grid, view, axis, look, kinds))
        .collect()
}

/// The groups of the stage [`pictures`] builds that dispatch only the
/// picture of `axis` and `look`.
#[must_use]
pub fn choosing(subgroup: u32, view: View, axis: Axis, look: Look) -> Vec<Groups> {
    SHOWN
        .iter()
        .map(|&shown| {
            if shown == (axis, look) {
                groups(subgroup, view)
            } else {
                Groups::Fixed(0)
            }
        })
        .collect()
}

const fn scaled(value: u32, from: u32, to: u32) -> u32 {
    if to >= from {
        value << (to - from)
    } else {
        value >> (from - to)
    }
}

/// The cell under pixel `(across, down)` of `view` in the slice across
/// `axis` at `layer`, as x, y and z; the layer wraps onto the axis.
#[must_use]
pub fn cell_under(
    grid: Grid,
    view: View,
    axis: Axis,
    layer: u32,
    (across, down): (u32, u32),
) -> [u32; 3] {
    let [wide, deep] = plane(grid, axis);
    let h = scaled(across, view.wide.trailing_zeros(), wide.trailing_zeros()) & (wide - 1);
    let v = scaled(down, view.deep.trailing_zeros(), deep.trailing_zeros()) & (deep - 1);
    let point = written([0; 3], axis.index(), layer & (extent(grid, axis) - 1));
    let point = written(point, axis.across(), h);
    written(point, axis.down(), v)
}

/// `colour` darkened for a cell `depth` cells past the layer, along an axis
/// of `2^bits` cells: each channel is scaled by `256 - depth * 128 / 2^bits`
/// over 256, and the top byte is dropped.
#[must_use]
pub const fn shaded(colour: u32, depth: u32, bits: u32) -> u32 {
    let keep = 256 - ((depth << 7) >> bits);
    let red = (((colour >> 16) & 0xFF) * keep) >> 8;
    let green = (((colour >> 8) & 0xFF) * keep) >> 8;
    let blue = ((colour & 0xFF) * keep) >> 8;
    (red << 16) | (green << 8) | blue
}

/// The pass that paints the picture of `axis` and `look` into `PIXELS`.
/// It refuses a flat grid and a view whose [`words`] pass a `u32`.
pub fn picture(
    subgroup: u32,
    grid: Grid,
    view: View,
    axis: Axis,
    look: Look,
    kinds: Kinds,
) -> Result<Vec<u32>, LaneError> {
    Picture {
        grid,
        view,
        axis,
        look,
        kinds,
    }
    .built(subgroup)
}

/// One picture: the world it shows, the view it fills, the axis and look,
/// and the rule's kinds.
#[derive(Debug, Clone, Copy)]
pub struct Picture {
    pub grid: Grid,
    pub view: View,
    pub axis: Axis,
    pub look: Look,
    pub kinds: Kinds,
}

impl Picture {
    /// The pass, as [`picture`] builds it.
    pub fn built(self, subgroup: u32) -> Result<Vec<u32>, LaneError> {
        let Self { grid, view, .. } = self;
        if grid.tall < 2 {
            return Err(LaneError::NoSuchForm {
                operation: "view3::picture",
                because: FLAT,
            });
        }
        if u32::try_from(words(subgroup, view)).is_err() {
            return Err(LaneError::NoSuchForm {
                operation: "view3::picture",
                because: TOO_LARGE,
            });
        }
        match subgroup {
            4 => self.emitted::<4>(subgroup),
            8 => self.emitted::<8>(subgroup),
            16 => self.emitted::<16>(subgroup),
            32 => self.emitted::<32>(subgroup),
            64 => self.emitted::<64>(subgroup),
            other => Err(LaneError::NoMapping {
                lanes: other,
                width: subgroup,
            }),
        }
    }

    /// The picture on the CPU: what the pass of `axis` and `look` paints into
    /// the first [`View::pixels`] words of `PIXELS`, given the world `cells`
    /// (a cell past them reads as 0), the layer word and the palette.
    #[must_use]
    pub fn painted(self, layer: u32, palette: &[u32; PALETTE], cells: &[u32]) -> Vec<u32> {
        let Self {
            grid,
            view,
            axis,
            look,
            kinds,
        } = self;
        let colour = |kind: u32| palette.get(kind as usize).copied().unwrap_or(0);
        let word = |point: [u32; 3]| {
            let at = point[0] as usize
                + grid.wide as usize * (point[1] as usize + grid.deep as usize * point[2] as usize);
            cells.get(at).copied().unwrap_or(0)
        };
        let length = extent(grid, axis);
        let mut out = Vec::with_capacity(view.pixels());
        for down in 0..view.deep {
            for across in 0..view.wide {
                let start = cell_under(grid, view, axis, layer, (across, down));
                out.push(match look {
                    Look::Slice => colour(kinds.of(word(start))),
                    Look::Depth => {
                        let from = read(start, axis.index());
                        let mut found = (0, 0);
                        for far in 0..length - from {
                            let point = written(start, axis.index(), from + far);
                            let kind = kinds.of(word(point));
                            if kind != 0 {
                                found = (kind, far);
                                break;
                            }
                        }
                        shaded(colour(found.0), found.1, length.trailing_zeros())
                    }
                });
            }
        }
        out
    }

    fn emitted<const LANES: u32>(&self, subgroup: u32) -> Result<Vec<u32>, LaneError> {
        let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, subgroup, BINDINGS))?;
        let uint = kernel.index_type();
        let group = kernel.workgroup_index();
        let lane = kernel.local_index();
        let width = kernel.module().constant_u32(subgroup)?;
        let base = kernel.module().i_mul(uint, group, width)?;
        let pixel = kernel.module().i_add(uint, base, lane)?;

        let slot = u32::try_from(self.axis.layer_word()).unwrap_or(u32::MAX);
        let slot = kernel.module().constant_u32(slot)?;
        let layer = kernel.load_at(PARAMS, slot)?;

        let (start, from) = {
            let mut lanes = kernel.lanes()?;
            self.start::<LANES>(&mut lanes, pixel, layer)?
        };

        let kind = match self.look {
            Look::Slice => {
                let held = kernel.load_at(STATE, start.id())?;
                let mut lanes = kernel.lanes()?;
                let held = lanes.splat_id::<U32, LANES>(held)?;
                (self.kind::<LANES>(&mut lanes, held)?, None)
            }
            Look::Depth => {
                let (kind, far) = self.walked::<LANES>(&mut kernel, start, from)?;
                (kind, Some(far))
            }
        };

        let (into, tint) = {
            let mut lanes = kernel.lanes()?;
            let seat = lanes.splat_id::<U32, LANES>(pixel)?;
            let count = u32::try_from(self.view.pixels()).unwrap_or(u32::MAX);
            let pixels = lanes.splat_bits::<U32, LANES>(count)?;
            let spare = lanes.splat_bits::<U32, LANES>(PALETTE as u32)?;
            let past = lanes.add(seat, spare)?;
            let inside = lanes.greater_than(pixels, seat)?;
            let into = lanes.select(inside, seat, past)?;
            let tint = lanes.add(pixels, kind.0)?;
            (into, tint)
        };
        let colour = kernel.load_at(PIXELS, tint.id())?;
        let colour = match kind.1 {
            None => colour,
            Some(far) => {
                let mut lanes = kernel.lanes()?;
                let colour = lanes.splat_id::<U32, LANES>(colour)?;
                let bits = extent(self.grid, self.axis).trailing_zeros();
                shade::<LANES>(&mut lanes, colour, far, bits)?.id()
            }
        };
        kernel.store_at(PIXELS, into.id(), colour)?;
        kernel.finish()
    }

    fn start<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        pixel: Id,
        layer: Id,
    ) -> Result<(Word<LANES>, Word<LANES>), LaneError> {
        let [wide, deep] = plane(self.grid, self.axis);
        let seat = lanes.splat_id::<U32, LANES>(pixel)?;
        let row_bits = self.view.wide.trailing_zeros();
        let across = low_bits(&mut Emit::<LANES>(lanes), seat, row_bits)?;
        let by = lanes.splat_bits::<U32, LANES>(row_bits)?;
        let down = lanes.shift_right_logical(seat, by)?;
        let across = rescaled::<LANES>(lanes, across, row_bits, wide.trailing_zeros())?;
        let down = rescaled::<LANES>(
            lanes,
            down,
            self.view.deep.trailing_zeros(),
            deep.trailing_zeros(),
        )?;
        let layer = lanes.splat_id::<U32, LANES>(layer)?;
        let bits = extent(self.grid, self.axis).trailing_zeros();
        let from = low_bits(&mut Emit::<LANES>(lanes), layer, bits)?;
        let normal = match self.look {
            Look::Slice => from,
            Look::Depth => lanes.splat_bits::<U32, LANES>(0)?,
        };
        let point = match self.axis {
            Axis::X => [normal, down, across],
            Axis::Y => [across, normal, down],
            Axis::Z => [across, down, normal],
        };
        let extents = Solid::extents(self.grid);
        let start = Solid::at(&mut Emit::<LANES>(lanes), extents, point)?;
        Ok((start, from))
    }

    fn kind<const LANES: u32>(
        &self,
        lanes: &mut Lanes<'_>,
        word: Word<LANES>,
    ) -> Result<Word<LANES>, LaneError> {
        let shift = lanes.splat_bits::<U32, LANES>(self.kinds.shift)?;
        let mask = lanes.splat_bits::<U32, LANES>(self.kinds.mask)?;
        let moved = lanes.shift_right_logical(word, shift)?;
        lanes.and(moved, mask)
    }

    fn walked<const LANES: u32>(
        &self,
        kernel: &mut Kernel<U32>,
        start: Word<LANES>,
        from: Word<LANES>,
    ) -> Result<(Word<LANES>, Word<LANES>), LaneError> {
        let length = extent(self.grid, self.axis);
        let bits = length.trailing_zeros();
        let stride = match self.axis {
            Axis::X => 0,
            Axis::Y => self.grid.wide.trailing_zeros(),
            Axis::Z => self.grid.wide.trailing_zeros() + self.grid.deep.trailing_zeros(),
        };
        let (mut kind, mut far) = {
            let mut lanes = kernel.lanes()?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            (none, none)
        };
        for step in (0..length).rev() {
            let cell = {
                let mut lanes = kernel.lanes()?;
                let ahead = lanes.splat_bits::<U32, LANES>(step)?;
                let along = lanes.add(from, ahead)?;
                let along = low_bits(&mut Emit::<LANES>(&mut lanes), along, bits)?;
                let by = lanes.splat_bits::<U32, LANES>(stride)?;
                let offset = lanes.shift_left(along, by)?;
                lanes.add(start, offset)?
            };
            let held = kernel.load_at(STATE, cell.id())?;
            let mut lanes = kernel.lanes()?;
            let held = lanes.splat_id::<U32, LANES>(held)?;
            let here = self.kind::<LANES>(&mut lanes, held)?;
            let ahead = lanes.splat_bits::<U32, LANES>(step)?;
            let along = lanes.add(from, ahead)?;
            let end = lanes.splat_bits::<U32, LANES>(length)?;
            let none = lanes.splat_bits::<U32, LANES>(0)?;
            let inside = lanes.greater_than(end, along)?;
            let here = lanes.select(inside, here, none)?;
            let hit = lanes.not_equal(here, none)?;
            kind = lanes.select(hit, here, kind)?;
            far = lanes.select(hit, ahead, far)?;
        }
        Ok((kind, far))
    }
}

fn rescaled<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    value: Word<LANES>,
    from: u32,
    to: u32,
) -> Result<Word<LANES>, LaneError> {
    if to >= from {
        let by = lanes.splat_bits::<U32, LANES>(to - from)?;
        lanes.shift_left(value, by)
    } else {
        let by = lanes.splat_bits::<U32, LANES>(from - to)?;
        lanes.shift_right_logical(value, by)
    }
}

fn shade<const LANES: u32>(
    lanes: &mut Lanes<'_>,
    colour: Word<LANES>,
    far: Word<LANES>,
    bits: u32,
) -> Result<Word<LANES>, LaneError> {
    let seven = lanes.splat_bits::<U32, LANES>(7)?;
    let by = lanes.splat_bits::<U32, LANES>(bits)?;
    let full = lanes.splat_bits::<U32, LANES>(256)?;
    let byte = lanes.splat_bits::<U32, LANES>(0xFF)?;
    let eight = lanes.splat_bits::<U32, LANES>(8)?;
    let lost = lanes.shift_left(far, seven)?;
    let lost = lanes.shift_right_logical(lost, by)?;
    let keep = lanes.sub(full, lost)?;
    let mut out = lanes.splat_bits::<U32, LANES>(0)?;
    for place in [16, 8, 0] {
        let at = lanes.splat_bits::<U32, LANES>(place)?;
        let channel = lanes.shift_right_logical(colour, at)?;
        let channel = lanes.and(channel, byte)?;
        let channel = lanes.mul(channel, keep)?;
        let channel = lanes.shift_right_logical(channel, eight)?;
        let channel = lanes.shift_left(channel, at)?;
        out = lanes.or(out, channel)?;
    }
    Ok(out)
}
