#![allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]

mod common;

use common::device;

use std::path::PathBuf;

use grit::Host;
use grit::serve3::{Options, Server};
use grit::wire::{Json, written};

fn line(name: &str, arguments: &str) -> String {
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"{name}\",\"arguments\":{arguments}}}}}"
    )
}

fn result(got: &Json, error: bool) -> Json {
    let held = got
        .get("result")
        .unwrap_or_else(|| panic!("{}", written(got)));
    assert_eq!(
        held.get("isError"),
        Some(&Json::Bool(error)),
        "{}",
        written(got)
    );
    held.get("structuredContent").cloned().unwrap()
}

fn ok(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), false)
}

fn no(server: &mut Server<'_>, name: &str, arguments: &str) -> Json {
    result(&server.heard(&line(name, arguments)).unwrap(), true)
}

fn int_at(held: &Json, key: &str) -> i64 {
    held.get(key)
        .and_then(Json::int)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
}

fn text_at(held: &Json, key: &str) -> String {
    held.get(key)
        .and_then(Json::text)
        .unwrap_or_else(|| panic!("no {key} in {}", written(held)))
        .to_owned()
}

fn two_planes(server: &mut Server<'_>) -> String {
    let got = ok(
        server,
        "grit_law_from_chip",
        "{\"inputs\":1,\"outputs\":1,\"gates\":[{\"kind\":10,\"left\":0,\"right\":0}],\"bind_inputs\":[{\"plane\":0,\"bit\":0}],\"bind_outputs\":[{\"plane\":1,\"bit\":31}],\"channels\":2}",
    );
    text_at(&got, "law")
}

fn created(server: &mut Server<'_>, law: &str, sparse: bool) -> i64 {
    let got = ok(
        server,
        "grit_world_create",
        &format!(
            "{{\"wide\":8,\"deep\":8,\"tall\":8,\"law\":{},\"sparse\":{sparse}}}",
            written(&Json::Text(law.to_owned()))
        ),
    );
    int_at(&got, "world")
}

fn read(server: &mut Server<'_>, world: i64, plane: u32) -> Vec<i64> {
    let got = ok(
        server,
        "grit_world_read",
        &format!("{{\"world\":{world},\"plane\":{plane},\"from\":[0,0,0],\"to\":[7,7,7]}}"),
    );
    got.get("cells")
        .and_then(Json::list)
        .unwrap()
        .iter()
        .map(|one| one.int().unwrap())
        .collect()
}

fn cells(seed: u32) -> (String, Vec<i64>) {
    let mut words = vec![0_i64; 512];
    let mut listed = Vec::new();
    for at in (0..512_u32).step_by(7) {
        let word = (at * 2_654_435_761_u32.wrapping_add(seed)) | 1;
        words[at as usize] = i64::from(word);
        listed.push(format!("[{},{},{},{word}]", at % 8, at / 8 % 8, at / 64));
    }
    (format!("[{}]", listed.join(",")), words)
}

fn written_planes(server: &mut Server<'_>, world: i64) -> (Vec<i64>, Vec<i64>) {
    let (zero, first) = cells(1);
    let got = ok(
        server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"cells\":{zero}}}"),
    );
    assert_eq!(int_at(&got, "plane"), 0, "plane 0 if left out");
    assert_eq!(read(server, world, 0), first);
    assert_eq!(read(server, world, 1), vec![0; 512]);
    let (one, second) = cells(2);
    let got = ok(
        server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"plane\":1,\"cells\":{one}}}"),
    );
    assert_eq!(int_at(&got, "plane"), 1);
    assert_eq!(read(server, world, 0), first, "plane 0 is unchanged");
    assert_eq!(read(server, world, 1), second, "plane 1 holds the write");
    (first, second)
}

#[test]
fn l6_writing_plane_1_of_a_two_plane_world_changes_only_plane_1() {
    let Some(host) = device("plane write") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = two_planes(&mut server);
    for sparse in [false, true] {
        let world = created(&mut server, &law, sparse);
        written_planes(&mut server, world);
    }
}

#[test]
fn a_plane_past_the_world_is_refused_at_plane_and_nothing_is_written() {
    let Some(host) = device("plane write refusal") else {
        return;
    };
    let mut server = Server::new(Some(&host));
    let law = two_planes(&mut server);
    let world = created(&mut server, &law, false);
    let (some, _) = cells(3);
    for plane in [2, 15] {
        let told = no(
            &mut server,
            "grit_world_write",
            &format!("{{\"world\":{world},\"plane\":{plane},\"cells\":{some}}}"),
        );
        assert_eq!(
            (text_at(&told, "refused"), text_at(&told, "field")),
            ("outside".to_owned(), "$.plane".to_owned()),
            "{}",
            written(&told)
        );
    }
    let told = no(
        &mut server,
        "grit_world_write",
        &format!("{{\"world\":{world},\"plane\":16,\"cells\":{some}}}"),
    );
    assert_eq!(text_at(&told, "field"), "$.plane", "{}", written(&told));
    assert_eq!(read(&mut server, world, 0), vec![0; 512]);
    assert_eq!(read(&mut server, world, 1), vec![0; 512]);
    let rule = ok(
        &mut server,
        "grit_world_create",
        "{\"wide\":8,\"deep\":8,\"tall\":8,\"rule\":\"sand\"}",
    );
    let told = no(
        &mut server,
        "grit_world_write",
        &format!(
            "{{\"world\":{},\"plane\":1,\"cells\":{some}}}",
            int_at(&rule, "world")
        ),
    );
    assert_eq!(text_at(&told, "field"), "$.plane", "{}", written(&told));
}

struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn served<'host>(host: &'host Host, dir: &Scratch) -> Server<'host> {
    Server::with(
        Some(host),
        &Options {
            dir: Some(dir.0.join("worlds")),
            memory: None,
        },
    )
    .unwrap()
}

#[test]
fn a_plane_write_comes_back_after_a_restart() {
    let Some(host) = device("plane write restart") else {
        return;
    };
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir =
        Scratch(std::env::temp_dir().join(format!("grit-plane-{}-{nanos}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let mut server = served(&host, &dir);
    let law = two_planes(&mut server);
    let world = created(&mut server, &law, false);
    let (first, second) = written_planes(&mut server, world);
    drop(server);
    let mut again = served(&host, &dir);
    assert_eq!(read(&mut again, world, 0), first, "plane 0 after a restart");
    assert_eq!(
        read(&mut again, world, 1),
        second,
        "plane 1 after a restart"
    );
}
