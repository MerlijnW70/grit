#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use grit::branch::{Made, World};
use grit::law::Law;
use grit::sand3::{Grid3, reference};
use grit::serve3::law::{background_class, stepping_of};

const SWAP: &str = r#"{"law":1,"name":"drift","params":0,"channels":1,"halo":0,"reversible":true,"tally":{"counts":1,"weighted":false},"consts":{},"defs":{},"step":["let",[["a0",["at","cells",0]],["b0",["at","cells",1]],["a2",["at","cells",2]],["b2",["at","cells",3]],["a4",["at","cells",4]],["b4",["at","cells",5]],["a6",["at","cells",6]],["b6",["at","cells",7]],["a0x",["xor","a0","b0"]],["b0x",["xor","b0","a0x"]],["a0y",["xor","a0x","b0x"]],["a2x",["xor","a2","b2"]],["b2x",["xor","b2","a2x"]],["a2y",["xor","a2x","b2x"]],["a4x",["xor","a4","b4"]],["b4x",["xor","b4","a4x"]],["a4y",["xor","a4x","b4x"]],["a6x",["xor","a6","b6"]],["b6x",["xor","b6","a6x"]],["a6y",["xor","a6x","b6x"]]],["vec",["vec","a0y","b0x","a2y","b2x","a4y","b4x","a6y","b6x"],["vec",["word",["ne","a0","b0"]]]]]}"#;

fn world<'h>(host: &'h grit::Host, law: &Law, size: [u32; 3], slots: Option<u32>) -> World<'h> {
    let class = background_class(law, &[]);
    assert!(class.proved());
    World::create_law_stepped(
        host,
        law,
        &Made {
            rule: law.name(),
            params: &[],
            size,
            sparse: true,
            slots,
        },
        stepping_of(&class, true),
    )
    .unwrap()
}

#[test]
fn a_sparse_walk_back_releases_bricks_as_the_forward_walk_does() {
    let Some(host) = device("review a1a sparse pool") else {
        return;
    };
    let law = Law::compile(SWAP).unwrap();
    assert!(law.inverse().unwrap().is_some());
    let size = [256, 32, 32];
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    let frames = 240;
    let mut start = vec![0_u32; grid.cells()];
    start[grid.index(0, 0, 0)] = 7;
    let mut far = start.clone();
    for frame in 0..frames {
        far = reference(&law, grid, frame, &far, &[]);
    }
    assert_ne!(far, start);
    let mut probe = world(&host, &law, size, None);
    probe.write(&[([0, 0, 0], 7)]).unwrap();
    let mut most_forward = 0;
    for _ in 0..frames / 16 {
        probe.step(16).unwrap();
        most_forward = most_forward.max(probe.pool_use().unwrap().unwrap().held);
    }
    assert_eq!(probe.cells().unwrap(), far, "forward on the device");
    let capacity = probe.pool_use().unwrap().unwrap().capacity;
    let mut most_back = 0;
    for _ in 0..frames / 16 {
        probe.back(16).unwrap();
        most_back = most_back.max(probe.pool_use().unwrap().unwrap().held);
    }
    println!(
        "capacity {capacity} most held forward {most_forward} most held walking back {most_back}"
    );
    assert!(
        most_back <= most_forward + 2,
        "walking back the same {frames} frames held {most_back} bricks of {capacity}; forward held at most {most_forward}"
    );
}

#[test]
fn a_sparse_world_that_runs_forward_in_its_pool_walks_the_same_frames_back() {
    let Some(host) = device("review a1a sparse tight pool") else {
        return;
    };
    let law = Law::compile(SWAP).unwrap();
    let size = [256, 32, 32];
    let grid = Grid3::new(size[0], size[1], size[2]).unwrap();
    let frames = 240;
    let mut start = vec![0_u32; grid.cells()];
    start[grid.index(0, 0, 0)] = 7;
    let mut far = start.clone();
    for frame in 0..frames {
        far = reference(&law, grid, frame, &far, &[]);
    }
    let slots = 4;
    let mut tight = world(&host, &law, size, Some(slots));
    tight.write(&[([0, 0, 0], 7)]).unwrap();
    tight.step(frames).unwrap();
    assert_eq!(tight.cells().unwrap(), far, "forward in {slots} slots");
    let walked = tight.back(frames);
    let cells = tight.cells();
    assert!(
        walked.is_ok() && cells.as_ref().is_ok_and(|cells| *cells == start),
        "a world that runs {frames} frames forward in {slots} slots cannot walk them back: back {:?}, cells {:?}",
        walked.err().map(|why| why.to_string()),
        cells.err().map(|why| why.to_string())
    );
}
