//! Bounded declaration recovery for files the grammar could not parse.
//!
//! Some real-world sources defeat their tree-sitter grammar outright — a
//! SystemVerilog `generate` block whose body is a macro invocation, a Groovy build
//! script, an Objective-C header behind availability macros. The parse yields an
//! `ERROR` tree, the walk finds no declarations, and the file contributes nothing
//! but its own file node. That is indistinguishable, in the graph, from a file that
//! genuinely declares nothing.
//!
//! More often the parse half-succeeds: the grammar emits the first declaration and
//! drops the rest, which is quieter and more common than total failure — across a
//! 24-repository corpus partial parses lost 1,290 declarations against 176 lost
//! outright.
//!
//! This pass runs whenever the grammar reported an error, and scans for
//! declaration-shaped lines with anchored regexes. It **supplements, never
//! overrules**: any name the grammar already produced for that file is skipped, so
//! parsed structure always wins and nothing is duplicated. A file that parsed
//! cleanly is never touched. Recovered nodes are marked `recovered` in `extra`, and
//! their `contains` edges carry `Inferred` confidence, so consumers can tell them
//! from parsed structure.
//!
//! Patterns use `^[ \t]*`, never `^\s*` — `\s` matches a newline, which would start
//! a match on the blank line above the declaration and report it a line early.

use std::sync::LazyLock;

use regex::Regex;
use synaptic_core::{NodeId, make_id};

use crate::common::{Builder, symbol_key};
use crate::paths::{file_node_id, file_stem};
use crate::result::ExtractionResult;

/// One recovered declaration: the symbol name, its 1-based line, and whether it
/// reads as a routine (rendered `name()`) or a container type (rendered `name`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Decl {
    pub name: String,
    pub line: usize,
    pub is_routine: bool,
}

static VERILOG_RE: LazyLock<Regex> = LazyLock::new(|| {
    // `class` included: SystemVerilog classes are the backbone of UVM testbenches
    // and live mostly in `.svh` headers, so a verification-heavy repo is largely
    // classes rather than modules.
    Regex::new(r"(?m)^[ \t]*(?:module|interface|package|program|class)[ \t]+(?:automatic[ \t]+|static[ \t]+)?([A-Za-z_]\w*)")
        .expect("valid verilog recovery regex")
});
static JULIA_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:mutable[ \t]+)?(?:struct|abstract[ \t]+type|primitive[ \t]+type)[ \t]+([A-Za-z_]\w*)")
        .expect("valid julia type recovery regex")
});
static JULIA_ROUTINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Qualified extensions (`function Base.size(...)`) are named for the method, not
    // the module qualifier -- the same rule the Julia extractor itself follows.
    Regex::new(r"(?m)^[ \t]*function[ \t]+(?:[A-Za-z_]\w*\.)*([A-Za-z_]\w*[!?]?)[ \t]*\(")
        .expect("valid julia routine recovery regex")
});
static VERILOG_ROUTINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:function|task)[ \t]+(?:automatic[ \t]+|static[ \t]+)?(?:[\w:\[\]$.-]+[ \t]+)*([A-Za-z_]\w*)[ \t]*[(;]")
        .expect("valid verilog routine recovery regex")
});
static GROOVY_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*(?:(?:public|private|protected|static|final|abstract|@[A-Za-z]\w*)[ \t]+)*(?:class|interface|enum|trait)[ \t]+([A-Za-z_]\w*)")
        .expect("valid groovy type recovery regex")
});
static GROOVY_ROUTINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)^[ \t]*(?:(?:public|private|protected|static|final|synchronized)[ \t]+)*(?:def|void|boolean|byte|short|int|long|float|double|char|[A-Z][\w.<>,\[\] ]*)[ \t]+([a-zA-Z_]\w*|"(?:\\[^\r\n]|[^"\\\r\n])+"|'(?:\\[^\r\n]|[^'\\\r\n])+')[ \t]*\("#)
        .expect("valid groovy routine recovery regex")
});
static OBJC_TYPE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^[ \t]*@(?:interface|implementation|protocol)[ \t]+([A-Za-z_]\w*)")
        .expect("valid objc recovery regex")
});
static POWERSHELL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?mi)^[ \t]*function[ \t]+([A-Za-z_][\w-]*)")
        .expect("valid powershell recovery re")
});

/// Blank out `//`-to-end-of-line and `/* … */` comment bodies, preserving every
/// newline so recovered line numbers stay exact. Declaration keywords inside
/// comments must not be recovered as real symbols.
fn blank_comments(text: &str) -> String {
    let b = text.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                out.push(b' ');
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            out.push(b' ');
            out.push(b' ');
            i += 2;
            while i < b.len() && !(b[i] == b'*' && i + 1 < b.len() && b[i + 1] == b'/') {
                out.push(if b[i] == b'\n' { b'\n' } else { b' ' });
                i += 1;
            }
            if i < b.len() {
                out.push(b' ');
                i += 1;
                if i < b.len() {
                    out.push(b' ');
                    i += 1;
                }
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Groovy specs embed whole source files in triple-quoted strings. Those
/// declarations belong to test data, not to the surrounding file's graph.
pub(crate) fn blank_groovy_noncode(text: &str) -> String {
    let mut bytes = text.as_bytes().to_vec();
    let mut i = 0;
    while i < bytes.len() {
        if bytes
            .get(i..i + 2)
            .is_some_and(|s| s == b"//" || s == b"/*")
        {
            let block = bytes[i + 1] == b'*';
            let start = i;
            i += 2;
            while i < bytes.len() {
                if block && bytes.get(i..i + 2) == Some(b"*/") {
                    i += 2;
                    break;
                }
                if !block && bytes[i] == b'\n' {
                    break;
                }
                i += 1;
            }
            for byte in &mut bytes[start..i] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            continue;
        }
        let dollar_slashy = bytes.get(i..i + 2) == Some(b"$/");
        // A slash after an operand is division; after an expression opener it
        // starts a string. Command-style argument ambiguity stays conservative.
        let slashy = bytes[i] == b'/' && {
            let prefix = text[..i].trim_end();
            prefix.is_empty()
                || prefix.ends_with(['=', '~', '(', '[', '{', ',', ':', '!'])
                || prefix
                    .split_whitespace()
                    .last()
                    .is_some_and(|word| matches!(word, "return" | "case" | "assert"))
        };
        if dollar_slashy || slashy {
            let start = i;
            i += if dollar_slashy { 2 } else { 1 };
            while i < bytes.len() {
                if dollar_slashy && bytes.get(i..i + 2) == Some(b"/$") {
                    i += 2;
                    break;
                }
                if !dollar_slashy && bytes[i] == b'/' {
                    i += 1;
                    break;
                }
                if (dollar_slashy
                    && bytes[i] == b'$'
                    && bytes.get(i + 1).is_some_and(|b| matches!(*b, b'$' | b'/')))
                    || (!dollar_slashy && bytes.get(i..i + 2) == Some(b"\\/"))
                {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            for byte in &mut bytes[start..i] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
            continue;
        }
        if !matches!(bytes[i], b'\'' | b'"') {
            i += 1;
            continue;
        }
        let quote = bytes[i];
        let triple = bytes.get(i..i + 3) == Some(&[quote; 3]);
        let width = if triple { 3 } else { 1 };
        let start = i;
        i += width;
        while i < bytes.len() {
            if bytes[i] == b'\\' {
                i = (i + 2).min(bytes.len());
                continue;
            }
            if bytes
                .get(i..i + width)
                .is_some_and(|s| s.iter().all(|b| *b == quote))
            {
                i += width;
                break;
            }
            i += 1;
        }
        if triple {
            for byte in &mut bytes[start..i] {
                if !matches!(*byte, b'\r' | b'\n') {
                    *byte = b' ';
                }
            }
        }
    }
    String::from_utf8(bytes).expect("blanking preserves UTF-8")
}

/// Declaration-shaped lines in `source` for the given extension, in source order.
/// Empty for extensions with no recovery patterns.
pub(crate) fn scan(ext: &str, source: &[u8]) -> Vec<Decl> {
    let raw = String::from_utf8_lossy(source);
    // PowerShell uses `#` comments, not `//`; blanking C-style comments there would
    // corrupt nothing but buys nothing either, so it is skipped.
    let text = if ext == "ps1" || ext == "psm1" {
        raw.into_owned()
    } else if matches!(ext, "groovy" | "gradle") {
        blank_groovy_noncode(&raw)
    } else {
        blank_comments(&raw)
    };
    let newlines: Vec<usize> = text.match_indices('\n').map(|(i, _)| i).collect();
    let line_at = |byte: usize| newlines.partition_point(|&nl| nl < byte) + 1;

    let patterns: &[(&LazyLock<Regex>, bool)] = match ext {
        "sv" | "v" | "svh" | "vh" => &[(&VERILOG_RE, false), (&VERILOG_ROUTINE_RE, true)],
        "groovy" | "gradle" => &[(&GROOVY_TYPE_RE, false), (&GROOVY_ROUTINE_RE, true)],
        "m" | "mm" | "h" => &[(&OBJC_TYPE_RE, false)],
        "ps1" | "psm1" => &[(&POWERSHELL_RE, true)],
        "jl" => &[(&JULIA_RE, false), (&JULIA_ROUTINE_RE, true)],
        _ => &[],
    };

    let mut out: Vec<Decl> = Vec::new();
    for (re, is_routine) in patterns {
        for cap in re.captures_iter(&text) {
            let m = cap.get(1).expect("capture group 1 exists");
            let name = m
                .as_str()
                .strip_prefix(['\'', '"'])
                .map_or(m.as_str(), |s| &s[..s.len() - 1])
                .to_string();
            // Control keywords can shadow the routine patterns (`if (`, `while (`).
            if matches!(
                name.as_str(),
                "if" | "for" | "while" | "switch" | "catch" | "return" | "case" | "do" | "else"
            ) {
                continue;
            }
            let line = line_at(cap.get(0).expect("group 0 is the full match").start());
            if !out.iter().any(|d| d.name == name && d.line == line) {
                out.push(Decl {
                    name,
                    line,
                    is_routine: *is_routine,
                });
            }
        }
    }
    out.sort_by_key(|d| (d.line, d.name.clone()));
    out
}

/// Fill in declarations the grammar failed to produce for this file.
///
/// No-op unless the grammar reported an error. Declarations the parse already named
/// are skipped, so this only ever adds what was missing.
pub(crate) fn apply(path: &str, ext: &str, source: &[u8], result: &mut ExtractionResult) {
    let file_nid = file_node_id(path);
    if !result.parse_error {
        return;
    }
    if !result.nodes.iter().any(|n| n.id == file_nid) {
        return; // nothing to hang recovered symbols from
    }
    let decls = scan(ext, source);
    if decls.is_empty() {
        return;
    }
    // Fill gaps, do not duplicate. A partial parse is the common case — the grammar
    // recovers enough to emit the first declaration and drops later ones — and it is
    // where most loss lives, so recovery supplements rather than only rescuing total
    // losses. Anything the grammar already named in this file is left untouched;
    // external import stubs carry no `source_location` and never mask a declaration.
    let already: std::collections::HashSet<String> = result
        .nodes
        .iter()
        .filter(|n| n.id != file_nid && n.source_location.is_some())
        .map(|n| {
            n.label
                .trim_start_matches('.')
                .trim_end_matches("()")
                .rsplit('.')
                .next()
                .unwrap_or(&n.label)
                .to_ascii_lowercase()
        })
        .collect();

    let stem = file_stem(path);
    let mut b = Builder::new(path);
    for d in decls
        .iter()
        .filter(|d| !already.contains(&d.name.to_ascii_lowercase()))
    {
        let nid = NodeId(make_id(&[&stem, &symbol_key(&d.name)]));
        let label = if d.is_routine {
            format!("{}()", d.name)
        } else {
            d.name.clone()
        };
        b.add_node(nid.clone(), label, d.line);
        b.add_edge(
            file_nid.clone(),
            nid,
            "contains",
            d.line,
            Some("recovered_declaration"),
        );
    }
    let mut recovered = b.into_result();
    for n in &mut recovered.nodes {
        n.extra
            .insert("recovered".into(), serde_json::Value::Bool(true));
    }
    for e in &mut recovered.edges {
        e.confidence = synaptic_core::Confidence::Inferred;
        e.confidence_score = Some(0.5);
    }
    result.nodes.extend(recovered.nodes);
    result.edges.extend(recovered.edges);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(ext: &str, src: &str) -> Vec<(String, usize)> {
        scan(ext, src.as_bytes())
            .into_iter()
            .map(|d| (d.name, d.line))
            .collect()
    }

    #[test]
    fn verilog_modules_and_interfaces_are_recovered_with_exact_lines() {
        let src = "// header\n\nmodule axi_demux_simple #(\n  parameter int W = 8\n) (\n  input logic clk\n);\nendmodule\n\ninterface AXI_BUS #(\n  parameter int W = 8\n);\nendinterface\n";
        assert_eq!(
            names("sv", src),
            vec![
                ("axi_demux_simple".to_string(), 3),
                ("AXI_BUS".to_string(), 10)
            ]
        );
    }

    /// SystemVerilog `class` is the backbone of UVM testbenches and lives mostly in
    /// `.svh` headers, so a verification-heavy repo is largely classes.
    #[test]
    fn systemverilog_classes_are_recovered() {
        let src = "class instr_trace_item #(\n  parameter int W = 8\n);\nendclass\n\nclass string_buffer extends uvm_component;\nendclass\n";
        assert_eq!(
            names("svh", src),
            vec![
                ("instr_trace_item".to_string(), 1),
                ("string_buffer".to_string(), 6)
            ]
        );
    }

    #[test]
    fn julia_functions_and_structs_are_recovered() {
        let src = "module M\n\nstruct Thing\nend\n\nfunction make_unique(names::AbstractVector{Symbol}; makeunique::Bool=false)\nend\n\nmutable struct Other\nend\nend\n";
        let got = names("jl", src);
        assert!(got.contains(&("Thing".to_string(), 3)), "{got:?}");
        assert!(got.contains(&("make_unique".to_string(), 6)), "{got:?}");
        assert!(got.contains(&("Other".to_string(), 9)), "{got:?}");
    }

    #[test]
    fn verilog_functions_and_tasks_are_recovered() {
        let src = "module m;\n  function automatic int idx_width(int x);\n  endfunction\n  task do_thing(input int a);\n  endtask\nendmodule\n";
        let got = names("sv", src);
        assert!(got.contains(&("idx_width".to_string(), 2)), "{got:?}");
        assert!(got.contains(&("do_thing".to_string(), 4)), "{got:?}");
    }

    #[test]
    fn declarations_inside_comments_are_not_recovered() {
        let src = "// module commented_out (\n/* interface AlsoCommented #(\n   still inside\n*/\nmodule real_one (input logic c);\nendmodule\n";
        assert_eq!(names("sv", src), vec![("real_one".to_string(), 5)]);
    }

    #[test]
    fn groovy_types_and_methods_are_recovered() {
        let src = "package p\n\n@CompileStatic\nclass DocsRedirects {\n  void generate() {\n  }\n  def helper(String a) {\n  }\n}\n";
        let got = names("groovy", src);
        assert!(got.contains(&("DocsRedirects".to_string(), 4)), "{got:?}");
        assert!(got.contains(&("generate".to_string(), 5)), "{got:?}");
        assert!(got.contains(&("helper".to_string(), 7)), "{got:?}");
    }

    #[test]
    fn groovy_control_flow_is_not_a_method() {
        let src = "class C {\n  void run() {\n    if (x) { }\n    while (y) { }\n  }\n}\n";
        let got = names("groovy", src);
        assert!(
            !got.iter().any(|(n, _)| n == "if" || n == "while"),
            "{got:?}"
        );
    }

    #[test]
    fn groovy_embedded_source_is_not_recovered() {
        let src = "def url = 'http://example.test'\ndef text = '''\nclass Fake {\n  def \"not a real feature\"() {}\n}\n'''\n\ndef \"real feature\"() {}\n";
        assert_eq!(names("groovy", src), vec![("real feature".into(), 8)]);
        assert_eq!(blank_groovy_noncode(src).len(), src.len());
    }

    #[test]
    fn groovy_slashy_strings_do_not_recover_embedded_declarations() {
        let src = r#"def text = /é
class Fake {}
def fake() {}
escaped \/ slash
/
def other = $/
class Other {}
def hidden() {}
$$ escaped dollar $/ escaped slash
/$
def ratio = total / count
def "escaped \"name\""() {}
"#;
        assert_eq!(
            names("groovy", src),
            vec![(r#"escaped \"name\""#.into(), 12)]
        );
        assert_eq!(blank_groovy_noncode(src).len(), src.len());
    }

    #[cfg(feature = "lang-groovy")]
    #[test]
    fn groovy_quoted_features_and_primitive_methods_are_recovered() {
        let src = "class Spec {\n  def \"empty input returns nothing\"() {\n    expect: true\n  }\n  def 'another feature'() {}\n  boolean ready() {}\n  // def \"commented feature\"() {}\n}\n";
        let got = names("groovy", src);
        assert!(
            got.contains(&("empty input returns nothing".into(), 2)),
            "{got:?}"
        );
        assert!(got.contains(&("another feature".into(), 5)));
        assert!(got.contains(&("ready".into(), 6)));
        assert!(!got.iter().any(|(n, _)| n == "commented feature"));
        let r = crate::extract_source("Spec.groovy", src.as_bytes()).unwrap();
        let feature = r
            .nodes
            .iter()
            .find(|n| n.label.trim_start_matches('.') == "empty input returns nothing()")
            .unwrap();
        assert_eq!(feature.source_location.as_deref(), Some("L2"));
        assert_ne!(
            feature.extra.get("recovered"),
            Some(&serde_json::Value::Bool(true))
        );
    }

    #[test]
    fn objc_interfaces_and_protocols_are_recovered() {
        let src = "#import <Foundation/Foundation.h>\n\n@protocol AFURLResponseSerialization <NSObject>\n@end\n\n@interface AFHTTPSessionManager : NSObject\n@end\n";
        assert_eq!(
            names("h", src),
            vec![
                ("AFURLResponseSerialization".to_string(), 3),
                ("AFHTTPSessionManager".to_string(), 6)
            ]
        );
    }

    #[test]
    fn powershell_functions_are_recovered_case_insensitively() {
        let src = "# comment\nFunction Get-Planet {\n}\nfunction Set-Thing {\n}\n";
        assert_eq!(
            names("ps1", src),
            vec![("Get-Planet".to_string(), 2), ("Set-Thing".to_string(), 4)]
        );
    }

    /// `make_id` maps every non-word character to `_`, so `make_unique!` and
    /// `make_unique` collide — and in Julia those are two different functions (the
    /// mutating one and its copy). Recovered punctuation-bearing names must keep
    /// distinct identities, as the Ruby extractor already does.
    #[test]
    fn punctuation_bearing_names_keep_distinct_ids() {
        let mut b = Builder::new("src/utils.jl");
        b.add_node(file_node_id("src/utils.jl"), "utils.jl".to_string(), 1);
        b.add_node(
            NodeId(make_id(&["src.utils", "make_unique"])),
            "make_unique()".into(),
            120,
        );
        let mut r = b.into_result();
        r.parse_error = true;

        apply(
            "src/utils.jl",
            "jl",
            b"function make_unique!(names)\nend\n\nfunction make_unique(names)\nend\n",
            &mut r,
        );
        let bang = r
            .nodes
            .iter()
            .find(|n| n.label == "make_unique!()")
            .expect("mutating variant must be recovered");
        let plain = r
            .nodes
            .iter()
            .find(|n| n.label == "make_unique()")
            .expect("parsed variant stays");
        assert_ne!(
            bang.id, plain.id,
            "`make_unique!` must not collapse onto `make_unique`"
        );
    }

    #[test]
    fn unknown_extension_recovers_nothing() {
        assert!(scan("rs", b"fn main() {}\n").is_empty());
    }

    /// Import stubs are not declarations. A header whose `#import`s resolved but
    /// whose `@interface` was swallowed still lost every symbol it defines, so the
    /// presence of stub nodes must not suppress recovery. (Real shape: AFNetworking
    /// headers came out as file node + `Foundation` + `Security` stubs and nothing
    /// else — 60 such files across the corpus.)
    #[test]
    fn stub_only_results_still_recover() {
        let mut b = Builder::new("AFNetworking/AFSecurityPolicy.h");
        b.add_node(
            file_node_id("AFNetworking/AFSecurityPolicy.h"),
            "AFSecurityPolicy.h".to_string(),
            1,
        );
        b.add_external_node(
            NodeId(make_id(&["objc", "hdr", "Foundation"])),
            "Foundation".into(),
        );
        let mut r = b.into_result();
        r.parse_error = true;

        apply(
            "AFNetworking/AFSecurityPolicy.h",
            "h",
            b"#import <Foundation/Foundation.h>\n\n@interface AFSecurityPolicy : NSObject\n@end\n",
            &mut r,
        );
        assert!(
            r.nodes.iter().any(|n| n.label == "AFSecurityPolicy"),
            "{:?}",
            r.nodes.iter().map(|n| &n.label).collect::<Vec<_>>()
        );
    }

    /// A partial parse drops declarations silently: the grammar recovers enough to
    /// emit the first module and loses the second. Measured across the corpus that is
    /// 1,290 real declarations (1,043 Groovy, 166 SystemVerilog, 46 PowerShell) --
    /// far more than total losses. Gaps are filled; what parsed is never duplicated.
    #[test]
    fn a_partial_parse_has_its_gaps_filled_without_duplication() {
        let mut b = Builder::new("src/axi_cdc_dst.sv");
        b.add_node(
            file_node_id("src/axi_cdc_dst.sv"),
            "axi_cdc_dst.sv".to_string(),
            1,
        );
        b.add_node(
            NodeId(make_id(&["src.axi_cdc_dst", "axi_cdc_dst"])),
            "axi_cdc_dst".into(),
            3,
        );
        let mut r = b.into_result();
        r.parse_error = true;

        apply(
            "src/axi_cdc_dst.sv",
            "sv",
            b"// c

module axi_cdc_dst;
endmodule

module axi_cdc_dst_intf;
endmodule
",
            &mut r,
        );
        let labels: Vec<&str> = r.nodes.iter().map(|n| n.label.as_str()).collect();
        assert!(
            labels.contains(&"axi_cdc_dst_intf"),
            "dropped module must be recovered: {labels:?}"
        );
        assert_eq!(
            labels.iter().filter(|l| **l == "axi_cdc_dst").count(),
            1,
            "what the grammar parsed must not be duplicated: {labels:?}"
        );
        let rec = r
            .nodes
            .iter()
            .find(|n| n.label == "axi_cdc_dst_intf")
            .expect("recovered node");
        assert_eq!(rec.source_location, Some("L6".to_string()));
    }

    #[test]
    fn apply_is_a_no_op_when_the_parse_succeeded() {
        let mut r = ExtractionResult {
            parse_error: false,
            ..Default::default()
        };
        apply("a/b.sv", "sv", b"module m;\nendmodule\n", &mut r);
        assert!(r.nodes.is_empty());
    }
}
