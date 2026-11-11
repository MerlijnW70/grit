//! The inside test: ray parity along each of the three axes, decided by a
//! majority vote.
//!
//! For each axis, a line runs through every row of voxel centres along it.
//! Every triangle the line pierces is a crossing, found by testing the
//! line's point against the triangle projected onto the other two axes. A
//! centre is inside along that axis when an odd number of crossings lie
//! before it on its line. A voxel is filled when at least two of the three
//! axes say inside.
//!
//! The projected test is decided with a symbolic perturbation: a point
//! that falls on an edge or a vertex is read as if it lay a vanishing step
//! off it, the same way for both triangles that share the edge. So a line
//! through a shared edge or vertex of a closed mesh is counted once, not
//! twice or never, and parity along one axis is exact on a closed mesh. The
//! vote is there for meshes that are not closed: a hole lets every line
//! through it miss a crossing, which flips the voxels along that line, and
//! the other two axes, whose lines miss the hole, outvote it.

/// One axis's lines: for line `(u, v)`, the positions along the axis where
/// it crosses a triangle.
struct Lines {
    across: usize,
    crossings: Vec<Vec<f64>>,
}

/// The voxel lattice: the corner it starts from, the voxel's side, and how
/// many voxels lie along each axis.
#[derive(Debug, Clone, Copy)]
pub struct Lattice {
    pub origin: [f64; 3],
    pub size: f64,
    pub count: [usize; 3],
}

impl Lattice {
    fn centre(&self, axis: usize, at: usize) -> f64 {
        self.origin.get(axis).copied().unwrap_or(0.0) + (at as f64 + 0.5) * self.size
    }

    fn along(&self, axis: usize) -> usize {
        self.count.get(axis).copied().unwrap_or(0)
    }

    /// The index of voxel `at`, x fastest, then y, then z.
    #[must_use]
    pub fn index(&self, at: [usize; 3]) -> usize {
        let [wide, deep, _] = self.count;
        (at[2] * deep + at[1]) * wide + at[0]
    }

    /// The range of voxel indices along `axis` whose centres may lie from
    /// `low` to `high`, one wider on each side than the exact range, so
    /// that rounding never leaves out a centre the edge test takes.
    fn span(&self, axis: usize, low: f64, high: f64) -> std::ops::Range<usize> {
        let origin = self.origin.get(axis).copied().unwrap_or(0.0);
        let first = ((low - origin) / self.size - 0.5).ceil() - 1.0;
        let last = ((high - origin) / self.size - 0.5).floor() + 1.0;
        let count = self.along(axis);
        let first = if first <= 0.0 { 0 } else { first as usize };
        let past = if last < 0.0 {
            0
        } else {
            (last as usize).saturating_add(1).min(count)
        };
        first.min(past)..past
    }
}

fn plane(axis: usize) -> (usize, usize) {
    match axis {
        0 => (1, 2),
        1 => (2, 0),
        _ => (0, 1),
    }
}

/// Which side of the line from `from` to `to` the point `at` lies on,
/// with a point on the line read as moved by a vanishing step along both
/// axes, the second smaller than the first. The edge is always evaluated
/// from its lesser end, so the same edge gives the same answer whichever
/// triangle asks.
fn side(from: [f64; 2], to: [f64; 2], at: [f64; 2]) -> bool {
    let swapped = (to[0], to[1]) < (from[0], from[1]);
    let (low, high) = if swapped { (to, from) } else { (from, to) };
    let across = high[0] - low[0];
    let rise = high[1] - low[1];
    let value = across * (at[1] - low[1]) - rise * (at[0] - low[0]);
    let positive = if value != 0.0 {
        value > 0.0
    } else if rise != 0.0 {
        rise < 0.0
    } else {
        across > 0.0
    };
    positive != swapped
}

fn pierces(corners: [[f64; 2]; 3], at: [f64; 2]) -> bool {
    let [a, b, c] = corners;
    let first = side(a, b, at);
    first == side(b, c, at) && first == side(c, a, at)
}

/// Where the line through `at` along `axis` meets the plane of
/// `triangle`, kept within the triangle's own span along the axis.
fn depth(triangle: &[[f64; 3]; 3], axis: usize, (u, v): (usize, usize), at: [f64; 2]) -> f64 {
    let get = |corner: &[f64; 3], which: usize| corner.get(which).copied().unwrap_or(0.0);
    let [a, b, c] = triangle;
    let flat = |one: &[f64; 3]| [get(one, u), get(one, v)];
    let (pa, pb, pc) = (flat(a), flat(b), flat(c));
    let area = (pb[0] - pa[0]) * (pc[1] - pa[1]) - (pb[1] - pa[1]) * (pc[0] - pa[0]);
    let depths = [get(a, axis), get(b, axis), get(c, axis)];
    let low = depths.iter().copied().fold(f64::INFINITY, f64::min);
    let high = depths.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if area == 0.0 {
        return (low + high) / 2.0;
    }
    let weight = |p: [f64; 2], q: [f64; 2]| {
        ((q[0] - p[0]) * (at[1] - p[1]) - (q[1] - p[1]) * (at[0] - p[0])) / area
    };
    let found =
        weight(pb, pc) * depths[0] + weight(pc, pa) * depths[1] + weight(pa, pb) * depths[2];
    if found.is_finite() {
        found.clamp(low, high)
    } else {
        (low + high) / 2.0
    }
}

fn crossings(lattice: &Lattice, triangles: &[[[f64; 3]; 3]], axis: usize) -> Lines {
    let (u, v) = plane(axis);
    let across = lattice.along(u);
    let mut crossings = vec![Vec::new(); across * lattice.along(v)];
    for triangle in triangles {
        let flat = triangle.map(|corner| {
            [
                corner.get(u).copied().unwrap_or(0.0),
                corner.get(v).copied().unwrap_or(0.0),
            ]
        });
        if flat[0] == flat[1] && flat[1] == flat[2] {
            continue;
        }
        let bound = |which: usize| {
            let values = flat.map(|corner| corner.get(which).copied().unwrap_or(0.0));
            (
                values.iter().copied().fold(f64::INFINITY, f64::min),
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            )
        };
        let (u_low, u_high) = bound(0);
        let (v_low, v_high) = bound(1);
        for line_v in lattice.span(v, v_low, v_high) {
            for line_u in lattice.span(u, u_low, u_high) {
                let at = [lattice.centre(u, line_u), lattice.centre(v, line_v)];
                if !pierces(flat, at) {
                    continue;
                }
                if let Some(line) = crossings.get_mut(line_v * across + line_u) {
                    line.push(depth(triangle, axis, (u, v), at));
                }
            }
        }
    }
    Lines { across, crossings }
}

/// Adds one vote to every voxel that parity along `axis` puts inside.
fn voted(lattice: &Lattice, lines: &mut Lines, axis: usize, votes: &mut [u8]) {
    let (u, v) = plane(axis);
    for (line, found) in lines.crossings.iter_mut().enumerate() {
        if found.is_empty() {
            continue;
        }
        found.sort_by(f64::total_cmp);
        let (line_u, line_v) = (line % lines.across, line / lines.across);
        let mut passed = 0;
        for at in 0..lattice.along(axis) {
            let centre = lattice.centre(axis, at);
            while found.get(passed).is_some_and(|&crossing| crossing < centre) {
                passed += 1;
            }
            if passed % 2 == 1 {
                let mut cell = [0; 3];
                for (which, value) in [(axis, at), (u, line_u), (v, line_v)] {
                    if let Some(slot) = cell.get_mut(which) {
                        *slot = value;
                    }
                }
                if let Some(vote) = votes.get_mut(lattice.index(cell)) {
                    *vote += 1;
                }
            }
        }
    }
}

/// The votes for each voxel of `lattice`, from 0 to 3, x fastest, as the
/// three axes give them for the surface `triangles`.
#[must_use]
pub fn votes(lattice: &Lattice, triangles: &[[[f64; 3]; 3]]) -> Vec<u8> {
    let mut votes = vec![0; lattice.count.iter().product()];
    for axis in 0..3 {
        let mut lines = crossings(lattice, triangles, axis);
        voted(lattice, &mut lines, axis, &mut votes);
    }
    votes
}

/// How many of the three axes must say inside for a voxel to be filled.
pub const MAJORITY: u8 = 2;
