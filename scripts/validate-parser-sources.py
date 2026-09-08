"""Validate all source-reviewed parser claims against pinned OSS graph snapshots.

Use --check to reproduce one historical pass; the default runs all four.
Compiler-derived evidence is checked by validate-parser-completion.py.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib


def check_review(args, graphs, node):
    cache = args.cache
    solve = node("lapack", "SRC/dgesv.f", "DGESV()")
    assert solve["source_location"] == "L121", "DGESV must anchor to code, not documentation"
    cases = [
        ("lapack", "SRC/dgesv.f", "DGESV()", "SRC/dgetrs.f", "DGETRS()", 170, "CALL DGETRS"),
        ("cminpack", "examples/hybdrv.f", "fcn()", "examples/vecfcn.f", "vecfcn()", 106, "call vecfcn"),
        ("cminpack", "fortran/chkder.f", "chkder()", "fortran/dpmpar.f", "dpmpar()", 93, "epsmch = dpmpar"),
    ]
    for repo, file, caller, target_file, callee, line, source in cases:
        text = (cache / repo / file).read_text(encoding="utf-8").splitlines()[line - 1]
        assert source in text, (file, line, text)
        a, b = node(repo, file, caller), node(repo, target_file, callee)
        assert any(e["source"] == a["id"] and e["target"] == b["id"]
                   and e["relation"] == "calls" for e in graphs[repo]["links"]), (caller, callee)
    a = node("cminpack", "examples/ssqfcn.f", "ssqfcn()")
    b = node("cminpack", "examples/ssqfcn.c", "ssqfcn()")
    assert a["id"] != b["id"], "C and Fortran translations must coexist"
    print("PASS: pinned revisions, DGESV anchor, 3 reviewed call edges, C/Fortran identities")


def check_followup(args, graphs, node):
    cache = args.cache
    for name in ("DSECND", "SECOND"):
        ids = [node("lapack", f"INSTALL/{name.lower()}_EXT_ETIME{suffix}.f", f"{name}()", 34)["id"] for suffix in ("", "_")]
        assert ids[0] != ids[1]
    interface = node("lapack", "SRC/la_xisnan.F90", "LA_ISNAN", 2)
    assert sum(e["source"] == interface["id"] and e["relation"] == "references" for e in graphs["lapack"]["links"]) == 2
    solve = node("lapack", "SRC/dgesv.f", "DGESV()", 121)
    for callee, line in (("XERBLA", 159), ("DGETRF", 165), ("DGETRS", 170)):
        assert f"CALL {callee}" in (cache / "lapack/SRC/dgesv.f").read_text().splitlines()[line - 1]
        target = next(n for n in graphs["lapack"]["nodes"] if n.get("source_file") == f"SRC/{callee.lower()}.f" and n["label"] == f"{callee}()")
        assert any(e["source"] == solve["id"] and e["target"] == target["id"] and e["relation"] == "calls" and e["source_location"] == f"L{line}" for e in graphs["lapack"]["links"])
    file = "src/test/groovy/wslite/rest/RESTClientSpec.groovy"
    feature = node("groovy-wslite", file, ".no args constructor and setting base url as a property()", 39)
    helper = node("groovy-wslite", file, ".getMockResponse()", 345)
    assert not feature.get("recovered")
    assert "getMockResponse()" in (cache / "groovy-wslite" / file).read_text().splitlines()[43]
    assert any(e["source"] == feature["id"] and e["target"] == helper["id"] and e["relation"] == "calls" and e["source_location"] == "L44" for e in graphs["groovy-wslite"]["links"])
    assert any(e["target"] == feature["id"] and e["relation"] == "method" for e in graphs["groovy-wslite"]["links"])

    if args.binary:
        with tempfile.TemporaryDirectory(dir=args.artifacts.resolve()) as temporary:
            root = Path(temporary)
            (root / "work.f").write_text("      SUBROUTINE WORK()\n      CALL " + " " * 66 + "HELPER()\n      END\n      SUBROUTINE HELPER()\n      END\n")
            for width in (72, 132, 0, 72):
                env = dict(os.environ, SYNAPTIC_FORTRAN_FIXED_LINE_LENGTH=str(width))
                subprocess.run([str(args.binary.resolve()), "extract", str(root), "--directed", "--no-store"], env=env, stdout=subprocess.DEVNULL, check=True)
                graph = json.loads((root / "synaptic-out/graph.json").read_text())
                assert any(e["relation"] == "calls" for e in graph["links"]) == (width != 72), width
    print("PASS: pinned repos, distinct ETIME wrappers, generic interface, 3 DGESV calls, quoted Groovy method ownership and call" + (", fixed-form settings/cache" if args.binary else ""))


def check_upgrade(args, graphs, node):
    cache = args.cache
    for file in ('chkdrv', 'hybdrv', 'hyjdrv', 'ibmdpdr', 'lmddrv', 'lmfdrv', 'lmsdrv'):
        plain = node('cminpack', f'examples/{file}.c', 'main()')
        variant = node('cminpack', f'examples/{file}_.c', 'main()')
        assert plain['id'] != variant['id'], file
        line = int(plain['source_location'][1:])
        assert 'main(' in (cache / 'cminpack' / f'examples/{file}.c').read_text().splitlines()[line - 1]
    assert node('cminpack', 'examples/hybdrv.c', 'refnum')['source_location'] == 'L36'

    compiler = 'spock-core/src/main/groovy/spock/util/EmbeddedSpecCompiler.groovy'
    cases = [
        ('spock', compiler, '.compile()', compiler, '.doCompile()', 105, 'doCompile(source, loader)'),
        ('spock', compiler, '.compileSpecBody()', compiler, '.compileWithImports()', 118, 'compileWithImports('),
        ('fpm', 'app/main.f90', '.has_manifest()', 'src/fpm_filesystem.F90', '.exists()', 107, 'exists(join_path('),
        ('fpm', 'app/main.f90', '.has_manifest()', 'src/fpm_filesystem.F90', '.join_path()', 107, 'exists(join_path('),
        ('fpm', 'src/fpm_backend.F90', '.build_package()', 'src/fpm_filesystem.F90', '.mkdir()', 88, 'call mkdir('),
        ('fpm', 'app/main.f90', 'main', 'src/fpm_command_line.f90', '.get_command_line_settings()', 31, 'call get_command_line_settings('),
    ]
    for repo, file, caller, target_file, callee, line, snippet in cases:
        assert snippet in (cache / repo / file).read_text(encoding='utf-8').splitlines()[line - 1]
        a, b = node(repo, file, caller), node(repo, target_file, callee)
        assert any(e['source'] == a['id'] and e['target'] == b['id'] and e['relation'] == 'calls'
                   and e.get('source_location') == f'L{line}' for e in graphs[repo]['links']), (repo, caller, callee, line)
    print('PASS: pinned revisions, seven C variant pairs, C aggregate anchor, annotated Groovy methods, Fortran host/USE/program calls')


def check_limits(args, graphs, node):
    cache = args.cache
    alias = node("cminpack", "examples/tchkderc.c", "fcndata_t")
    assert alias["source_location"] == "L17"
    assert alias["kind"] == "type_alias"
    for file, label in [("examples/tcppwrap.cpp", "real")]:
        declaration = node("cminpack", file, label)
        source = (cache / "cminpack" / file).read_text().splitlines()
        assert label in source[int(declaration["source_location"][1:]) - 1]
    aliases = [n for n in graphs["cminpack"]["nodes"] if n["source_file"] == "include/cminpackcpp.hpp" and n["label"] == "Fn"]
    assert len(aliases) == len({n["id"] for n in aliases}) == 9
    assert any(n["source_location"] == "L210" for n in aliases)

    caller = node("cminpack", "src/chkder.c", "__cminpack_func__(chkder)()")
    target = node("cminpack", "src/dpmpar.c", "__cminpack_func__(dpmpar)()")
    calls = [e for e in graphs["cminpack"]["links"] if e["relation"] == "calls"
             and e["source"] == caller["id"] and e["target"] == target["id"]]
    assert len(calls) == 1, calls
    source = (cache / "cminpack/src/chkder.c").read_text().splitlines()
    assert "__cminpack_func__(dpmpar)" in source[int(calls[0]["source_location"][1:]) - 1]
    assert not any(n["label"] == "__cminpack_func__()" for n in graphs["cminpack"]["nodes"])

    file = "CBLAS/src/cblas_dgemm.c"
    assert not node("lapack", file, "cblas_dgemm.c").get("parse_error", False)
    caller = node("lapack", file, "API_SUFFIX(cblas_dgemm)()")
    assert caller["source_location"] == "L12"
    assert len(caller["signature"]["params"]) == 14
    target = node("lapack", "CBLAS/src/cblas_xerbla.c", "API_SUFFIX(cblas_xerbla)()")
    assert any(e["relation"] == "calls" and e["source"] == caller["id"] and e["target"] == target["id"]
               for e in graphs["lapack"]["links"])

    for name in ("dblat1", "sblat1"):
        file = f"BLAS/TESTING/{name}.f"
        declaration = node("lapack", file, f"{name}.f")
        assert not declaration.get("parse_error", False), declaration
        assert node("lapack", file, name.upper())["source_location"] == "L36"
    file = "spock-specs/src/test-groovy-ge-3.0/groovy/org/spockframework/smoke/lamba/LambdaSpec.groovy"
    assert not node("spock", file, "LambdaSpec.groovy").get("parse_error", False)
    assert node("spock", file, ".allow to use Java lambda in spec()")["source_location"] == "L12"
    print("Pinned source checks pass: C/C++ aliases, wrapped C calls, LAPACK fixed form, and Spock lambdas.")


def main():
    workspace = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifacts", type=Path)
    parser.add_argument("--cache", type=Path, default=workspace / "synaptic-out/bench")
    parser.add_argument("--stage", default="delivered", help="graph snapshot filename suffix")
    parser.add_argument("--check", choices=("all", "review", "followup", "upgrade", "limits"), default="all")
    parser.add_argument("--binary", type=Path, help="also test fixed-form settings and warm-cache changes in the followup check")
    args = parser.parse_args()
    checks = {
        "review": (check_review, ("lapack", "cminpack")),
        "followup": (check_followup, ("lapack", "groovy-wslite")),
        "upgrade": (check_upgrade, ("spock", "cminpack", "fpm")),
        "limits": (check_limits, ("cminpack", "lapack", "spock")),
    }
    selected = list(checks.values()) if args.check == "all" else [checks[args.check]]
    manifest = tomllib.loads((workspace / "eval/parser-validation.toml").read_text(encoding="utf-8"))
    pins = {r["url"].rsplit("/", 1)[-1]: r["sha"] for r in manifest["repo"]}
    graphs = {}
    for repo in sorted({repo for _, repos in selected for repo in repos}):
        revision = subprocess.check_output(
            ["git", "-C", str(args.cache / repo), "rev-parse", "HEAD"], text=True).strip()
        assert revision == pins[repo], f"{repo}: wrong checkout revision"
        graphs[repo] = json.loads((args.artifacts / f"{repo}-{args.stage}.json").read_text(encoding="utf-8"))

    def node(repo, file, label, line=None):
        matches = [n for n in graphs[repo]["nodes"]
                   if n.get("source_file") == file and n["label"] == label]
        assert len(matches) == 1, (repo, file, label, len(matches))
        if line is not None:
            assert matches[0]["source_location"] == f"L{line}", matches[0]
        return matches[0]

    for check, _ in selected:
        check(args, graphs, node)


if __name__ == "__main__":
    main()
