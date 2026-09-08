//! Build-aware extraction. Compilation databases supply flags, never shell commands.
use crate::{ExtractionResult, cached_extract_source};
use serde::Deserialize;
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Deserialize)]
struct Entry {
    directory: PathBuf,
    file: PathBuf,
    #[serde(default)]
    arguments: Vec<String>,
    #[serde(default)]
    command: String,
}

pub struct Project {
    root: PathBuf,
    entries: HashMap<PathBuf, Vec<Entry>>,
    facts: crate::compiler_facts::CompilerFacts,
    targets: HashMap<PathBuf, (BTreeSet<String>, BTreeSet<String>)>,
}

impl Project {
    pub fn load(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
        let mut project = Self {
            root: root.clone(),
            entries: HashMap::new(),
            facts: crate::compiler_facts::CompilerFacts::load(&root),
            targets: HashMap::new(),
        };
        let database = std::env::var_os("SYNAPTIC_COMPILE_COMMANDS")
            .map(PathBuf::from)
            .or_else(|| {
                [
                    root.join("compile_commands.json"),
                    root.join("build/compile_commands.json"),
                ]
                .into_iter()
                .find(|p| p.is_file())
            });
        if let Some(database) = database {
            project.targets = build_targets(database.parent().unwrap_or(&root));
            let entries = std::fs::read(&database)
                .ok()
                .and_then(|b| serde_json::from_slice::<Vec<Entry>>(&b).ok());
            if let Some(entries) = entries {
                for mut entry in entries {
                    if entry.directory.is_relative() {
                        entry.directory = database.parent().unwrap_or(&root).join(&entry.directory);
                    }
                    let file = entry.directory.join(&entry.file);
                    if let Ok(file) = file.canonicalize()
                        && file.starts_with(&root)
                    {
                        project.entries.entry(file).or_default().push(entry);
                    }
                }
            } else {
                eprintln!(
                    "warning: cannot read compilation database {}",
                    database.display()
                );
            }
        }
        // Headers listed by CMake inherit the actual target's translation-unit
        // flags. Conflicting contexts retain the diagnostic/fallback below.
        let mut headers = Vec::new();
        for (header, (targets, _)) in &project.targets {
            if project.entries.contains_key(header)
                || !header.starts_with(&root)
                || !header
                    .extension()
                    .and_then(|s| s.to_str())
                    .is_some_and(|s| matches!(s, "h" | "hpp" | "hxx" | "hh"))
            {
                continue;
            }
            let mut contexts = Vec::new();
            for (file, entries) in &project.entries {
                if let Some((owners, _)) = project.targets.get(file)
                    && !owners.is_disjoint(targets)
                    && file
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|s| matches!(s, "c" | "cpp" | "cc" | "cxx"))
                {
                    for entry in entries {
                        let mut context = entry.clone();
                        if context.arguments.is_empty() {
                            let Some(args) = split_command(&context.command) else {
                                continue;
                            };
                            context.arguments = args;
                        }
                        let language = if file.extension().and_then(|s| s.to_str()) == Some("c") {
                            "c"
                        } else {
                            "c++"
                        };
                        context.arguments.extend(["-x".into(), language.into()]);
                        contexts.push(context);
                    }
                }
            }
            if !contexts.is_empty() {
                headers.push((header.clone(), contexts));
            }
        }
        project.entries.extend(headers);
        project
    }

    pub fn has_build_configuration(&self) -> bool {
        !self.entries.is_empty() || self.facts.is_configured()
    }

    pub fn extract(
        &self,
        cache: Option<&Path>,
        path: &str,
        source: &[u8],
    ) -> Option<ExtractionResult> {
        let mut result = self.extract_build(cache, path, source)?;
        self.facts.apply(path, source, &mut result);
        if let Ok(file) = self.root.join(path).canonicalize()
            && let Some((targets, links)) = self.targets.get(&file)
        {
            for node in &mut result.nodes {
                if node.source_file.as_str() == path {
                    node.extra
                        .insert("build_targets".into(), serde_json::json!(targets));
                    node.extra
                        .insert("link_targets".into(), serde_json::json!(links));
                }
            }
        }
        Some(result)
    }

    fn extract_build(
        &self,
        cache: Option<&Path>,
        path: &str,
        source: &[u8],
    ) -> Option<ExtractionResult> {
        let file = self.root.join(path).canonicalize().ok();
        let Some(entries) = file.as_ref().and_then(|p| self.entries.get(p)) else {
            return cached_extract_source(cache, path, source);
        };
        let file = file.unwrap();
        let mut variants = Vec::new();
        for entry in entries {
            let args = if entry.arguments.is_empty() {
                let Some(args) = split_command(&entry.command) else {
                    let mut result = cached_extract_source(cache, path, source)?;
                    note(
                        &mut result,
                        "build_diagnostic",
                        "invalid compilation command quoting",
                    );
                    return Some(result);
                };
                args
            } else {
                entry.arguments.clone()
            };
            let args = match expand_arguments(&args, &entry.directory, 0) {
                Ok(args) => args,
                Err(error) => {
                    let mut result = cached_extract_source(cache, path, source)?;
                    note(&mut result, "build_diagnostic", &error);
                    return Some(result);
                }
            };
            let flags = compiler_flags(&args);
            let variant = (flags, entry.directory.clone());
            if !variants.contains(&variant) {
                variants.push(variant);
            }
        }
        // ponytail: one active build; variant-specific graph identities are needed to merge builds.
        if variants.len() != 1 {
            let mut result = cached_extract_source(cache, path, source)?;
            note(
                &mut result,
                "build_diagnostic",
                "multiple compile configurations; select one in the compilation database",
            );
            return Some(result);
        }
        let flags = &variants[0].0;
        let fortran = file.extension().and_then(|s| s.to_str()).is_some_and(|s| {
            matches!(
                s.to_ascii_lowercase().as_str(),
                "f" | "for" | "f90" | "f95" | "f03" | "f08"
            )
        });
        let compiler = std::env::var(if fortran {
            "SYNAPTIC_FORTRAN_COMPILER"
        } else {
            "SYNAPTIC_NATIVE_COMPILER"
        })
        .unwrap_or_else(|_| if fortran { "gfortran" } else { "gcc" }.into());
        let mut inputs: Vec<_> = entries.iter().map(|e| e.directory.join(&e.file)).collect();
        inputs.sort();
        inputs.dedup();
        let mut failure = "source absent from compiler output".to_owned();
        for input in inputs {
            let mut command = Command::new(&compiler);
            command
                .current_dir(&entries[0].directory)
                .args(flags)
                .arg("-E");
            if fortran {
                command.arg("-cpp");
            }
            let output = command.arg(compiler_path(&input)).output();
            failure = match output {
                Ok(output) if output.status.success() => {
                    let Some(normalized) = main_source(
                        &output.stdout,
                        &file,
                        &entries[0].directory,
                        source.split(|&b| b == b'\n').count(),
                    ) else {
                        continue;
                    };
                    // Preprocess every time: included headers and build flags are inputs too.
                    // The ordinary AST cache remains available to unconfigured files.
                    let mut result;
                    #[cfg(feature = "lang-fortran")]
                    if fortran {
                        let width = flags
                            .iter()
                            .find_map(|f| f.strip_prefix("-ffixed-line-length-"))
                            .map(|s| {
                                if s == "none" {
                                    0
                                } else {
                                    s.parse().unwrap_or(72)
                                }
                            })
                            .unwrap_or_else(crate::fortran::fixed_line_length);
                        let fixed = if flags.iter().any(|f| f == "-ffree-form") {
                            Some(false)
                        } else if flags.iter().any(|f| f == "-ffixed-form") {
                            Some(true)
                        } else {
                            None
                        };
                        result = crate::fortran::extract_fortran_source_with_form(
                            path,
                            &normalized,
                            width,
                            fixed,
                        );
                    } else {
                        result = extract_expanded(path, &normalized, flags)?;
                    }
                    #[cfg(not(feature = "lang-fortran"))]
                    {
                        result = extract_expanded(path, &normalized, flags)?;
                    }
                    note(&mut result, "compiler_preprocessed", &compiler);
                    if input.canonicalize().is_ok_and(|p| p != file) {
                        note(
                            &mut result,
                            "header_translation_unit",
                            &input
                                .canonicalize()
                                .ok()
                                .and_then(|p| {
                                    p.strip_prefix(&self.root)
                                        .ok()
                                        .map(|p| p.to_string_lossy().replace('\\', "/"))
                                })
                                .unwrap_or_else(|| input.to_string_lossy().into_owned()),
                        );
                    }
                    return Some(result);
                }
                Ok(output) => String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .take(4)
                    .collect::<Vec<_>>()
                    .join("\n"),
                Err(error) => error.to_string(),
            };
        }
        let mut result = cached_extract_source(cache, path, source)?;
        note(&mut result, "build_diagnostic", &failure);
        Some(result)
    }
}

fn compiler_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc) = path.strip_prefix("\\\\?\\UNC\\") {
        return format!("\\\\{unc}");
    }
    path.strip_prefix("\\\\?\\").unwrap_or(&path).to_owned()
}

fn extract_expanded(path: &str, source: &[u8], flags: &[String]) -> Option<ExtractionResult> {
    let language = flags
        .windows(2)
        .rev()
        .find(|pair| pair[0] == "-x")
        .map(|pair| pair[1].as_str());
    #[cfg(feature = "lang-cpp")]
    if language.is_some_and(|s| s.starts_with("c++")) {
        return Some(crate::cpp::extract_cpp_source(path, source));
    }
    #[cfg(feature = "lang-c")]
    if language.is_some_and(|s| matches!(s, "c" | "c-header")) {
        return Some(crate::c::extract_c_source(path, source));
    }
    let _ = language;
    crate::extract_source(path, source)
}

/// CMake File API target membership and dependency reachability. This constrains
/// build-time candidates; dynamic linker interposition still needs runtime data.
fn build_targets(build: &Path) -> HashMap<PathBuf, (BTreeSet<String>, BTreeSet<String>)> {
    fn read(path: &Path) -> Option<serde_json::Value> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }
    let reply = build.join(".cmake/api/v1/reply");
    let index = std::fs::read_dir(&reply)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("index-") && n.ends_with(".json"))
        })
        .max();
    let Some(index) = index.and_then(|p| read(&p)) else {
        return HashMap::new();
    };
    let model = index["objects"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|o| o["kind"] == "codemodel" && o["version"]["major"] == 2)
        .and_then(|o| o["jsonFile"].as_str())
        .and_then(|f| read(&reply.join(f)));
    let Some(model) = model else {
        return HashMap::new();
    };
    let Some(source) = model["paths"]["source"].as_str().map(PathBuf::from) else {
        return HashMap::new();
    };
    let mut result: HashMap<PathBuf, (BTreeSet<String>, BTreeSet<String>)> = HashMap::new();
    for config in model["configurations"].as_array().into_iter().flatten() {
        let mut dependencies = HashMap::<String, Vec<String>>::new();
        let mut files = Vec::new();
        for reference in config["targets"].as_array().into_iter().flatten() {
            let Some(target) = reference["jsonFile"]
                .as_str()
                .and_then(|f| read(&reply.join(f)))
            else {
                continue;
            };
            let Some(id) = target["id"].as_str() else {
                continue;
            };
            dependencies.insert(
                id.into(),
                target["dependencies"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|d| d["id"].as_str().map(str::to_owned))
                    .collect(),
            );
            for file in target["sources"].as_array().into_iter().flatten() {
                if let Some(path) = file["path"]
                    .as_str()
                    .and_then(|p| source.join(p).canonicalize().ok())
                {
                    files.push((path, id.to_owned()));
                }
            }
        }
        for (file, id) in files {
            let mut reachable = BTreeSet::new();
            let mut pending = vec![id.clone()];
            while let Some(target) = pending.pop() {
                if reachable.insert(target.clone()) {
                    pending.extend(dependencies.get(&target).into_iter().flatten().cloned());
                }
            }
            let entry = result.entry(file).or_default();
            let config_name = config["name"].as_str().unwrap_or("");
            entry.0.insert(format!("{config_name}:{id}"));
            entry.1.extend(
                reachable
                    .into_iter()
                    .map(|id| format!("{config_name}:{id}")),
            );
        }
    }
    result
}

fn expand_arguments(
    args: &[String],
    directory: &Path,
    depth: usize,
) -> Result<Vec<String>, String> {
    let mut expanded = Vec::new();
    for arg in args {
        if let Some(path) = arg.strip_prefix('@') {
            let path = directory.join(path);
            if depth >= 8
                || std::fs::metadata(&path).map_err(|e| e.to_string())?.len() >= 1024 * 1024
            {
                return Err("recursive or oversized compiler response file".into());
            }
            let contents = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let args = split_command(&contents).ok_or("invalid response file quoting")?;
            expanded.extend(expand_arguments(&args, directory, depth + 1)?);
        } else {
            expanded.push(arg.clone());
        }
    }
    Ok(expanded)
}

fn note(result: &mut ExtractionResult, key: &str, value: &str) {
    if let Some(file) = result.nodes.first_mut() {
        file.extra.insert(key.into(), value.into());
    }
}

/// Decode quoting without invoking a shell. Metacharacters remain ordinary data.
fn split_command(command: &str) -> Option<Vec<String>> {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && chars
                .peek()
                .is_some_and(|&n| n == '"' || n == '\\' || (!cfg!(windows) && quote != Some('\'')))
        {
            word.push(chars.next()?);
        } else if quote == Some(c) {
            quote = None;
        } else if quote.is_none() && matches!(c, '\'' | '"') {
            quote = Some(c);
        } else if quote.is_none() && c.is_whitespace() {
            if !word.is_empty() {
                args.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() {
        return None;
    }
    if !word.is_empty() {
        args.push(word);
    }
    Some(args)
}

/// Only preprocessing/dialect inputs are forwarded. Output files, plugins,
/// response files, compiler wrappers, and arbitrary driver actions are excluded.
fn compiler_flags(args: &[String]) -> Vec<String> {
    let mut flags = Vec::new();
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        if matches!(
            arg.as_str(),
            "-I" | "-D"
                | "-U"
                | "-isystem"
                | "-iquote"
                | "-include"
                | "-imacros"
                | "-isysroot"
                | "-x"
        ) {
            if let Some(value) = args.next() {
                flags.extend([arg.clone(), value.clone()]);
            }
        } else if [
            "-I",
            "-D",
            "-U",
            "-std=",
            "--sysroot=",
            "-ffixed-line-length-",
            "-ffree-line-length-",
        ]
        .iter()
        .any(|prefix| arg.starts_with(prefix))
            || matches!(
                arg.as_str(),
                "-ffree-form"
                    | "-ffixed-form"
                    | "-fopenmp"
                    | "-pthread"
                    | "-m32"
                    | "-m64"
                    | "-nostdinc"
                    | "-nostdinc++"
            )
        {
            flags.push(arg.clone());
        }
    }
    flags
}

/// Keep compiler-expanded main-file tokens at their original line numbers.
/// Headers supply macro definitions; their declarations are extracted separately.
fn main_source(output: &[u8], file: &Path, directory: &Path, line_count: usize) -> Option<Vec<u8>> {
    let mut lines = vec![Vec::new(); line_count];
    let mut row = 0usize;
    let mut main = false;
    let mut found = false;
    for line in output.split(|&b| b == b'\n') {
        let text = String::from_utf8_lossy(line);
        if let Some(marker) = text.trim_start().strip_prefix('#') {
            let marker = marker
                .trim_start()
                .strip_prefix("line ")
                .unwrap_or(marker.trim_start());
            if let Some((number, rest)) = marker.split_once(char::is_whitespace)
                && let Ok(number) = number.parse::<usize>()
                && let Some(quoted) = rest.trim_start().strip_prefix('"')
                && let Some(end) = quoted.find('"')
            {
                let name = quoted[..end].replace("\\\\", "\\");
                main = directory.join(name).canonicalize().is_ok_and(|p| p == file);
                found |= main;
                row = number.saturating_sub(1);
                continue;
            }
        }
        if main && let Some(target) = lines.get_mut(row) {
            if !target.is_empty() {
                target.push(b' ');
            }
            target.extend_from_slice(line);
        }
        row = row.saturating_add(1);
    }
    let mut source = Vec::new();
    for line in lines {
        source.extend(line);
        source.push(b'\n');
    }
    found.then_some(source)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn build_flags_are_data_and_line_markers_preserve_main_source() {
        let args=split_command("gcc -I 'include with spaces' -DNAME=solve -o delete-me -fplugin=bad @response ; touch bad -c unit.c").unwrap();
        assert_eq!(
            compiler_flags(&args),
            ["-I", "include with spaces", "-DNAME=solve"]
        );
        assert!(split_command("gcc 'unterminated").is_none());
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("unit.c");
        std::fs::write(&file, "\n\nint NAME(void) { return 0; }\n").unwrap();
        let file = file.canonicalize().unwrap();
        std::fs::write(
            dir.path().join("flags.rsp"),
            "-I 'include with spaces' -DNAME=solve -fplugin=bad",
        )
        .unwrap();
        let expanded =
            expand_arguments(&["gcc".into(), "@flags.rsp".into()], dir.path(), 0).unwrap();
        assert_eq!(
            compiler_flags(&expanded),
            ["-I", "include with spaces", "-DNAME=solve"]
        );
        std::fs::write(dir.path().join("cycle.rsp"), "@cycle.rsp").unwrap();
        assert!(expand_arguments(&["@cycle.rsp".into()], dir.path(), 0).is_err());
        assert!(expand_arguments(&["@missing.rsp".into()], dir.path(), 0).is_err());
        let output = format!(
            "# 1 \"header.h\"\nint ignored;\n# 3 \"{}\"\nint solve(void) {{ return 0; }}\n",
            file.display().to_string().replace('\\', "/")
        );
        assert_eq!(
            String::from_utf8(main_source(output.as_bytes(), &file, dir.path(), 4).unwrap())
                .unwrap(),
            "\n\nint solve(void) { return 0; }\n\n"
        );
        assert!(main_source(b"# 1 \"absent.h\"\nignored\n", &file, dir.path(), 4).is_none());
    }

    #[test]
    #[cfg(feature = "lang-cpp")]
    fn compiler_language_override_applies_to_header_grammar() {
        let result = extract_expanded(
            "api.h",
            b"namespace api { class Worker { public: void run() {} }; }",
            &["-x".into(), "c++".into()],
        )
        .unwrap();
        assert!(!result.parse_error);
        assert!(result.nodes.iter().any(|n| n.label == "Worker"));
    }

    #[test]
    fn cmake_targets_follow_transitive_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let reply = dir.path().join(".cmake/api/v1/reply");
        std::fs::create_dir_all(&reply).unwrap();
        for file in ["app.c", "lib.c", "other.c"] {
            std::fs::write(dir.path().join(file), "").unwrap();
        }
        let write = |name: &str, value: serde_json::Value| {
            std::fs::write(reply.join(name), value.to_string()).unwrap()
        };
        write(
            "index-1.json",
            serde_json::json!({"objects":[{"kind":"codemodel","version":{"major":2},"jsonFile":"model.json"}]}),
        );
        write(
            "model.json",
            serde_json::json!({"paths":{"source":dir.path()},"configurations":[{"name":"Debug","targets":[{"jsonFile":"app.json"},{"jsonFile":"lib.json"},{"jsonFile":"other.json"}]}]}),
        );
        write(
            "app.json",
            serde_json::json!({"id":"app","sources":[{"path":"app.c"}],"dependencies":[{"id":"lib"}]}),
        );
        write(
            "lib.json",
            serde_json::json!({"id":"lib","sources":[{"path":"lib.c"}]}),
        );
        write(
            "other.json",
            serde_json::json!({"id":"other","sources":[{"path":"other.c"}]}),
        );
        let targets = build_targets(dir.path());
        assert_eq!(
            targets[&dir.path().join("app.c").canonicalize().unwrap()].1,
            BTreeSet::from(["Debug:app".into(), "Debug:lib".into()])
        );
    }
}
