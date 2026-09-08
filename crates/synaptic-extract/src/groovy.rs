//! Groovy extractor using a native Groovy grammar and shared JVM graph extraction.

#[cfg(feature = "lang-groovy")]
use crate::config::{HeritageStyle, ImportStyle, LanguageConfig, TypeRefStyle};
#[cfg(feature = "lang-groovy")]
use crate::result::ExtractionResult;
#[cfg(feature = "lang-groovy")]
use crate::walker::extract_with_config;

/// The Groovy grammar's declaration, call, and type vocabulary.
#[cfg(feature = "lang-groovy")]
pub fn groovy_config() -> LanguageConfig {
    LanguageConfig {
        language: || tree_sitter_groovy::LANGUAGE.into(),
        class_types: &[
            "class_declaration",
            "interface_declaration",
            "enum_declaration",
            "trait_declaration",
            "record_declaration",
            "annotation_type_declaration",
        ],
        function_types: &["method_declaration", "constructor_declaration"],
        call_types: &["method_invocation", "command_chain"],
        name_field: "name",
        body_field: "body",
        call_function_field: "function",
        call_accessor_node_types: &[
            "field_access",
            "safe_navigation_expression",
            "safe_chain_dot_expression",
            "spread_dot_expression",
        ],
        call_accessor_field: "field",
        function_boundary_types: &["method_declaration", "constructor_declaration"],
        superclasses_field: None,
        decorated_types: &[],
        builtins: &[],
        import_types: &["import_declaration"],
        import_style: Some(ImportStyle::Java),
        type_ref_style: Some(TypeRefStyle::Java),
        heritage_style: Some(HeritageStyle::Java),
        constructor_call_type: None,
        body_kinds: &[],
    }
}

/// Extract a Groovy source file already in memory.
#[cfg(feature = "lang-groovy")]
pub fn extract_groovy_source(path: &str, source: &[u8]) -> ExtractionResult {
    extract_with_config(path, source, &groovy_config())
}

/// Read and extract a Groovy file from disk.
#[cfg(feature = "lang-groovy")]
pub fn extract_groovy_file(path: &std::path::Path) -> std::io::Result<ExtractionResult> {
    let source = std::fs::read(path)?;
    let path_str = path.to_string_lossy();
    Ok(extract_groovy_source(&path_str, &source))
}

#[cfg(all(test, feature = "lang-groovy"))]
mod tests {
    #[test]
    fn build_dsl_and_statement_boundaries_preserve_member_calls() {
        let source = br#"class Dsl {
  private final Number value
  private final Number epsilon
  Dsl(Number value, Number epsilon) { this.value = value; this.epsilon = epsilon }
  def run(client) {
    def data = client.decode 'text', String
    client.configure mode: 'test', count: 2
    def action = { target = 1 }
    [1, *[2, 3]].each { value -> client.send value, data }
    def values = new Object[2]
    def (String name, int count) = ['a', 2]
    assert count == 2, 'count'
  }
}
"#;
        let result = super::extract_groovy_source("Dsl.groovy", source);
        assert!(!result.parse_error);
        let constructor = result.nodes.iter().find(|n| n.label == ".Dsl()").unwrap();
        assert_eq!(constructor.source_location.as_deref(), Some("L4"));
        for name in ["client.decode", "client.configure", "client.send"] {
            assert!(
                result
                    .raw_calls
                    .iter()
                    .any(|c| c.callee == name && c.is_member_call),
                "{name}: {:?}",
                result.raw_calls
            );
        }
        let script = super::extract_groovy_source("core.gradle", b"tasks.named(\"processResources\") {\n def tokens = [version: version.toString()]\n inputs.property \"tokens\", tokens\n filter(org.apache.tools.ant.filters.ReplaceTokens, tokens: tokens)\n}\n");
        assert!(!script.parse_error);
        assert!(!script.nodes.iter().any(|n| n.label == "filter()"));
        assert!(script.raw_calls.iter().any(|c| c.callee == "filter"));
    }
    #[test]
    fn implicit_return_methods_and_java_lambdas_retain_calls() {
        let source = b"class Features {\nfinal static CMP = { a,b -> a <=> b }\n@Unroll 'implicit return feature'() {\ndef values = new ArrayList<>()\nvalues.map(x -> helper(x))\nvalues.map((String x) -> { helper(x) })\n}\npublic run() { helper(1) }\ndef helper(x) { x }\n}\n";
        let r = super::extract_groovy_source("Features.groovy", source);
        assert!(!r.parse_error, "{r:?}");
        for label in [".implicit return feature()", ".run()", ".helper()"] {
            assert!(r.nodes.iter().any(|n| n.label == label), "{r:?}");
        }
        let helper = &r.nodes.iter().find(|n| n.label == ".helper()").unwrap().id;
        assert_eq!(
            r.edges
                .iter()
                .filter(|e| e.relation == "calls" && &e.target == helper)
                .count(),
            2
        );
    }

    use super::extract_groovy_source;
    use crate::result::ExtractionResult;

    #[test]
    fn native_groovy_syntax_keeps_types_annotations_and_call_forms() {
        let r = extract_groovy_source(
            "Dsl.groovy",
            br#"import pkg.Base
trait Named extends pkg.Base { String title() { 'title' } }
class Dsl {
  def pattern = /class Fake { def missing() {} }/
  @Deprecated
  pkg.Result work(@Deprecated final pkg.Input value) {
    def chosen = value ?: 'fallback'
    helper 'command'
    this?.helper(chosen)
    value*.send()
    value.each { helper(it) }
    value.each(1) { helper(it) }
    return helper("${chosen}")
  }
  void helper(value) {}
  static class Nested { void run() { nestedHelper() } void nestedHelper() {} }
}
"#,
        );
        assert!(!r.parse_error, "{r:?}");
        assert!(
            !labels(&r)
                .iter()
                .any(|n| n.contains("Fake") || n.contains("missing"))
        );
        assert!(rels(&r, "inherits").contains(&("Named".into(), "Base".into())));
        assert!(rels(&r, "references").contains(&(".work()".into(), "Result".into())));
        assert!(rels(&r, "references").contains(&(".work()".into(), "Input".into())));
        assert!(rels(&r, "calls").contains(&(".work()".into(), ".helper()".into())));
        assert!(rels(&r, "calls").contains(&(".run()".into(), ".nestedHelper()".into())));
        assert!(
            r.raw_calls
                .iter()
                .any(|c| c.callee == "value.each" && c.is_member_call)
        );
        assert!(
            r.raw_calls
                .iter()
                .any(|c| c.callee == "this.helper" && c.is_member_call)
        );
        assert!(
            r.raw_calls
                .iter()
                .any(|c| c.callee == "value.send" && c.is_member_call)
        );
    }

    #[test]
    fn quoted_methods_keep_bodies_owners_and_distinct_ids() {
        let src = br#"class Spec {
  def "it's \"ready\""() { helper() }
  def 'a-b'() { helper() }
  def 'a b'() { helper() }
  void helper() {}
}
"#;
        let r = extract_groovy_source("Spec.groovy", src);
        let feature = r
            .nodes
            .iter()
            .find(|n| n.label == r#".it's "ready"()"#)
            .unwrap();
        assert_eq!(feature.source_location.as_deref(), Some("L2"));
        assert_eq!(feature.extra["source_name"], r#""it's \"ready\"""#);
        assert_eq!(feature.span().unwrap().end_line, 2);
        assert!(
            r.edges
                .iter()
                .any(|e| e.target == feature.id && e.relation == "method")
        );
        assert!(rels(&r, "calls").contains(&(feature.label.clone(), ".helper()".into())));
        let ids: std::collections::HashSet<_> = r
            .nodes
            .iter()
            .filter(|n| n.label.starts_with(".a"))
            .map(|n| &n.id)
            .collect();
        assert_eq!(ids.len(), 2);
    }

    const SAMPLE: &[u8] = b"package p\nimport a.B\n\nclass Dog extends Animal implements Greeter {\n  String bark() { return sound() }\n  String sound() { return 'woof' }\n}\n";

    #[test]
    fn overloads_and_anonymous_methods_keep_distinct_source_identity() {
        let r = extract_groovy_source(
            "Overloads.gradle",
            br#"class Overloads {
  def run() { pick(1) }
  def pick(int x) {} def pick(String x) {} def pick(int x, int y) {}
  def first = new Runnable() { void apply() {} }
  def second = new Runnable() { void apply() {} }
  def 'escaped\\name\u0021\041'() {}
}
"#,
        );
        assert!(!r.parse_error, "{r:?}");
        for (label, count) in [(".pick()", 3), ("apply()", 2)] {
            let ids: std::collections::HashSet<_> = r
                .nodes
                .iter()
                .filter(|n| n.label == label)
                .map(|n| &n.id)
                .collect();
            assert_eq!(ids.len(), count, "{label}: {r:?}");
            assert!(
                !r.edges
                    .iter()
                    .any(|e| e.relation == "calls" && ids.contains(&e.target))
            );
        }
        assert!(r.raw_calls.iter().any(|c| c.callee == "pick"));
        assert!(r.nodes.iter().any(|n| n.label == ".escaped\\name!!()"));
    }

    fn extract() -> ExtractionResult {
        extract_groovy_source("src/Dog.groovy", SAMPLE)
    }

    fn labels(r: &ExtractionResult) -> Vec<String> {
        r.nodes.iter().map(|n| n.label.clone()).collect()
    }

    fn rels(r: &ExtractionResult, relation: &str) -> Vec<(String, String)> {
        let lbl = |id: &synaptic_core::NodeId| {
            r.nodes
                .iter()
                .find(|n| &n.id == id)
                .map(|n| n.label.clone())
                .unwrap_or_else(|| id.0.clone())
        };
        r.edges
            .iter()
            .filter(|e| e.relation == relation)
            .map(|e| (lbl(&e.source), lbl(&e.target)))
            .collect()
    }

    #[test]
    fn class_and_method_nodes() {
        let ls = labels(&extract());
        assert!(ls.contains(&"Dog".to_string()), "{ls:?}");
        assert!(ls.contains(&".bark()".to_string()));
        assert!(ls.contains(&".sound()".to_string()));
    }

    #[test]
    fn import_extends_implements() {
        let r = extract();
        assert!(rels(&r, "imports").iter().any(|(_, t)| t == "B"));
        assert!(rels(&r, "inherits").contains(&("Dog".to_string(), "Animal".to_string())));
        assert!(rels(&r, "implements").contains(&("Dog".to_string(), "Greeter".to_string())));
    }

    #[test]
    fn calls_resolve() {
        assert!(
            rels(&extract(), "calls").contains(&(".bark()".to_string(), ".sound()".to_string())),
            "{:?}",
            rels(&extract(), "calls")
        );
    }

    #[test]
    fn malformed_method_prefix_keeps_name_anchor_and_body_span() {
        let src = b"class Close {\n  private final Number value\n  private final Number epsilon\n\n  Close(Number value, Number epsilon) {\n    this.value = value\n    this.epsilon = epsilon\n  }\n}\n";
        let r = extract_groovy_source("Close.groovy", src);
        let constructor = r.nodes.iter().find(|n| n.label == ".Close()").unwrap();
        assert_eq!(constructor.source_location.as_deref(), Some("L5"));
        let span = constructor.span().unwrap();
        assert_eq!((span.start_line, span.end_line), (5, 8));
        assert!(r.edges.iter().any(|e| e.target == constructor.id
            && e.relation == "method"
            && e.source_location.as_deref() == Some("L5")));
    }
}
