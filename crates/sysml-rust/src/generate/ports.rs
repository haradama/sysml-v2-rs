//! Ports, and the Rust they reach into.
//!
//! A port bound to an existing crate's API through `@code` metadata
//! becomes a generic parameter with a trait bound, so a part that
//! declares one is written once and instantiated with whatever satisfies
//! it. A port the model bound to nothing becomes a plain field of a
//! generated struct. What a part `perform`s over such a port becomes a
//! method that delegates to it.

use sysml_model::{ElementId, ElementKind};

use super::*;

impl<'a> Generator<'a> {
    /// An unbound port as a plain field of its generated port struct.
    pub(crate) fn plain_port(&self, def: ElementId, usage: ElementId) -> Option<Field> {
        let target = self.model.type_of(usage)?;
        if self.shapes.get(&target) != Some(&Shape::Struct) || self.has_api_ports(target) {
            return None;
        }
        let container = multiplicity(self.model, usage);
        let name = self.member_name(usage)?;
        Some(Field {
            spelled: ident(&name),
            name,
            usage,
            ty: FieldType::Generated(target),
            // a port of the definition's own type composes it as surely
            // as a part does, and needs the same `Box` at the edge that
            // closes the circle
            boxed: container != Container::Many
                && (target == def || self.boxed.contains(&(def, target))),
            container,
            default: None,
        })
    }
    /// The ports of a definition that hold the generated struct of their
    /// port definition rather than a bound API. They become fields like
    /// any other, so what is asked of a field is asked of them too.
    pub(crate) fn plain_ports(&self, def: ElementId) -> Vec<Field> {
        self.members(def)
            .iter()
            .filter(|&&child| self.model.kind(child) == ElementKind::PortUsage)
            .filter_map(|&child| self.plain_port(def, child))
            .collect()
    }
    /// Does this definition hold ports bound to external APIs -- which
    /// generate as generic parameters, not composable fields?
    pub(crate) fn has_api_ports(&self, def: ElementId) -> bool {
        self.members(def).iter().any(|&child| {
            self.model.kind(child) == ElementKind::PortUsage && self.port_of(child).is_some()
        })
    }
    pub(crate) fn port_of(&self, usage: ElementId) -> Option<Port> {
        let port_def = self.model.type_of(usage)?;
        let bound = binding(self.model, port_def)?;
        let path = bound.get(binding::ITEM)?.clone();
        Some(Port {
            name: self.member_name(usage)?,
            type_name: self.model.name(port_def)?.to_string(),
            parameter: String::new(),
            trait_path: path,
        })
    }
    /// One performed API action as a delegating method.
    pub(crate) fn method(
        &self,
        part_name: &str,
        usage: ElementId,
        ports: &[Port],
        fields: &[Field],
    ) -> Result<String, RustgenError> {
        let model = self.model;
        let usage_name = &self.member_name(usage).expect("named, or it was skipped");
        // Every way this method declines says the same two things -- which
        // performed action, and why -- so it says them in one place, to the
        // reader of the file and to whoever has to write what is missing.
        let declined = |why: String| {
            self.open(OpenKind::Skipped, usage_name, None, why.clone());
            Ok(format!(
                "    // not generated: perform `{usage_name}` -- {why}\n"
            ))
        };
        let Some(action) = self.performed(usage) else {
            return declined("its action did not resolve".to_string());
        };
        let Some(bound) = binding(model, action) else {
            // no binding to delegate through, but the model still named
            // the behaviour: the trait its definition generated is what
            // the part performs it with
            if self.shapes.get(&action) == Some(&Shape::Action) {
                let Some(returns) = self.action_result(action) else {
                    return declined("a result of it has no Rust type".to_string());
                };
                return Ok(
                    match self.delegation(usage, action, fields, "perform", &returns, true, "&mut ")
                    {
                        Ok(method) => method,
                        Err(note) => {
                            self.open(OpenKind::Skipped, usage_name, None, note.clone());
                            format!("    // not generated: {note}\n")
                        }
                    },
                );
            }
            return declined(format!(
                "`{}` carries no `@code` binding",
                model.name(action).unwrap_or("?")
            ));
        };
        let path = bound.get(binding::ITEM).cloned().unwrap_or_default();
        let takes_self = bound
            .get(binding::TAKES_SELF)
            .map(String::as_str)
            .unwrap_or("");
        if takes_self == "self" {
            return declined(format!("`{path}` consumes its receiver"));
        }
        let provider = ports
            .iter()
            .find(|port| path.starts_with(&format!("{}::", port.trait_path)));
        let Some(provider) = provider else {
            return Err(RustgenError::NoPortForAction {
                part: part_name.to_string(),
                action: usage_name.to_string(),
                path,
            });
        };
        // said outright to take no receiver, it is called on the type and
        // not on the port, which is a value of it
        if bound.get(binding::TAKES_SELF).map(String::as_str) == Some("") {
            return declined(format!(
                "`{path}` takes no receiver, and a port is called through one"
            ));
        }
        let callee = path.rsplit("::").next().unwrap_or(&path);

        let mut inputs = Vec::new();
        let mut output = None;
        let mut error = None;
        for &parameter in model.owned(action) {
            let Some(direction) = model.direction(parameter) else {
                continue;
            };
            let name = model.name(parameter).unwrap_or("_");
            let Some((base, container)) = self.parameter_shape(parameter) else {
                return declined(format!("parameter `{name}` has no Rust type"));
            };
            match (direction, name) {
                ("in", _) => inputs.push((ident(name), contained(base, container))),
                // the `Result` around it is what says it may be absent
                ("out", "error") => error = Some(base),
                ("out", _) => output = Some(contained(base, container)),
                _ => {}
            }
        }

        let is_async = bound.get(binding::IS_ASYNC).map(String::as_str) == Some("true");
        let fallible = bound.get(binding::IS_FALLIBLE).map(String::as_str) == Some("true");
        // A call that can fail returns a `Result`, and this method has to
        // be declared as returning the same one. Without an `out error`
        // there is no name for its second half, and guessing would put a
        // type in the signature that the real function does not return.
        if fallible && error.is_none() {
            return declined("it can fail, but the model does not say with what".to_string());
        }
        let receiver = if takes_self == "&mut self" {
            "&mut self"
        } else {
            "&self"
        };
        let mut signature = format!(
            "    pub {}fn {}({receiver}",
            if is_async { "async " } else { "" },
            ident(usage_name),
        );
        for (name, ty) in &inputs {
            write!(signature, ", {name}: {ty}").unwrap();
        }
        signature.push(')');
        let returned = match (&output, &error) {
            (Some(ok), Some(err)) if fallible => Some(format!("Result<{ok}, {err}>")),
            (Some(ok), _) => Some(ok.clone()),
            (None, Some(err)) if fallible => Some(format!("Result<(), {err}>")),
            _ => None,
        };
        if let Some(returned) = &returned {
            write!(signature, " -> {returned}").unwrap();
        }

        let mut call = format!(
            "self.{}.{callee}({})",
            ident(&provider.name),
            inputs
                .iter()
                .map(|(name, _)| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        if is_async {
            call.push_str(".await");
        }

        let mut body = String::new();
        writeln!(
            body,
            "    /// SysML: `perform action {usage_name} : {}` -> `{path}`",
            model.name(action).unwrap_or("?")
        )
        .unwrap();
        body.push_str(&self.unnamed_note(usage));
        writeln!(body, "{signature} {{").unwrap();
        writeln!(body, "        {call}").unwrap();
        writeln!(body, "    }}").unwrap();
        Ok(body)
    }
    /// The Rust type of an action parameter.
    pub(crate) fn parameter_type(&self, usage: ElementId) -> Option<String> {
        let (base, container) = self.parameter_shape(usage)?;
        Some(contained(base, container))
    }
    /// How Rust spells a call to a named calculation written somewhere
    /// inside `scope`: the function its definition generated, where the
    /// definition generated one. An `abstract` definition became a trait
    /// instead, and a trait method is not callable out of nowhere.
    ///
    /// What the call names is what the model resolved it to -- the
    /// invocation's own membership -- and not whichever definition happens
    /// to share the name: two packages may each have an `F`, and the one
    /// Rust named `f` need not be the one this call means.
    pub(crate) fn callable(&self, scope: ElementId, name: &str) -> Option<expr::Callee> {
        let model = self.model;
        let mut invoked = model
            .descendants(scope)
            .into_iter()
            .filter(|&id| model.kind(id).is_a(ElementKind::InvocationExpression))
            .filter_map(|invocation| {
                model.instantiated_type(invocation).or_else(|| {
                    model
                        .owned(invocation)
                        .iter()
                        .find(|&&child| model.kind(child) == ElementKind::Membership)
                        .and_then(|&membership| model.member_element(membership))
                })
            })
            .filter(|&target| model.name(target) == Some(name));
        let target = invoked.next()?;
        // one name, two things it was resolved to in the same place: the
        // text alone cannot say which call is which
        if invoked.any(|other| other != target) {
            return None;
        }
        // a definition that lost its Rust name to another was never
        // written, and calling the other would be calling something else
        if self.shapes.get(&target) != Some(&Shape::Calculation) {
            return None;
        }
        if self.model.is_abstract(target) {
            return None;
        }
        // in declaration order, which is the order the generated function
        // takes them in, so a call that named its arguments can be put
        // back into it
        let parameters = self
            .model
            .owned(target)
            .iter()
            .filter(|&&child| self.model.direction(child) == Some("in"))
            .filter_map(|&child| self.model.name(child).map(str::to_string))
            .collect();
        Some(expr::Callee {
            function: ident(name),
            parameters,
        })
    }
    /// The Rust type of a parameter and the container its multiplicity
    /// asks for, apart -- the error of a `Result` states its own
    /// optionality by being the error, and must not be wrapped again.
    pub(crate) fn parameter_shape(&self, usage: ElementId) -> Option<(String, Container)> {
        // a parameter bound by a usage -- `in previous = stateVector` --
        // declares no type of its own; it keeps the one it redefines
        //
        // A subset does not borrow the multiplicity: `in m :> ISQ::mass`
        // is one mass of the many `ISQ::mass` allows, and says so by
        // saying nothing.
        let ty = held_type(self.model, usage)?;
        let declaring = redefined(self.model, usage)
            .filter(|_| self.model.type_of(usage).is_none())
            .unwrap_or(usage);
        // a parameter carries its multiplicity the way a field does: a
        // `[1..*]` of something is a `Vec` of it, not one of it
        let container = multiplicity(self.model, declaring);
        if let Some(bound) = binding(self.model, ty) {
            return bound
                .get(binding::ITEM)
                .cloned()
                .map(|path| (path, container));
        }
        if self.written_as_a_type(ty) {
            return self
                .model
                .name(ty)
                .map(|name| (type_ident(name), container));
        }
        Some((scalar_type(self.model, ty)?.to_string(), container))
    }
}
