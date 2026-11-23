use crate::halo::{BOX, Halo};
use crate::law::dag::{Dag, Node};
use crate::law::parse::{Path, Program};
use crate::law::{Code, Refusal};
use crate::ops::Ops;
use crate::rule3::{CORNERS, Oct, Planed};

fn refusal(code: Code, path: &str, expected: String, found: String) -> Refusal {
    Refusal {
        code,
        path: path.to_owned(),
        expected,
        found,
        limit: None,
    }
}

fn index_of(plane: u32) -> usize {
    usize::try_from(plane).unwrap_or(usize::MAX)
}

fn input_of(dag: &Dag, plane: u32, corner: u32) -> u32 {
    let corners = u32::try_from(CORNERS).unwrap_or(8);
    match plane {
        0 => corner,
        _ => dag
            .first_plane()
            .saturating_add((plane - 1).saturating_mul(corners))
            .saturating_add(corner),
    }
}

fn outs_of(dag: &Dag, plane: u32) -> Option<&[u32; CORNERS]> {
    match plane {
        0 => Some(dag.out()),
        _ => dag.planes().get(index_of(plane - 1)),
    }
}

#[must_use]
pub fn written(dag: &Dag, plane: u32) -> bool {
    outs_of(dag, plane).is_some_and(|outs| {
        outs.iter()
            .zip(0_u32..)
            .any(|(&id, corner)| id != input_of(dag, plane, corner))
    })
}

fn children(node: Node) -> [Option<u32>; 3] {
    match node {
        Node::Input(_) | Node::Const(_) => [None; 3],
        Node::Not(one) | Node::Shl(one, _) | Node::Shr(one, _) => [Some(one), None, None],
        Node::Select(flag, then, otherwise) => [Some(flag), Some(then), Some(otherwise)],
        Node::And(left, right)
        | Node::Or(left, right)
        | Node::Xor(left, right)
        | Node::Add(left, right)
        | Node::Sub(left, right)
        | Node::Mul(left, right)
        | Node::Div(left, right)
        | Node::DotS(left, right)
        | Node::DotU(left, right)
        | Node::Eq(left, right)
        | Node::Ne(left, right)
        | Node::Lt(left, right)
        | Node::Le(left, right)
        | Node::Gt(left, right)
        | Node::Ge(left, right) => [Some(left), Some(right), None],
    }
}

#[must_use]
pub fn read(dag: &Dag, plane: u32) -> bool {
    if dag.halo().looked(index_of(plane)) {
        return true;
    }
    let corners = u32::try_from(CORNERS).unwrap_or(8);
    let wanted: Vec<u32> = (0..corners)
        .map(|corner| input_of(dag, plane, corner))
        .collect();
    let mut roots: Vec<u32> = dag.counts().to_vec();
    let planes = u32::try_from(dag.planes().len()).unwrap_or(0) + 1;
    for other in (0..planes).filter(|&other| other != plane) {
        if let Some(outs) = outs_of(dag, other) {
            roots.extend_from_slice(outs);
        }
    }
    let mut seen = vec![false; dag.len()];
    while let Some(id) = roots.pop() {
        let Some(slot) = seen.get_mut(index_of(id)) else {
            continue;
        };
        if *slot {
            continue;
        }
        *slot = true;
        if wanted.contains(&id) {
            return true;
        }
        if let Some(&node) = dag.nodes().get(index_of(id)) {
            roots.extend(children(node).into_iter().flatten());
        }
    }
    false
}

pub fn checked(pair: [u32; 2], program: &Program<'_>, dag: &Dag) -> Result<(), Refusal> {
    let root = Path::root();
    let path = root.key("alternate");
    let at = |which: usize| path.index(which).shown();
    let [looked, target] = pair;
    let channels = program.channels.max(1);
    if looked == target || looked >= channels || target >= channels {
        return Err(refusal(
            Code::AlternatePlane,
            &path.shown(),
            format!("two different planes below the law's {channels} channels"),
            format!("[{looked}, {target}]"),
        ));
    }
    if program.reversible {
        return Err(refusal(
            Code::AlternateReversible,
            &path.shown(),
            String::from("a law that is not reversible: an iterating world is not walked back"),
            String::from("\"reversible\": true"),
        ));
    }
    if written(dag, looked) {
        return Err(refusal(
            Code::AlternateWritten,
            &at(0),
            format!(
                "plane {looked} frozen as the law is written: it is the plane even frames look at"
            ),
            format!("a law that writes plane {looked}"),
        ));
    }
    if !written(dag, target) {
        return Err(refusal(
            Code::AlternateUnwritten,
            &at(1),
            format!("a law that writes plane {target}, the plane even frames write"),
            format!("a law that leaves plane {target} unchanged"),
        ));
    }
    if let Some(other) = (0..channels).find(|&plane| plane != target && written(dag, plane)) {
        return Err(refusal(
            Code::AlternateOther,
            &path.shown(),
            format!("a law whose only written plane is {target}"),
            format!("a law that also writes plane {other}, which would not alternate"),
        ));
    }
    if !read(dag, looked) {
        return Err(refusal(
            Code::AlternateUnread,
            &at(0),
            format!("a law that reads plane {looked}"),
            format!("a law that never reads plane {looked}"),
        ));
    }
    Ok(())
}

#[must_use]
pub fn mirrored(halo: Halo, [looked, target]: [u32; 2]) -> Halo {
    let (looked, target) = (index_of(looked), index_of(target));
    let mut out = halo;
    for cell in 0..BOX {
        if (halo.cells(looked) >> cell) & 1 == 1 {
            out = out.with(target, cell);
        }
        if (halo.cells(target) >> cell) & 1 == 1 {
            out = out.with(looked, cell);
        }
    }
    out
}

fn swapped<W: Copy, const N: usize, O: Ops<Word = W>>(
    ops: &mut O,
    odd: W,
    held: &mut [[W; N]],
    [looked, target]: [usize; 2],
    cells: Option<u64>,
) {
    let (Some(&first), Some(&second)) = (held.get(looked), held.get(target)) else {
        return;
    };
    let mut one = first;
    let mut two = second;
    for (cell, (left, right)) in one.iter_mut().zip(two.iter_mut()).enumerate() {
        if cells.is_some_and(|cells| (cells >> cell) & 1 == 0) {
            continue;
        }
        let raised = crate::ops::Flag::raised(odd);
        let flipped = ops.select(raised, *right, *left);
        let raised = crate::ops::Flag::raised(odd);
        let kept = ops.select(raised, *left, *right);
        *left = flipped;
        *right = kept;
    }
    if let Some(slot) = held.get_mut(looked) {
        *slot = one;
    }
    if let Some(slot) = held.get_mut(target) {
        *slot = two;
    }
}

pub fn stepped<O: Ops>(
    dag: &Dag,
    pair: [u32; 2],
    ops: &mut O,
    oct: Oct<'_, O::Word>,
    planes: &[[O::Word; CORNERS]],
    halo: &[[O::Word; BOX]],
) -> Planed<O::Word> {
    let pair = pair.map(index_of);
    let one = ops.constant(1);
    let low = ops.and(oct.frame, one);
    let zero = ops.constant(0);
    let odd = ops.not_equal(low, zero).word();
    let mut words = Vec::with_capacity(planes.len() + 1);
    words.push(oct.words);
    words.extend_from_slice(planes);
    swapped(ops, odd, &mut words, pair, None);
    let mut boxes = halo.to_vec();
    let looked = dag.halo().cells(pair[0]);
    swapped(ops, odd, &mut boxes, pair, Some(looked));
    let (first, upper) = words
        .split_first()
        .map_or((oct.words, &[][..]), |(first, upper)| (*first, upper));
    let ran = dag.replay_halo(
        ops,
        Oct {
            words: first,
            ..oct
        },
        upper,
        &boxes,
    );
    let mut outs = Vec::with_capacity(ran.planes.len() + 1);
    outs.push(ran.ruled.out);
    outs.extend(ran.planes);
    swapped(ops, odd, &mut outs, pair, None);
    let mut outs = outs.into_iter();
    let out = outs.next().unwrap_or(oct.words);
    Planed {
        ruled: crate::rule3::Ruled {
            out,
            counts: ran.ruled.counts,
        },
        planes: outs.collect(),
    }
}
