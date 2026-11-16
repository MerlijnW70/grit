use simdr::kernel::{Kernel, Shape};
use simdr::lanes::{LaneError, U32};

use crate::layout::{BINDINGS, PARAMS};
use crate::live::{self, Loop};
use crate::resident::Groups;

use super::{Failed, World};

const UNLIKE: &str = "the law and its inverse do not build the same frame";

#[derive(Debug)]
pub struct NotReversible {
    pub rule: String,
}

impl std::fmt::Display for NotReversible {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            out,
            "the world runs {}, which is not a law declared \"reversible\": true, so it has no inverse to walk back with",
            self.rule
        )
    }
}

impl std::error::Error for NotReversible {}

pub fn untick(subgroup: u32) -> Result<Vec<u32>, LaneError> {
    let mut kernel = Kernel::<U32>::new(Shape::new(subgroup, 1, BINDINGS))?;
    let slot = kernel.module().constant_u32(0)?;
    let one = kernel.module().constant_u32(1)?;
    let element = kernel.element();
    let now = kernel.load_at(PARAMS, slot)?;
    let next = kernel.module().i_sub(element, now, one)?;
    kernel.store_at(PARAMS, slot, next)?;
    kernel.finish()
}

fn unlike() -> LaneError {
    LaneError::NoSuchForm {
        operation: "branch::back::merged",
        because: UNLIKE,
    }
}

pub(crate) type Merged = (Vec<Vec<u32>>, Vec<Vec<Groups>>, Vec<Vec<Groups>>);

pub(crate) fn merged(forward: Loop, inverse: Loop, subgroup: u32) -> Result<Merged, LaneError> {
    let (ahead, ahead_stages) = forward;
    let (behind, behind_stages) = inverse;
    let shaped = |stages: &[Vec<Groups>]| stages.iter().map(Vec::len).collect::<Vec<_>>();
    if ahead.len() != behind.len()
        || ahead_stages != behind_stages
        || shaped(&ahead_stages) != shaped(&behind_stages)
    {
        return Err(unlike());
    }
    let tick = live::tick(subgroup)?;
    let ticks = ahead.iter().filter(|pass| **pass == tick).count();
    if ticks != 1 || ahead.first() != Some(&tick) {
        return Err(unlike());
    }
    let mut passes = Vec::with_capacity(ahead.len() + 4);
    let mut onward = Vec::with_capacity(ahead_stages.len() + 1);
    let mut backward = Vec::with_capacity(ahead_stages.len() + 1);
    let mut pairs = ahead.into_iter().zip(behind);
    for stage in &ahead_stages {
        let mut on = Vec::with_capacity(stage.len());
        let mut back = Vec::with_capacity(stage.len());
        for &groups in stage {
            let (one, other) = pairs.next().ok_or_else(unlike)?;
            if one == other {
                back.push(if one == tick {
                    Groups::Fixed(0)
                } else {
                    groups
                });
                on.push(groups);
                passes.push(one);
                continue;
            }
            passes.push(one);
            passes.push(other);
            on.extend([groups, Groups::Fixed(0)]);
            back.extend([Groups::Fixed(0), groups]);
        }
        onward.push(on);
        backward.push(back);
    }
    passes.push(untick(subgroup)?);
    onward.push(vec![Groups::Fixed(0)]);
    backward.push(vec![Groups::Fixed(1)]);
    Ok((passes, onward, backward))
}

impl World<'_> {
    #[must_use]
    pub const fn reversible(&self) -> bool {
        self.back.is_some()
    }

    pub(crate) fn turned(&mut self, backward: bool) -> Result<(), Failed> {
        if self.backward == backward {
            return Ok(());
        }
        self.reseed()?;
        self.live
            .write(PARAMS as usize, &[self.frame.wrapping_sub(1)])?;
        self.backward = backward;
        Ok(())
    }

    pub fn back(&mut self, frames: u32) -> Result<(), Failed> {
        let Some(back) = self.back.as_ref() else {
            return Err(Box::new(NotReversible {
                rule: self.rule.name().to_owned(),
            }));
        };
        let groups: Vec<Vec<Groups>> = back.clone();
        self.turned(true)?;
        let groups: Vec<&[Groups]> = groups.iter().map(Vec::as_slice).collect();
        let mut done = 0_u32;
        while done < frames {
            let step = super::BATCH.min(frames - done);
            self.live.staged_frames(&groups, step)?;
            done += step;
            self.frame = self.frame.wrapping_sub(step);
        }
        Ok(())
    }
}
