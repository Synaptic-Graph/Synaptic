"""Check compiler evidence against delivered graph snapshots.

Run after the commands documented in eval/parser-validation.md.
The negative control deliberately changes a callee and must be detected.
"""
import argparse
import json
from pathlib import Path


def read(path):
    data = path.read_bytes()
    return json.loads(data.decode("utf-16" if data.startswith(b"\xff\xfe") else "utf-8"))


def groovy_check(graph, facts):
    symbols = {n["compiler_symbol"]: n for n in graph["nodes"] if "compiler_symbol" in n}
    expected = {m["symbol"] for f in facts["files"].values() for m in f["methods"]}
    assert expected <= symbols.keys(), sorted(expected - symbols.keys())
    generated = {m["symbol"] for f in facts["files"].values() for m in f["methods"] if m["generated"]}
    assert all(symbols[s].get("compiler_generated") for s in generated)
    expected_calls = {(m["symbol"], c["target"]) for f in facts["files"].values() for m in f["methods"] for c in m["calls"] if c["target"] in expected and c["target"] != m["symbol"]}
    ids = {n["id"]: n for n in graph["nodes"]}
    actual = {(ids[e["source"]].get("compiler_symbol"), ids[e["target"]].get("compiler_symbol")) for e in graph["links"] if e["relation"] == "calls" and e.get("context") == "compiler_resolved_call"}
    assert actual == expected_calls, {"missing": sorted(expected_calls - actual), "unexpected": sorted(actual - expected_calls)}
    return {"sources": len(facts["files"]), "methods": len(expected), "generated": len(generated), "resolved_repository_calls": len(actual)}


def groovy_declaration_check(graphs, oracle):
    index = {}
    for repo, graph in graphs.items():
        for node in graph["nodes"]:
            if node.get("kind") in {"function", "method", "class", "interface", "enum", "constructor", "trait", "record"}:
                key = (repo + "/" + node["source_file"], node["label"].removeprefix(".").removesuffix("()"))
                index.setdefault(key, []).append(node)
    missing, failures, used = [], [], set()
    total = 0
    for file in oracle["files"]:
        if "error" in file:
            failures.append({"file": file["file"], "error": file["error"]})
        for declaration in file.get("declarations", []):
            total += 1
            for node in index.get((file["file"], declaration["name"]), []):
                span = node.get("span", {})
                start = (span.get("start_line", 0), span.get("start_col", 0))
                end = (span.get("end_line", 0), span.get("end_col", 0))
                identity = (file["file"], node["id"])
                if identity not in used and start < (declaration["end"], declaration["endColumn"]) and end > (declaration["line"], declaration["column"]):
                    used.add(identity)
                    break
            else:
                missing.append({"file": file["file"], **declaration})
    return {"compiler": oracle["compiler"], "phase": oracle["phase"],
            "files": len(oracle["files"]), "parsed_files": len(oracle["files"]) - len(failures),
            "declarations": total, "matched": total - len(missing),
            "missing": missing, "compiler_failures": failures}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    root = args.evidence
    results = {}
    groovy_graphs = {repo: read(root / (repo + "-delivered.json"))
                     for repo in ["spock", "http-builder-ng", "groovy-wslite"]}
    groovy_oracle = read(root / "groovy-declaration-oracle.json")
    declarations = groovy_declaration_check(groovy_graphs, groovy_oracle)
    assert not declarations["missing"], declarations["missing"]
    # This unchanged pinned source duplicates a static import rejected by Groovy 5.
    assert len(declarations["compiler_failures"]) == 1, declarations["compiler_failures"]
    failure = declarations["compiler_failures"][0]
    assert failure["file"] == "http-builder-ng/http-builder-ng-core/src/test/groovy/groovyx/net/http/EncodersSpec.groovy"
    assert "MULTIPART_MIXED" in failure["error"], failure
    results["groovy_declarations"] = declarations
    # Removing an overload must fail even when another declaration has its name.
    broken = {repo: {"nodes": [n for n in graph["nodes"] if "overload" not in n["id"]]}
              for repo, graph in groovy_graphs.items()}
    assert groovy_declaration_check(broken, groovy_oracle)["missing"]
    for name in ["groovy-fixture", "wslite"]:
        graph = read(root / (name + "-configured.json"))
        facts = read(root / (name + "-compiler-facts.json"))
        results[name] = groovy_check(graph, facts)
    graph = read(root / "dispatch-graph.json")
    nodes = {n["id"]: n for n in graph["nodes"]}
    direct = {nodes[e["target"]]["label"] for e in graph["links"] if e["relation"] == "calls" and nodes[e["source"]]["label"] == "check_dispatch"}
    expected = {".child_run()", ".double_value()", ".vector_value()", ".polymorphic()"}
    assert direct == expected, direct
    dump = (root / "fortran-compiler-dump.txt").read_text(encoding="utf-8")
    for name in ["child_run[[", "double_value[[", "vector_value[[", "CALL polymorphic"]:
        assert name in dump, name
    results["fortran"] = {"compiler_confirmed_direct_targets": len(expected)}
    fftpack = read(root / "fftpack-configured.json")
    nodes = {n["id"]: n for n in fftpack["nodes"]}
    expected = {
        ("example/example_complex_transforms.f90", "src/fftpack_fft.f90", ".fft_rk()"),
        ("example/example_complex_transforms.f90", "src/fftpack_ifft.f90", ".ifft_rk()"),
        ("example/example_real_transforms.f90", "src/fftpack_rfft.f90", ".rfft_rk()"),
        ("example/example_real_transforms.f90", "src/fftpack_irfft.f90", ".irfft_rk()"),
    }
    actual = {(nodes[e["source"]]["source_file"], nodes[e["target"]]["source_file"], nodes[e["target"]]["label"])
              for e in fftpack["links"] if e["relation"] == "calls"}
    assert expected <= actual, sorted(expected - actual)
    for file, _, label in expected:
        dump = (root / "fftpack-build" / (Path(file).stem + "-compiler.txt")).read_text(encoding="utf-8")
        assert label.strip('.').removesuffix('()') + '[[' in dump
    results["fftpack"] = {"configured_units": sum("compiler_preprocessed" in n for n in nodes.values()),
                          "compiler_confirmed_submodule_targets": len(expected)}
    native = read(root / "cminpack-compiler-oracle.json")
    for key in ["compile_failures", "missing_definitions", "missing_calls", "unexpected_calls"]:
        assert not native[key], (key, native[key])
    results["native"] = {k: native[k] for k in ["translation_units", "definitions", "direct_repository_calls"]}
    fortran = read(root / "fftpack-compiler-oracle.json")
    for key in ["compile_failures", "missing_definitions", "missing_calls", "unexpected_calls"]:
        assert not fortran[key], (key, fortran[key])
    results["fftpack"].update({k: fortran[k] for k in ["definitions", "direct_repository_calls"]})
    native_graph = read(root / "cminpack-configured.json")
    assert not any(n.get("parse_error") or n.get("build_diagnostic") for n in native_graph["nodes"])
    results["native"]["preprocessed_files"] = sum("compiler_preprocessed" in n for n in native_graph["nodes"])
    negative = read(root / "negative-native-oracle.json")
    assert negative["missing_calls"] and negative["unexpected_calls"], "wrong-file callees must fail even when names match"
    # The validator must reject a damaged compiler call graph.
    broken = read(root / "groovy-fixture-configured.json")
    broken["links"] = [e for e in broken["links"] if e.get("context") != "compiler_resolved_call"]
    try:
        groovy_check(broken, read(root / "groovy-fixture-compiler-facts.json"))
    except AssertionError:
        results["negative_control"] = "passed"
    else:
        raise AssertionError("missing compiler calls were not detected")
    (root / "compiler-validation.json").write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
