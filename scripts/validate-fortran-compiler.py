"""Compare GFortran's resolved procedure names and calls with a graph.

The compilation database must contain Fortran units in module dependency order,
or point to an already-built module directory. Objects/modules stay in a scratch
directory beside the output report. Runtime dispatch is outside this oracle.
"""
import argparse
import json
from pathlib import Path
import re
import shlex
import subprocess
import tempfile


def read(path):
    data = path.read_bytes()
    return json.loads(data.decode("utf-16" if data.startswith(b"\xff\xfe") else "utf-8"))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--graph", type=Path, required=True)
    parser.add_argument("--compiler", default="gfortran")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    root = args.repo.resolve()
    args.out.parent.mkdir(parents=True, exist_ok=True)
    units = []
    modules = set()
    with tempfile.TemporaryDirectory(dir=args.out.parent) as scratch:
        for index, entry in enumerate(read(args.database)):
            directory = Path(entry["directory"]).resolve()
            file = (directory / entry["file"]).resolve()
            if file.suffix.lower() not in (".f", ".for", ".f90", ".f95", ".f03", ".f08"):
                continue
            source = file.read_text(encoding="utf-8")
            modules.update(re.findall(r"^\s*module\s+(\w+)\s*(?:!.*)?$", source, re.M | re.I))
            original = iter((entry.get("arguments") or shlex.split(entry["command"]))[1:])
            flags = []
            for flag in original:
                if flag == "-I":
                    flags.extend(["-I", str(directory / next(original))])
                elif flag.startswith("-I"):
                    flags.append("-I" + str(directory / flag[2:]))
                elif flag.startswith(("-D", "-U", "-std=", "-ffixed-line-length-", "-ffree-line-length-")) or flag in ("-cpp", "-ffixed-form", "-ffree-form"):
                    flags.append(flag)
            command = [args.compiler, *flags, "-I" + str(directory), "-J" + str(Path(scratch).resolve()),
                       "-c", "-fdump-fortran-original", str(file), "-o", str(Path(scratch).resolve() / f"{index}.o")]
            result = subprocess.run(command, cwd=scratch, capture_output=True, timeout=120)
            relative = file.relative_to(root).as_posix()
            if result.returncode:
                units.append({"file": relative, "error": result.stderr.decode(errors="replace")})
                continue
            dump = result.stdout.decode(errors="replace")
            definitions = set(re.findall(r"^\s*procedure name = (\w+)", dump, re.M))
            calls = set()
            caller = None
            code = False
            for line in dump.splitlines():
                match = re.match(r"^\s*procedure name = (\w+)", line)
                if match:
                    caller, code = match[1], False
                if line.strip().startswith("Namespace:"):
                    code = False
                if line.strip() == "code:":
                    code = True
                elif code and caller:
                    for call, function in re.findall(r"\bCALL\s+(\w+)|\b(\w+)\[\[", line):
                        if (call or function) != caller:
                            calls.add((caller, call or function))
            units.append({"file": relative, "definitions": sorted(definitions), "calls": sorted(calls)})
    modules = {name.lower() for name in modules}
    expected_defs = {(u["file"], name) for u in units for name in u.get("definitions", []) if name not in modules}
    by_name = {}
    for file, name in expected_defs:
        by_name.setdefault(name, set()).add(file)
    expected_calls, ambiguous = set(), set()
    for unit in units:
        for caller, target in unit.get("calls", []):
            files = by_name.get(target, set())
            if unit["file"] in files:
                files = {unit["file"]}
            if len(files) == 1:
                expected_calls.add((unit["file"], caller, next(iter(files)), target))
            elif files:
                ambiguous.add((unit["file"], caller, target))
    graph = read(args.graph)
    nodes = {n["id"]: n for n in graph["nodes"]}
    def name(node):
        return node["label"].removeprefix(".").removesuffix("()").lower()
    actual_defs = {(n["source_file"], name(n)) for n in nodes.values()}
    actual_calls = {(nodes[e["source"]]["source_file"], name(nodes[e["source"]]), nodes[e["target"]]["source_file"], name(nodes[e["target"]]))
                    for e in graph["links"] if e["relation"] == "calls"}
    measured = {c for c in actual_calls if (c[0], c[1]) in expected_defs and c[3] in by_name and (c[0], c[1], c[3]) not in ambiguous}
    report = {"compiler": subprocess.check_output([args.compiler, "--version"], text=True).splitlines()[0],
              "revision": subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip(),
              "translation_units": len(units), "compile_failures": [u for u in units if "error" in u],
              "definitions": len(expected_defs), "definitions_matched": len(expected_defs & actual_defs),
              "missing_definitions": sorted(expected_defs - actual_defs),
              "direct_repository_calls": len(expected_calls), "calls_matched": len(expected_calls & actual_calls),
              "missing_calls": sorted(expected_calls - actual_calls), "unexpected_calls": sorted(measured - expected_calls),
              "ambiguous_link_targets_excluded": sorted(ambiguous), "units": units}
    args.out.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: v for k, v in report.items() if k != "units"}, indent=2))
    if any(report[k] for k in ("compile_failures", "missing_definitions", "missing_calls", "unexpected_calls")):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
