use crate::wire::{self, Json, map, text};

use super::super::law_real::{self, Fixed, Term, constant, law_word, offset, rounded};
use super::model::{Basis, Grid, NONE};

pub const BITS_LEAST: u32 = 12;

pub const FRAC_MOST: u32 = 30;

const HALF: u32 = 1 << 31;

pub fn bounds(basis: &Basis, frac: u32, reach: u64) -> Option<Vec<u128>> {
    let half = if frac == 0 { 0 } else { 1_u128 << (frac - 1) };
    let room = 1_u128 << 31;
    let product = |left: u128, right: u128| -> Option<u128> {
        let product_bound = left.saturating_mul(right);
        if product_bound.saturating_add(half) >= room {
            return None;
        }
        Some(if frac == 0 {
            product_bound
        } else {
            (product_bound >> frac) + 1
        })
    };
    let word = u128::from(reach);
    let others = u128::try_from(basis.places.len().saturating_sub(1)).unwrap_or(u128::MAX);
    basis
        .terms
        .iter()
        .map(|term| match *term {
            Term::Place { .. } => Some(word),
            Term::Pair { .. } => product(word, word),
            Term::Spread { .. } => {
                let sum = others.saturating_mul(word);
                if sum >= room {
                    return None;
                }
                product(word, sum)
            }
            Term::Cube { .. } => product(product(word, word)?, word),
        })
        .collect()
}

#[must_use]
pub fn reach_of(range: [f64; 2], frac: u32) -> u64 {
    let unit = f64::from(1_u32 << frac);
    let most = range[0].abs().max(range[1].abs()) * unit;
    let upper = most.ceil();
    if upper >= 4.0e18 {
        u64::MAX
    } else {
        upper as u64
    }
}

pub struct Chosen {
    pub frac: u32,
    pub fixed: Vec<Fixed>,
    pub step_error: f64,
}

pub fn quantised(
    basis: &Basis,
    frac: u32,
    range: [f64; 2],
    coefficients: &[Vec<f64>],
    biases: &[f64],
) -> Option<Chosen> {
    let bound = bounds(basis, frac, reach_of(range, frac))?;
    let unit = f64::from(1_u32 << frac);
    let fixed = coefficients
        .iter()
        .zip(biases)
        .map(|(one, bias)| law_real::scaled(&bound, one, bias * unit))
        .collect::<Option<Vec<Fixed>>>()?;
    let step_error = fixed
        .iter()
        .map(|one| {
            let terms: f64 = bound.iter().map(|value| *value as f64 / unit).sum();
            let coefficient = 0.5 / f64::from(1_u32 << one.scale);
            0.5 / unit + terms * coefficient + coefficient / unit
        })
        .fold(0.0, f64::max);
    Some(Chosen {
        frac,
        fixed,
        step_error,
    })
}

pub fn automatic(
    basis: &Basis,
    range: [f64; 2],
    coefficients: &[Vec<f64>],
    biases: &[f64],
) -> Result<Chosen, u32> {
    let mut best: Option<Chosen> = None;
    let mut most_bits = 0;
    for frac in 0..=FRAC_MOST {
        let Some(chosen) = quantised(basis, frac, range, coefficients, biases) else {
            continue;
        };
        let least = chosen.fixed.iter().map(|one| one.scale).min().unwrap_or(0);
        most_bits = most_bits.max(least);
        if least < BITS_LEAST {
            continue;
        }
        if best
            .as_ref()
            .is_none_or(|held| chosen.step_error <= held.step_error)
        {
            best = Some(chosen);
        }
    }
    best.ok_or(most_bits)
}

#[must_use]
pub fn word_range(range: [f64; 2], frac: u32) -> [i32; 2] {
    let unit = f64::from(1_u32 << frac);
    let clip = |value: f64| value.clamp(-2_147_483_648.0, 2_147_483_647.0) as i32;
    [
        clip((range[0] * unit).ceil()),
        clip((range[1] * unit).floor()),
    ]
}

pub struct Words<'a> {
    pub basis: &'a Basis,
    pub fixed: &'a [Fixed],
    pub frac: u32,
    pub clamps: &'a [[i32; 2]],
}

fn word_at(planes: &[Vec<u32>], plane: usize, cell: u32) -> u32 {
    if cell == NONE {
        return 0;
    }
    planes
        .get(plane)
        .and_then(|one| one.get(cell as usize))
        .copied()
        .unwrap_or(0)
}

impl Words<'_> {
    pub fn values(&self, grid: &Grid, planes: &[Vec<u32>], cell: usize, out: &mut Vec<u32>) {
        out.clear();
        let basis = self.basis;
        let own = u32::try_from(cell).unwrap_or(NONE);
        let centre = |plane: usize| word_at(planes, plane, own);
        let frac = self.frac;
        for term in &basis.terms {
            let value = match *term {
                Term::Place { read, place } => {
                    word_at(planes, read, grid.near(cell, basis.place_at(place)))
                }
                Term::Pair { left, right } => {
                    rounded(frac, centre(left).wrapping_mul(centre(right)))
                }
                Term::Spread {
                    centre: one,
                    around,
                } => {
                    let sum = (0..grid.width)
                        .filter(|at| *at != basis.centre)
                        .fold(0_u32, |sum, at| {
                            sum.wrapping_add(word_at(planes, around, grid.near(cell, at)))
                        });
                    rounded(frac, centre(one).wrapping_mul(sum))
                }
                Term::Cube { left, right, third } => rounded(
                    frac,
                    rounded(frac, centre(left).wrapping_mul(centre(right)))
                        .wrapping_mul(centre(third)),
                ),
            };
            out.push(value);
        }
    }

    pub fn advance(&self, grid: &Grid, planes: &[Vec<u32>], writes: &[usize]) -> Vec<Vec<u32>> {
        let mut out = planes.to_vec();
        let mut values = Vec::with_capacity(self.basis.terms.len());
        for cell in 0..grid.cells {
            self.values(grid, planes, cell, &mut values);
            for &write in writes {
                if let (Some(fixed), Some(slot)) = (
                    self.fixed.get(write),
                    out.get_mut(write).and_then(|one| one.get_mut(cell)),
                ) {
                    *slot = law_word(fixed, self.clamps.get(write).copied(), &values);
                }
            }
        }
        out
    }
}

pub fn between(from: &[u32], to: &[u32], sub: u32, steps: u32) -> Vec<u32> {
    let steps = i64::from(steps.max(1));
    let sub = i64::from(sub);
    from.iter()
        .zip(to)
        .map(|(one, other)| {
            let left = i64::from(one.cast_signed());
            let right = i64::from(other.cast_signed());
            let mixed = (left * (steps - sub) + right * sub + steps / 2).div_euclid(steps);
            i32::try_from(mixed).unwrap_or(0).cast_unsigned()
        })
        .collect()
}

fn op(name: &str, args: Vec<Json>) -> Json {
    let mut held = vec![text(name)];
    held.extend(args);
    Json::List(held)
}

fn named(name: &str) -> Json {
    text(name)
}

fn word(value: u32) -> Json {
    Json::Int(i64::from(value))
}

fn own(plane: u32, corner: &str) -> Json {
    op(
        "at",
        vec![op("at", vec![named("cells"), named(corner)]), word(plane)],
    )
}

fn rounding(frac: u32, value: Json) -> Json {
    if frac == 0 {
        return value;
    }
    let lift = (1_u32 << (frac - 1)).wrapping_add(HALF);
    op(
        "sub",
        vec![
            op("shr", vec![op("add", vec![value, word(lift)]), word(frac)]),
            word(1 << (31 - frac)),
        ],
    )
}

struct Bound {
    held: Vec<Json>,
}

impl Bound {
    fn bind(&mut self, name: &str, value: Json) {
        self.held.push(Json::List(vec![text(name), value]));
    }

    fn signed(&mut self, name: &str) {
        self.bind(
            &format!("{name}_n"),
            op("ge", vec![named(name), word(HALF)]),
        );
        self.bind(
            &format!("{name}_m"),
            op(
                "sel",
                vec![
                    named(&format!("{name}_n")),
                    op("sub", vec![word(0), named(name)]),
                    named(name),
                ],
            ),
        );
    }

    fn product(&mut self, name: &str, left: &str, right: &str) {
        let whole = format!("{name}_p");
        self.bind(
            &whole,
            op(
                "mul",
                vec![named(&format!("{left}_m")), named(&format!("{right}_m"))],
            ),
        );
        let negated = op("sub", vec![word(0), named(&whole)]);
        self.bind(
            name,
            op(
                "sel",
                vec![
                    named(&format!("{left}_n")),
                    op(
                        "sel",
                        vec![named(&format!("{right}_n")), named(&whole), negated.clone()],
                    ),
                    op(
                        "sel",
                        vec![named(&format!("{right}_n")), negated, named(&whole)],
                    ),
                ],
            ),
        );
    }
}

pub struct Text<'a> {
    pub basis: &'a Basis,
    pub fixed: &'a [Fixed],
    pub frac: u32,
    pub clamps: &'a [[i32; 2]],
    pub reads: &'a [u32],
    pub writes: &'a [u32],
    pub channels: u32,
    pub name: &'a str,
}

#[must_use]
pub fn param_words(basis: &Basis, fixed: &[Fixed]) -> Vec<u32> {
    let mut out = vec![HALF; basis.params()];
    for (write, one) in fixed.iter().enumerate() {
        for (term, coefficient) in one.coefficients.iter().enumerate() {
            if let Some(slot) = out.get_mut(basis.coefficient(write, term)) {
                *slot = coefficient.cast_unsigned().wrapping_add(HALF);
            }
        }
        if let Some(slot) = out.get_mut(basis.bias(write)) {
            *slot = one.bias.cast_unsigned().wrapping_add(HALF);
        }
    }
    out
}

#[must_use]
pub fn fixed_of(basis: &Basis, scales: &[u32], params: &[u32]) -> Vec<Fixed> {
    scales
        .iter()
        .enumerate()
        .map(|(write, scale)| Fixed {
            scale: *scale,
            coefficients: (0..basis.terms.len())
                .map(|term| {
                    params
                        .get(basis.coefficient(write, term))
                        .copied()
                        .unwrap_or(HALF)
                        .wrapping_sub(HALF)
                        .cast_signed()
                })
                .collect(),
            bias: params
                .get(basis.bias(write))
                .copied()
                .unwrap_or(HALF)
                .wrapping_sub(HALF)
                .cast_signed(),
        })
        .collect()
}

impl Text<'_> {
    fn corner(&self) -> Json {
        let basis = self.basis;
        let mut bound = Bound { held: Vec::new() };
        let centre_place = basis.stencil.place([0, 0, 0]);
        let mut names: Vec<String> = Vec::with_capacity(basis.terms.len());
        for (read, plane) in self.reads.iter().enumerate() {
            let places: Vec<u32> = if read < basis.observed {
                basis.places.clone()
            } else {
                vec![centre_place]
            };
            for place in places {
                let value = if place == centre_place {
                    own(*plane, "c")
                } else {
                    let [dx, dy, dz] = basis.stencil.offset(place);
                    op(
                        "look",
                        vec![
                            word(*plane),
                            Json::Int(dx),
                            Json::Int(dy),
                            Json::Int(dz),
                            named("c"),
                        ],
                    )
                };
                bound.bind(&format!("x{read}_{place}"), value);
            }
        }
        let centre = |read: usize| format!("x{read}_{centre_place}");
        let mut signed: Vec<String> = Vec::new();
        let mut sign = |bound: &mut Bound, name: &str| {
            if !signed.iter().any(|one| one == name) {
                bound.signed(name);
                signed.push(name.to_owned());
            }
        };
        let mut sums: Vec<usize> = Vec::new();
        for (at, term) in basis.terms.iter().enumerate() {
            let name = match *term {
                Term::Place { read, place } => format!("x{read}_{place}"),
                Term::Pair { left, right } => {
                    let name = format!("p{left}_{right}");
                    sign(&mut bound, &centre(left));
                    sign(&mut bound, &centre(right));
                    bound.product(&format!("{name}_w"), &centre(left), &centre(right));
                    bound.bind(&name, rounding(self.frac, named(&format!("{name}_w"))));
                    name
                }
                Term::Spread {
                    centre: one,
                    around,
                } => {
                    let sum = format!("n{around}");
                    if !sums.contains(&around) {
                        let mut total: Option<Json> = None;
                        for place in basis.places.iter().filter(|one| **one != centre_place) {
                            let next = named(&format!("x{around}_{place}"));
                            total = Some(match total {
                                None => next,
                                Some(before) => op("add", vec![before, next]),
                            });
                        }
                        bound.bind(&sum, total.unwrap_or_else(|| word(0)));
                        sums.push(around);
                    }
                    let name = format!("t{at}");
                    sign(&mut bound, &centre(one));
                    sign(&mut bound, &sum);
                    bound.product(&format!("{name}_w"), &centre(one), &sum);
                    bound.bind(&name, rounding(self.frac, named(&format!("{name}_w"))));
                    name
                }
                Term::Cube { left, right, third } => {
                    let pair = format!("p{left}_{right}");
                    let name = format!("t{at}");
                    sign(&mut bound, &pair);
                    sign(&mut bound, &centre(third));
                    bound.product(&format!("{name}_w"), &pair, &centre(third));
                    bound.bind(&name, rounding(self.frac, named(&format!("{name}_w"))));
                    name
                }
            };
            sign(&mut bound, &name);
            names.push(name);
        }
        for (write, one) in self.fixed.iter().enumerate() {
            let mut total = format!("d{write}");
            for (term, name) in names.iter().enumerate() {
                let coefficient = format!("q{write}_{term}");
                let next = format!("a{write}_{term}");
                let product = format!("{next}_s");
                bound.product(&product, &coefficient, name);
                bound.bind(&next, op("add", vec![named(&total), named(&product)]));
                total = next;
            }
            let clamp = self
                .clamps
                .get(write)
                .copied()
                .unwrap_or([i32::MIN, i32::MAX]);
            let [least, most] = law_real::clamp_bounds(clamp, one.scale);
            bound.bind(
                &format!("z{write}"),
                op("shr", vec![named(&total), word(one.scale)]),
            );
            bound.bind(
                &format!("l{write}"),
                op(
                    "sel",
                    vec![
                        op("lt", vec![named(&format!("z{write}")), word(least)]),
                        word(least),
                        named(&format!("z{write}")),
                    ],
                ),
            );
            bound.bind(
                &format!("y{write}"),
                op(
                    "sub",
                    vec![
                        op(
                            "sel",
                            vec![
                                op("gt", vec![named(&format!("l{write}")), word(most)]),
                                word(most),
                                named(&format!("l{write}")),
                            ],
                        ),
                        word(offset(one.scale)),
                    ],
                ),
            );
        }
        let planes = (0..self.channels)
            .map(
                |plane| match self.writes.iter().position(|one| *one == plane) {
                    Some(write) => named(&format!("y{write}")),
                    None => own(plane, "c"),
                },
            )
            .collect();
        op("let", vec![Json::List(bound.held), op("vec", planes)])
    }

    fn prepared(&self) -> Vec<Json> {
        let basis = self.basis;
        let mut bound = Bound { held: Vec::new() };
        for (write, one) in self.fixed.iter().enumerate() {
            for term in 0..basis.terms.len() {
                let name = format!("q{write}_{term}");
                let param = op(
                    "param",
                    vec![word(
                        u32::try_from(basis.coefficient(write, term)).unwrap_or(0),
                    )],
                );
                bound.bind(
                    &format!("{name}_n"),
                    op("lt", vec![param.clone(), word(HALF)]),
                );
                bound.bind(
                    &format!("{name}_m"),
                    op(
                        "sel",
                        vec![
                            named(&format!("{name}_n")),
                            op("sub", vec![word(HALF), param.clone()]),
                            op("sub", vec![param, word(HALF)]),
                        ],
                    ),
                );
            }
            bound.bind(
                &format!("d{write}"),
                op(
                    "add",
                    vec![
                        op(
                            "param",
                            vec![word(u32::try_from(basis.bias(write)).unwrap_or(0))],
                        ),
                        word(constant(0, one.scale).wrapping_sub(HALF)),
                    ],
                ),
            );
        }
        bound.held
    }

    #[must_use]
    pub fn written(&self) -> String {
        let corners = op(
            "map",
            vec![named("c"), op("range", vec![word(8)]), self.corner()],
        );
        let mut changed: Option<Json> = None;
        for plane in self.writes {
            let one = op(
                "ne",
                vec![
                    own(*plane, "k"),
                    op(
                        "at",
                        vec![op("at", vec![named("out"), named("k")]), word(*plane)],
                    ),
                ],
            );
            changed = Some(match changed {
                None => one,
                Some(before) => op("either", vec![before, one]),
            });
        }
        let count = op(
            "count",
            vec![op(
                "map",
                vec![
                    named("k"),
                    op("range", vec![word(8)]),
                    changed.unwrap_or_else(|| op("ne", vec![word(0), word(0)])),
                ],
            )],
        );
        let mut bindings = self.prepared();
        bindings.push(Json::List(vec![text("out"), corners]));
        let step = op(
            "let",
            vec![
                Json::List(bindings),
                op("vec", vec![named("out"), op("vec", vec![count])]),
            ],
        );
        let centre_place = self.basis.stencil.place([0, 0, 0]);
        let halo = u32::from(self.basis.places.iter().any(|one| *one != centre_place));
        let pairs: Vec<Json> = self
            .reads
            .iter()
            .zip(self.writes)
            .map(|(read, write)| Json::List(vec![word(*read), word(*write)]))
            .collect();
        let alternate = match pairs.as_slice() {
            [one] => one.clone(),
            _ => Json::List(pairs),
        };
        wire::written(&map(vec![
            ("law", word(crate::law::limits::VERSION)),
            ("name", text(self.name)),
            (
                "params",
                word(u32::try_from(self.basis.params()).unwrap_or(u32::MAX)),
            ),
            ("channels", word(self.channels)),
            ("halo", word(halo)),
            ("alternate", alternate),
            (
                "tally",
                map(vec![("counts", word(1)), ("weighted", Json::Bool(false))]),
            ),
            ("consts", Json::Map(Vec::new())),
            ("defs", Json::Map(Vec::new())),
            ("step", step),
        ]))
    }
}
