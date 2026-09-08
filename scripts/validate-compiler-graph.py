"""Compare built C translation units with Clang's resolved AST, without running repo code.

The scope is explicit: main-file function definitions and direct calls whose
callee has a definition in the selected compilation database. Indirect calls,
system/library calls, inactive build variants, and C++ overloads are excluded.
"""
import argparse
import concurrent.futures
import json
from pathlib import Path
import shlex
import subprocess


def read_json(path):
    data = path.read_bytes()
    return json.loads(data.decode("utf-16" if data.startswith(b"\xff\xfe") else "utf-8"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--clang", default="clang")
    parser.add_argument("--clang-arg", action="append", default=[])
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = args.repo.resolve()
    entries = [e for e in read_json(args.database) if Path(e["file"]).suffix.lower() == ".c"]
    selected = []
    for entry in entries:
        file = (Path(entry["directory"]) / entry["file"]).resolve()
        selected.append((entry, file))

    def compile_entry(item):
        entry, file = item
        original = entry.get("arguments") or shlex.split(entry["command"], posix=True)
        def expand(arguments, depth=0):
            if depth > 8:
                raise ValueError("recursive compiler response file")
            result = []
            for arg in arguments:
                if arg.startswith("@"):
                    result.extend(expand(shlex.split((Path(entry["directory"]) / arg[1:]).read_text()), depth + 1))
                else:
                    result.append(arg)
            return result
        original = expand(original)
        flags = []
        original = iter(original[1:])
        for arg in original:
            if arg in ("-I", "-D", "-U", "-isystem", "-iquote", "-include", "-imacros"):
                flags.extend((arg, next(original)))
            elif arg.startswith(("-I", "-D", "-U", "-std=", "--sysroot=")):
                flags.append(arg)
        command = [args.clang, *args.clang_arg, *flags, "-fsyntax-only", "-Xclang", "-ast-dump=json", str(file)]
        result = subprocess.run(command, cwd=entry["directory"], capture_output=True, timeout=120)
        relative = file.relative_to(root).as_posix()
        if result.returncode:
            return {"file": relative, "error": result.stderr.decode(errors="replace")}
        ast = json.loads(result.stdout)
        definitions = set()
        calls = set()

        def callee(node):
            if node.get("kind") == "DeclRefExpr" and node.get("referencedDecl", {}).get("kind") == "FunctionDecl":
                return node["referencedDecl"]["name"]
            for child in node.get("inner", []):
                name = callee(child)
                if name:
                    return name
            return None

        def walk(node, caller=None):
            if node.get("kind") == "FunctionDecl":
                loc = node.get("loc", {})
                loc = loc.get("expansionLoc", loc)
                if any(n.get("kind") == "CompoundStmt" for n in node.get("inner", [])) and "includedFrom" not in loc:
                    caller = node["name"]
                    definitions.add(caller)
                else:
                    caller = None
            if caller and node.get("kind") == "CallExpr" and node.get("inner"):
                target = callee(node["inner"][0])
                if target and target != caller:
                    calls.add((caller, target))
            for child in node.get("inner", []):
                walk(child, caller)
        walk(ast)
        return {"file": relative, "definitions": sorted(definitions), "calls": sorted(calls)}

    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        units = list(pool.map(compile_entry, selected))
    errors = [u for u in units if "error" in u]
    oracle_defs = {(u["file"], name) for u in units for name in u.get("definitions", [])}
    known_names = {name for _, name in oracle_defs}
    oracle_calls = set()
    ambiguous_targets = set()
    definitions_by_name = {}
    for file, symbol in oracle_defs:
        definitions_by_name.setdefault(symbol, set()).add(file)
    for unit in units:
        for caller, target in unit.get("calls", []):
            files = definitions_by_name.get(target, set())
            if unit["file"] in files:
                files = {unit["file"]}
            if len(files) == 1:
                oracle_calls.add((unit["file"], caller, next(iter(files)), target))
            elif files:
                ambiguous_targets.add((unit["file"], caller, target))
    graph = read_json(args.graph)
    nodes = {n["id"]: n for n in graph["nodes"]}
    def name(node):
        return node["label"].removeprefix(".").removesuffix("()")
    graph_defs = {(n["source_file"], name(n)) for n in nodes.values() if n["label"].endswith("()")}
    graph_calls = {(nodes[e["source"]]["source_file"], name(nodes[e["source"]]), nodes[e["target"]]["source_file"], name(nodes[e["target"]])) for e in graph["links"] if e["relation"] == "calls"}
    measured_graph_calls = {c for c in graph_calls if (c[0], c[1]) in oracle_defs and c[3] in known_names and (c[0], c[1], c[3]) not in ambiguous_targets}
    report = {
        "compiler": subprocess.check_output([args.clang, "--version"], text=True).splitlines()[0],
        "revision": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
        "translation_units": len(units), "compile_failures": errors,
        "definitions": len(oracle_defs), "definitions_matched": len(oracle_defs & graph_defs),
        "missing_definitions": sorted(oracle_defs - graph_defs),
        "direct_repository_calls": len(oracle_calls), "calls_matched": len(oracle_calls & graph_calls),
        "missing_calls": sorted(oracle_calls - graph_calls),
        "unexpected_calls": sorted(measured_graph_calls - oracle_calls),
        "ambiguous_link_targets_excluded": sorted(ambiguous_targets),
        "units": units,
    }
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k != "units"}, indent=2))
    if errors or report["missing_definitions"] or report["missing_calls"] or report["unexpected_calls"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
