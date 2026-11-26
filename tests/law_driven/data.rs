use super::pinned::LONG;
use super::served::{FRAC, Rng, fixed};

pub const ALPHA: f64 = 0.15;

pub const CENTRE: f64 = 1.0 - 4.0 * ALPHA;

pub const SIDE: u32 = 16;

pub const SOURCE: [u32; 3] = [5, 7, 0];

pub const STEPS: u32 = 2;

pub const SHOTS: usize = 20;

pub const HOLDOUT: usize = 2;

pub fn at_of(size: [u32; 3], cell: [u32; 3]) -> usize {
    ((cell[2] * size[1] + cell[1]) * size[0] + cell[0]) as usize
}

fn diffused(size: [u32; 3], field: &[f64]) -> Vec<f64> {
    let side = |value: u32, by: i64, length: u32| {
        (i64::from(value) + by).rem_euclid(i64::from(length)) as u32
    };
    (0..field.len())
        .map(|at| {
            let x = at as u32 % size[0];
            let y = at as u32 / size[0] % size[1];
            let z = at as u32 / (size[0] * size[1]);
            let near = |dx: i64, dy: i64| {
                field[at_of(size, [side(x, dx, size[0]), side(y, dy, size[1]), z])]
            };
            CENTRE * field[at] + ALPHA * (near(1, 0) + near(-1, 0) + near(0, 1) + near(0, -1))
        })
        .collect()
}

pub fn pulse(example: usize, sub: usize) -> f64 {
    if (sub + 5 * example) % 12 < 3 {
        1.0
    } else {
        0.0
    }
}

pub struct Example {
    pub shots: Vec<Vec<u32>>,
    pub pulses: Vec<u32>,
}

pub fn example(
    rng: &mut Rng,
    size: [u32; 3],
    at: usize,
    source: [u32; 3],
    shots: usize,
    steps: u32,
) -> Example {
    let cells = (size[0] * size[1] * size[2]) as usize;
    let spot = at_of(size, source);
    let mut field: Vec<f64> = (0..cells)
        .map(|_| rng.below(1 << 15) as f64 / 65536.0)
        .collect();
    let subs = (shots - 1) * steps as usize;
    let pulses: Vec<u32> = (0..=subs).map(|sub| fixed(pulse(at, sub), FRAC)).collect();
    let mut recorded = Vec::with_capacity(shots);
    for (sub, word) in pulses.iter().enumerate() {
        if sub % steps as usize == 0 {
            let mut shot: Vec<u32> = field.iter().map(|one| fixed(*one, FRAC)).collect();
            shot[spot] = *word;
            recorded.push(shot);
        }
        if sub == subs {
            break;
        }
        field[spot] = f64::from(word.cast_signed()) / 65536.0;
        field = diffused(size, &field);
    }
    Example {
        shots: recorded,
        pulses: pulses[..subs].to_vec(),
    }
}

pub fn request(examples: &[String], more: &str) -> String {
    format!(
        "{{\"stencil\":\"3x3\",\"reads\":[{{\"plane\":0,\"bit\":0}}],\"writes\":[{{\"plane\":1,\"bit\":0}}],\"border\":\"wrapped\",\"rule\":\"linear\",\"places\":\"von_neumann\",\"frac\":{FRAC},\"steps\":{STEPS},\"holdout\":{HOLDOUT}{more},\"examples\":[{}],\"budget_ms\":{LONG}}}",
        examples.join(",")
    )
}
