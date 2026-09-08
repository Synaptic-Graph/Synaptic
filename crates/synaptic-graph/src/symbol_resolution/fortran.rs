//! Fortran lexical and USE association. Module names are independent of filenames.
use super::*;

use synaptic_core::fortran::INTRINSICS;

struct Scopes<'a> {
    graph: &'a KnowledgeGraph,
    parents: HashMap<NodeId, NodeId>,
    members: HashMap<(NodeId, String), HashSet<NodeId>>,
    modules: HashMap<String, Vec<NodeId>>,
    externals: HashMap<String, HashSet<NodeId>>,
    uses: HashMap<NodeId, Vec<&'a Edge>>,
    implementations: HashMap<NodeId, NodeId>,
    declarations: HashMap<NodeId, NodeId>,
}

impl Scopes<'_> {
    fn variable(&self, scope: &NodeId, name: &str) -> Option<&serde_json::Value> {
        let mut scope = Some(scope);
        for _ in 0..128 {
            let owner = scope?;
            if let Some(value) = self
                .graph
                .node(owner)?
                .extra
                .get("fortran_variables")
                .and_then(|v| v.get(name))
            {
                return Some(value);
            }
            if let Some(value) = self
                .declarations
                .get(owner)
                .and_then(|id| self.graph.node(id))
                .and_then(|n| n.extra.get("fortran_variables"))
                .and_then(|v| v.get(name))
            {
                return Some(value);
            }
            scope = self.parents.get(owner);
        }
        None
    }

    fn actuals(&self, call: &RawCall) -> Vec<String> {
        let line = call
            .source_location
            .as_deref()
            .unwrap_or("L0")
            .trim_start_matches('L');
        let key = format!(
            "{line}:{}",
            call.callee
                .to_ascii_lowercase()
                .replace(char::is_whitespace, "")
        );
        self.graph
            .node(&call.caller)
            .and_then(|n| n.extra.get("fortran_actuals"))
            .and_then(|a| a.get(&key))
            .and_then(|v| v.as_array())
            .map(|v| {
                v.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn argument_type(&self, scope: &NodeId, expression: &str) -> Option<(String, usize)> {
        let expression = expression.trim();
        if let Some(value) = self.variable(scope, expression) {
            return Some((
                value["type"].as_str()?.to_owned(),
                value["rank"].as_u64().unwrap_or(0) as usize,
            ));
        }
        if expression.starts_with(['\'', '"']) {
            return Some(("character".into(), 0));
        }
        if matches!(expression, ".true." | ".false.") {
            return Some(("logical".into(), 0));
        }
        let number = expression.split('_').next()?.trim_start_matches(['+', '-']);
        if !number.is_empty() && number.bytes().all(|c| c.is_ascii_digit()) {
            return Some((
                expression
                    .split_once('_')
                    .map_or_else(|| "integer".into(), |(_, k)| format!("integer({k})")),
                0,
            ));
        }
        if number.contains(['.', 'e', 'd']) && number.replace('d', "e").parse::<f64>().is_ok() {
            return Some((
                expression.split_once('_').map_or_else(
                    || {
                        if number.contains('d') {
                            "doubleprecision".into()
                        } else {
                            "real".into()
                        }
                    },
                    |(_, k)| format!("real({k})"),
                ),
                0,
            ));
        }
        None
    }

    fn accepts(&self, target: &NodeId, caller: &NodeId, actuals: &[String]) -> bool {
        let Some(node) = self
            .graph
            .node(self.declarations.get(target).unwrap_or(target))
        else {
            return false;
        };
        let Some(parameters) = node
            .extra
            .get("fortran_parameters")
            .and_then(|v| v.as_array())
        else {
            return true;
        };
        let variables = node.extra.get("fortran_variables");
        if actuals.len() > parameters.len() {
            return false;
        }
        let mut supplied = HashSet::new();
        for (i, actual) in actuals.iter().enumerate() {
            let (parameter, expression) = if let Some((key, value)) = actual.split_once('=') {
                let Some(p) = parameters.iter().find(|p| p.as_str() == Some(key.trim())) else {
                    return false;
                };
                (p.as_str().unwrap(), value)
            } else {
                (parameters[i].as_str().unwrap_or(""), actual.as_str())
            };
            if !supplied.insert(parameter) {
                return false;
            }
            if let Some(expected) = variables.and_then(|v| v.get(parameter))
                && let Some((kind, rank)) = self.argument_type(caller, expression)
                && (expected["type"].as_str().is_some_and(|t| {
                    type_category(t) != type_category(&kind)
                        || type_kind(t)
                            .zip(type_kind(&kind))
                            .is_some_and(|(a, b)| a != b)
                }) || expected["rank"]
                    .as_u64()
                    .is_some_and(|r| r as usize != rank))
            {
                return false;
            }
        }
        parameters.iter().filter_map(|p| p.as_str()).all(|p| {
            supplied.contains(p)
                || variables
                    .and_then(|v| v.get(p))
                    .is_some_and(|v| v["optional"] == true)
        })
    }

    fn specifics(&self, target: &NodeId, call: &RawCall) -> HashSet<NodeId> {
        if self.graph.node(target).is_none_or(|n| {
            n.extra.get("fortran_scope").and_then(|s| s.as_str()) != Some("interface")
        }) {
            if let Some(node) = self.graph.node(target)
                && node.extra.get("fortran_declaration") == Some(&serde_json::Value::Bool(true))
                && node.extra.get("fortran_module_procedure")
                    == Some(&serde_json::Value::Bool(false))
                && let Some(definitions) = self.externals.get(&normalize_label(&node.label))
                && definitions.len() == 1
            {
                return definitions.clone();
            }
            return HashSet::from([target.clone()]);
        }
        let actuals = self.actuals(call);
        let mut candidates = HashSet::new();
        for edge in self.graph.edges().filter(|e| {
            &e.source == target && matches!(e.relation.as_str(), "references" | "method")
        }) {
            if self.accepts(&edge.target, &call.caller, &actuals) {
                candidates.insert(
                    self.implementations
                        .get(&edge.target)
                        .unwrap_or(&edge.target)
                        .clone(),
                );
            }
        }
        // Interfaces without an explicit procedure list still provide a useful
        // scope target; an ambiguous explicit overload is kept unresolved.
        if candidates.is_empty()
            && !self.graph.edges().any(|e| {
                &e.source == target && matches!(e.relation.as_str(), "references" | "method")
            })
        {
            candidates.insert(target.clone());
        }
        candidates
    }

    fn bound_method(
        &self,
        ty: &NodeId,
        name: &str,
        caller: &NodeId,
        actuals: &[String],
        depth: usize,
    ) -> HashSet<NodeId> {
        if depth >= 32 {
            return HashSet::new();
        }
        let Some(node) = self.graph.node(ty) else {
            return HashSet::new();
        };
        let mut candidates = HashSet::new();
        if let Some(method) = node.extra.get("fortran_methods").and_then(|m| m.get(name)) {
            if let Some(generic) = method["generic"].as_array() {
                for name in generic.iter().filter_map(|v| v.as_str()) {
                    candidates.extend(self.bound_method(ty, name, caller, actuals, depth + 1));
                }
            } else if method["deferred"] != true
                && let Some(target) = method["target"].as_str()
                && let Some(owner) = self.parents.get(ty)
                && let Some(targets) = self.lookup(owner, target, false, &mut HashSet::new())
            {
                for target in targets {
                    let mut args = actuals.to_vec();
                    if method["nopass"] != true {
                        if let Some(pass) = method["pass"].as_str() {
                            args.push(format!("{pass}=@passed_object"));
                        } else {
                            args.insert(0, "@passed_object".into());
                        }
                    }
                    if self.accepts(&target, caller, &args) {
                        candidates.insert(target);
                    }
                }
            }
        } else if let Some(base) = node.extra.get("fortran_base").and_then(|b| b.as_str())
            && let Some(owner) = self.parents.get(ty)
            && let Some(bases) = self.lookup(owner, base, false, &mut HashSet::new())
        {
            for base in bases {
                candidates.extend(self.bound_method(&base, name, caller, actuals, depth + 1));
            }
        }
        candidates
    }

    fn member_targets(&self, call: &RawCall) -> Option<HashSet<NodeId>> {
        let name = call
            .callee
            .to_ascii_lowercase()
            .replace(char::is_whitespace, "");
        let (receiver, method) = name.rsplit_once('%')?;
        let mut candidates = HashSet::new();
        if let Some(variable) = self.variable(&call.caller, receiver)
            && let Some(ty) = variable["type"].as_str()
            && let Some(types) =
                self.lookup(&call.caller, &type_category(ty), false, &mut HashSet::new())
        {
            let mut types = types;
            if ty.starts_with("class(") {
                // The runtime type may be any repository extension. Preserve
                // possible override targets as inferred dispatch candidates.
                loop {
                    let before = types.len();
                    for node in self.graph.nodes() {
                        if let Some(base) = node.extra.get("fortran_base").and_then(|v| v.as_str())
                            && let Some(owner) = self.parents.get(&node.id)
                            && let Some(bases) =
                                self.lookup(owner, base, false, &mut HashSet::new())
                            && bases.iter().any(|b| types.contains(b))
                        {
                            types.insert(node.id.clone());
                        }
                    }
                    if types.len() == before {
                        break;
                    }
                }
            }
            for ty in types {
                candidates.extend(self.bound_method(
                    &ty,
                    method,
                    &call.caller,
                    &self.actuals(call),
                    0,
                ));
            }
        }
        Some(candidates)
    }

    // None means absent; an empty set means a binding exists but cannot be
    // resolved safely. In particular, failed ONLY imports must not fall back
    // to an unrelated external procedure with the same name.
    fn lookup(
        &self,
        scope: &NodeId,
        name: &str,
        exported: bool,
        visiting: &mut HashSet<NodeId>,
    ) -> Option<HashSet<NodeId>> {
        if visiting.len() >= 128 || !visiting.insert(scope.clone()) {
            return Some(HashSet::new());
        }
        let result = self.lookup_inner(scope, name, exported, visiting);
        visiting.remove(scope);
        result
    }

    fn lookup_inner(
        &self,
        scope: &NodeId,
        name: &str,
        exported: bool,
        visiting: &mut HashSet<NodeId>,
    ) -> Option<HashSet<NodeId>> {
        if exported
            && let Some(access) = self
                .graph
                .node(scope)
                .and_then(|n| n.extra.get("fortran_access"))
        {
            let public = access["names"][name]
                .as_bool()
                .unwrap_or_else(|| access["default"].as_bool().unwrap_or(true));
            if !public {
                return None;
            }
        }
        if let Some(binding) = self
            .graph
            .node(scope)
            .and_then(|n| n.extra.get("fortran_bindings"))
            .and_then(|names| names.get(name))
            .and_then(|v| v.as_str())
        {
            return Some(if binding == "external" {
                let candidates = self.externals.get(name).cloned().unwrap_or_default();
                let file = self.graph.node(scope).map(|n| &n.source_file);
                let local: HashSet<_> = candidates
                    .iter()
                    .filter(|id| self.graph.node(id).map(|n| &n.source_file) == file)
                    .cloned()
                    .collect();
                if local.len() == 1 {
                    local
                } else {
                    let siblings: HashSet<_> = candidates
                        .iter()
                        .filter(|id| {
                            self.graph
                                .node(id)
                                .and_then(|n| Path::new(n.source_file.as_str()).parent())
                                == file.and_then(|path| Path::new(path.as_str()).parent())
                        })
                        .cloned()
                        .collect();
                    if siblings.len() == 1 {
                        siblings
                    } else {
                        candidates
                    }
                }
            } else {
                HashSet::new()
            });
        }
        if let Some(targets) = self.members.get(&(scope.clone(), name.to_owned()))
            && (!INTRINSICS.contains(&name)
                || self
                    .graph
                    .node(scope)
                    .is_some_and(|n| n.extra.contains_key("fortran_scope")))
        {
            return Some(targets.clone());
        }
        let mut targets = HashSet::new();
        let mut bound = false;
        let mut unknown = false;
        for edge in self.uses.get(scope).into_iter().flatten() {
            let evidence = &edge.extra["fortran_use"];
            let aliases = evidence["names"].as_object();
            let renamed = aliases
                .and_then(|names| names.get(name))
                .and_then(|value| value.as_str());
            let only = evidence["only"].as_bool() == Some(true);
            if renamed.is_none()
                && (only
                    || aliases
                        .is_some_and(|names| names.values().any(|v| v.as_str() == Some(name))))
            {
                continue;
            }
            if evidence["intrinsic"].as_bool() == Some(true) {
                // Intrinsic module procedures are external to the repository.
                if renamed.is_some() {
                    bound = true;
                    unknown = true;
                }
                continue;
            }
            let module = self
                .graph
                .node(&edge.target)
                .map(|n| normalize_label(&n.label));
            let definitions = module.as_ref().and_then(|name| self.modules.get(name));
            let Some(definitions) = definitions.filter(|defs| defs.len() == 1) else {
                bound = true;
                unknown = true;
                continue;
            };
            if let Some(found) =
                self.lookup(&definitions[0], renamed.unwrap_or(name), true, visiting)
            {
                bound = true;
                unknown |= found.is_empty();
                targets.extend(found);
            } else if renamed.is_some() {
                bound = true;
                unknown = true;
            }
        }
        if bound {
            return Some(if unknown { HashSet::new() } else { targets });
        }
        if !exported && let Some(parent) = self.parents.get(scope) {
            return self.lookup(parent, name, false, visiting);
        }
        None
    }
}

fn type_category(ty: &str) -> String {
    let ty = ty.trim().to_ascii_lowercase();
    if let Some(derived) = ty
        .strip_prefix("type(")
        .or_else(|| ty.strip_prefix("class("))
    {
        return derived.trim_end_matches(')').to_owned();
    }
    if ty.starts_with("doubleprecision") {
        return "real".into();
    }
    ty.split(['(', '*']).next().unwrap_or(&ty).to_owned()
}

fn type_kind(ty: &str) -> Option<&str> {
    if ["type(", "class(", "character"]
        .iter()
        .any(|p| ty.starts_with(p))
    {
        return None;
    }
    if ty == "doubleprecision" {
        return None;
    }
    if let Some((_, kind)) = ty.split_once('(') {
        let kind = kind
            .trim_end_matches(')')
            .strip_prefix("kind=")
            .unwrap_or(kind.trim_end_matches(')'));
        // Named kind constants require compiler evaluation; do not guess their
        // numeric value from spelling or incorrectly reject an alias.
        return kind.bytes().all(|b| b.is_ascii_digit()).then_some(kind);
    }
    if let Some((_, kind)) = ty.split_once('*') {
        return Some(kind);
    }
    None
}

pub(super) fn resolve(
    kg: &KnowledgeGraph,
    calls: &[RawCall],
    known: &mut HashSet<(NodeId, NodeId, String)>,
) -> (Vec<Edge>, HashSet<usize>) {
    let mut scopes = Scopes {
        graph: kg,
        parents: HashMap::new(),
        members: HashMap::new(),
        modules: HashMap::new(),
        externals: HashMap::new(),
        uses: HashMap::new(),
        implementations: HashMap::new(),
        declarations: HashMap::new(),
    };
    for node in kg.nodes().filter(|n| {
        n.label.ends_with("()")
            && !n.label.starts_with('.')
            && source_family(&n.source_file).as_deref() == Some("fortran")
            && n.extra.get("fortran_declaration") != Some(&serde_json::Value::Bool(true))
    }) {
        scopes
            .externals
            .entry(normalize_label(&node.label))
            .or_default()
            .insert(node.id.clone());
    }
    for node in kg
        .nodes()
        .filter(|n| n.extra.get("fortran_scope").and_then(|v| v.as_str()) == Some("module"))
    {
        scopes
            .modules
            .entry(normalize_label(&node.label))
            .or_default()
            .push(node.id.clone());
    }
    for edge in kg
        .edges()
        .filter(|e| source_family(&e.source_file).as_deref() == Some("fortran"))
    {
        if edge.extra.contains_key("fortran_use") {
            scopes
                .uses
                .entry(edge.source.clone())
                .or_default()
                .push(edge);
        } else if matches!(edge.relation.as_str(), "method" | "contains") {
            scopes
                .parents
                .insert(edge.target.clone(), edge.source.clone());
            if let Some(node) = kg.node(&edge.target)
                && (node.label.ends_with("()")
                    || node
                        .extra
                        .get("fortran_scope")
                        .and_then(|v| v.as_str())
                        .is_some_and(|s| matches!(s, "interface" | "derived_type_definition")))
            {
                scopes
                    .members
                    .entry((edge.source.clone(), normalize_label(&node.label)))
                    .or_default()
                    .insert(node.id.clone());
            }
        }
    }
    let mut prototypes: HashMap<(NodeId, String), Vec<NodeId>> = HashMap::new();
    for node in kg
        .nodes()
        .filter(|n| n.extra.get("fortran_declaration") == Some(&serde_json::Value::Bool(true)))
    {
        let mut owner = scopes.parents.get(&node.id);
        for _ in 0..128 {
            let Some(id) = owner else {
                break;
            };
            if kg.node(id).is_some_and(|n| {
                n.extra.get("fortran_scope").and_then(|v| v.as_str()) == Some("module")
            }) {
                prototypes
                    .entry((id.clone(), normalize_label(&node.label)))
                    .or_default()
                    .push(node.id.clone());
                break;
            }
            owner = scopes.parents.get(id);
        }
    }
    // A submodule inherits its ancestor's private and public host scope.
    for node in kg
        .nodes()
        .filter(|n| n.extra.contains_key("fortran_ancestor"))
    {
        let ancestor = node.extra["fortran_ancestor"].as_str().unwrap_or("");
        let parent = node.extra.get("fortran_parent").and_then(|v| v.as_str());
        let parents: Vec<_> = if let Some(parent) = parent {
            kg.nodes()
                .filter(|n| {
                    normalize_label(&n.label) == parent
                        && n.extra.get("fortran_ancestor").and_then(|v| v.as_str())
                            == Some(ancestor)
                })
                .map(|n| n.id.clone())
                .collect()
        } else {
            scopes.modules.get(ancestor).cloned().unwrap_or_default()
        };
        if parents.len() == 1 {
            scopes.parents.insert(node.id.clone(), parents[0].clone());
        }
        if let Some(modules) = scopes.modules.get(ancestor)
            && modules.len() == 1
        {
            for edge in kg
                .edges()
                .filter(|e| e.source == node.id && e.relation == "method")
            {
                if let Some(procedure) = kg.node(&edge.target) {
                    let key = (modules[0].clone(), normalize_label(&procedure.label));
                    if let Some(declarations) = prototypes.get(&key) {
                        for id in declarations {
                            scopes
                                .implementations
                                .insert(id.clone(), procedure.id.clone());
                            scopes.declarations.insert(procedure.id.clone(), id.clone());
                        }
                        if let Some(existing) = scopes.members.get_mut(&key) {
                            existing.retain(|id| !declarations.contains(id));
                            existing.insert(procedure.id.clone());
                        }
                    }
                }
            }
        }
    }
    let mut out = Vec::new();
    let mut bound = HashSet::new();
    for (index, call) in calls
        .iter()
        .enumerate()
        .filter(|(_, c)| source_family(&c.source_file).as_deref() == Some("fortran"))
    {
        let targets = scopes.member_targets(call).or_else(|| {
            scopes.lookup(
                &call.caller,
                &call.callee.to_ascii_lowercase(),
                false,
                &mut HashSet::new(),
            )
        });
        let Some(targets) = targets else {
            if INTRINSICS.contains(&call.callee.to_ascii_lowercase().as_str()) {
                bound.insert(index);
            }
            continue;
        };
        bound.insert(index);
        let targets: HashSet<_> = targets
            .iter()
            .flat_map(|target| scopes.specifics(target, call))
            .collect();
        let dispatch = call.callee.contains('%');
        if targets.len() != 1 && !dispatch {
            continue;
        }
        let ambiguous = targets.len() > 1;
        for target in targets {
            if target != call.caller
                && known.insert((call.caller.clone(), target.clone(), "calls".into()))
            {
                let external = call.callee.contains('%')
                    || kg.node(&target).is_some_and(|node| {
                        !node.label.starts_with('.')
                            && node.label.ends_with("()")
                            && node.source_file.as_str() != call.source_file
                    });
                out.push(calls_edge(
                    call.caller.clone(),
                    target,
                    if external {
                        Confidence::Inferred
                    } else {
                        Confidence::Extracted
                    },
                    if external {
                        Confidence::Inferred.default_score()
                    } else {
                        1.0
                    },
                    if dispatch && ambiguous {
                        "fortran_dispatch_candidate"
                    } else if dispatch {
                        "fortran_type_bound_call"
                    } else if external {
                        "fortran_external_call"
                    } else {
                        "fortran_scope_call"
                    },
                    call.source_file.clone(),
                    call.source_location.clone(),
                ));
            }
        }
    }
    (out, bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_kinds_array_ranks_and_polymorphic_overrides() {
        let (graph, calls) = graph(&[(
            "dispatch.f90",
            include_str!("../../../../eval/fixtures/parser-completion/dispatch.f90"),
        )]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        let targets = |caller: &str| -> HashSet<_> {
            graph
                .edges()
                .chain(edges.iter())
                .filter(|e| e.relation == "calls" && graph.node(&e.source).unwrap().label == caller)
                .map(|e| graph.node(&e.target).unwrap().label.as_str())
                .collect()
        };
        assert_eq!(
            targets("check_dispatch"),
            HashSet::from([
                ".child_run()",
                ".double_value()",
                ".vector_value()",
                ".polymorphic()"
            ])
        );
        assert_eq!(
            targets(".polymorphic()"),
            HashSet::from([".base_run()", ".child_run()"])
        );
        assert!(
            edges
                .iter()
                .filter(|e| graph.node(&e.source).unwrap().label == ".polymorphic()")
                .all(|e| e.context.as_deref() == Some("fortran_dispatch_candidate"))
        );
    }

    #[test]
    fn external_interface_calls_reach_implementation_files() {
        let (graph, calls) = graph(&[
            (
                "api.f90",
                "module api\n interface\n subroutine work(x)\n integer :: x\n end subroutine\n end interface\n end module\n",
            ),
            (
                "work.f90",
                "subroutine work(x)\n integer :: x\n end subroutine\n",
            ),
            (
                "client.f90",
                "program client\n use api\n call work(1)\n end program\n",
            ),
        ]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        assert!(
            edges
                .iter()
                .any(|e| graph.node(&e.source).unwrap().label == "client"
                    && graph.node(&e.target).unwrap().source_file == "work.f90")
        );
    }

    #[test]
    fn named_generic_prototypes_retarget_submodule_implementations() {
        let (graph, calls) = graph(&[
            (
                "api.f90",
                "module api\n interface convert\n module function from_int(x) result(y)\n integer :: x,y\n end function\n end interface\n end module\n",
            ),
            (
                "impl.f90",
                "submodule(api) impl\n contains\n module procedure from_int\n y=x\n end procedure\n end submodule\n",
            ),
            (
                "client.f90",
                "program client\n use api\n integer :: n\n n=convert(1)\n end program\n",
            ),
        ]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        assert!(
            edges
                .iter()
                .any(|e| graph.node(&e.source).unwrap().label == "client"
                    && graph.node(&e.target).unwrap().source_file == "impl.f90"
                    && graph.node(&e.target).unwrap().label == ".from_int()")
        );
    }

    #[test]
    fn typed_bindings_overloads_and_submodule_host_association() {
        let (graph, calls) = graph(&[
            (
                "library.f90",
                "module library\n type shape\n contains\n procedure :: area => shape_area\n end type\n interface convert\n module procedure from_int, from_real\n end interface\n interface\n module subroutine work()\n end subroutine\n end interface\n contains\n real function shape_area(self)\n class(shape) :: self\n shape_area=1.0\n end function\n integer function from_int(x)\n integer :: x\n from_int=x\n end function\n integer function from_real(x)\n real :: x\n from_real=int(x)\n end function\n subroutine helper()\n end subroutine\n end module\n",
            ),
            (
                "implementation.f90",
                "submodule(library) implementation\n contains\n module procedure work\n call helper()\n end procedure\n end submodule\n",
            ),
            (
                "client.f90",
                "subroutine client()\n use library\n type(shape) :: s\n integer :: n\n real :: x\n x=s%area()\n n=convert(1)\n n=convert(1.0)\n call work()\n end subroutine\n",
            ),
        ]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        let targets: HashSet<_> = edges
            .iter()
            .filter(|e| graph.node(&e.source).unwrap().label == "client()" && e.relation == "calls")
            .map(|e| {
                (
                    graph.node(&e.target).unwrap().source_file.as_str(),
                    graph.node(&e.target).unwrap().label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            targets,
            HashSet::from([
                ("library.f90", ".shape_area()"),
                ("library.f90", ".from_int()"),
                ("library.f90", ".from_real()"),
                ("implementation.f90", ".work()")
            ])
        );
        assert!(
            edges
                .iter()
                .any(
                    |e| graph.node(&e.source).unwrap().source_file == "implementation.f90"
                        && graph.node(&e.target).unwrap().label == ".helper()"
                )
        );
    }

    fn graph(files: &[(&str, &str)]) -> (KnowledgeGraph, Vec<RawCall>) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let mut calls = Vec::new();
        for (path, source) in files {
            let extraction = synaptic_extract::extract_source(path, source.as_bytes()).unwrap();
            assert!(!extraction.parse_error, "{path}");
            nodes.extend(extraction.nodes);
            edges.extend(extraction.edges);
            calls.extend(extraction.raw_calls);
        }
        (
            KnowledgeGraph::from_graph_data(synaptic_core::GraphData {
                nodes,
                links: edges,
                directed: true,
                ..Default::default()
            }),
            calls,
        )
    }

    #[test]
    fn intrinsic_names_respect_use_host_and_explicit_external_or_intrinsic() {
        let (graph, calls) = graph(&[
            (
                "math.f90",
                "module custom\ncontains\nreal function abs(x)\nreal :: x\nabs=x\nend function\nsubroutine host()\nreal :: x\nx=abs(1.0)\nend subroutine\nend module\n",
            ),
            (
                "external.f90",
                "real function abs(x)\nreal :: x\nabs=x\nend function\nsubroutine default_in_same_file()\nreal :: x\nx=abs(1.0)\nend subroutine\n",
            ),
            (
                "client.f90",
                "subroutine imported()\nuse custom, only: abs\nreal :: x\nx=abs(1.0)\nend subroutine\nsubroutine explicit_external()\nexternal :: abs\nreal :: abs,x\nx=abs(1.0)\nend subroutine\nsubroutine forced_intrinsic()\nintrinsic :: abs\nreal :: x\nx=abs(1.0)\nend subroutine\nsubroutine default_intrinsic()\nreal :: x\nx=abs(1.0)\nend subroutine\n",
            ),
        ]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        let pairs: HashSet<_> = graph
            .edges()
            .chain(edges.iter())
            .filter(|e| e.relation == "calls")
            .map(|e| {
                (
                    graph.node(&e.source).unwrap().label.as_str(),
                    graph.node(&e.target).unwrap().label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            pairs,
            HashSet::from([
                (".host()", ".abs()"),
                ("imported()", ".abs()"),
                ("explicit_external()", "abs()")
            ])
        );
    }

    #[test]
    fn use_aliases_reexports_host_scope_privacy_and_ambiguity() {
        let (graph, calls) = graph(&[
            (
                "different_filename.f90",
                "module api\nprivate\npublic :: work\ncontains\nsubroutine work()\nend subroutine\nsubroutine hidden()\nend subroutine\nend module\n",
            ),
            (
                "facade.f90",
                "module facade\nuse api, only: renamed => work\nend module\n",
            ),
            (
                "other.f90",
                "module other\ncontains\nsubroutine renamed()\nend subroutine\nend module\n",
            ),
            (
                "caller.f90",
                "module client\nuse facade\ncontains\nsubroutine run()\ncall RENAMED()\ncontains\nsubroutine inner()\ncall renamed()\nend subroutine\nend subroutine\nsubroutine denied()\nuse api, only: hidden\ncall hidden()\nend subroutine\nsubroutine ambiguous()\nuse facade\nuse other\ncall renamed()\nend subroutine\nend module\nsubroutine hidden()\nend subroutine\n",
            ),
            (
                "app.f90",
                "program app\nuse api, only: work\ncall work()\nend program\n",
            ),
        ]);
        let edges = super::super::resolve_symbols(&graph, &calls, &[]);
        let pairs: HashSet<_> = edges
            .iter()
            .map(|e| {
                (
                    graph.node(&e.source).unwrap().label.as_str(),
                    graph.node(&e.target).unwrap().label.as_str(),
                )
            })
            .collect();
        assert_eq!(
            pairs,
            HashSet::from([
                (".run()", ".work()"),
                (".inner()", ".work()"),
                ("app", ".work()")
            ])
        );
        assert!(edges.iter().all(|e| e.confidence == Confidence::Extracted));
    }

    #[test]
    fn unrelated_modules_and_internal_procedures_never_win_by_file_order() {
        let (graph, calls) = graph(&[(
            "scopes.f90",
            "module a\ncontains\nsubroutine run()\ncall work()\nend subroutine\nsubroutine work()\nend subroutine\nend module\nmodule b\ncontains\nsubroutine work()\nend subroutine\nend module\nsubroutine outer()\ncontains\nsubroutine internal()\nend subroutine\nend subroutine\nsubroutine stranger()\ncall internal()\nend subroutine\n",
        )]);
        let local: Vec<_> = graph.edges().filter(|e| e.relation == "calls").collect();
        assert_eq!(local.len(), 1);
        let owner = graph
            .edges()
            .find(|e| e.target == local[0].target && e.relation == "method")
            .unwrap();
        assert_eq!(graph.node(&owner.source).unwrap().label, "a");
        assert!(super::super::resolve_symbols(&graph, &calls, &[]).is_empty());
    }
}
