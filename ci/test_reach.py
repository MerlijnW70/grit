import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import reach

LIB = {
    "src/lib.rs": "pub mod a;\npub mod b;\npub mod c;\npub mod d;\npub use d::Held;\npub mod types;\nmod ext;\npub use types::Foo;\n",
    "src/a.rs": "use crate::b::step;\npub fn go() -> u32 { step() }\n",
    "src/b.rs": "pub fn step() -> u32 { 1 }\n",
    "src/c.rs": "pub struct Kind;\nmod extra;\n",
    "src/c/extra.rs": "use super::Kind;\nimpl Kind { pub fn more(&self) -> u32 { 2 } }\n",
    "src/d.rs": "pub struct Held;\n",
    "src/types.rs": "pub struct Foo;\n",
    "src/ext.rs": "use crate::Foo;\nimpl Foo { pub fn m(&self) {} }\n",
    "src/bin/tool.rs": "fn main() { let _ = grit::b::step(); }\n",
}


class Fake:
    def __init__(self, extra=None):
        self.dir = Path(tempfile.mkdtemp())
        files = dict(LIB)
        files["Cargo.toml"] = '[package]\nname = "grit"\nversion = "0.0.0"\n'
        for name in reach.CORE:
            files["tests/%s.rs" % name] = "#[test]\nfn holds() {}\n"
        files["tests/uses_a.rs"] = "use grit::a;\n#[test]\nfn t() { a::go(); }\n"
        files["tests/uses_c.rs"] = "use grit::c::Kind;\n#[test]\nfn t() { Kind.more(); }\n"
        files["tests/uses_m.rs"] = "use grit::types::Foo;\n#[test]\nfn t() { Foo.m(); }\n"
        files["tests/uses_root.rs"] = "use grit::*;\n#[test]\nfn t() { let _ = Held; let _ = b::step(); }\n"
        files["tests/reads.rs"] = '#[test]\nfn t() { let _ = std::fs::read("tests/data/table.txt"); }\n'
        files["tests/scans.rs"] = '#[test]\nfn t() { let _ = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"); }\n'
        files["tests/spawns.rs"] = '#[test]\nfn t() { let _ = env!("CARGO_BIN_EXE_tool"); }\n'
        files["tests/data/table.txt"] = "1\n"
        files["README.md"] = "x\n"
        files.update(extra or {})
        for path, text in files.items():
            self.write(path, text)
        subprocess.run(["git", "init", "-q"], cwd=self.dir, check=True)
        subprocess.run([sys.executable, reach.__file__, "--root", str(self.dir), "write"], check=True)

    def write(self, path, text):
        target = self.dir / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(text, encoding="utf-8")

    def close(self):
        shutil.rmtree(self.dir, ignore_errors=True)


class Selection(unittest.TestCase):
    def setUp(self):
        self.fake = Fake()
        self.root = self.fake.dir

    def tearDown(self):
        self.fake.close()

    def picked(self, *paths):
        return reach.select(self.root, list(paths))[0]

    def test_a_fresh_mapping_has_no_problems(self):
        self.assertEqual(reach.problems(self.root), [])

    def test_cargo_lock_selects_every_target(self):
        self.assertIsNone(self.picked("Cargo.lock"))

    def test_each_unsure_path_selects_every_target(self):
        for path in (
            "Cargo.toml",
            "build.rs",
            ".github/workflows/ci.yml",
            "tests/common/mod.rs",
            "ci/reach.py",
            "ci/skips.txt",
            "src/kernel.spv",
            "src/gone.rs",
            ".gitattributes",
            ".cargo/config.toml",
            "rust-toolchain.toml",
            "clippy.toml",
        ):
            self.assertIsNone(self.picked(path), path)

    def test_a_change_selects_the_targets_that_reach_it_and_the_core(self):
        picked = self.picked("src/b.rs")
        self.assertEqual(set(picked), set(reach.CORE) | {"uses_a", "uses_root", "scans", "spawns"})

    def test_an_impl_in_a_child_is_reached_through_its_type(self):
        self.assertIn("uses_c", self.picked("src/c/extra.rs"))
        self.assertNotIn("uses_a", self.picked("src/c/extra.rs"))

    def test_every_target_that_uses_the_library_reaches_its_root(self):
        self.assertIn("uses_c", self.picked("src/lib.rs"))

    def test_a_root_reexport_reaches_its_module(self):
        self.assertIn("uses_root", self.picked("src/d.rs"))
        self.assertNotIn("uses_a", self.picked("src/d.rs"))

    def test_a_test_that_reads_src_reaches_every_src_file(self):
        self.assertIn("scans", self.picked("src/c/extra.rs"))

    def test_a_test_that_runs_a_bin_reaches_the_bin(self):
        self.assertIn("spawns", self.picked("src/bin/tool.rs"))
        self.assertIn("spawns", self.picked("src/b.rs"))

    def test_a_fixture_selects_the_targets_that_name_it(self):
        self.assertEqual(set(self.picked("tests/data/table.txt")), set(reach.CORE) | {"reads"})

    def test_an_unnamed_file_adds_only_the_core(self):
        self.assertEqual(set(self.picked("README.md")), set(reach.CORE))

    def test_each_quiet_path_adds_only_the_core(self):
        for path in ("CONTRIBUTING.md", "LICENSE-MIT", "docs/guide.md", "demos/x.txt", "front/a.js", ".gitignore"):
            self.assertEqual(set(self.picked(path)), set(reach.CORE), path)

    def test_an_unknown_top_level_path_selects_every_target(self):
        self.assertIsNone(self.picked(".cargo/config.toml"))
        self.assertIsNone(self.picked("rust-toolchain.toml"))

    def test_an_impl_of_a_reexported_type_is_reached(self):
        self.assertIn("uses_m", self.picked("src/ext.rs"))
        self.assertNotIn("uses_a", self.picked("src/ext.rs"))

    def test_a_touched_target_is_selected(self):
        self.assertIn("uses_a", self.picked("tests/uses_a.rs"))

    def test_a_rust_file_in_no_target_selects_every_target(self):
        self.fake.write("tests/stray/loose.rs", "fn f() {}\n")
        self.assertIsNone(self.picked("tests/stray/loose.rs"))

    def test_a_new_test_file_without_a_mapping_entry_fails_the_check(self):
        self.fake.write("tests/fresh.rs", "#[test]\nfn t() {}\n")
        found = reach.problems(self.root)
        self.assertIn("target fresh is not in ci/reach.txt", found)
        self.assertIn("target fresh has no row in ci/skips.txt", found)
        self.assertIsNone(self.picked("tests/fresh.rs"))

    def test_a_new_reach_fails_the_check(self):
        self.fake.write("tests/uses_c.rs", "use grit::{a, c::Kind};\n#[test]\nfn t() { a::go(); Kind.more(); }\n")
        self.assertTrue(any(line.startswith("target uses_c reach changed") for line in reach.problems(self.root)))

    def test_a_src_module_that_maps_to_nothing_fails_the_check(self):
        self.fake.write("tests/scans.rs", "#[test]\nfn t() {}\n")
        self.fake.write("tests/uses_c.rs", "#[test]\nfn t() {}\n")
        subprocess.run([sys.executable, reach.__file__, "--root", str(self.root), "write"], check=True)
        found = reach.problems(self.root)
        self.assertIn("src module src/c/extra.rs maps to no test target", found)

    def test_a_doc_test_in_src_fails_the_check(self):
        self.fake.write("src/b.rs", "/// ```\n/// assert!(true);\n/// ```\npub fn step() -> u32 { 1 }\n")
        self.assertIn("src file src/b.rs holds a doc test, which a selected run leaves out", reach.problems(self.root))

    def test_an_include_fails_the_check(self):
        self.fake.write("tests/uses_a.rs", 'include!("../src/b.rs");\n#[test]\nfn t() {}\n')
        self.assertIn("tests/uses_a.rs uses include!, which ci/reach.py does not follow", reach.problems(self.root))

    def test_a_path_from_the_extern_root_is_followed(self):
        self.fake.write("tests/uses_c.rs", "#[test]\nfn t() { ::grit::a::go(); }\n")
        self.assertIn("uses_c", reach.Repo(self.root).owners("tests/uses_c.rs"))
        self.assertIn("src/b.rs", reach.Repo(self.root).reach("uses_c"))

    def test_a_json_block_in_a_doc_comment_is_no_doc_test(self):
        self.fake.write("src/b.rs", "/// ```json\n/// {}\n/// ```\npub fn step() -> u32 { 1 }\n")
        self.assertEqual(reach.problems(self.root), [])


class Unfollowed(unittest.TestCase):
    def refusals(self, ext):
        lib = LIB["src/lib.rs"] + "pub mod hub;\nmod ext2;\npub trait Tg {}\n"
        fake = Fake({"src/lib.rs": lib, "src/hub.rs": "pub use crate::types::*;\n", "src/ext2.rs": ext})
        try:
            return [p for p in reach.problems(fake.dir) if "does not follow this impl" in p]
        finally:
            fake.close()

    def assert_refused(self, ext):
        found = self.refusals(ext)
        self.assertEqual(len(found), 1, found)
        self.assertTrue(found[0].startswith("src/ext2.rs has an impl naming Foo, whose definition in src/types.rs"), found)

    def test_an_impl_through_an_alias_fails_the_check(self):
        self.assert_refused("use crate::Foo as Bar;\nimpl Bar { pub fn m2(&self) {} }\n")

    def test_an_impl_through_a_glob_reexport_path_fails_the_check(self):
        self.assert_refused("use crate::hub;\nimpl hub::Foo { pub fn m2(&self) {} }\n")

    def test_an_impl_on_a_generic_of_a_crate_type_fails_the_check(self):
        self.assert_refused("use crate::Foo;\nuse crate::Tg;\nimpl Tg for Vec<Foo> {}\n")

    def test_an_impl_with_a_crate_type_as_generic_argument_fails_the_check(self):
        self.assert_refused("use crate::Foo;\nimpl From<Foo> for u32 { fn from(_: Foo) -> u32 { 0 } }\n")

    def test_a_followed_reexport_impl_passes_the_check(self):
        self.assertEqual(self.refusals("use crate::Foo;\nimpl Foo { pub fn m2(&self) {} }\n"), [])


class Lexing(unittest.TestCase):
    def test_strings_and_comments_leave_the_code(self):
        code, strings = reach.lex('let a = "x::y"; // crate::z\nlet b = r#"q"#; /* crate::w */ let c = \'"\';')
        self.assertNotIn("crate", code)
        self.assertEqual(strings, ["x::y", "q"])

    def test_use_groups_expand(self):
        leaves = reach.use_leaves("use crate::{a::{self, b}, c as d, e::*};")
        self.assertEqual(leaves, ["crate::a::self", "crate::a::b", "crate::c@d", "crate::e::*"])


class Planning(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())
        (self.dir / "ci").mkdir()
        (self.dir / "ci" / "times.txt").write_text(
            "target 4 8 16\nbig 1000 1000 1000\nsmall 100 100 100\n", encoding="utf-8"
        )
        (self.dir / "ci" / "skips.txt").write_text("target 4 8 16\nbig 1 0 0\nsmall 0 0 2\n", encoding="utf-8")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def test_the_whole_suite_is_one_job_per_width(self):
        matrix = reach.plan(self.dir, None)
        self.assertEqual([(m["width"], m["work"]) for m in matrix], [(4, "*"), (8, "*"), (16, "*")])

    def test_a_target_past_the_budget_is_split_and_every_part_planned(self):
        matrix = reach.plan(self.dir, ["big", "small"], 540, 8)
        work = [m["work"] for m in matrix if m["width"] == 4]
        items = sorted(i for w in work for i in w.split())
        self.assertEqual(items, ["big@0/2", "big@1/2", "small"])
        self.assertTrue(all(m["estimate"] <= 540 for m in matrix))

    def test_each_target_in_a_shard_pays_its_link(self):
        matrix = reach.plan(self.dir, ["small"], 540, 8)
        self.assertEqual(matrix[0]["estimate"], 100 + reach.LINK_SECS)

    def test_the_shard_cap_raises_the_budget(self):
        matrix = reach.plan(self.dir, ["big", "small"], 100, 2)
        self.assertEqual(len([m for m in matrix if m["width"] == 4]), 2)

    def result(self, width, work, skips):
        return {"width": width, "work": work, "skips": skips}

    def test_a_skip_count_that_differs_from_the_table_fails(self):
        matrix = [{"width": 4, "work": "small"}]
        lines, errors = reach.tally(self.dir, matrix, [self.result(4, "small", {"small": 1})])
        self.assertEqual(errors, ["width 4: small skipped 1, ci/skips.txt says 0", "width 4: skipped 1 != 0"])

    def test_the_parts_of_a_split_target_are_summed(self):
        matrix = [{"width": 4, "work": "big@0/2"}, {"width": 4, "work": "big@1/2 small"}]
        results = [
            self.result(4, "big@0/2", {"big@0/2": 0}),
            self.result(4, "big@1/2 small", {"big@1/2": 1, "small": 0}),
        ]
        self.assertEqual(reach.tally(self.dir, matrix, results)[1], [])

    def test_a_missing_shard_fails(self):
        matrix = [{"width": 4, "work": "big@0/2"}, {"width": 4, "work": "big@1/2 small"}]
        errors = reach.tally(self.dir, matrix, [self.result(4, "big@0/2", {"big@0/2": 1})])[1]
        self.assertEqual(errors, ["width 4: 1 of 2 shards reported"])

    def test_a_failed_shard_fails(self):
        matrix = [{"width": 4, "work": "small"}]
        result = dict(self.result(4, "small", {"small": 0}), failed=["small"])
        self.assertEqual(reach.tally(self.dir, matrix, [result])[1], ["width 4: small failed"])

    def test_a_shard_whose_build_failed_does_not_match_zero(self):
        matrix = [{"width": 8, "work": "small"}]
        result = dict(self.result(8, "small", {}), failed=["build"])
        errors = reach.tally(self.dir, matrix, [result])[1]
        self.assertIn("width 8: build failed", errors)
        self.assertIn("width 8: small reported no result", errors)

    def test_a_planned_part_with_no_result_fails(self):
        matrix = [{"width": 4, "work": "big@0/2 big@1/2"}]
        errors = reach.tally(self.dir, matrix, [self.result(4, "big@0/2 big@1/2", {"big@0/2": 1})])[1]
        self.assertEqual(errors, ["width 4: big@1/2 reported no result"])

    def test_a_whole_run_missing_a_target_fails(self):
        matrix = [{"width": 4, "work": "*"}]
        errors = reach.tally(self.dir, matrix, [self.result(4, "*", {"big": 1})])[1]
        self.assertEqual(errors, ["width 4: small reported no result"])

    def test_a_whole_run_is_held_to_every_row(self):
        matrix = [{"width": 16, "work": "*"}]
        skips = {"big": 0, "small": 2, "src/lib.rs": 0}
        self.assertEqual(reach.tally(self.dir, matrix, [self.result(16, "*", skips)])[1], [])
        skips["small"] = 1
        self.assertIn("width 16: skipped 1 != 2", reach.tally(self.dir, matrix, [self.result(16, "*", skips)])[1])

    def test_skips_are_counted_per_target_from_the_cargo_output(self):
        text = (
            "     Running unittests src/lib.rs (target/x)\n"
            "     Running tests/big.rs (target/y)\n"
            "SKIPPED one\nSKIPPED\n"
            "test result: ok. 1 passed; finished in 2.50s\n"
            "     Running tests/small/main.rs (target/z)\n"
            "SKIPPED two SKIPPED\n"
        )
        skips, secs = reach.attribute(text)
        self.assertEqual(skips, {"src/lib.rs": 0, "big": 2, "small": 2})
        self.assertEqual(secs, {"big": 2.5})


if __name__ == "__main__":
    unittest.main()
