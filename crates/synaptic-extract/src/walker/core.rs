//! `core` extraction methods on `Extractor` (split from walker.rs).

use super::Extractor;
use super::{
    COMMENT_TOKENS, MAX_DEPTH, RATIONALE_MARKERS, c_family_function_id_part, first_docstring,
    is_macro_identifier,
};
use crate::config::{HeritageStyle, ImportStyle, TypeRefStyle};
use crate::paths::file_node_id;
use crate::result::RawCall;
use serde_json::Map;
use std::collections::{HashMap, HashSet};
use synaptic_core::{Confidence, Edge, FileType, Node, NodeId, make_id};
use tree_sitter::Node as TsNode;

/// Groovy quoted names use string escapes; graph labels identify the decoded name.
fn decode_quoted_identifier(text: &str) -> String {
    let mut chars = text.chars().peekable();
    let mut decoded = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }
        let Some(escape) = chars.next() else {
            decoded.push('\\');
            break;
        };
        let value = match escape {
            'b' => '\u{8}',
            't' => '\t',
            'n' => '\n',
            'f' => '\u{c}',
            'r' => '\r',
            's' => ' ',
            '\n' => continue,
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                continue;
            }
            'u' | '0'..='7' => {
                let mut digits = String::new();
                let (radix, count) = if escape == 'u' {
                    (16, 4)
                } else {
                    digits.push(escape);
                    (8, if escape <= '3' { 2 } else { 1 })
                };
                for _ in 0..count {
                    if chars.peek().is_some_and(|c| c.is_digit(radix)) {
                        digits.push(chars.next().unwrap());
                    } else {
                        break;
                    }
                }
                if let Some(value) = u32::from_str_radix(&digits, radix)
                    .ok()
                    .and_then(char::from_u32)
                {
                    decoded.push(value);
                } else {
                    decoded.push('\\');
                    if escape == 'u' {
                        decoded.push('u');
                    }
                    decoded.push_str(&digits);
                }
                continue;
            }
            other => other,
        };
        decoded.push(value);
    }
    decoded
}

impl<'tree> Extractor<'_, '_, 'tree> {
    pub(crate) fn text(&self, node: TsNode<'tree>) -> String {
        let text = node.utf8_text(self.source).unwrap_or("");
        if node.kind() == "quoted_identifier" && text.starts_with(['\'', '"']) && text.len() >= 2 {
            decode_quoted_identifier(&text[1..text.len() - 1])
        } else {
            text.to_string()
        }
    }

    pub(crate) fn line(node: TsNode<'tree>) -> usize {
        node.start_position().row + 1
    }

    /// The node's full source range (1-based lines and columns).
    pub(crate) fn span(node: TsNode<'tree>) -> synaptic_core::Span {
        let s = node.start_position();
        let e = node.end_position();
        synaptic_core::Span {
            start_line: s.row as u32 + 1,
            start_col: s.column as u32 + 1,
            end_line: e.row as u32 + 1,
            end_col: e.column as u32 + 1,
        }
    }

    fn declaration_span(&self, node: TsNode<'tree>) -> synaptic_core::Span {
        let mut span = Self::span(node);
        // Error recovery can swallow preceding fields into a method (Groovy's
        // optional semicolons). The explicit name remains a reliable anchor.
        if node.has_error()
            && self.cfg.function_types.contains(&node.kind())
            && let Some(name) = self.function_name_node(node)
        {
            span.start_line = name.start_position().row as u32 + 1;
            span.start_col = name.start_position().column as u32 + 1;
        }
        span
    }

    pub(crate) fn add_node(&mut self, id: NodeId, label: String, line: usize) {
        if self.seen.insert(id.clone()) {
            self.nodes.push(Node {
                id,
                label,
                file_type: FileType::Code,
                source_file: self.path.clone().into(),
                source_location: Some(format!("L{line}")),
                community: None,
                repo: None,
                extra: Map::new(),
                origin: Some("ast".into()),
                ..Default::default()
            });
        }
    }

    /// Add a located code node enriched with kind, optional visibility, and the
    /// full source span (derived from `node`). Deduped by id like [`add_node`].
    pub(crate) fn add_code_node(
        &mut self,
        id: NodeId,
        label: String,
        node: TsNode<'tree>,
        kind: synaptic_core::NodeKind,
        visibility: Option<synaptic_core::Visibility>,
        signature: Option<synaptic_core::Signature>,
    ) {
        let span = self.declaration_span(node);
        if self.seen.insert(id.clone()) {
            let mut n = Node {
                id,
                label,
                file_type: FileType::Code,
                source_file: self.path.clone().into(),
                source_location: Some(format!("L{}", span.start_line)),
                community: None,
                repo: None,
                extra: Map::new(),
                origin: Some("ast".into()),
                ..Default::default()
            };
            n.set_kind(kind);
            n.set_span(span);
            if let Some(v) = visibility {
                n.set_visibility(v);
            }
            if let Some(s) = signature {
                n.set_signature(s);
            }
            self.nodes.push(n);
        } else if let Some(n) = self.nodes.iter_mut().find(|n| n.id == id) {
            // Enrich a plain stub created earlier (e.g. a name referenced before its
            // declaration), without overwriting an already-enriched node.
            if n.kind().is_none() {
                n.set_kind(kind);
                n.set_span(span);
                n.source_location = Some(format!("L{}", span.start_line));
                if let Some(v) = visibility {
                    n.set_visibility(v);
                }
                if let Some(s) = signature {
                    n.set_signature(s);
                }
            }
        }
    }

    /// Map a class-family grammar node kind to a [`NodeKind`].
    pub(crate) fn class_kind(ts_kind: &str) -> synaptic_core::NodeKind {
        use synaptic_core::NodeKind::*;
        let k = ts_kind.to_ascii_lowercase();
        if k.contains("type_alias") {
            TypeAlias
        } else if k.contains("annotation") || k.contains("interface") {
            Interface
        } else if k.contains("trait") {
            Trait
        } else if k.contains("enum") {
            Enum
        } else if k.contains("struct") || k.contains("record") || k.contains("union") {
            Struct
        } else if k.contains("protocol") {
            Protocol
        } else if k.contains("object") {
            Object
        } else {
            Class
        }
    }

    /// Best-effort declared visibility from a declaration node: scans an immediate
    /// `modifiers`/`modifier`/`visibility` child (Java/C#/Kotlin/Swift/TS/Rust) or a
    /// bare `public`/`private`/`protected`/`internal` keyword child. None = unknown.
    pub(crate) fn visibility_of(&self, node: TsNode<'tree>) -> Option<synaptic_core::Visibility> {
        use synaptic_core::Visibility::*;
        let kw = |w: &str| match w {
            "public" => Some(Public),
            "protected" => Some(Protected),
            "private" => Some(Private),
            "internal" => Some(Internal),
            _ => None,
        };
        let mut cur = node.walk();
        for child in node.children(&mut cur) {
            let k = child.kind();
            // A bare keyword child is unambiguous.
            if let Some(v) = kw(k) {
                return Some(v);
            }
            if k == "modifiers"
                || k == "modifier"
                || k == "visibility_modifier"
                || k == "visibility"
            {
                // Tokenize so an annotation whose NAME contains a keyword substring
                // (e.g. `@PublicApi private`) can't masquerade as a modifier: skip
                // any `@...` token and match keywords as whole words, in order.
                for tok in self.text(child).split_whitespace() {
                    if tok.starts_with('@') {
                        continue;
                    }
                    if let Some(v) = kw(&tok.to_ascii_lowercase()) {
                        return Some(v);
                    }
                }
            }
        }
        None
    }

    /// Python has no AST modifiers: a leading underscore is the private convention.
    pub(crate) fn python_visibility(name: &str) -> Option<synaptic_core::Visibility> {
        name.starts_with('_')
            .then_some(synaptic_core::Visibility::Private)
    }

    /// Visibility for a declaration named `name`: the Python underscore convention
    /// for Python configs, else the AST-modifier scan.
    fn decl_visibility(
        &self,
        node: TsNode<'tree>,
        name: &str,
    ) -> Option<synaptic_core::Visibility> {
        if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Python)) {
            Self::python_visibility(name)
        } else {
            self.visibility_of(node)
        }
    }

    /// Append a `FileType::Rationale` node (deduped by id) + a `rationale_for`
    /// edge to `target`. The label is collapsed to one line and capped at 80 chars.
    pub(crate) fn add_rationale(&mut self, label: String, line: usize, target: NodeId, stem: &str) {
        let rid = NodeId(make_id(&[stem, "rationale", &line.to_string()]));
        if self.seen.insert(rid.clone()) {
            let label: String = label
                .chars()
                .take(80)
                .collect::<String>()
                .replace(['\r', '\n'], " ")
                .trim()
                .to_string();
            self.nodes.push(Node {
                id: rid.clone(),
                label,
                file_type: FileType::Rationale,
                source_file: self.path.clone().into(),
                source_location: Some(format!("L{line}")),
                community: None,
                repo: None,
                extra: Map::new(),
                origin: Some("ast".into()),
                ..Default::default()
            });
        }
        self.add_edge(rid, target, "rationale_for", line, None);
    }

    /// Line scan for rationale comment markers (`# NOTE:`, `// HACK:`, `-- TODO:`,
    /// …), each linked to the file node. Language-agnostic comment pass.
    pub(crate) fn scan_rationale_comments(&mut self, file_nid: &NodeId, stem: &str) {
        // Collect first (borrowing the source via `Cow`, no full-file clone), then
        // mutate. The block scopes the borrow so `add_rationale` can take `&mut self`.
        let hits: Vec<(usize, String)> = {
            let text = String::from_utf8_lossy(self.source);
            text.lines()
                .enumerate()
                .filter_map(|(i, raw)| {
                    let s = raw.trim_start();
                    let tok = COMMENT_TOKENS.iter().find(|t| s.starts_with(**t))?;
                    let rest = s[tok.len()..].trim_start();
                    let is_marker = RATIONALE_MARKERS
                        .iter()
                        .any(|m| rest.strip_prefix(m).is_some_and(|r| r.starts_with(':')));
                    is_marker.then(|| (i + 1, s.to_string()))
                })
                .collect()
        };
        for (line, label) in hits {
            self.add_rationale(label, line, file_nid.clone(), stem);
        }
    }

    pub(crate) fn add_external_node(&mut self, id: NodeId, label: String) {
        if self.seen.insert(id.clone()) {
            self.nodes.push(Node {
                id,
                label,
                file_type: FileType::Code,
                source_file: String::new().into(),
                source_location: None,
                community: None,
                repo: None,
                extra: Map::new(),
                origin: Some("ast".into()),
                ..Default::default()
            });
        }
    }

    pub(crate) fn add_edge(
        &mut self,
        source: NodeId,
        target: NodeId,
        relation: &str,
        line: usize,
        context: Option<&str>,
    ) {
        self.edges.push(Edge {
            source,
            target,
            relation: relation.to_string().into(),
            confidence: Confidence::Extracted,
            source_file: self.path.clone().into(),
            source_location: Some(format!("L{line}")),
            confidence_score: None,
            weight: 1.0,
            context: context.map(str::to_string),
            cross_repo: false,
            extra: Map::new(),
        });
    }

    pub(crate) fn field(&self, node: TsNode<'tree>, name: &str) -> Option<TsNode<'tree>> {
        node.child_by_field_name(name)
    }

    pub(crate) fn children(node: TsNode<'tree>) -> Vec<TsNode<'tree>> {
        let mut cur = node.walk();
        node.children(&mut cur).collect()
    }

    /// The function name node: the named `name_field` if present, else (C/C++,
    /// where the name is buried in a declarator chain) the identifier reached by
    /// unwrapping `function_definition.declarator` through pointer/reference/
    /// function declarators. No-op for grammars that expose a `name` field.
    pub(crate) fn function_name_node(&self, node: TsNode<'tree>) -> Option<TsNode<'tree>> {
        if let Some(n) = self.field(node, self.cfg.name_field) {
            return Some(n);
        }
        if let Some(declarator) = self.bound_function_declarator(node) {
            return self.field(declarator, "name");
        }
        if self.cfg.type_ref_style == Some(TypeRefStyle::Cpp)
            && let Some(name) = self
                .c_function_declarator(node)
                .and_then(|fd| fd.child_by_field_name("declarator"))
                .filter(|name| name.kind() == "function_declarator")
        {
            return Some(name); // NAME(symbol)(parameters): preserve the source name expression.
        }
        Self::declarator_name(node.child_by_field_name("declarator")?, 0)
    }

    /// The variable declarator naming an anonymous JS/TS function expression.
    pub(crate) fn bound_function_declarator(&self, node: TsNode<'tree>) -> Option<TsNode<'tree>> {
        if !matches!(node.kind(), "arrow_function" | "function_expression")
            || self.field(node, self.cfg.name_field).is_some()
        {
            return None;
        }
        let parent = node.parent().filter(|parent| {
            parent.kind() == "variable_declarator"
                && self
                    .field(*parent, "value")
                    .is_some_and(|value| value.id() == node.id())
        })?;
        self.field(parent, "name")
            .is_some_and(|name| name.kind() == "identifier")
            .then_some(parent)
    }

    pub(crate) fn function_name(&self, node: TsNode<'tree>) -> Option<String> {
        let name = self.function_name_node(node)?;
        if name.kind() == "function_declarator" {
            return Some(self.text(name).replace(char::is_whitespace, ""));
        }
        if name.kind() == "operator_cast" {
            let raw = self.text(name);
            return Some(
                raw.find("()")
                    .map_or(raw.as_str(), |end| &raw[..end])
                    .trim()
                    .to_string(),
            );
        }
        Some(self.text(name))
    }

    /// The `function_declarator` inside a C/C++ `function_definition`'s declarator
    /// chain (holds the `parameters`), or `None`.
    pub(crate) fn c_function_declarator(&self, node: TsNode<'tree>) -> Option<TsNode<'tree>> {
        Self::declarator_kind(
            node.child_by_field_name("declarator")?,
            "function_declarator",
            0,
        )
    }

    pub(crate) fn declarator_name(node: TsNode<'tree>, depth: usize) -> Option<TsNode<'tree>> {
        if matches!(
            node.kind(),
            "identifier"
                | "field_identifier"
                | "type_identifier"
                | "qualified_identifier"
                | "destructor_name"
                | "operator_name"
                | "operator_cast"
        ) {
            return Some(node);
        }
        if depth == MAX_DEPTH {
            return None;
        }
        if let Some(declarator) = node.child_by_field_name("declarator")
            && let Some(name) = Self::declarator_name(declarator, depth + 1)
        {
            return Some(name);
        }
        Self::children(node)
            .into_iter()
            .filter(|child| child.is_named())
            .find_map(|child| Self::declarator_name(child, depth + 1))
    }

    fn declarator_kind(node: TsNode<'tree>, kind: &str, depth: usize) -> Option<TsNode<'tree>> {
        if node.kind() == kind {
            return Some(node);
        }
        if depth == MAX_DEPTH {
            return None;
        }
        Self::children(node)
            .into_iter()
            .filter(|child| child.is_named())
            .find_map(|child| Self::declarator_kind(child, kind, depth + 1))
    }

    /// The class/function body: the named `body_field` if present, else (for
    /// grammars that attach the body positionally, e.g. Kotlin) the first child
    /// whose kind is in `body_kinds`.
    pub(crate) fn body_of(&self, node: TsNode<'tree>) -> Option<TsNode<'tree>> {
        self.field(node, self.cfg.body_field).or_else(|| {
            if self.cfg.body_kinds.is_empty() {
                None
            } else {
                Self::children(node)
                    .into_iter()
                    .find(|c| self.cfg.body_kinds.contains(&c.kind()))
            }
        })
    }

    pub(crate) fn walk(
        &mut self,
        node: TsNode<'tree>,
        file_nid: &NodeId,
        parent_class: Option<&NodeId>,
        stem: &str,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return; // guard against stack overflow on pathologically nested input
        }
        let t = node.kind();

        // Macro definitions describe generated code, not declarations present in
        // the source file. Descending into their replacement text creates fake
        // functions from test/diagnostic macros.
        if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Cpp))
            && matches!(t, "preproc_def" | "preproc_function_def")
        {
            return;
        }

        // Transparent wrappers (e.g. `decorated_definition`): recurse preserving
        // the parent-class scope so decorated methods stay methods (not functions).
        if self.cfg.decorated_types.contains(&t) {
            // Iterate the cursor directly: no per-node `Vec` allocation.
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                self.walk(child, file_nid, parent_class, stem, depth + 1);
            }
            return;
        }

        // Import statements: emit `imports`/`imports_from` edges + records.
        if self.cfg.import_types.contains(&t) {
            match self.cfg.import_style {
                Some(ImportStyle::Python) => self.python_imports(node, file_nid),
                Some(ImportStyle::EcmaScript) => self.ecmascript_imports(node, file_nid),
                Some(ImportStyle::Java) => self.dotted_import(
                    node,
                    file_nid,
                    &["scoped_identifier", "qualified_name", "identifier"],
                ),
                Some(ImportStyle::CSharp) => {
                    self.dotted_import(node, file_nid, &["qualified_name", "identifier"])
                }
                Some(ImportStyle::Kotlin) => {
                    self.dotted_import(node, file_nid, &["qualified_identifier"])
                }
                Some(ImportStyle::Swift) => self.dotted_import(node, file_nid, &["identifier"]),
                Some(ImportStyle::CInclude) => self.c_include(node, file_nid),
                Some(ImportStyle::Php) => self.php_imports(node, file_nid),
                Some(ImportStyle::Scala) => self.scala_imports(node, file_nid),
                None => {}
            }
            return;
        }

        // EcmaScript `export { x } from 'm'` re-exports. Handled here (not via
        // `import_types`) without an early return, so an inline `export class X`
        // declaration is still extracted by the structural recursion below.
        if matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript)) && t == "export_statement"
        {
            self.ecmascript_reexport(node, file_nid);
        }

        // EcmaScript dynamic imports (`import()`/`require()`/`System.import()`).
        // Non-early-return so the normal call-edge extraction still runs.
        if matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript)) && t == "call_expression"
        {
            self.ecmascript_dynamic_import(node, file_nid);
        }

        if self.cfg.type_ref_style == Some(TypeRefStyle::Cpp)
            && matches!(t, "type_definition" | "alias_declaration")
        {
            let owner = parent_class.cloned().unwrap_or_else(|| NodeId(stem.into()));
            self.cpp_type_aliases(node, &owner, stem);
            // A typedef can also define a named aggregate with its own members.
            if let Some(ty) = node.child_by_field_name("type") {
                self.walk(ty, file_nid, parent_class, stem, depth + 1);
            }
            return;
        }

        if self.cfg.class_types.contains(&t) {
            if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Cpp))
                && t.ends_with("_specifier")
                && self.body_of(node).is_none()
            {
                return; // `struct S *value` is a type use, not a definition.
            }
            // C++ forward declarations are references, not class definitions.
            if matches!(self.cfg.heritage_style, Some(HeritageStyle::Cpp))
                && self.body_of(node).is_none()
            {
                return;
            }
            let class_name = if let Some(name_node) = self.field(node, self.cfg.name_field) {
                self.text(name_node)
            } else if t == "anonymous_class"
                || matches!(self.cfg.heritage_style, Some(HeritageStyle::Cpp))
            {
                format!("anonymous@{}", Self::line(node))
            } else {
                return;
            };
            let line = Self::line(node);
            let base = NodeId(make_id(&[stem, &class_name]));
            let class_nid = if matches!(self.cfg.heritage_style, Some(HeritageStyle::EcmaScript))
                && self.seen.contains(&base)
            {
                NodeId(make_id(&[base.as_str(), "overload", &line.to_string()]))
            } else {
                base
            };
            let kind = Self::class_kind(t);
            let vis = self.decl_visibility(node, &class_name);
            self.add_code_node(class_nid.clone(), class_name, node, kind, vis, None);
            self.add_edge(file_nid.clone(), class_nid.clone(), "contains", line, None);

            if let Some(field) = self.cfg.superclasses_field
                && let Some(args) = self.field(node, field)
            {
                for arg in Self::children(args) {
                    if arg.kind() == "identifier" {
                        let base = self.text(arg);
                        self.link_heritage(&class_nid, base, stem, line, "inherits");
                    }
                }
            }

            // Grammar-specific `extends`/`implements` heritage (a different shape
            // than Python's `superclasses` field).
            match self.cfg.heritage_style {
                Some(HeritageStyle::EcmaScript) => {
                    self.ecmascript_heritage(node, &class_nid, stem, line)
                }
                Some(HeritageStyle::Java) => self.java_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::CSharp) => self.csharp_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::Kotlin) => self.kotlin_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::Swift) => self.swift_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::Cpp) => self.cpp_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::Php) => self.php_heritage(node, &class_nid, stem, line),
                Some(HeritageStyle::Scala) => self.scala_heritage(node, &class_nid, stem, line),
                None => {}
            }

            // Non-method members: fields/properties become type `references`
            // (ctx `field`); C++ turns method prototypes into method nodes; Swift
            // captures init/deinit/subscript. Takes the declaration (it reads the
            // body and, for Kotlin, the primary constructor) so it runs even for a
            // body-less class like `class Dog(val x: Foo)`.
            self.class_members(node, &class_nid, stem);
            if let Some(body) = self.body_of(node) {
                // Python class docstring becomes rationale (reuses this parse/walk).
                if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Python))
                    && let Some((doc, dline)) = first_docstring(body, self.source)
                {
                    self.add_rationale(doc, dline, class_nid.clone(), stem);
                }
                for child in Self::children(body) {
                    self.walk(child, file_nid, Some(&class_nid), stem, depth + 1);
                }
            } else if t == "type_alias_declaration" {
                for child in Self::children(node) {
                    self.walk(child, file_nid, Some(&class_nid), stem, depth + 1);
                }
            }
            return;
        }

        if self.cfg.function_types.contains(&t) {
            let Some(func_name) = self.function_name(node) else {
                for child in Self::children(node) {
                    self.walk(child, file_nid, parent_class, stem, depth + 1);
                }
                return;
            };
            let mut ancestor = node.parent();
            let mut nested_function = false;
            while let Some(parent) = ancestor {
                if self.cfg.function_boundary_types.contains(&parent.kind()) {
                    nested_function = true;
                    break;
                }
                ancestor = parent.parent();
            }
            if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Cpp))
                && is_macro_identifier(func_name.as_bytes())
                && (nested_function
                    || (parent_class.is_none() && node.child_by_field_name("type").is_none())
                    || self.text(node).contains("#define"))
            {
                return;
            }
            let declaration = self.bound_function_declarator(node).unwrap_or(node);
            let line = self.declaration_span(declaration).start_line as usize;
            let vis = self.decl_visibility(declaration, &func_name);
            let sig = crate::signature::extract_signature(node, self.source);
            let id_name = if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Cpp)) {
                c_family_function_id_part(&func_name)
            } else if self.function_name_node(node).is_some_and(|name| {
                name.utf8_text(self.source)
                    .unwrap_or("")
                    .starts_with(['\'', '"'])
            }) {
                std::borrow::Cow::Owned(crate::paths::symbol_key(&func_name))
            } else {
                std::borrow::Cow::Borrowed(func_name.as_str())
            };
            let standalone_method = matches!(
                node.kind(),
                "method_definition" | "method_signature" | "abstract_method_signature"
            ) && parent_class.is_none();
            let mut ancestor = node.parent();
            let mut ecmascript_constructor = false;
            while let Some(parent) = ancestor {
                if matches!(
                    parent.kind(),
                    "class" | "class_declaration" | "abstract_class_declaration"
                ) {
                    ecmascript_constructor =
                        matches!(self.cfg.heritage_style, Some(HeritageStyle::EcmaScript))
                            && func_name == "constructor";
                    break;
                }
                ancestor = parent.parent();
            }
            let groovy = self.cfg.call_types.contains(&"command_chain");
            let overload_position = if groovy {
                format!("{line}:{}", node.start_position().column)
            } else {
                line.to_string()
            };
            let func_nid = if let Some(class_nid) = parent_class {
                let base = NodeId(make_id(&[class_nid.as_str(), id_name.as_ref()]));
                let nid = if (groovy
                    || matches!(
                        self.cfg.heritage_style,
                        Some(HeritageStyle::Cpp | HeritageStyle::EcmaScript)
                    ))
                    && self.seen.contains(&base)
                {
                    NodeId(make_id(&[base.as_str(), "overload", &overload_position]))
                } else {
                    base
                };
                self.add_code_node(
                    nid.clone(),
                    format!(".{func_name}()"),
                    declaration,
                    if ecmascript_constructor {
                        synaptic_core::NodeKind::Constructor
                    } else {
                        synaptic_core::NodeKind::Method
                    },
                    vis,
                    Some(sig),
                );
                self.add_edge(class_nid.clone(), nid.clone(), "method", line, None);
                nid
            } else {
                let base = NodeId(make_id(&[stem, id_name.as_ref()]));
                let nid = if (matches!(
                    self.cfg.heritage_style,
                    Some(HeritageStyle::Cpp | HeritageStyle::EcmaScript)
                ) || standalone_method
                    || groovy)
                    && self.seen.contains(&base)
                {
                    NodeId(make_id(&[base.as_str(), "overload", &overload_position]))
                } else {
                    base
                };
                self.add_code_node(
                    nid.clone(),
                    format!("{func_name}()"),
                    declaration,
                    if ecmascript_constructor {
                        synaptic_core::NodeKind::Constructor
                    } else if standalone_method {
                        synaptic_core::NodeKind::Method
                    } else {
                        synaptic_core::NodeKind::Function
                    },
                    vis,
                    Some(sig),
                );
                self.add_edge(file_nid.clone(), nid.clone(), "contains", line, None);
                nid
            };
            if let Some(name) = self.function_name_node(node)
                && name.kind() == "quoted_identifier"
                && let Some(function) = self.nodes.iter_mut().find(|n| n.id == func_nid)
            {
                function.extra.insert(
                    "source_name".into(),
                    name.utf8_text(self.source).unwrap_or("").into(),
                );
            }
            if matches!(
                node.kind(),
                "function_signature" | "method_signature" | "abstract_method_signature"
            ) && let Some(function) = self.nodes.iter_mut().find(|n| n.id == func_nid)
            {
                function
                    .extra
                    .insert("_declaration_only".into(), serde_json::Value::Bool(true));
            }
            // Type-reference edges from parameter/return annotations.
            if self.cfg.type_ref_style == Some(TypeRefStyle::Cpp) {
                let mut c_linkage = !self.cfg.class_types.contains(&"class_specifier");
                let mut parent = node.parent();
                while let Some(p) = parent {
                    if p.kind() == "linkage_specification"
                        && self.text(p).starts_with("extern \"C\"")
                    {
                        c_linkage = true;
                        break;
                    }
                    parent = p.parent();
                }
                let internal = parent_class.is_none()
                    && Self::children(node).iter().any(|n| {
                        n.kind() == "storage_class_specifier" && self.text(*n) == "static"
                    });
                if let Some(function) = self.nodes.iter_mut().find(|n| n.id == func_nid) {
                    function.extra.insert(
                        "native_linkage".into(),
                        (if internal {
                            "internal"
                        } else if c_linkage {
                            "c"
                        } else {
                            "cpp"
                        })
                        .into(),
                    );
                }
            }
            match self.cfg.type_ref_style {
                Some(TypeRefStyle::Python) => self.python_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::EcmaScript) => {
                    self.ecmascript_type_refs(node, &func_nid, stem, line)
                }
                Some(TypeRefStyle::Java) => self.java_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::CSharp) => self.csharp_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::Kotlin) => self.kotlin_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::Swift) => self.swift_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::Cpp) => self.cpp_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::Php) => self.php_type_refs(node, &func_nid, stem, line),
                Some(TypeRefStyle::Scala) => self.scala_type_refs(node, &func_nid, stem, line),
                None => {}
            }
            // Annotation/attribute references (Java `@Anno`, C# `[Attr]`) become
            // `references` edges with context "attribute".
            let anno_names = match self.cfg.type_ref_style {
                Some(TypeRefStyle::Java) => self.java_annotation_names(node),
                Some(TypeRefStyle::CSharp) => self.csharp_attribute_names(node),
                _ => Vec::new(),
            };
            for name in anno_names {
                let tgt = self.ensure_named_node(&name, stem, line);
                if tgt != func_nid {
                    self.add_edge(func_nid.clone(), tgt, "references", line, Some("attribute"));
                }
            }
            if let Some(body) = self.body_of(node) {
                // Python function docstring becomes rationale (reuses this parse/walk).
                if matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Python))
                    && let Some((doc, dline)) = first_docstring(body, self.source)
                {
                    self.add_rationale(doc, dline, func_nid.clone(), stem);
                }
                self.function_bodies.push((func_nid.clone(), body));
                // Mark this function node as own-bodied so the call pass skips it
                // (it is walked here) but still recurses into anonymous callbacks.
                self.owned_fn_nodes.insert(node.id());
                // Named functions may be nested inside this body. Their calls are
                // handled separately by `owned_fn_nodes`; walk structurally here
                // so the declarations themselves are not lost.
                for child in Self::children(body) {
                    self.walk(child, file_nid, None, &func_nid.0, depth + 1);
                }
            }
            return;
        }

        // Default: recurse, retaining a named TS type alias through its direct
        // type wrapper and otherwise resetting class scope.
        // Iterate the cursor directly: no per-node `Vec` allocation.
        let nested_parent = if matches!(self.cfg.heritage_style, Some(HeritageStyle::EcmaScript))
            && matches!(
                t,
                "object_type" | "union_type" | "intersection_type" | "parenthesized_type"
            ) {
            parent_class
        } else {
            None
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child, file_nid, nested_parent, stem, depth + 1);
        }
    }

    /// Base type names inside a heritage clause, including generic bases
    /// (`extends Base<T>` → `Base`) via [`ts_base_head`](Self::ts_base_head).
    pub(crate) fn heritage_bases(&self, clause: TsNode<'tree>) -> Vec<String> {
        Self::children(clause)
            .into_iter()
            .filter_map(|c| self.ts_base_head(c))
            .collect()
    }

    /// Link `class_nid` to a base/interface by name, creating an external stub
    /// when the base is not defined in this file (so the edge survives build's
    /// dangling-edge drop). Used by the `superclasses_field` path.
    pub(crate) fn link_heritage(
        &mut self,
        class_nid: &NodeId,
        base: String,
        stem: &str,
        line: usize,
        relation: &str,
    ) {
        let local = NodeId(make_id(&[stem, &base]));
        let base_nid = if local != *class_nid && self.seen.contains(&local) {
            local
        } else {
            let global = NodeId(make_id(&[base.as_str()]));
            self.add_external_node(global.clone(), base.clone());
            global
        };
        self.add_edge(class_nid.clone(), base_nid, relation, line, None);
    }

    // Java / C# (dotted-name imports)
    /// An `imports` edge to the tail of a dotted import name (Java `import
    /// a.b.C;`, C# `using A.B.C;`) as an external stub. `name_kinds` are the node
    /// kinds that carry the dotted name; for a wildcard/package the tail is the
    /// last segment.
    fn dotted_import(&mut self, node: TsNode<'tree>, file_nid: &NodeId, name_kinds: &[&str]) {
        let line = Self::line(node);
        let name_node = Self::children(node)
            .into_iter()
            .find(|c| name_kinds.contains(&c.kind()));
        let Some(nn) = name_node else { return };
        let full = self.text(nn);
        let tail = full.rsplit('.').next().unwrap_or(&full).trim();
        if tail.is_empty() {
            return;
        }
        let tgt = NodeId(make_id(&[tail]));
        self.add_external_node(tgt.clone(), tail.to_string());
        self.add_edge(file_nid.clone(), tgt, "imports", line, Some("import"));
    }

    /// First identifier under the first `user_type` in `node` (the base type name
    /// of a Kotlin delegation specifier / constructor invocation).
    pub(crate) fn user_type_head(&self, node: TsNode<'tree>) -> Option<String> {
        let mut stack = vec![node];
        while let Some(n) = stack.pop() {
            if n.kind() == "user_type" {
                return Self::children(n)
                    .into_iter()
                    .find(|c| matches!(c.kind(), "identifier" | "type_identifier"))
                    .map(|c| self.text(c));
            }
            for c in Self::children(n) {
                stack.push(c);
            }
        }
        None
    }

    /// All children of `node` attached under field `name` (Swift reuses `name`
    /// for both the identifier and the type).
    pub(crate) fn named_field_nodes(&self, node: TsNode<'tree>, field: &str) -> Vec<TsNode<'tree>> {
        let mut cur = node.walk();
        node.children_by_field_name(field, &mut cur).collect()
    }

    // class members (fields/properties/prototypes)
    /// Per-language class-body member handling, run before the method walk:
    /// fields/properties become type `references` (ctx `field`); C++ method
    /// prototypes and Swift init/deinit/subscript become method nodes.
    fn class_members(&mut self, decl: TsNode<'tree>, class_nid: &NodeId, stem: &str) {
        match self.cfg.type_ref_style {
            Some(TypeRefStyle::Java) => {
                if let Some(body) = self.body_of(decl) {
                    self.java_class_members(body, class_nid, stem);
                }
            }
            Some(TypeRefStyle::CSharp) => {
                if let Some(body) = self.body_of(decl) {
                    self.csharp_class_members(body, class_nid, stem);
                }
            }
            Some(TypeRefStyle::Kotlin) => self.kotlin_class_members(decl, class_nid, stem),
            Some(TypeRefStyle::Swift) => {
                if let Some(body) = self.body_of(decl) {
                    self.swift_class_members(body, class_nid, stem);
                }
            }
            Some(TypeRefStyle::Cpp) => {
                if let Some(body) = self.body_of(decl) {
                    self.cpp_class_members(body, class_nid, stem);
                }
            }
            Some(TypeRefStyle::Php) => {
                if let Some(body) = self.body_of(decl) {
                    self.php_class_members(body, class_nid, stem);
                }
            }
            _ => {}
        }
    }

    /// Emit `references` (ctx `field`, or `generic_arg`) from `owner` to each
    /// collected member type.
    pub(crate) fn emit_field_refs(
        &mut self,
        refs: Vec<(String, bool)>,
        owner: &NodeId,
        stem: &str,
        line: usize,
    ) {
        for (name, generic) in refs {
            let ctx = if generic { "generic_arg" } else { "field" };
            let tgt = self.ensure_named_node(&name, stem, line);
            if &tgt != owner {
                self.add_edge(owner.clone(), tgt, "references", line, Some(ctx));
            }
        }
    }

    // pre-scan
    /// Collect in-file interface/protocol names (by node kind, per the language's
    /// heritage style) so heritage classification can tell interfaces from base
    /// classes. No-op for languages that don't need it.
    pub(crate) fn pre_scan(&mut self, root: TsNode<'tree>) {
        let mut stack = vec![root];
        while let Some(n) = stack.pop() {
            if self.cfg.type_ref_style == Some(TypeRefStyle::Cpp)
                && matches!(n.kind(), "type_definition" | "alias_declaration")
            {
                let mut ancestor = n.parent();
                let mut scoped = false;
                while let Some(parent) = ancestor {
                    scoped |= self.cfg.class_types.contains(&parent.kind())
                        || self.cfg.function_boundary_types.contains(&parent.kind());
                    ancestor = parent.parent();
                }
                let mut cursor = n.walk();
                if !scoped && n.kind() == "type_definition" {
                    for declarator in n.children_by_field_name("declarator", &mut cursor) {
                        if let Some(name) = Self::declarator_name(declarator, 0) {
                            self.declared_types.insert(self.text(name));
                        }
                    }
                } else if !scoped
                    && n.kind() == "alias_declaration"
                    && let Some(name) = n.child_by_field_name("name")
                {
                    self.declared_types.insert(self.text(name));
                }
            }
            if self.cfg.class_types.contains(&n.kind())
                && !(matches!(self.cfg.type_ref_style, Some(TypeRefStyle::Cpp))
                    && n.kind().ends_with("_specifier")
                    && self.body_of(n).is_none())
                && let Some(name) = self.field(n, self.cfg.name_field)
            {
                let name = self.text(name);
                self.declared_types.insert(name.clone());
                if matches!(
                    (self.cfg.heritage_style, n.kind()),
                    (Some(HeritageStyle::CSharp), "interface_declaration")
                        | (Some(HeritageStyle::Swift), "protocol_declaration")
                ) {
                    self.interface_names.insert(name);
                }
            }
            for c in Self::children(n) {
                stack.push(c);
            }
        }
    }

    pub(crate) fn run_call_pass(&mut self, root: TsNode<'tree>) {
        // Map normalized label -> node id: "run_analysis()" -> "run_analysis",
        // ".forward()" -> "forward" (reference: raw.strip("()").lstrip(".")).
        let mut label_to_nid: HashMap<String, NodeId> = HashMap::new();
        let mut ambiguous = HashSet::new();
        for n in self.nodes.iter().filter(|n| !n.source_file.is_empty()) {
            let key = n.label.strip_suffix("()").unwrap_or(&n.label).to_string();
            if (matches!(self.cfg.heritage_style, Some(HeritageStyle::Cpp))
                || self.cfg.call_types.contains(&"command_chain"))
                && label_to_nid.contains_key(&key)
            {
                ambiguous.insert(key);
            } else {
                label_to_nid.insert(key, n.id.clone());
            }
        }
        for key in ambiguous {
            label_to_nid.remove(&key);
        }

        let file_nid = file_node_id(&self.path);
        let bodies = std::mem::take(&mut self.function_bodies);
        let mut seen_pairs: HashSet<(NodeId, NodeId)> = HashSet::new();
        self.walk_calls(root, &file_nid, &label_to_nid, &mut seen_pairs, 0);
        for (caller, body) in bodies {
            self.walk_calls(body, &caller, &label_to_nid, &mut seen_pairs, 0);
        }
    }

    fn walk_calls(
        &mut self,
        node: TsNode<'tree>,
        caller: &NodeId,
        label_to_nid: &HashMap<String, NodeId>,
        seen_pairs: &mut HashSet<(NodeId, NodeId)>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            return;
        }
        if self.cfg.function_boundary_types.contains(&node.kind()) {
            // A NAMED nested function got its own node + body, walked separately --
            // stop so its calls aren't double-attributed to this caller. An
            // ANONYMOUS callback (arrow / function expression passed inline, e.g. an
            // `ipcMain.handle(ch, () => helper())` body or `arr.map(x => f(x))`) has
            // no node of its own, so recurse: its calls belong to this caller.
            if self.owned_fn_nodes.contains(&node.id()) {
                return;
            }
        }

        if self.cfg.call_types.contains(&node.kind())
            && let Some((callee, is_member)) = self.callee_name(node)
        {
            let line = Self::line(node);
            self.record_call(
                caller,
                callee,
                is_member,
                line,
                Some(Self::span(node)),
                label_to_nid,
                seen_pairs,
            );
        }
        // `new X(...)` constructor call: the callee is the constructed type.
        if Some(node.kind()) == self.cfg.constructor_call_type
            && let Some(ctor) = self.field(node, "constructor")
            && matches!(ctor.kind(), "identifier" | "type_identifier")
        {
            let callee = self.text(ctor);
            let line = Self::line(node);
            self.record_call(
                caller,
                callee,
                false,
                line,
                Some(Self::span(node)),
                label_to_nid,
                seen_pairs,
            );
        }

        // EcmaScript dynamic imports inside function/method bodies (`walk()` covers
        // module-scope ones). Emits a file -> module-stub `imports_from` edge.
        if matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript))
            && self.cfg.call_types.contains(&node.kind())
        {
            let file_nid = file_node_id(&self.path);
            self.ecmascript_dynamic_import(node, &file_nid);
        }

        for child in Self::children(node) {
            self.walk_calls(child, caller, label_to_nid, seen_pairs, depth + 1);
        }
    }

    /// Resolve a discovered callee to a `calls` edge (in-file target) or a
    /// `RawCall` (unresolved, for cross-file resolution). Builtins are skipped.
    #[allow(clippy::too_many_arguments)]
    fn record_call(
        &mut self,
        caller: &NodeId,
        callee: String,
        is_member: bool,
        line: usize,
        span: Option<synaptic_core::Span>,
        label_to_nid: &HashMap<String, NodeId>,
        seen_pairs: &mut HashSet<(NodeId, NodeId)>,
    ) {
        if self.cfg.builtins.contains(&callee.as_str()) {
            return;
        }
        let imported_bare = !is_member
            && matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript))
            && self
                .imports
                .iter()
                .any(|import| import.local_name == callee);
        let defer_member = is_member
            && (matches!(
                self.cfg.import_style,
                Some(
                    ImportStyle::EcmaScript
                        | ImportStyle::Java
                        | ImportStyle::Swift
                        | ImportStyle::CSharp
                )
            ) || (matches!(self.cfg.import_style, Some(ImportStyle::Python))
                && callee.contains('.')));
        let member_lookup = format!(".{callee}");
        let target = if defer_member || imported_bare {
            None
        } else if is_member {
            label_to_nid.get(&member_lookup)
        } else if matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript)) {
            label_to_nid.get(&callee)
        } else {
            label_to_nid
                .get(&callee)
                .or_else(|| label_to_nid.get(&member_lookup))
        };
        match target {
            Some(tgt) if tgt != caller => {
                let pair = (caller.clone(), tgt.clone());
                if seen_pairs.insert(pair) {
                    self.add_edge(caller.clone(), tgt.clone(), "calls", line, Some("call"));
                }
            }
            Some(_) => {} // self-call, ignore
            None => {
                self.raw_calls.push(RawCall {
                    caller: caller.clone(),
                    callee,
                    is_member_call: is_member,
                    source_file: self.path.clone(),
                    source_location: Some(format!("L{line}")),
                    span,
                });
            }
        }
    }

    /// Returns `(callee_name, is_member_call)` for a call node, or `None`. When
    /// `call_function_field` is empty the callee is the first named child (for
    /// grammars whose call node names the callee positionally, e.g. Kotlin/Swift
    /// `call_expression`).
    fn callee_name(&self, call: TsNode<'tree>) -> Option<(String, bool)> {
        if matches!(self.cfg.import_style, Some(ImportStyle::Java))
            && call.kind() == "method_invocation"
            && self.cfg.call_function_field == "name"
        {
            let name = self.text(self.field(call, "name")?);
            return Some(match self.field(call, "object") {
                Some(object) => (
                    format!(
                        "{}.{}",
                        self.text(object).replace(char::is_whitespace, ""),
                        name
                    ),
                    true,
                ),
                None => (name, false),
            });
        }
        let func = if call.kind() == "command_chain" {
            self.field(call, "receiver")?
        } else if self.cfg.call_function_field.is_empty() {
            Self::children(call).into_iter().find(|c| c.is_named())?
        } else {
            // Fall back to a `name` field for grammars with separate call-node
            // types (PHP `member_call_expression`/`scoped_call_expression` carry
            // the callee in `name`, not the `function` field of a
            // `function_call_expression`).
            match self.field(call, self.cfg.call_function_field) {
                Some(f) => f,
                None => self.field(call, "name")?,
            }
        };
        if call.child_by_field_name("closure").is_some() && func.kind() == "method_invocation" {
            return None; // Groovy trailing closure: the inner invocation owns the call.
        }
        if self.cfg.type_ref_style == Some(TypeRefStyle::Cpp) && func.kind() == "call_expression" {
            return Some((self.text(func).replace(char::is_whitespace, ""), false));
        }
        if matches!(self.cfg.import_style, Some(ImportStyle::Java))
            && self.cfg.call_accessor_node_types.contains(&func.kind())
        {
            let member = self
                .field(func, "field")
                .or_else(|| self.field(func, "property"))?;
            let object = self.field(func, "object")?;
            return Some((
                format!(
                    "{}.{}",
                    self.text(object).replace(char::is_whitespace, ""),
                    self.text(member)
                ),
                true,
            ));
        }
        if matches!(func.kind(), "identifier" | "simple_identifier") {
            Some((self.text(func), false))
        } else if self.cfg.call_accessor_node_types.contains(&func.kind()) {
            let attr = self.field(func, self.cfg.call_accessor_field)?;
            let member = self.text(attr);
            let full = self.text(func).replace(char::is_whitespace, "");
            let explicit_type_receiver = matches!(self.cfg.import_style, Some(ImportStyle::Python))
                && self
                    .field(func, "object")
                    .map(|object| self.text(object))
                    .and_then(|object| object.rsplit('.').next().map(str::to_string))
                    .and_then(|receiver| receiver.chars().next())
                    .is_some_and(char::is_uppercase);
            Some((
                if (matches!(self.cfg.import_style, Some(ImportStyle::EcmaScript))
                    || self.cfg.heritage_style == Some(crate::config::HeritageStyle::CSharp)
                    || explicit_type_receiver)
                    && full.contains('.')
                {
                    full
                } else {
                    member
                },
                true,
            ))
        } else if func.kind() == "navigation_expression" {
            // Kotlin/Swift member call `recv.method()`: the member is the last
            // identifier (directly, or inside the last `navigation_suffix`).
            if matches!(self.cfg.import_style, Some(ImportStyle::Swift)) {
                Some((self.text(func).replace(char::is_whitespace, ""), true))
            } else {
                self.navigation_member(func).map(|m| (m, true))
            }
        } else {
            Some((self.text(func), false))
        }
    }

    /// The trailing member name of a `navigation_expression` (`a.b.method` → `method`).
    fn navigation_member(&self, nav: TsNode<'tree>) -> Option<String> {
        for c in Self::children(nav).into_iter().rev() {
            match c.kind() {
                "identifier" | "simple_identifier" => return Some(self.text(c)),
                "navigation_suffix" => {
                    if let Some(id) = Self::children(c)
                        .into_iter()
                        .rev()
                        .find(|x| matches!(x.kind(), "identifier" | "simple_identifier"))
                    {
                        return Some(self.text(id));
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Resolve a name to an existing in-file node id, else a global id (creating a
    /// stub node when unseen).
    pub(crate) fn ensure_named_node(&mut self, name: &str, stem: &str, _line: usize) -> NodeId {
        let local = NodeId(make_id(&[stem, name]));
        if self.seen.contains(&local) || self.declared_types.contains(name) {
            return local;
        }
        let global = NodeId(make_id(&[name]));
        if !self.seen.contains(&global) {
            self.add_external_node(global.clone(), name.to_string());
        }
        global
    }
}
