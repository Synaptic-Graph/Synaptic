//! Fortran extractor — custom walker.
//!
//! `module`/`program`/`submodule` → container nodes; `subroutine`/`function` →
//! `.name()` / `name()`; `use` → `imports_from`; `call X` / function references
//! → `calls`.

#[cfg(feature = "lang-fortran")]
use std::collections::HashSet;

#[cfg(feature = "lang-fortran")]
use synaptic_core::{NodeId, make_id};
#[cfg(feature = "lang-fortran")]
use tree_sitter::{Node as TsNode, Parser};

#[cfg(feature = "lang-fortran")]
use crate::common::Builder;
#[cfg(feature = "lang-fortran")]
use crate::paths::file_node_id;
#[cfg(feature = "lang-fortran")]
use crate::result::ExtractionResult;

const MAX_DEPTH: usize = 2000;

/// Match the build's `-ffixed-line-length-N` (0 = unlimited). Invalid settings
/// retain the standard 72-column default, as do unset settings.
pub(crate) fn fixed_line_length() -> usize {
    std::env::var("SYNAPTIC_FORTRAN_FIXED_LINE_LENGTH")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|n| *n == 0 || *n >= 7)
        .unwrap_or(72)
}

/// Extract a Fortran source file already in memory.
#[cfg(feature = "lang-fortran")]
pub fn extract_fortran_source(path: &str, source: &[u8]) -> ExtractionResult {
    extract_fortran_source_with_line_length(path, source, fixed_line_length())
}

/// Extract with the compiler's fixed-form line length (0 means unlimited).
/// Free-form files ignore this option. The default entry point uses 72 columns.
#[cfg(feature = "lang-fortran")]
pub fn extract_fortran_source_with_line_length(
    path: &str,
    source: &[u8],
    line_length: usize,
) -> ExtractionResult {
    extract_fortran_source_with_form(path, source, line_length, None)
}

pub fn extract_fortran_source_with_form(
    path: &str,
    source: &[u8],
    line_length: usize,
    fixed: Option<bool>,
) -> ExtractionResult {
    let fixed = fixed.unwrap_or_else(|| {
        std::path::Path::new(path)
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext.to_ascii_lowercase().as_str(), "f" | "for"))
    });
    let normalized;
    let source = if fixed {
        normalized = normalize_fixed_form(source, line_length);
        normalized.as_slice()
    } else {
        source
    };
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_fortran::LANGUAGE.into())
        .expect("load tree-sitter-fortran");
    let Some(tree) = parser.parse(source, None) else {
        return ExtractionResult::default();
    };
    let file_nid = file_node_id(path);
    let filename = std::path::Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string());
    let mut ex = Fortran {
        src: source,
        b: Builder::new(path),
        file_nid: file_nid.clone(),
        function_bodies: Vec::new(),
    };
    ex.b.add_node(file_nid, filename, 1);
    ex.b.note_parse_health(tree.root_node());
    ex.walk(tree.root_node(), None, 0);
    ex.run_call_pass();
    ex.b.into_result()
}

/// Adapt fixed-form comments and column-six continuations to the free-form
/// grammar. Keep every source line so graph locations still refer to the input.
fn normalize_fixed_form(source: &[u8], line_length: usize) -> Vec<u8> {
    let mut lines: Vec<Vec<u8>> = source
        .split_inclusive(|b| *b == b'\n')
        .map(Vec::from)
        .collect();
    let mut previous = None;
    let mut quote = None;
    let mut hollerith = 0;
    for i in 0..lines.len() {
        let line = &mut lines[i];
        if matches!(line.first(), Some(b'c' | b'C' | b'*' | b'!')) {
            line[0] = b'!';
            continue;
        }
        if line
            .iter()
            .find(|b| !b.is_ascii_whitespace())
            .is_none_or(|b| matches!(*b, b'!' | b'#'))
        {
            continue;
        }
        for byte in line.iter_mut().skip(if line_length == 0 {
            usize::MAX
        } else {
            line_length
        }) {
            if !matches!(*byte, b'\n' | b'\r') {
                *byte = b' ';
            }
        }
        let continuation = line.len() > 6
            && line[..5].iter().all(|b| *b == b' ')
            && !matches!(line[5], b' ' | b'0' | b'\t' | b'\r' | b'\n');
        if continuation {
            line[5] = b'&';
            if let Some(prev) = previous {
                let prior: &mut Vec<u8> = &mut lines[prev];
                // Insert before an inline comment, outside quoted strings.
                let mut quote = None;
                let end = prior
                    .iter()
                    .position(|b| {
                        if matches!(*b, b'\'' | b'"') {
                            if quote == Some(*b) {
                                quote = None;
                            } else if quote.is_none() {
                                quote = Some(*b);
                            }
                        }
                        (*b == b'!' && quote.is_none()) || matches!(*b, b'\r' | b'\n')
                    })
                    .unwrap_or(prior.len());
                prior.insert(end, b'&');
            }
        } else if line.get(5) == Some(&b'0') {
            line[5] = b' ';
        }
        compact_fixed_exponents(&mut lines[i], &mut quote, &mut hollerith);
        previous = Some(i);
    }
    lines.concat()
}

/// Fixed form permits blanks inside numbers (`1. D - 3`). The free-form
/// grammar needs the exponent joined. Strings, Hollerith data and comments
/// retain their bytes; only columns after the continuation marker are scanned.
fn compact_fixed_exponents(line: &mut Vec<u8>, quote: &mut Option<u8>, hollerith: &mut usize) {
    let mut i = 6.min(line.len());
    while i < line.len() && !matches!(line[i], b'\r' | b'\n') {
        if *hollerith > 0 {
            *hollerith -= 1;
        } else if let Some(q) = *quote {
            if line[i] == q {
                *quote = None;
            }
        } else if matches!(line[i], b'\'' | b'"') {
            *quote = Some(line[i]);
        } else if line[i] == b'!' {
            break;
        } else if line[i].is_ascii_digit() && (i == 6 || !line[i - 1].is_ascii_alphanumeric()) {
            let end = i + line[i..].iter().take_while(|b| b.is_ascii_digit()).count();
            if matches!(line.get(end), Some(b'h' | b'H')) {
                *hollerith = std::str::from_utf8(&line[i..end])
                    .ok()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                i = end;
            }
        } else if line[i] == b'.'
            && ((i > 6 && line[i - 1].is_ascii_digit())
                || line.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let start = i + 1;
            let mut end = start;
            while line
                .get(end)
                .is_some_and(|b| b.is_ascii_digit() || *b == b' ')
            {
                end += 1;
            }
            if matches!(line.get(end), Some(b'd' | b'D' | b'e' | b'E' | b'q' | b'Q')) {
                end += 1;
                while line.get(end) == Some(&b' ') {
                    end += 1;
                }
                if matches!(line.get(end), Some(b'+' | b'-')) {
                    end += 1;
                }
                while line.get(end) == Some(&b' ') {
                    end += 1;
                }
                let digits = end;
                while line.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
                if end > digits {
                    let compact: Vec<_> = line[start..end]
                        .iter()
                        .copied()
                        .filter(|b| *b != b' ')
                        .collect();
                    line.splice(start..end, compact);
                }
            }
        }
        i += 1;
    }
}

/// Read and extract a Fortran file from disk.
#[cfg(feature = "lang-fortran")]
pub fn extract_fortran_file(path: &std::path::Path) -> std::io::Result<ExtractionResult> {
    let source = std::fs::read(path)?;
    let path_str = path.to_string_lossy();
    Ok(extract_fortran_source(&path_str, &source))
}

#[cfg(feature = "lang-fortran")]
struct Fortran<'a, 'tree> {
    src: &'a [u8],
    b: Builder,
    file_nid: NodeId,
    function_bodies: Vec<(NodeId, TsNode<'tree>)>,
}

#[cfg(feature = "lang-fortran")]
impl<'tree> Fortran<'_, 'tree> {
    fn text(&self, n: TsNode<'tree>) -> String {
        n.utf8_text(self.src).unwrap_or("").to_string()
    }

    fn line(n: TsNode<'tree>) -> usize {
        n.start_position().row + 1
    }

    fn children(n: TsNode<'tree>) -> Vec<TsNode<'tree>> {
        let mut c = n.walk();
        n.children(&mut c).collect()
    }

    /// The `name` of a container/procedure (its leading `*_statement`'s `name`,
    /// which is a field on `subroutine_statement` but positional on
    /// `module_statement`).
    fn decl_name(&self, node: TsNode<'tree>, stmt_kind: &str) -> Option<String> {
        let stmt = Self::children(node)
            .into_iter()
            .find(|c| c.kind() == stmt_kind)?;
        let name = stmt.child_by_field_name("name").or_else(|| {
            Self::children(stmt)
                .into_iter()
                .find(|c| matches!(c.kind(), "name" | "type_name"))
        })?;
        Some(self.text(name))
    }

    fn walk(&mut self, node: TsNode<'tree>, scope: Option<NodeId>, depth: usize) {
        if depth >= MAX_DEPTH {
            return;
        }
        match node.kind() {
            "module" | "program" | "submodule" | "interface" | "derived_type_definition" => {
                let stmt = match node.kind() {
                    "module" => "module_statement",
                    "program" => "program_statement",
                    "interface" => "interface_statement",
                    "derived_type_definition" => "derived_type_statement",
                    _ => "submodule_statement",
                };
                if let Some(name) = self.decl_name(node, stmt).filter(|n| !n.is_empty()) {
                    let line = Self::line(node);
                    let owner = scope.as_ref().unwrap_or(&self.file_nid);
                    let nid = NodeId(make_id(&[owner.as_str(), &name]));
                    self.b.add_node(nid.clone(), name, line);
                    self.b
                        .nodes
                        .last_mut()
                        .unwrap()
                        .extra
                        .insert("fortran_scope".into(), node.kind().into());
                    if let Some(statement) =
                        Self::children(node).into_iter().find(|n| n.kind() == stmt)
                    {
                        for (field, key) in [
                            ("ancestor", "fortran_ancestor"),
                            ("parent", "fortran_parent"),
                            ("base", "fortran_base"),
                        ] {
                            if let Some(value) = statement.child_by_field_name(field) {
                                let text = self
                                    .text(value)
                                    .to_ascii_lowercase()
                                    .replace(char::is_whitespace, "");
                                let text = text
                                    .strip_prefix("extends(")
                                    .and_then(|s| s.strip_suffix(')'))
                                    .unwrap_or(&text);
                                self.b
                                    .nodes
                                    .last_mut()
                                    .unwrap()
                                    .extra
                                    .insert(key.into(), text.trim().into());
                            }
                        }
                    }
                    if node.kind() == "module" {
                        let mut access = serde_json::Map::new();
                        let mut public = true;
                        for statement in Self::children(node).into_iter().filter(|n| {
                            matches!(n.kind(), "public_statement" | "private_statement")
                        }) {
                            let visible = statement.kind() == "public_statement";
                            let names: Vec<_> = Self::children(statement)
                                .into_iter()
                                .filter(|n| n.kind() == "identifier")
                                .collect();
                            if names.is_empty() {
                                public = visible;
                            }
                            for name in names {
                                access.insert(self.text(name).to_ascii_lowercase(), visible.into());
                            }
                        }
                        self.b.nodes.last_mut().unwrap().extra.insert(
                            "fortran_access".into(),
                            serde_json::json!({"default": public, "names": access}),
                        );
                    }
                    self.b
                        .add_edge(owner.clone(), nid.clone(), "contains", line, None);
                    if node.kind() == "interface" {
                        for statement in Self::children(node)
                            .into_iter()
                            .filter(|n| n.kind() == "procedure_statement")
                        {
                            for name in Self::children(statement)
                                .into_iter()
                                .filter(|n| n.kind() == "method_name")
                            {
                                let target = NodeId(make_id(&[owner.as_str(), &self.text(name)]));
                                self.b.add_edge(
                                    nid.clone(),
                                    target,
                                    "references",
                                    Self::line(name),
                                    Some("generic_procedure"),
                                );
                            }
                        }
                    }
                    for c in Self::children(node) {
                        self.walk(c, Some(nid.clone()), depth + 1);
                    }
                    if node.kind() == "program" {
                        self.function_bodies.push((nid, node));
                    }
                } else {
                    let scope = if node.kind() == "program" {
                        Some(self.file_nid.clone())
                    } else {
                        scope
                    };
                    for c in Self::children(node) {
                        self.walk(c, scope.clone(), depth + 1);
                    }
                    if node.kind() == "program" {
                        self.function_bodies.push((self.file_nid.clone(), node));
                    }
                }
            }
            "subroutine" | "function" | "module_procedure" => {
                let stmt = if node.kind() == "subroutine" {
                    "subroutine_statement"
                } else if node.kind() == "function" {
                    "function_statement"
                } else {
                    "module_procedure_statement"
                };
                if let Some(name) = self.decl_name(node, stmt).filter(|n| !n.is_empty()) {
                    let line = Self::line(node);
                    let nid = if let Some(m) = &scope {
                        let f = NodeId(make_id(&[m.as_str(), &name]));
                        self.b.add_node(f.clone(), format!(".{name}()"), line);
                        self.b.add_edge(m.clone(), f.clone(), "method", line, None);
                        f
                    } else {
                        let f = NodeId(make_id(&[self.file_nid.as_str(), &name]));
                        self.b.add_node(f.clone(), format!("{name}()"), line);
                        self.b
                            .add_edge(self.file_nid.clone(), f.clone(), "contains", line, None);
                        f
                    };
                    if let Some(procedure) = self.b.nodes.iter_mut().find(|n| n.id == nid) {
                        procedure
                            .extra
                            .insert("fortran_scope".into(), node.kind().into());
                    }
                    for child in Self::children(node) {
                        self.walk(child, Some(nid.clone()), depth + 1);
                    }
                    if let Some(statement) =
                        Self::children(node).into_iter().find(|n| n.kind() == stmt)
                    {
                        let parameters: Vec<_> = Self::children(statement)
                            .into_iter()
                            .find(|n| n.kind() == "parameters")
                            .map(|n| {
                                Self::children(n)
                                    .into_iter()
                                    .filter(|n| n.kind() == "identifier")
                                    .map(|n| self.text(n).to_ascii_lowercase())
                                    .collect()
                            })
                            .unwrap_or_default();
                        let mut parent = node.parent();
                        let mut declaration = false;
                        while let Some(p) = parent {
                            if p.kind() == "interface" {
                                declaration = true;
                                break;
                            }
                            if matches!(
                                p.kind(),
                                "module" | "submodule" | "program" | "subroutine" | "function"
                            ) {
                                break;
                            }
                            parent = p.parent();
                        }
                        let procedure = self.b.nodes.iter_mut().find(|n| n.id == nid).unwrap();
                        procedure
                            .extra
                            .insert("fortran_parameters".into(), serde_json::json!(parameters));
                        if declaration {
                            procedure
                                .extra
                                .insert("fortran_declaration".into(), true.into());
                            let module_procedure = statement
                                .utf8_text(self.src)
                                .unwrap_or("")
                                .split_ascii_whitespace()
                                .take_while(|word| {
                                    !word.eq_ignore_ascii_case("subroutine")
                                        && !word.eq_ignore_ascii_case("function")
                                })
                                .any(|word| word.eq_ignore_ascii_case("module"));
                            procedure
                                .extra
                                .insert("fortran_module_procedure".into(), module_procedure.into());
                        }
                    }
                    self.function_bodies.push((nid, node));
                }
            }
            "variable_modification" | "variable_declaration" => {
                if let Some(ty) = node.child_by_field_name("type") {
                    let ty = self
                        .text(ty)
                        .to_ascii_lowercase()
                        .replace(char::is_whitespace, "");
                    let qualifiers: Vec<_> = Self::children(node)
                        .into_iter()
                        .filter(|n| n.kind() == "type_qualifier")
                        .map(|n| {
                            self.text(n)
                                .to_ascii_lowercase()
                                .replace(char::is_whitespace, "")
                        })
                        .collect();
                    let optional = qualifiers.iter().any(|q| q == "optional");
                    let dimension = qualifiers
                        .iter()
                        .find(|q| q.starts_with("dimension("))
                        .map(|q| q.matches(',').count() + 1)
                        .unwrap_or(0);
                    let mut cursor = node.walk();
                    let mut variables = Vec::new();
                    for declarator in node.children_by_field_name("declarator", &mut cursor) {
                        let decl = declarator.child_by_field_name("left").unwrap_or(declarator);
                        let name = if decl.kind() == "identifier" {
                            Some(decl)
                        } else {
                            Self::children(decl)
                                .into_iter()
                                .find(|n| n.kind() == "identifier")
                        };
                        if let Some(name) = name {
                            let rank = Self::children(decl)
                                .into_iter()
                                .find(|n| n.kind() == "size")
                                .map(|n| self.text(n).matches(',').count() + 1)
                                .unwrap_or(dimension);
                            variables.push((
                                self.text(name).to_ascii_lowercase(),
                                serde_json::json!({"type":ty,"rank":rank,"optional":optional}),
                            ));
                        }
                    }
                    let owner = scope.as_ref().unwrap_or(&self.file_nid);
                    if let Some(owner) = self.b.nodes.iter_mut().find(|n| &n.id == owner) {
                        let types = owner
                            .extra
                            .entry("fortran_variables")
                            .or_insert_with(|| serde_json::json!({}))
                            .as_object_mut()
                            .unwrap();
                        types.extend(variables);
                    }
                }
                let binding = Self::children(node)
                    .into_iter()
                    .filter(|n| n.kind() == "type_qualifier")
                    .map(|n| self.text(n).to_ascii_lowercase())
                    .find(|name| matches!(name.as_str(), "intrinsic" | "external"));
                if let Some(binding) = binding {
                    let mut cursor = node.walk();
                    let names: Vec<_> = node
                        .children_by_field_name("declarator", &mut cursor)
                        .filter(|n| n.kind() == "identifier")
                        .map(|n| self.text(n).to_ascii_lowercase())
                        .collect();
                    let owner = scope.as_ref().unwrap_or(&self.file_nid);
                    if let Some(owner) = self.b.nodes.iter_mut().find(|n| &n.id == owner) {
                        let bindings = owner
                            .extra
                            .entry("fortran_bindings")
                            .or_insert_with(|| serde_json::json!({}))
                            .as_object_mut()
                            .unwrap();
                        for name in names {
                            bindings.insert(name, binding.clone().into());
                        }
                    }
                }
            }
            "procedure_statement" | "generic_statement"
                if scope.as_ref().is_some_and(|scope| {
                    self.b.nodes.iter().any(|n| {
                        &n.id == scope
                            && n.extra.get("fortran_scope").and_then(|s| s.as_str())
                                == Some("derived_type_definition")
                    })
                }) =>
            {
                let text = self
                    .text(node)
                    .to_ascii_lowercase()
                    .replace(char::is_whitespace, "");
                if let Some((attributes, bindings)) = text.split_once("::") {
                    let mut methods = Vec::new();
                    if node.kind() == "generic_statement" {
                        if let Some((name, targets)) = bindings.split_once("=>") {
                            methods.push((name.to_string(), serde_json::json!({"generic":targets.split(',').collect::<Vec<_>>()})));
                        }
                    } else {
                        for binding in bindings.split(',') {
                            let (name, target) =
                                binding.split_once("=>").unwrap_or((binding, binding));
                            let pass = attributes
                                .split_once("pass(")
                                .and_then(|(_, s)| s.split_once(')'))
                                .map(|(s, _)| s);
                            methods.push((name.to_string(), serde_json::json!({"target":target,"nopass":attributes.contains("nopass"),"pass":pass,"deferred":attributes.contains("deferred")})));
                        }
                    }
                    if let Some(owner) = self
                        .b
                        .nodes
                        .iter_mut()
                        .find(|n| Some(&n.id) == scope.as_ref())
                    {
                        owner
                            .extra
                            .entry("fortran_methods")
                            .or_insert_with(|| serde_json::json!({}))
                            .as_object_mut()
                            .unwrap()
                            .extend(methods);
                    }
                }
            }
            "use_statement" => {
                if let Some(name) = Self::children(node)
                    .into_iter()
                    .find(|c| matches!(c.kind(), "name" | "identifier" | "module_name"))
                    .map(|c| self.text(c))
                    .filter(|n| !n.is_empty())
                {
                    let tgt = NodeId(make_id(&["fortran", "mod", &name.to_ascii_lowercase()]));
                    self.b.add_external_node(tgt.clone(), name);
                    self.b.add_edge(
                        scope.unwrap_or_else(|| self.file_nid.clone()),
                        tgt,
                        "imports_from",
                        Self::line(node),
                        Some("import"),
                    );
                    let included = Self::children(node)
                        .into_iter()
                        .find(|n| n.kind() == "included_items");
                    let mut names = serde_json::Map::new();
                    for item in Self::children(included.unwrap_or(node)) {
                        if item.kind() == "use_alias" {
                            let children = Self::children(item);
                            if let (Some(local), Some(remote)) = (
                                children.iter().find(|n| n.kind() == "local_name"),
                                children.iter().find(|n| n.kind() == "identifier"),
                            ) {
                                names.insert(
                                    self.text(*local).to_ascii_lowercase(),
                                    self.text(*remote).to_ascii_lowercase().into(),
                                );
                            }
                        } else if included.is_some() && item.kind() == "identifier" {
                            let name = self.text(item).to_ascii_lowercase();
                            names.insert(name.clone(), name.into());
                        }
                    }
                    let intrinsic = Self::children(node)
                        .iter()
                        .any(|n| self.text(*n).eq_ignore_ascii_case("intrinsic"));
                    self.b.edges.last_mut().unwrap().extra.insert("fortran_use".into(), serde_json::json!({"only": included.is_some(), "names": names, "intrinsic": intrinsic}));
                }
            }
            _ => {
                for c in Self::children(node) {
                    self.walk(c, scope.clone(), depth + 1);
                }
            }
        }
    }

    fn run_call_pass(&mut self) {
        let parents: std::collections::HashMap<_, _> = self
            .b
            .edges
            .iter()
            .filter(|e| matches!(e.relation.as_str(), "method" | "contains"))
            .map(|e| (e.target.clone(), e.source.clone()))
            .collect();
        let bodies = std::mem::take(&mut self.function_bodies);
        let mut seen: HashSet<(NodeId, NodeId)> = HashSet::new();
        for (caller, body) in bodies {
            // Nearest lexical scope wins. USE association needs the completed
            // repository graph; defer at that scope instead of guessing a host.
            let mut index = std::collections::HashMap::new();
            let mut scope = Some(&caller);
            while let Some(owner) = scope {
                if self
                    .b
                    .nodes
                    .iter()
                    .any(|n| &n.id == owner && n.extra.contains_key("fortran_bindings"))
                {
                    break; // Explicit INTRINSIC/EXTERNAL association needs scope-aware lookup.
                }
                for node in &self.b.nodes {
                    if owner == &self.file_nid
                        && synaptic_core::fortran::INTRINSICS.contains(
                            &node
                                .label
                                .trim_end_matches("()")
                                .to_ascii_lowercase()
                                .as_str(),
                        )
                    {
                        continue; // An external definition is not an intrinsic override without EXTERNAL.
                    }
                    if parents.get(&node.id) == Some(owner) && (node.label.ends_with("()")) {
                        index
                            .entry(
                                node.label
                                    .trim_start_matches('.')
                                    .trim_end_matches("()")
                                    .to_ascii_lowercase(),
                            )
                            .or_insert_with(|| node.id.clone());
                    }
                }
                if self
                    .b
                    .edges
                    .iter()
                    .any(|e| &e.source == owner && e.extra.contains_key("fortran_use"))
                {
                    break;
                }
                scope = parents.get(owner);
            }
            for child in Self::children(body) {
                self.walk_calls(child, &caller, &index, &mut seen, 0);
            }
        }
    }

    fn walk_calls(
        &mut self,
        node: TsNode<'tree>,
        caller: &NodeId,
        index: &std::collections::HashMap<String, NodeId>,
        seen: &mut HashSet<(NodeId, NodeId)>,
        depth: usize,
    ) {
        if depth >= MAX_DEPTH {
            return;
        }
        if matches!(node.kind(), "subroutine" | "function" | "module_procedure") {
            return;
        }
        let callee = match node.kind() {
            "subroutine_call" => node.child_by_field_name("subroutine").map(|s| self.text(s)),
            "call_expression" => Self::children(node)
                .into_iter()
                .find(|c| matches!(c.kind(), "identifier" | "derived_type_member_expression"))
                .map(|c| self.text(c)),
            _ => None,
        };
        if let Some(callee) = callee
            && !callee.is_empty()
        {
            if let Some(arguments) = node.child_by_field_name("arguments").or_else(|| {
                Self::children(node)
                    .into_iter()
                    .find(|n| n.kind() == "argument_list")
            }) {
                let actuals: Vec<_> = Self::children(arguments)
                    .into_iter()
                    .filter(|n| n.is_named())
                    .map(|n| self.text(n).to_ascii_lowercase())
                    .collect();
                let key = format!(
                    "{}:{}",
                    Self::line(node),
                    callee.to_ascii_lowercase().replace(char::is_whitespace, "")
                );
                if let Some(owner) = self.b.nodes.iter_mut().find(|n| &n.id == caller) {
                    owner
                        .extra
                        .entry("fortran_actuals")
                        .or_insert_with(|| serde_json::json!({}))
                        .as_object_mut()
                        .unwrap()
                        .insert(key, serde_json::json!(actuals));
                }
            }
            let folded = callee.to_ascii_lowercase();
            let lookup = if index.contains_key(&folded) {
                &folded
            } else {
                &callee
            };
            self.b
                .resolve_call(caller, lookup, false, Self::line(node), index, seen, true);
        }
        for c in Self::children(node) {
            self.walk_calls(c, caller, index, seen, depth + 1);
        }
    }
}

#[cfg(all(test, feature = "lang-fortran"))]
mod tests {
    use super::extract_fortran_source;
    use crate::result::ExtractionResult;

    #[test]
    fn fixed_form_spaced_exponents_preserve_strings_hollerith_comments_and_lines() {
        let source = b"      SUBROUTINE WORK\n      REAL*8 X\n      DATA X/-1. D0/\n      X=.5 D - 3\n      PRINT *, '1. D0' ! 1. D0\n  100 FORMAT(5H1. D0)\n      END\n";
        let normalized = String::from_utf8(super::normalize_fixed_form(source, 72)).unwrap();
        assert!(normalized.contains("DATA X/-1.D0/"));
        assert!(normalized.contains("X=.5D-3"));
        assert!(normalized.contains("'1. D0' ! 1. D0"));
        assert!(normalized.contains("5H1. D0"));
        assert_eq!(
            normalized.lines().count(),
            source.split(|b| *b == b'\n').count() - 1
        );
        assert!(!extract_fortran_source("work.f", source).parse_error);
    }

    fn extract() -> ExtractionResult {
        extract_fortran_source(
            "src/m.f90",
            b"module m\ncontains\nsubroutine bark(x)\n  call sound(x)\nend subroutine\nsubroutine sound(x)\nend subroutine\nend module\n",
        )
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
    fn module_and_procedure_nodes() {
        let ls = labels(&extract());
        assert!(ls.contains(&"m".to_string()), "{ls:?}");
        assert!(ls.contains(&".bark()".to_string()));
        assert!(ls.contains(&".sound()".to_string()));
    }

    #[test]
    fn unnamed_main_program_calls_are_attributed_to_the_file() {
        let r = extract_fortran_source(
            "driver.f90",
            b"implicit none\ncall work()\nend\nsubroutine work()\nend subroutine\n",
        );
        assert!(!r.parse_error);
        assert!(rels(&r, "calls").contains(&("driver.f90".into(), "work()".into())));
    }

    #[test]
    fn call_resolves() {
        // bark calls sound
        assert!(
            rels(&extract(), "calls").contains(&(".bark()".to_string(), ".sound()".to_string())),
            "{:?}",
            rels(&extract(), "calls")
        );
    }

    #[test]
    fn fixed_form_ignores_documentation_and_keeps_continued_calls() {
        let source = b"*     SUBROUTINE SOLVE(A)\r\nC     CALL FAKE(A)\r\n      SUBROUTINE SOLVE(A)\r\n      CALL Work(A, ! argument list\r\n*     intervening comment\r\n     $          A)\r\n      END\r\n      SUBROUTINE WORK(A, B)\r\n      END\r\n";
        let r = extract_fortran_source("solve.F", source);
        assert!(!r.parse_error, "{r:?}");
        let solve = r.nodes.iter().find(|n| n.label == "SOLVE()").unwrap();
        assert_eq!(solve.source_location.as_deref(), Some("L3"));
        assert!(rels(&r, "calls").contains(&("SOLVE()".into(), "WORK()".into())));
        assert!(!labels(&r).iter().any(|n| n.contains("FAKE")));
        assert_eq!(
            super::normalize_fixed_form(source, 72)
                .iter()
                .filter(|b| **b == b'\n')
                .count(),
            9
        );
    }

    #[test]
    fn internal_procedures_and_assignment_calls_keep_their_owners() {
        let r = extract_fortran_source("m.f90", b"subroutine outer()\ninteger x\nx = VALUE()\ncall inner()\ncontains\ninteger function value()\nvalue = 1\nend function\nsubroutine inner()\ncall helper()\nend subroutine\nend subroutine\nsubroutine helper()\nend subroutine\n");
        assert!(!r.parse_error);
        let calls = rels(&r, "calls");
        assert!(
            calls.contains(&("outer()".into(), ".value()".into())),
            "{calls:?}"
        );
        assert!(calls.contains(&("outer()".into(), ".inner()".into())));
        assert!(calls.contains(&(".inner()".into(), "helper()".into())));
        assert!(!calls.contains(&("outer()".into(), "helper()".into())));
    }

    #[test]
    fn interfaces_and_extended_fixed_form_preserve_structure() {
        let r = extract_fortran_source("m.f90", b"module m\ninterface generic\nmodule procedure work\nend interface\ncontains\nsubroutine work()\nend subroutine\nend module\n");
        assert!(!r.parse_error);
        assert!(labels(&r).contains(&"generic".into()));
        assert!(rels(&r, "references").contains(&("generic".into(), ".work()".into())));
        let source = format!(
            "      SUBROUTINE WORK()\n      CALL {}HELPER()\n      END\n",
            " ".repeat(66)
        );
        let standard = extract_fortran_source("work.f", source.as_bytes());
        assert!(!standard.raw_calls.iter().any(|c| c.callee == "HELPER"));
        for width in [132, 0] {
            let extended =
                super::extract_fortran_source_with_line_length("work.f", source.as_bytes(), width);
            assert!(!extended.parse_error, "{extended:?}");
            assert!(
                extended
                    .raw_calls
                    .iter()
                    .any(|c| c.callee == "HELPER" && c.source_location.as_deref() == Some("L2"))
            );
        }
    }

    #[cfg(feature = "lang-c")]
    #[test]
    fn fortran_and_c_translations_do_not_share_symbol_ids() {
        let f = extract_fortran_source("ssqfcn.f", b"      SUBROUTINE SSQFCN()\n      END\n");
        let c = crate::extract_source("ssqfcn.c", b"void ssqfcn() {}\n").unwrap();
        let f = f.nodes.iter().find(|n| n.label == "SSQFCN()").unwrap();
        let c = c.nodes.iter().find(|n| n.label == "ssqfcn()").unwrap();
        assert_ne!(f.id, c.id);
    }
}
