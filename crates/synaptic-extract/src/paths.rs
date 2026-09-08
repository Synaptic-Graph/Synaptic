//! Language-agnostic path → id/scope helpers shared by every extractor.

use std::path::Path;

pub(crate) use synaptic_core::file_node_id;

/// Id-safe key for a symbol whose punctuation `make_id` would erase.
///
/// `make_id` maps every non-word character to `_` and collapses runs, so `sort!`,
/// `sort?` and `sort` all normalize to one id. In Ruby, Elixir and Julia those are
/// *different functions* (`Map.fetch` vs `Map.fetch!`, `valid?`, `push!`), and the
/// collision silently deleted one of each pair from the graph. Appending a hex tag
/// of the stripped punctuation keeps them distinct while leaving plain names
/// untouched.
pub(crate) fn symbol_key(name: &str) -> String {
    let mut punctuation: String = name
        .chars()
        .filter(|c| !c.is_alphanumeric() && *c != '_')
        .map(|c| format!("{:x}", c as u32))
        .collect();
    // `make_id` trims *and* collapses underscore runs, so the count has to be
    // encoded, not merely flagged: easyasp ships `Search`, `Search_` and
    // `Search__` side by side, and a flat marker collapses the last two.
    let edge_underscores = (name.len() - name.trim_start_matches('_').len())
        + (name.len() - name.trim_end_matches('_').len());
    if edge_underscores > 0 {
        punctuation.push_str(&format!("5f{edge_underscores}"));
    }
    if punctuation.is_empty() {
        name.to_string()
    } else {
        format!("{name}_{punctuation}")
    }
}

/// Extension-less path used to namespace symbol ids within a file. Keeping the
/// full relative path avoids collisions between same-named files in parallel
/// source sets (`src/jvmTest/.../testUtil.kt` vs `src/nonJvmTest/.../testUtil.kt`).
pub(crate) fn file_stem(path: &str) -> String {
    let mut p = Path::new(path).to_path_buf();
    p.set_extension("");
    p.to_string_lossy().replace(['/', '\\'], ".")
}

/// Lexically resolve `target` relative to the directory containing `from_file`,
/// normalizing `.`/`..` segments **without touching the filesystem** — purely a
/// string operation, so it is deterministic and yields the same portable
/// relative path the scanner feeds the extractors (matching a referenced file's
/// own `file_node_id`). Absolute `target`s (POSIX `/…` or Windows `C:\…`) are
/// normalized as-is. Backslashes are treated as separators throughout.
///
/// This is Synaptic's path-relative resolution policy: the relative form is
/// used because Synaptic keys file nodes by the root-relative path for
/// portability, rather than by absolute path.
#[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
pub(crate) fn resolve_relative_path(from_file: &str, target: &str) -> String {
    let target = target.replace('\\', "/");
    // POSIX-absolute (`/usr/...`) or Windows-drive-absolute (`C:/...`).
    let posix_abs = target.starts_with('/');
    let drive_abs = target.as_bytes().get(1) == Some(&b':');
    let absolute = posix_abs || drive_abs;

    let mut stack: Vec<String> = Vec::new();
    if !absolute {
        // Seed with the parent-directory components of the sourcing file.
        let from = from_file.replace('\\', "/");
        let mut parts: Vec<&str> = from.split('/').collect();
        parts.pop(); // drop the file name itself
        for p in parts {
            push_component(&mut stack, p);
        }
    }
    for p in target.split('/') {
        push_component(&mut stack, p);
    }
    let joined = stack.join("/");
    if posix_abs {
        format!("/{joined}")
    } else {
        joined
    }
}

#[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
fn push_component(stack: &mut Vec<String>, p: &str) {
    match p {
        "" | "." => {}
        ".." => {
            stack.pop();
        }
        _ => stack.push(p.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stem_qualifies_with_parent_dir() {
        assert_eq!(file_stem("pkg/mod.py"), "pkg.mod");
        assert_eq!(file_stem("mod.py"), "mod");
        assert_ne!(
            file_stem("src/jvmTest/kotlin/app/testUtil.kt"),
            file_stem("src/nonJvmTest/kotlin/app/testUtil.kt")
        );
    }

    #[test]
    fn file_id_distinguishes_normalized_collisions() {
        assert_ne!(file_node_id("pkg/mod.py"), file_node_id("pkg/mod_.py"));
    }

    #[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
    #[test]
    fn resolve_relative_handles_dot_dotdot_and_siblings() {
        assert_eq!(
            resolve_relative_path("src/App/App.csproj", "../Lib/Lib.csproj"),
            "src/Lib/Lib.csproj"
        );
        assert_eq!(
            resolve_relative_path("scripts/app.sh", "./lib.sh"),
            "scripts/lib.sh"
        );
        assert_eq!(
            resolve_relative_path("scripts/app.sh", "lib.sh"),
            "scripts/lib.sh"
        );
        assert_eq!(resolve_relative_path("a/b/c.sh", "../../x/y.sh"), "x/y.sh");
    }

    #[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
    #[test]
    fn resolve_relative_distinguishes_same_name_in_different_dirs() {
        // The core of the I-18 fix: two scripts named lib.sh in different dirs
        // must resolve to distinct paths (and therefore distinct file-node ids).
        let a = resolve_relative_path("a/app.sh", "./lib.sh");
        let b = resolve_relative_path("b/app.sh", "./lib.sh");
        assert_eq!(a, "a/lib.sh");
        assert_eq!(b, "b/lib.sh");
        assert_ne!(file_node_id(&a), file_node_id(&b));
    }

    #[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
    #[test]
    fn resolve_relative_passes_absolute_through() {
        assert_eq!(
            resolve_relative_path("a/b.sh", "/usr/lib/x.sh"),
            "/usr/lib/x.sh"
        );
        assert_eq!(
            resolve_relative_path("a/b.sh", "C:\\proj\\x.sh"),
            "C:/proj/x.sh"
        );
    }

    #[cfg(any(feature = "lang-dotnet", feature = "lang-bash"))]
    #[test]
    fn resolve_relative_normalizes_backslashes() {
        assert_eq!(
            resolve_relative_path("src/App/App.csproj", "..\\Lib\\Lib.csproj"),
            "src/Lib/Lib.csproj"
        );
    }
}
