//! Optional facts exported by the project's compiler, bound to its source inputs.
use crate::{ExtractionResult, RawCall};
use serde::Deserialize;
use std::{collections::HashMap, path::Path};
use synaptic_core::{
    Confidence, DynamicKind, DynamicSite, Edge, Node, NodeId, NodeKind, Param, Signature,
};

#[derive(Default, Deserialize)]
pub(crate) struct CompilerFacts {
    version: u32,
    compiler: String,
    files: HashMap<String, FileFacts>,
    #[serde(default)]
    classpath: Vec<Classpath>,
    #[serde(skip)]
    diagnostic: Option<String>,
}

#[cfg(all(test, feature = "lang-groovy"))]
mod tests {
    #[test]
    fn generated_methods_resolved_calls_and_stale_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let source = "class Worker {\n def run() { work() }\n}\n";
        std::fs::write(root.join("Worker.groovy"), source).unwrap();
        std::fs::create_dir(root.join(".synaptic")).unwrap();
        let facts = serde_json::json!({"version":1,"compiler":"Groovy test","files":{"Worker.groovy":{"source":source,"methods":[
            {"symbol":"Worker#run()","owner":"Worker","name":"run","line":2,"generated":false,"parameters":[],"calls":[
                {"line":2,"target":"Worker#work()","name":"work","snippet":"work()"},
                {"line":2,"target":"Worker#getName()","name":"getName","snippet":"getName()"},
                {"line":2,"target":null,"name":null,"snippet":"this[name]()"}]},
            {"symbol":"Worker#work()","owner":"Worker","name":"work","line":1,"generated":true,"parameters":[],"calls":[]}
        ]}}});
        std::fs::write(
            root.join(".synaptic/compiler-facts.json"),
            facts.to_string(),
        )
        .unwrap();
        let project = crate::project::Project::load(&root);
        let result = project
            .extract(None, "Worker.groovy", source.as_bytes())
            .unwrap();
        assert_eq!(
            result.nodes.iter().filter(|n| n.label == ".run()").count(),
            1
        );
        assert!(result.nodes.iter().any(
            |n| n.label == ".work()" && n.extra.get("compiler_generated") == Some(&true.into())
        ));
        assert_eq!(
            result
                .raw_calls
                .iter()
                .filter(|c| c.callee.starts_with("compiler:"))
                .count(),
            2
        );
        assert!(
            result
                .nodes
                .iter()
                .any(|n| n.dynamic_sites().iter().any(|s| s.key.is_none()))
        );
        std::fs::write(root.join("Worker.groovy"), "class Changed {}\n").unwrap();
        let stale = crate::project::Project::load(&root)
            .extract(None, "Worker.groovy", b"class Changed {}\n")
            .unwrap();
        assert!(stale.nodes[0].extra.contains_key("compiler_diagnostic"));
        assert!(
            !stale
                .nodes
                .iter()
                .any(|n| n.extra.contains_key("compiler_generated"))
        );
    }
}

#[derive(Deserialize)]
struct Classpath {
    path: String,
    #[serde(default)]
    directory: bool,
    size: u64,
    modified: u64,
}

#[derive(Deserialize)]
struct FileFacts {
    source: String,
    methods: Vec<Method>,
}

#[derive(Deserialize)]
struct Method {
    symbol: String,
    owner: String,
    name: String,
    line: u32,
    generated: bool,
    parameters: Vec<Param>,
    calls: Vec<Call>,
}

#[derive(Deserialize)]
struct Call {
    line: u32,
    target: Option<String>,
    name: Option<String>,
    snippet: String,
}

impl CompilerFacts {
    pub(crate) fn load(root: &Path) -> Self {
        let path = std::env::var_os("SYNAPTIC_COMPILER_FACTS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join(".synaptic/compiler-facts.json"));
        if !path.exists() {
            return Self::default();
        }
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| serde_json::from_slice::<Self>(&b).map_err(|e| e.to_string()));
        let mut facts = match result {
            Ok(facts) => facts,
            Err(error) => {
                return Self {
                    diagnostic: Some(format!("invalid compiler facts: {error}")),
                    ..Self::default()
                };
            }
        };
        let fresh = facts.version == 1
            && facts.files.iter().all(|(path, fact)| {
                root.join(path)
                    .canonicalize()
                    .is_ok_and(|p| p.starts_with(root))
                    && std::fs::read(root.join(path)).is_ok_and(|b| b == fact.source.as_bytes())
            })
            && facts.classpath.iter().all(|entry| {
                std::fs::metadata(&entry.path).is_ok_and(|m| {
                    m.is_dir() == entry.directory
                        && (entry.directory || m.len() == entry.size)
                        && m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .is_some_and(|d| d.as_millis() == entry.modified as u128)
                })
            });
        if !fresh {
            facts.files.clear();
            facts.diagnostic =
                Some("stale compiler facts; rerun the compiler exporter for changed inputs".into());
        }
        facts
    }

    pub(crate) fn is_configured(&self) -> bool {
        !self.files.is_empty() || self.diagnostic.is_some()
    }

    pub(crate) fn apply(&self, path: &str, source: &[u8], result: &mut ExtractionResult) {
        if let Some(error) = &self.diagnostic {
            if let Some(file) = result.nodes.first_mut() {
                file.extra
                    .insert("compiler_diagnostic".into(), error.clone().into());
            }
            return;
        }
        let Some(facts) = self.files.get(&path.replace('\\', "/")) else {
            return;
        };
        if facts.source.as_bytes() != source {
            return;
        }
        let mut ids = HashMap::new();
        for method in &facts.methods {
            let label = format!(".{}()", method.name);
            let owners: Vec<_> = result
                .nodes
                .iter()
                .filter(|n| n.label == method.owner)
                .map(|n| n.id.clone())
                .collect();
            if owners.len() != 1 {
                continue;
            }
            let owner = &owners[0];
            let existing: Vec<_> = result
                .nodes
                .iter()
                .filter(|n| {
                    !method.generated
                        && n.label == label
                        && (n.source_location.as_deref() == Some(&format!("L{}", method.line))
                            || n.span.is_some_and(|s| {
                                s.start_line <= method.line && method.line <= s.end_line
                            }))
                        && n.signature.as_ref().is_none_or(|s| {
                            s.params.len() == method.parameters.len()
                                && s.params.iter().zip(&method.parameters).all(|(a, b)| {
                                    a.type_ref.as_ref().zip(b.type_ref.as_ref()).is_none_or(
                                        |(a, b)| {
                                            a.split('<').next().unwrap_or(a).rsplit('.').next()
                                                == b.split('<')
                                                    .next()
                                                    .unwrap_or(b)
                                                    .rsplit('.')
                                                    .next()
                                        },
                                    )
                                })
                        })
                        && result.edges.iter().any(|e| {
                            &e.source == owner && e.target == n.id && e.relation == "method"
                        })
                })
                .map(|n| n.id.clone())
                .collect();
            let id = if existing.len() == 1 {
                existing[0].clone()
            } else {
                let id = NodeId::new(&[
                    path,
                    "compiler",
                    blake3::hash(method.symbol.as_bytes()).to_hex().as_ref(),
                ]);
                let mut node = Node {
                    id: id.clone(),
                    label,
                    source_file: path.into(),
                    file_type: synaptic_core::FileType::Code,
                    source_location: Some(format!("L{}", method.line)),
                    origin: Some("compiler".into()),
                    ..Default::default()
                };
                node.set_kind(if method.name == method.owner {
                    NodeKind::Constructor
                } else {
                    NodeKind::Method
                });
                node.signature = Some(Box::new(Signature {
                    params: method.parameters.clone(),
                    return_type: None,
                    raw: method.symbol.clone(),
                }));
                node.extra
                    .insert("compiler_generated".into(), method.generated.into());
                if method.generated {
                    node.extra
                        .insert("anchor_kind".into(), "generating_class".into());
                }
                result.nodes.push(node);
                result.edges.push(Edge {
                    source: owner.clone(),
                    target: id.clone(),
                    relation: "method".into(),
                    confidence: Confidence::Extracted,
                    confidence_score: Some(1.0),
                    source_file: path.into(),
                    source_location: Some(format!("L{}", method.line)),
                    weight: 1.0,
                    context: Some("compiler_declaration".into()),
                    cross_repo: false,
                    extra: Default::default(),
                });
                id
            };
            if let Some(node) = result.nodes.iter_mut().find(|n| n.id == id) {
                node.extra
                    .insert("compiler_symbol".into(), method.symbol.clone().into());
                node.extra
                    .insert("compiler".into(), self.compiler.clone().into());
            }
            ids.insert(&method.symbol, id);
        }
        for method in &facts.methods {
            let Some(caller) = ids.get(&method.symbol) else {
                continue;
            };
            for call in &method.calls {
                if let Some(target) = &call.target {
                    // Compiler resolution replaces heuristic evidence at this site.
                    let location = format!("L{}", call.line);
                    result.edges.retain(|e| {
                        !(e.relation == "calls"
                            && &e.source == caller
                            && e.source_location.as_deref() == Some(&location))
                    });
                    result.raw_calls.retain(|c| {
                        c.callee.starts_with("compiler:")
                            || !(&c.caller == caller
                                && c.source_location.as_deref() == Some(&location))
                    });
                    result.raw_calls.push(RawCall {
                        caller: caller.clone(),
                        callee: format!("compiler:{target}"),
                        is_member_call: false,
                        source_file: path.into(),
                        source_location: Some(location),
                        span: None,
                    });
                } else if let Some(node) = result.nodes.iter_mut().find(|n| &n.id == caller) {
                    node.push_dynamic_site(DynamicSite {
                        kind: DynamicKind::Reflection,
                        line: call.line,
                        key: call.name.clone(),
                        snippet: call.snippet.clone(),
                    });
                }
            }
        }
        if let Some(file) = result.nodes.first_mut() {
            file.extra
                .insert("compiler".into(), self.compiler.clone().into());
        }
    }
}
