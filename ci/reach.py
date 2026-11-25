import argparse
import json
import math
import re
import subprocess
import sys
import time
import tomllib
from pathlib import Path

WIDTHS = (4, 8, 16)
VECTORS = {4: 128, 8: 256, 16: 512}
CORE = ("bricks", "golden", "law", "law_devices", "listed3", "one_step", "sparse3", "sparse_halo")
WHOLE_DIRS = ("tests/common/",)
QUIET_FILES = (".gitignore",)
QUIET_DIRS = ("docs/", "demos/", "front/")
INERT = ("ci/reach.txt",)
LIBRARY = "grit"
BUDGET = 540
MOST_SHARDS = 4
UNKNOWN_SECS = 300
LINK_SECS = 23
CARGO = ["cargo", "test", "--release", "--locked", "--no-default-features", "--features", "deep,faults"]
IDENT = r"[A-Za-z_][A-Za-z0-9_]*"
STAMP = re.compile(r"^\d{4}-\d\d-\d\dT[0-9:.]+Z ")
COLOUR = re.compile(r"\x1b\[[0-9;]*m")


def lex(text):
    code = []
    strings = []
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if text.startswith("/*", i):
            depth = 0
            while i < n:
                if text.startswith("/*", i):
                    depth += 1
                    i += 2
                elif text.startswith("*/", i):
                    depth -= 1
                    i += 2
                    if depth == 0:
                        break
                else:
                    i += 1
            code.append(" ")
            continue
        joined = i > 0 and (text[i - 1].isalnum() or text[i - 1] == "_")
        raw = None if joined else re.match(r"b?r(#*)\"", text[i : i + 260])
        if raw:
            hashes = raw.group(1)
            start = i + raw.end()
            end = text.find('"' + hashes, start)
            end = n if end < 0 else end
            strings.append(text[start:end])
            code.append('"%d"' % (len(strings) - 1))
            i = end + 1 + len(hashes)
            continue
        if c == '"' or (c == "b" and not joined and text.startswith('b"', i)):
            j = i + (2 if c == "b" else 1)
            out = []
            while j < n and text[j] != '"':
                if text[j] == "\\" and j + 1 < n:
                    out.append(text[j : j + 2])
                    j += 2
                else:
                    out.append(text[j])
                    j += 1
            strings.append("".join(out))
            code.append('"%d"' % (len(strings) - 1))
            i = j + 1
            continue
        if c == "'":
            if text.startswith("'\\", i):
                j = text.find("'", i + 2)
                i = n if j < 0 else j + 1
                code.append("' '")
                continue
            if i + 2 < n and text[i + 2] == "'":
                i += 3
                code.append("' '")
                continue
        code.append(c)
        i += 1
    return "".join(code), strings


class Source:
    def __init__(self, text):
        code, self.strings = lex(text)
        self.code = re.sub(r"(?<![\w:>])::\s*(?=[A-Za-z_])", "", code)


class Tree:
    def __init__(self, root):
        self.root = Path(root)
        self.held = {}

    def files(self):
        out = subprocess.run(
            ["git", "ls-files", "-z", "--cached", "--others", "--exclude-standard"],
            cwd=self.root,
            capture_output=True,
            check=True,
        ).stdout.decode()
        return sorted({p for p in out.split("\0") if p and (self.root / p).is_file()})

    def source(self, path):
        if path not in self.held:
            self.held[path] = Source((self.root / path).read_text(encoding="utf-8", errors="replace"))
        return self.held[path]


def norm(path):
    parts = []
    for part in path.replace("\\", "/").split("/"):
        if part in ("", "."):
            continue
        if part == ".." and parts and parts[-1] != "..":
            parts.pop()
        else:
            parts.append(part)
    return "/".join(parts)


class Crate:
    def __init__(self, tree, root):
        self.tree = tree
        self.root = root
        self.modules = {}
        self.children = {}
        self.parent = {}
        self.walk(root, (), True)

    def walk(self, path, module, owns_dir):
        self.modules[module] = path
        self.children[module] = {}
        source = self.tree.source(path)
        base = path.rsplit("/", 1)[0] if "/" in path else ""
        own = base if owns_dir else path[: -len(".rs")]
        pattern = re.compile(r"((?:#\s*\[[^\]]*\]\s*)*)(?:pub(?:\s*\([^)]*\))?\s+)?mod\s+(" + IDENT + r")\s*;")
        for found in pattern.finditer(source.code):
            name = found.group(2)
            fixed = re.search(r'\bpath\s*=\s*"(\d+)"', found.group(1))
            if fixed:
                options = [norm(base + "/" + source.strings[int(fixed.group(1))])]
            else:
                options = [norm(own + "/" + name + ".rs"), norm(own + "/" + name + "/mod.rs")]
            chosen = next((o for o in options if (self.tree.root / o).is_file()), None)
            if chosen is None:
                raise SystemExit("no file for mod %s in %s: %s" % (name, path, " ".join(options)))
            child = module + (name,)
            self.children[module][name] = child
            self.parent[child] = module
            self.walk(chosen, child, chosen.endswith("/mod.rs"))

    def files(self):
        return sorted(set(self.modules.values()))


def use_leaves(code):
    out = []
    for found in re.finditer(r"(?<![\w:])use\s+", code):
        end = code.find(";", found.end())
        if end < 0:
            continue
        body = re.sub(r"\s+as\s+", "@", code[found.end() : end])
        out.extend(expand(re.sub(r"\s+", "", body)))
    return out


def expand(body):
    if "{" not in body:
        return [body]
    i = body.index("{")
    prefix = body[:i]
    inner = body[i + 1 : body.rindex("}")]
    parts = []
    depth = 0
    start = 0
    for j, c in enumerate(inner):
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
        elif c == "," and depth == 0:
            parts.append(inner[start:j])
            start = j + 1
    parts.append(inner[start:])
    return [prefix + leaf for part in parts if part for leaf in expand(part)]


class Resolver:
    def __init__(self, crate, library):
        self.crate = crate
        self.library = library

    def descend(self, crate, at, rest):
        for i, segment in enumerate(rest):
            if segment == "self":
                continue
            if segment == "*":
                return at, True
            if segment == "super" and at in crate.parent:
                at = crate.parent[at]
                continue
            child = crate.children.get(at, {}).get(segment)
            if child is None:
                return at, False
            at = child
        return at, True

    def resolve(self, module, segments, aliases):
        crate = self.crate
        head = segments[0]
        rest = segments[1:]
        if head == "crate":
            return ("own",) + self.descend(crate, (), rest)
        if head == "self":
            return ("own",) + self.descend(crate, module, rest)
        if head == "super":
            at = module
            while segments and segments[0] == "super":
                if at not in crate.parent:
                    return None
                at = crate.parent[at]
                segments = segments[1:]
            return ("own",) + self.descend(crate, at, segments)
        if head == LIBRARY and self.library is not None:
            return ("lib",) + self.descend(self.library, (), rest)
        if head in crate.children.get(module, {}):
            return ("own",) + self.descend(crate, crate.children[module][head], rest)
        if head in aliases:
            kind, at = aliases[head]
            owner = self.library if kind == "lib" else crate
            return (kind,) + self.descend(owner, at, rest)
        return None

    def scope(self, module, code):
        hits = []
        aliases = {}
        items = {}
        globs = []
        for leaf in use_leaves(code):
            path, _, alias = leaf.partition("@")
            segments = [s for s in path.split("::") if s]
            if not segments:
                continue
            hit = self.resolve(module, segments, aliases)
            if hit is None:
                continue
            kind, at, complete = hit
            hits.append((kind, at))
            last = segments[-1]
            if not complete:
                items.setdefault(alias or last, set()).add((kind, at))
                continue
            owner = self.library if kind == "lib" else self.crate
            if last == "*":
                globs.append((kind, at))
                for name, child in owner.children.get(at, {}).items():
                    aliases[name] = (kind, child)
                continue
            name = alias or (segments[-2] if last == "self" and len(segments) > 1 else last)
            aliases[name] = (kind, at)
        return hits, aliases, items, globs

    def paths(self, module, code, aliases):
        hits = []
        for found in re.finditer(r"(?<![\w:])(" + IDENT + r"(?:\s*::\s*" + IDENT + r")+)", code):
            segments = [s.strip() for s in found.group(1).split("::")]
            hit = self.resolve(module, segments, aliases)
            if hit is not None:
                hits.append(hit[:2])
        return hits

    def references(self, module, code):
        hits, aliases, _, _ = self.scope(module, code)
        return hits + self.paths(module, code, aliases)


def strip_generics(text):
    text = text.strip()
    if not text.startswith("<"):
        return " " + text + " "
    depth = 0
    for i, c in enumerate(text):
        if c == "<":
            depth += 1
        elif c == ">" and text[i - 1] != "-":
            depth -= 1
            if depth == 0:
                return " " + text[i + 1 :].strip() + " "
    return " " + text + " "


def impl_names(header):
    bare = None
    while bare != header:
        bare, header = header, re.sub(r"<[^<>]*>", " ", header)
    qualified = r"(?:" + IDENT + r"\s*::\s*)+" + IDENT
    names = set(re.findall(IDENT, re.sub(qualified, " ", header)))
    for found in re.finditer(qualified, header):
        parts = re.findall(IDENT, found.group(0))
        if parts[0] in ("crate", "self", "super"):
            names.add(parts[-1])
    return names


class Library:
    def __init__(self, tree, crate):
        self.crate = crate
        files = crate.files()
        self.edges = {path: set() for path in files}
        self.defined = {}
        self.macros = {}
        for path in files:
            code = tree.source(path).code
            for found in re.finditer(r"\b(?:struct|enum|union|trait)\s+(" + IDENT + ")", code):
                self.defined.setdefault(found.group(1), set()).add(path)
            for found in re.finditer(r"(?m)^(?:pub(?:\s*\([^)]*\))?\s+)?type\s+(" + IDENT + ")", code):
                self.defined.setdefault(found.group(1), set()).add(path)
            for found in re.finditer(r"\bmacro_rules!\s*(" + IDENT + ")", code):
                self.macros.setdefault(found.group(1), set()).add(path)
        resolver = Resolver(crate, None)
        for module, path in crate.modules.items():
            code = tree.source(path).code
            for _, target in resolver.references(module, code):
                self.edges[path].add(crate.modules[target])
            for home in self.impl_homes(resolver, module, path, code):
                self.edges[home].add(path)
            self.edges[path] |= self.macro_homes(code)
        for path in files:
            self.edges[path].discard(path)

    def impl_homes(self, resolver, module, path, code):
        _, aliases, items, globs = resolver.scope(module, code)
        local = {name for name, homes in self.defined.items() if path in homes}
        out = set()
        for found in re.finditer(r"(?m)^[ \t]*(?:unsafe\s+)?impl\b([^{;]*)\{", code):
            header = re.split(r"\bwhere\b", strip_generics(found.group(1)))[0]
            trait, _, own = header.rpartition(" for ")
            homes = self.named(resolver, module, own, aliases, items, globs, local, path)
            named = homes | self.named(resolver, module, trait, aliases, items, globs, local, path)
            if not homes and trait:
                homes = named
            out |= homes
            for name in impl_names(header):
                if not named & self.defined.get(name, set()):
                    out |= self.defined.get(name, set())
        out.discard(path)
        return out

    def named(self, resolver, module, text, aliases, items, globs, local, path):
        homes = set()
        for _, at in resolver.paths(module, text, aliases):
            homes.add(self.crate.modules[at])
        bare = re.sub(r"(?:" + IDENT + r"\s*::\s*)+" + IDENT, " ", text)
        for name in set(re.findall(IDENT, bare)):
            if name in local:
                homes.add(path)
                continue
            for _, at in items.get(name, ()):
                homes.add(self.crate.modules[at])
            for _, at in globs:
                if self.crate.modules[at] in self.defined.get(name, ()):
                    homes.add(self.crate.modules[at])
        return homes

    def macro_homes(self, code):
        out = set()
        for name, homes in self.macros.items():
            if re.search(r"(?<![\w!])" + name + r"\s*!", code):
                out |= homes
        return out

    def closure(self, starts):
        seen = set()
        stack = list(starts)
        while stack:
            path = stack.pop()
            if path not in seen:
                seen.add(path)
                stack.extend(self.edges.get(path, ()))
        return seen


class Repo:
    def __init__(self, root):
        self.root = Path(root)
        self.tree = Tree(root)
        self.all_files = self.tree.files()
        manifest = tomllib.loads((self.root / "Cargo.toml").read_text(encoding="utf-8"))
        self.lib = Crate(self.tree, "src/lib.rs")
        self.library = Library(self.tree, self.lib)
        self.bins = self.declared(manifest, "bin", r"src/bin/([^/]+)\.rs", r"src/bin/([^/]+)/main\.rs")
        self.targets = self.declared(manifest, "test", r"tests/([^/]+)\.rs", r"tests/([^/]+)/main\.rs")
        self.crates = {name: Crate(self.tree, root) for name, root in self.targets.items()}
        self.bin_crates = {}
        self.src_files = sorted(p for p in self.all_files if p.startswith("src/") and p.endswith(".rs"))
        self.held_reach = {}

    def declared(self, manifest, kind, flat, nested):
        out = {}
        for path in self.all_files:
            m = re.fullmatch(flat, path) or re.fullmatch(nested, path)
            if m:
                out[m.group(1)] = path
        for entry in manifest.get(kind, []):
            if "path" in entry:
                out = {k: v for k, v in out.items() if v != norm(entry["path"])}
                out[entry["name"]] = norm(entry["path"])
        return dict(sorted(out.items()))

    def bin_crate(self, name):
        if name not in self.bin_crates:
            self.bin_crates[name] = Crate(self.tree, self.bins[name])
        return self.bin_crates[name]

    def lib_entries(self, crate):
        resolver = Resolver(crate, self.lib)
        starts = set()
        for module, path in crate.modules.items():
            code = self.tree.source(path).code
            for kind, target in resolver.references(module, code):
                if kind == "lib":
                    starts.add(self.lib.modules[target])
            starts |= self.library.macro_homes(code)
        return starts

    def strings(self, crate):
        return [text for path in crate.files() for text in self.tree.source(path).strings]

    def reads_src(self, crate):
        return any(text in ("src", "CARGO", "--lib") or text.startswith("src/") for text in self.strings(crate))

    def bin_uses(self, crate):
        out = set()
        for path in crate.files():
            source = self.tree.source(path)
            for found in re.finditer(r"CARGO_BIN_EXE_([A-Za-z0-9_-]+)", source.code + " " + " ".join(source.strings)):
                out.add(found.group(1))
        return out

    def reach(self, name):
        if name in self.held_reach:
            return self.held_reach[name]
        crate = self.crates[name]
        if self.reads_src(crate):
            out = set(self.src_files)
        else:
            out = self.library.closure(self.lib_entries(crate))
            out |= {p for p in crate.files() if p.startswith("src/")}
            for used in self.bin_uses(crate):
                if used not in self.bins:
                    raise SystemExit("%s runs bin %s, which is not a bin" % (name, used))
                bin_crate = self.bin_crate(used)
                out |= set(bin_crate.files())
                out |= self.library.closure(self.lib_entries(bin_crate))
            if out & set(self.lib.files()):
                out.add(self.lib.root)
        self.held_reach[name] = out
        return out

    def mapping(self):
        return {name: sorted(self.reach(name)) for name in self.targets}

    def owners(self, path):
        return {name for name, crate in self.crates.items() if path in crate.modules.values()}

    def naming(self, path, crates):
        return {name for name, crate in crates.items() if any(path_matches(s, path) for s in self.strings(crate))}


def path_matches(text, path):
    if not text or "\n" in text or len(text) > 300:
        return False
    parts = path.split("/")
    if text in parts[1:] or (text == parts[0] and text != "tests"):
        return True
    if len(parts[-1]) >= 4 and parts[-1] in text:
        return True
    if "/" not in text and "{" not in text:
        return False
    fixed = re.sub(r"\{[^}]*\}", "", text)
    if len(fixed.strip("/")) < 4:
        return False
    pattern = re.compile(re.sub(r"\\\{[^}]*\\\}", "[^/]*", re.escape(text.strip("/").removeprefix("./"))))
    for i in range(len(parts)):
        for j in range(i + 1, len(parts) + 1):
            if pattern.fullmatch("/".join(parts[i:j])):
                return True
    return False


def read_reach(path):
    targets = {}
    none = set()
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            head, _, rest = line.partition(":")
            if not head.strip():
                continue
            if head == "none":
                none.update(rest.split())
            else:
                targets[head] = rest.split()
    return targets, none


def write_reach(path, mapping, none):
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["%s: %s" % (name, " ".join(files)) for name, files in sorted(mapping.items())]
    lines.append("none: %s" % " ".join(sorted(none)))
    path.write_text("\n".join(line.rstrip() for line in lines) + "\n", encoding="utf-8")


def read_table(path, kind):
    rows = {}
    if path.is_file():
        for line in path.read_text(encoding="utf-8").splitlines():
            parts = line.split()
            if not parts or parts[0] == "target":
                continue
            if len(parts) != len(WIDTHS) + 1:
                raise SystemExit("%s: bad row %r" % (path, line))
            rows[parts[0]] = {w: kind(v) for w, v in zip(WIDTHS, parts[1:])}
    return rows


def skips_table(root):
    return read_table(Path(root) / "ci" / "skips.txt", int)


def times_table(root):
    return read_table(Path(root) / "ci" / "times.txt", float)


def write_table(path, table, shape):
    path.parent.mkdir(parents=True, exist_ok=True)
    lines = ["target " + " ".join(str(w) for w in WIDTHS)]
    for name in sorted(table):
        lines.append("%s %s" % (name, " ".join(shape % table[name][w] for w in WIDTHS)))
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")


def problems(root, repo=None):
    root = Path(root)
    repo = repo or Repo(root)
    mapping = repo.mapping()
    held, none = read_reach(root / "ci" / "reach.txt")
    out = []
    for name in repo.targets:
        if name not in held:
            out.append("target %s is not in ci/reach.txt" % name)
        elif held[name] != mapping[name]:
            gone = sorted(set(held[name]) - set(mapping[name]))
            new = sorted(set(mapping[name]) - set(held[name]))
            out.append("target %s reach changed: +%s -%s" % (name, " ".join(new), " ".join(gone)))
    for name in held:
        if name not in repo.targets:
            out.append("ci/reach.txt names %s, which is not a test target" % name)
    reached = {path for files in mapping.values() for path in files}
    for path in repo.src_files:
        if path not in reached and path not in none:
            out.append("src module %s maps to no test target" % path)
        if path in reached and path in none:
            out.append("src module %s is reached but listed under none" % path)
    for path in sorted(none):
        if path not in repo.src_files:
            out.append("none names %s, which is not a src module" % path)
    skips = skips_table(root)
    for name in repo.targets:
        if name not in skips:
            out.append("target %s has no row in ci/skips.txt" % name)
    for name in skips:
        if name not in repo.targets:
            out.append("ci/skips.txt names %s, which is not a test target" % name)
    for name in CORE:
        if name not in repo.targets:
            out.append("core target %s is not a test target" % name)
    for path in sorted({p for c in [repo.lib, *repo.crates.values()] for p in c.files()}):
        if re.search(r"(?<![\w!])include\s*!", repo.tree.source(path).code):
            out.append("%s uses include!, which ci/reach.py does not follow" % path)
    for path in repo.src_files:
        if re.search(r"#\s*\[\s*(test\b|cfg\s*\(\s*test\b)", repo.tree.source(path).code):
            out.append("src file %s holds unit tests, which a selected run leaves out" % path)
        text = (root / path).read_text(encoding="utf-8", errors="replace")
        opening = True
        for fence in re.finditer(r"(?m)^\s*//[/!]\s*```\s*([\w,]*)", text):
            words = fence.group(1).split(",")
            if opening and words[0] in ("", "rust") and "ignore" not in words:
                out.append("src file %s holds a doc test, which a selected run leaves out" % path)
                break
            opening = not opening
    return out


def changed(root, base, head):
    out = subprocess.run(
        ["git", "diff", "--name-only", "--no-renames", base, head],
        cwd=root,
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return sorted(p for p in out.splitlines() if p)


def quiet(path):
    if path in QUIET_FILES or path.startswith(QUIET_DIRS):
        return True
    return "/" not in path and (path.endswith(".md") or path.startswith("LICENSE-"))


def select(root, paths):
    root = Path(root)
    repo = Repo(root)
    reasons = []
    picked = set(CORE)
    whole = False
    if problems(root, repo):
        reasons.append("ci/reach.txt or ci/skips.txt is out of date: every target")
        whole = True
    mapping = repo.mapping()
    present = set(repo.all_files)
    for path in paths:
        if path in INERT:
            reasons.append("%s: checked by library" % path)
            continue
        if path.startswith(WHOLE_DIRS):
            reasons.append("%s: every target" % path)
            whole = True
            continue
        if path.startswith("src/"):
            if not path.endswith(".rs"):
                reasons.append("%s: not Rust source, every target" % path)
                whole = True
            elif path not in present:
                reasons.append("%s: removed, every target" % path)
                whole = True
            else:
                hits = {name for name, files in mapping.items() if path in files}
                picked |= hits
                reasons.append("%s: %s" % (path, " ".join(sorted(hits))))
            continue
        if path.startswith("tests/"):
            rest = path[len("tests/") :]
            first = rest.split("/")[0]
            name = first[: -len(".rs")] if "/" not in rest else first
            hits = repo.owners(path)
            if name in repo.targets:
                hits.add(name)
            if path.endswith(".rs") and path in present and not hits:
                reasons.append("%s: in no target, every target" % path)
                whole = True
                continue
            if not path.endswith(".rs"):
                hits |= repo.naming(path, repo.crates)
            picked |= hits
            reasons.append("%s: %s" % (path, " ".join(sorted(hits))))
            continue
        if any(path_matches(s, path) for s in repo.strings(repo.lib)):
            reasons.append("%s: named in src, every target" % path)
            whole = True
            continue
        if not quiet(path):
            reasons.append("%s: outside src and tests, every target" % path)
            whole = True
            continue
        hits = repo.naming(path, repo.crates)
        picked |= hits
        reasons.append("%s: %s" % (path, " ".join(sorted(hits))))
    if whole:
        return None, reasons
    return sorted(picked & set(repo.targets)), reasons


def pack(costs, budget):
    items = []
    for name, secs in costs.items():
        parts = max(1, math.ceil(secs / budget))
        for part in range(parts):
            item = name if parts == 1 else "%s@%d/%d" % (name, part, parts)
            items.append((secs / parts + LINK_SECS, item))
    items.sort(key=lambda x: (-x[0], x[1]))
    shards = []
    for secs, item in items:
        for shard in shards:
            if shard[0] + secs <= budget:
                shard[0] += secs
                shard[1].append(item)
                break
        else:
            shards.append([secs, [item]])
    return shards


def plan(root, picked, budget=BUDGET, most=MOST_SHARDS):
    times = times_table(root)
    matrix = []
    for width in WIDTHS:
        if picked is None:
            matrix.append({"width": width, "vector": VECTORS[width], "shard": "whole", "work": "*"})
            continue
        costs = {name: times.get(name, {}).get(width, UNKNOWN_SECS) for name in picked}
        wide = max(budget, math.ceil((sum(costs.values()) + LINK_SECS * len(costs)) / most))
        shards = pack(costs, wide)
        while len(shards) > most:
            wide = math.ceil(wide * 1.05)
            shards = pack(costs, wide)
        for i, (secs, work) in enumerate(shards):
            matrix.append(
                {
                    "width": width,
                    "vector": VECTORS[width],
                    "shard": "%d/%d" % (i + 1, len(shards)),
                    "work": " ".join(sorted(work)),
                    "estimate": round(secs),
                }
            )
    return matrix


def parse_item(item):
    m = re.fullmatch(r"(.+)@(\d+)/(\d+)", item)
    if m:
        return m.group(1), int(m.group(2)), int(m.group(3))
    return item, 0, 1


def clean(line):
    return COLOUR.sub("", STAMP.sub("", line))


def target_of(line):
    m = re.match(r"\s*Running (?:unittests )?(\S+)", line)
    if not m:
        return None
    path = m.group(1)
    t = re.fullmatch(r"tests/([^/]+)\.rs", path) or re.fullmatch(r"tests/([^/]+)/main\.rs", path)
    return t.group(1) if t else path


def attribute(text):
    skips = {}
    secs = {}
    current = None
    for raw in text.splitlines():
        line = clean(raw)
        named = target_of(line)
        if named is not None:
            current = named
            skips.setdefault(current, 0)
            continue
        if re.match(r"\s*Doc-tests ", line):
            current = "doc-tests"
            skips.setdefault(current, 0)
            continue
        found = re.search(r"test result: .*finished in ([0-9.]+)s", line)
        if found and current is not None:
            secs[current] = secs.get(current, 0.0) + float(found.group(1))
        count = line.count("SKIPPED")
        if count:
            key = current if current is not None else "outside any target"
            skips[key] = skips.get(key, 0) + count
    return skips, secs


def stream(cmd, cwd):
    proc = subprocess.Popen(cmd, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
    chunks = []
    for raw in iter(proc.stdout.readline, b""):
        line = raw.decode("utf-8", errors="replace")
        sys.stdout.write(line)
        sys.stdout.flush()
        chunks.append(line)
    return proc.wait(), "".join(chunks)


def test_names(root, name):
    out = subprocess.run(
        CARGO + ["--test", name, "--", "--list", "--format", "terse"],
        cwd=root,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        sys.stderr.write(out.stdout + out.stderr)
        raise SystemExit("cannot list the tests of %s" % name)
    return sorted(line[: -len(": test")] for line in out.stdout.splitlines() if line.endswith(": test"))


def run(root, width, work):
    result = {"width": width, "work": work, "skips": {}, "parts": {}, "failed": [], "seconds": {}, "times": {}}
    if work == "*":
        began = time.time()
        code, text = stream(CARGO + ["--no-fail-fast", "--", "--nocapture"], root)
        result["seconds"]["*"] = round(time.time() - began, 1)
        result["skips"], result["times"] = attribute(text)
        if code != 0:
            result["failed"].append("*")
        return result
    items = [parse_item(item) for item in work.split()]
    build = []
    for name in sorted({name for name, _, _ in items}):
        build += ["--test", name]
    began = time.time()
    code, _ = stream(CARGO + ["--no-run"] + build, root)
    result["seconds"]["build"] = round(time.time() - began, 1)
    if code != 0:
        result["failed"].append("build")
        return result
    for name, part, parts in items:
        key = name if parts == 1 else "%s@%d/%d" % (name, part, parts)
        args = CARGO + ["--no-fail-fast", "--test", name, "--", "--nocapture"]
        if parts > 1:
            mine = [t for i, t in enumerate(test_names(root, name)) if i % parts == part]
            result["parts"][key] = mine
            if not mine:
                result["skips"][key] = 0
                continue
            args += ["--exact"] + mine
        began = time.time()
        code, text = stream(args, root)
        result["seconds"][key] = round(time.time() - began, 1)
        result["skips"][key] = text.count("SKIPPED")
        _, secs = attribute(text)
        result["times"][key] = secs.get(name, 0.0)
        if code != 0:
            result["failed"].append(key)
    return result


def tally(root, matrix, results):
    table = skips_table(root)
    errors = []
    lines = []
    for width in sorted({m["width"] for m in matrix}):
        planned = sorted(m["work"] for m in matrix if m["width"] == width)
        mine = [r for r in results if r["width"] == width]
        if sorted(r["work"] for r in mine) != planned:
            errors.append("width %d: %d of %d shards reported" % (width, len(mine), len(planned)))
            continue
        for r in mine:
            for key in r.get("failed", []):
                errors.append("width %d: %s failed" % (width, key))
        reported = {key for r in mine for key in r["skips"]}
        if planned == ["*"]:
            wanted = set(table)
        else:
            wanted = {item for work in planned for item in work.split()}
        for item in sorted(wanted - reported):
            errors.append("width %d: %s reported no result" % (width, item))
        observed = {}
        for r in mine:
            for key, count in r["skips"].items():
                name = parse_item(key)[0]
                observed[name] = observed.get(name, 0) + count
        if planned == ["*"]:
            expected = {name: row[width] for name, row in table.items()}
        else:
            names = {parse_item(item)[0] for work in planned for item in work.split()}
            expected = {name: table.get(name, {}).get(width, 0) for name in names}
        for name in sorted(set(expected) | set(observed)):
            got = observed.get(name, 0)
            want = expected.get(name, 0)
            if got != want:
                errors.append("width %d: %s skipped %d, ci/skips.txt says %d" % (width, name, got, want))
        total = sum(observed.values())
        want_total = sum(expected.values())
        lines.append("width %d: skipped %d, expected %d" % (width, total, want_total))
        if total != want_total:
            errors.append("width %d: skipped %d != %d" % (width, total, want_total))
    return lines, errors


def own(root, result):
    whole = [key for key in result["skips"] if "@" not in key]
    work = result["work"] if result["work"] == "*" else " ".join(i for i in result["work"].split() if "@" not in i)
    if not work:
        return [], []
    kept = dict(result, work=work, skips={key: result["skips"][key] for key in whole})
    return tally(root, [{"width": result["width"], "work": work}], [kept])


def from_logs(pairs):
    skips = {}
    secs = {}
    for pair in pairs:
        width, _, path = pair.partition("=")
        width = int(width)
        kept = []
        for raw in Path(path).read_text(encoding="utf-8", errors="replace").splitlines():
            line = clean(raw)
            if line.startswith(("skip: ", "skipped ", "##[")) or "awk" in line or "::error::" in line:
                continue
            kept.append(line)
        s, t = attribute("\n".join(kept))
        for name, count in s.items():
            skips.setdefault(name, {}).setdefault(width, count)
        for name, value in t.items():
            secs.setdefault(name, {})[width] = value
    return skips, secs


def main(argv=None):
    parser = argparse.ArgumentParser(prog="reach.py")
    parser.add_argument("--root", default=".")
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser("check")
    writer = sub.add_parser("write")
    writer.add_argument("--log", action="append", default=[])
    picker = sub.add_parser("select")
    picker.add_argument("--base", required=True)
    picker.add_argument("--head", default="HEAD")
    planner = sub.add_parser("plan")
    planner.add_argument("--whole", action="store_true")
    planner.add_argument("--base")
    planner.add_argument("--head", default="HEAD")
    planner.add_argument("--budget", type=int, default=BUDGET)
    planner.add_argument("--most", type=int, default=MOST_SHARDS)
    runner = sub.add_parser("run")
    runner.add_argument("--width", type=int, required=True)
    runner.add_argument("--work", required=True)
    runner.add_argument("--out", required=True)
    counter = sub.add_parser("tally")
    counter.add_argument("--matrix", required=True)
    counter.add_argument("results", nargs="*")
    args = parser.parse_args(argv)
    root = Path(args.root)
    if args.cmd == "check":
        found = problems(root)
        for line in found:
            print(line)
        if found:
            print("python3 ci/reach.py write updates ci/reach.txt and adds rows to ci/skips.txt")
            return 1
        print("ci/reach.txt and ci/skips.txt know every test target and src module")
        return 0
    if args.cmd == "write":
        repo = Repo(root)
        _, none = read_reach(root / "ci" / "reach.txt")
        write_reach(root / "ci" / "reach.txt", repo.mapping(), none & set(repo.src_files))
        table = skips_table(root)
        if args.log:
            skips, secs = from_logs(args.log)
            table = {n: {w: skips.get(n, {}).get(w, 0) for w in WIDTHS} for n in repo.targets}
            write_table(root / "ci" / "times.txt", {n: {w: secs.get(n, {}).get(w, 0.0) for w in WIDTHS} for n in repo.targets if n in secs}, "%.1f")
            for name in sorted(set(skips) - set(repo.targets)):
                if any(skips[name].values()):
                    print("skips outside a test target: %s %s" % (name, skips[name]))
        write_table(root / "ci" / "skips.txt", {n: table.get(n, {w: 0 for w in WIDTHS}) for n in repo.targets}, "%d")
        return 0
    if args.cmd == "select":
        picked, reasons = select(root, changed(root, args.base, args.head))
        for line in reasons:
            print(line, file=sys.stderr)
        print("*" if picked is None else " ".join(picked))
        return 0
    if args.cmd == "plan":
        if args.whole:
            picked, reasons = None, ["the whole suite was asked for"]
        else:
            picked, reasons = select(root, changed(root, args.base, args.head))
        for line in reasons:
            print(line, file=sys.stderr)
        print(json.dumps(plan(root, picked, args.budget, args.most)))
        return 0
    if args.cmd == "run":
        result = run(root, args.width, args.work)
        Path(args.out).write_text(json.dumps(result, indent=1, sort_keys=True), encoding="utf-8")
        lines, errors = own(root, result)
        for line in lines + errors + ["failed: %s" % key for key in result["failed"]]:
            print(line)
        return 1 if result["failed"] or errors else 0
    if args.cmd == "tally":
        matrix = json.loads(Path(args.matrix).read_text(encoding="utf-8"))
        results = [json.loads(Path(p).read_text(encoding="utf-8")) for p in args.results]
        lines, errors = tally(root, matrix, results)
        for line in lines + errors:
            print(line)
        return 1 if errors else 0
    return 2


if __name__ == "__main__":
    sys.exit(main())
