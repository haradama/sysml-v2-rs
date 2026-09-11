//! What the model claims about itself, written where a reader will meet
//! it.
//!
//! A requirement becomes documentation on what satisfies it and a test for
//! the verification case that answers for it -- `#[ignore]` where the
//! model names no such case, so the gap is a test that says it is a gap
//! rather than a silence. An `assert constraint` becomes an assertion.

use sysml_model::{ElementId, ElementKind, Value};

use super::*;

impl<'a> Generator<'a> {
    /// An `assert constraint` usage as the check it stands for. The model
    /// claims the constraint holds; nothing in Rust can hold anyone to
    /// that, so what is generated is the means of asking.
    pub(crate) fn assertion(&self, usage: ElementId, fields: &[Field]) -> Result<String, String> {
        let model = self.model;
        let name = model.name(usage).expect("named, or it was skipped");
        let Some(def) = model.type_of(usage) else {
            return Err(format!("assert `{name}` -- its constraint did not resolve"));
        };
        if self.shapes.get(&def) != Some(&Shape::Calculation) {
            return Err(format!(
                "assert `{name}` -- `{}` is no generated constraint",
                model.name(def).unwrap_or("?")
            ));
        }
        let performer = model.is_abstract(def);
        self.delegation(usage, def, fields, "assert", "bool", performer, "&")
    }
    /// How the model says a behaviour is put together: the subactions it
    /// is composed of, the order it puts them in, and what flows between
    /// them. None of it becomes code; all of it is what the code has to
    /// do.
    pub(crate) fn decomposition(&self, def: ElementId) -> Vec<String> {
        let model = self.model;
        let mut steps = Vec::new();
        let mut subactions = Vec::new();
        for &child in model.owned(def) {
            match model.kind(child) {
                ElementKind::ActionUsage | ElementKind::PerformActionUsage => {
                    if let Some(name) = model.name(child) {
                        let of = model
                            .type_of(child)
                            .and_then(|ty| model.name(ty))
                            .map(|ty| format!(" : {ty}"))
                            .unwrap_or_default();
                        subactions.push(format!("`{name}{of}`"));
                    }
                }
                kind if kind.is_a(ElementKind::SuccessionAsUsage) => {
                    if let Some(order) = self.ends_of(child) {
                        steps.push(format!("then: {order}"));
                    }
                }
                kind if kind.is_a(ElementKind::FlowUsage) => {
                    if let Some(order) = self.ends_of(child) {
                        steps.push(format!("flow: {order}"));
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        if !subactions.is_empty() {
            out.push(format!("Made of {}.", subactions.join(", ")));
        }
        out.extend(steps);
        out
    }
    /// The model's requirements, as tests.
    ///
    /// Each names what the model says satisfies it. A requirement a
    /// verification case answers for becomes a test that runs that
    /// case's Rust; one with no verification becomes an ignored stub
    /// that says so, which is a list of what has been promised and not
    /// yet kept.
    pub(crate) fn requirements(&self, roots: &[ElementId], out: &mut String) {
        let model = self.model;
        let mut found = Vec::new();
        let mut named = HashSet::new();
        for &root in roots {
            for id in model.descendants(root) {
                let keyword = match model.kind(id) {
                    ElementKind::RequirementDefinition => "requirement def",
                    // A requirement can be stated as a usage and never
                    // defined -- `requirement transportRequirements;` --
                    // and then it is its own. One typed by a definition
                    // only restates it, under another name and for
                    // another subject, so verifying the definition is
                    // what verifies it.
                    ElementKind::RequirementUsage if model.type_of(id).is_none() => "requirement",
                    _ => continue,
                };
                let Some(name) = model.name(id) else {
                    continue;
                };
                if !named.insert(name.to_string()) {
                    continue;
                }
                found.push((keyword, id, name.to_string()));
            }
        }
        if found.is_empty() {
            return;
        }
        // What satisfies and what verifies is stated anywhere in the
        // model, so it takes a walk of the whole of it -- the loaded
        // standard library included -- to find. One walk answers for
        // every requirement at once; asking per requirement, as this
        // did, walked a hundred thousand elements twice over for each.
        let claims = self.claims();
        let stubs: Vec<_> = found
            .into_iter()
            .map(|(keyword, id, name)| {
                (
                    keyword,
                    name,
                    documentation(model, id),
                    claims.satisfiers.get(&id).cloned().unwrap_or_default(),
                    claims.verifications.get(&id).cloned().unwrap_or_default(),
                    declared_values(model, id),
                )
            })
            .collect();
        writeln!(
            out,
            "\n/// The model's requirements: one test per requirement, running\n\
             /// the verification the model names for it, or ignored and\n\
             /// saying so where it names none.\n\
             #[cfg(test)]\n\
             mod requirements {{"
        )
        .unwrap();
        for (at, (keyword, name, doc, satisfiers, verifications, values)) in
            stubs.iter().enumerate()
        {
            if at > 0 {
                writeln!(out).unwrap();
            }
            writeln!(out, "    /// SysML: `{keyword} {name}`").unwrap();
            if let Some(doc) = doc {
                doc_comment(doc, "    ", out);
            }
            for satisfier in satisfiers {
                writeln!(out, "    /// Satisfied by `{satisfier}`.").unwrap();
            }
            for (case, _) in verifications {
                writeln!(out, "    /// Verified by `{case}`.").unwrap();
            }
            writeln!(out, "    #[test]").unwrap();
            match verifications.iter().find_map(|(_, path)| path.as_ref()) {
                Some(path) => {
                    writeln!(out, "    fn {}() {{", ident(name)).unwrap();
                    writeln!(out, "        {path}({});", values.join(", ")).unwrap();
                    writeln!(out, "    }}").unwrap();
                }
                None => {
                    writeln!(out, "    #[ignore = \"verification not written yet\"]").unwrap();
                    writeln!(out, "    fn {}() {{}}", ident(name)).unwrap();
                }
            }
        }
        writeln!(out, "}}").unwrap();
    }
    /// What the model claims about its requirements, in one walk of it:
    /// what satisfies each, and which verification cases answer for it
    /// with the Rust their `@code` bindings name.
    fn claims(&self) -> Claims {
        let model = self.model;
        let mut claims = Claims::default();
        for id in model.ids() {
            if model.kind(id).is_a(ElementKind::SatisfyRequirementUsage)
                // `not satisfy r by p;` says p does not, so naming it as
                // what answers for `r` would put the opposite of the
                // model in the doc
                && !model.flag(id, "isNegated")
            {
                let satisfied = model.get(id, "satisfiedRequirement").and_then(Value::as_id);
                let by = model
                    .get(id, "satisfyingFeature")
                    .and_then(Value::as_id)
                    .and_then(|feature| model.name(feature));
                if let Some((requirement, name)) = satisfied.zip(by) {
                    claims
                        .satisfiers
                        .entry(requirement)
                        .or_default()
                        .push(name.to_string());
                }
            }
            if let (Some(Value::RefList(verified)), Some(name)) =
                (model.maybe(id, "verifiedRequirement"), model.name(id))
            {
                let path = binding(model, id).and_then(|bound| bound.get(binding::ITEM).cloned());
                for &requirement in verified {
                    claims
                        .verifications
                        .entry(requirement)
                        .or_default()
                        .push((name.to_string(), path.clone()));
                }
            }
        }
        claims
    }
}
