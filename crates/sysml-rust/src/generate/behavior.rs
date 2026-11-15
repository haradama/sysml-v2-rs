//! What a model says happens, written as Rust that does it.
//!
//! A `calc def` becomes a function and a method, with its result
//! expression translated where it is within the subset and a `todo!`
//! naming what the model wrote where it is not. A `state def` becomes a
//! state machine -- an enum of states and a step that reads an event. An
//! `action def` whose dataflow the model wired end to end becomes the body
//! that performs it; one it did not becomes a trait for a person to write.

use std::collections::{HashMap, HashSet};

use sysml_model::{ElementId, ElementKind, Value};

use super::*;

impl<'a> Generator<'a> {
    /// A `state def` as a runnable machine: states, events, hooks and a
    /// `step` over the transition table.
    pub(crate) fn state_machine(&self, def: ElementId, out: &mut String) {
        let model = self.model;
        let name = type_ident(model.name(def).expect("collected named"));
        let states: Vec<(ElementId, &str)> = model
            .owned(def)
            .iter()
            .filter_map(|&child| {
                (model.kind(child) == ElementKind::StateUsage)
                    .then(|| model.name(child).map(|n| (child, n)))
                    .flatten()
            })
            .collect();
        let mut transitions: Vec<Transition> = model
            .owned(def)
            .iter()
            .filter_map(|&child| self.transition(child, &states))
            .collect();
        // a transition the model left unnamed is still a row of the
        // table; it is called after its ends, clear of every name the
        // model did give one
        let mut taken: HashSet<String> = transitions
            .iter()
            .filter(|transition| transition.named)
            .map(|transition| transition.name.clone())
            .collect();
        for transition in transitions.iter_mut().filter(|t| !t.named) {
            let mut name = transition.name.clone();
            let mut count = 1;
            while !taken.insert(name.clone()) {
                count += 1;
                name = format!("{}_{count}", transition.name);
            }
            transition.event = type_ident(&camel(&name));
            transition.name = name;
        }

        writeln!(out).unwrap();
        if let Some(doc) = documentation(model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: `state def {name}` -- the states").unwrap();
        // a transition written inside a state -- `state a { accept go
        // then b; }` -- names only where it goes, and the table is over
        // transitions that say both ends, so it is said out loud rather
        // than left out
        for (state, state_name) in &states {
            let nested = model.owned(*state).iter().any(|&child| {
                let kind = model.kind(child);
                kind == ElementKind::TransitionUsage || kind == ElementKind::SuccessionAsUsage
            });
            if nested {
                self.open(
                    OpenKind::Skipped,
                    *state_name,
                    None,
                    "a transition inside it -- only a transition naming both its ends \
                     joins the table",
                );
                writeln!(
                    out,
                    "// not generated: a transition inside state `{state_name}` -- \
                     only a transition naming both its ends joins the table"
                )
                .unwrap();
            }
        }
        writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]").unwrap();
        // a state's name is written into an enum, so it goes through
        // the same mangling a type name does: `'red light'` is a state
        // in SysML and `Red light` is not a variant in Rust
        let mut spelling = Namespace::default();
        let variants: HashMap<&str, String> = states
            .iter()
            .map(|(_, state)| (*state, spelling.take(type_ident(&camel(state)))))
            .collect();
        let variant = |state: &str| variants.get(state).cloned().unwrap_or_default();
        writeln!(out, "pub enum {name}State {{").unwrap();
        for (_, state) in &states {
            writeln!(out, "    {},", variant(state)).unwrap();
        }
        writeln!(out, "}}").unwrap();
        // where the machine starts is what the model says it enters --
        // `entry; then b;` -- and only where it says nothing is it the
        // first state declared
        let entered = initial_state(model, model.owned(def))
            .and_then(|state| states.iter().find(|(id, _)| *id == state));
        if let Some((_, first)) = entered.or(states.first()) {
            writeln!(out, "\nimpl {name}State {{").unwrap();
            let said = if entered.is_some() {
                format!("The state the definition enters first: `entry; then {first};`.")
            } else {
                "The first state the definition declares.".to_string()
            };
            writeln!(out, "    /// {said}\n    #[must_use]").unwrap();
            writeln!(out, "    pub fn initial() -> Self {{").unwrap();
            writeln!(out, "        {name}State::{}", variant(first)).unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
        }

        // one event per transition: the payload it accepts, or a bare
        // signal named after the transition
        writeln!(out, "\n/// What `{name}State::step` reacts to.").unwrap();
        let mut spelling = Namespace::default();
        let events: Vec<String> = transitions
            .iter()
            .map(|transition| spelling.take(transition.event.clone()))
            .collect();
        writeln!(out, "pub enum {name}Event {{").unwrap();
        for (transition, event) in transitions.iter().zip(&events) {
            match &transition.payload {
                Some((_, ty)) => writeln!(out, "    {event}({ty}),").unwrap(),
                None => writeln!(out, "    {event},").unwrap(),
            }
        }
        writeln!(out, "}}").unwrap();

        // guards, effects, and entry/exit notifications, defaulted so an
        // implementation only writes what it cares about
        writeln!(
            out,
            "\n/// The machine's open decisions; every hook has a default."
        )
        .unwrap();
        self.open(
            OpenKind::Hooks,
            self.model.name(def).unwrap_or(&name),
            Some(format!("{name}Hooks")),
            "the guards, effects and entry/exit notifications of a state machine",
        );
        // A hook is named after its state or transition behind a prefix,
        // so it is never a keyword -- `on_entry_r#move` is no name at
        // all -- and two that spell alike take a number, as their
        // variants do: `'a b'` and `a_b` are two states.
        let bare = |name: &str| ident(name).trim_start_matches("r#").to_string();
        let mut hooked = Namespace::default();
        let state_hooks: HashMap<&str, String> = states
            .iter()
            .map(|(_, state)| (*state, hooked.take(bare(state))))
            .collect();
        let state_hook = |state: &str| state_hooks.get(state).cloned().unwrap_or_default();
        let mut hooked = Namespace::default();
        let transition_hooks: Vec<String> = transitions
            .iter()
            .map(|transition| hooked.take(bare(&transition.name)))
            .collect();
        writeln!(out, "#[allow(unused_variables)]").unwrap();
        writeln!(out, "pub trait {name}Hooks {{").unwrap();
        for (transition, hook) in transitions.iter().zip(&transition_hooks) {
            let payload = transition
                .payload
                .as_ref()
                .map(|(param, ty)| format!(", {}: &{ty}", ident(param)))
                .unwrap_or_default();
            if let Some(guard) = &transition.guard {
                // a guard over the event's payload translates into the
                // default body; anything else stays an open decision
                let resolve = |chain: &str| {
                    expr::led_by(chain, |leading: &str| {
                        transition
                            .payload
                            .as_ref()
                            .filter(|(param, _)| param == leading)
                            .map(|(param, _)| ident(param))
                    })
                };
                let body = translate(guard, &resolve, &|name| {
                    self.callable(transition.usage, name)
                })
                .filter(|translated| translated.boolean)
                .map(|translated| translated.rust)
                .unwrap_or_else(|| "true".to_string());
                writeln!(out, "    /// SysML guard: `[{}]`", on_one_line(guard)).unwrap();
                writeln!(
                    out,
                    "    fn guard_{hook}(&self{payload}) -> bool {{\n        {body}\n    }}"
                )
                .unwrap();
            }
            if transition.effect {
                writeln!(out, "    /// SysML effect of {}.", transition.shown()).unwrap();
                writeln!(out, "    fn effect_{hook}(&mut self{payload}) {{}}").unwrap();
            }
        }
        for (_, state) in &states {
            let hook = state_hook(state);
            writeln!(out, "    fn on_entry_{hook}(&mut self) {{}}").unwrap();
            writeln!(out, "    fn on_exit_{hook}(&mut self) {{}}").unwrap();
        }
        writeln!(out, "}}").unwrap();

        writeln!(out, "\nimpl {name}State {{").unwrap();
        writeln!(
            out,
            "    /// One step of the machine: the transition table, guards\n    \
             /// first, exit-effect-entry in order."
        )
        .unwrap();
        writeln!(out, "    #[must_use]").unwrap();
        writeln!(
            out,
            "    pub fn step(self, event: &{name}Event, hooks: &mut impl {name}Hooks) -> Self {{"
        )
        .unwrap();
        writeln!(out, "        match (self, event) {{").unwrap();
        for ((transition, event), hook) in transitions.iter().zip(&events).zip(&transition_hooks) {
            let pattern = match &transition.payload {
                // only a guard or an effect reads the payload; binding it
                // for an arm that does neither is an unused variable
                Some((param, _)) => format!(
                    "({name}State::{}, {name}Event::{event}({}))",
                    variant(&transition.source),
                    if transition.guard.is_some() || transition.effect {
                        ident(param)
                    } else {
                        "_".to_string()
                    }
                ),
                None => format!(
                    "({name}State::{}, {name}Event::{event})",
                    variant(&transition.source)
                ),
            };
            let guard = match (&transition.guard, &transition.payload) {
                (Some(_), Some((param, _))) => {
                    format!(" if hooks.guard_{hook}({})", ident(param))
                }
                (Some(_), None) => format!(" if hooks.guard_{hook}()"),
                (None, _) => String::new(),
            };
            writeln!(out, "            {pattern}{guard} => {{").unwrap();
            writeln!(
                out,
                "                hooks.on_exit_{}();",
                state_hook(&transition.source)
            )
            .unwrap();
            if transition.effect {
                let argument = transition
                    .payload
                    .as_ref()
                    .map(|(param, _)| ident(param))
                    .unwrap_or_default();
                writeln!(out, "                hooks.effect_{hook}({argument});").unwrap();
            }
            writeln!(
                out,
                "                hooks.on_entry_{}();",
                state_hook(&transition.target)
            )
            .unwrap();
            writeln!(
                out,
                "                {name}State::{}",
                variant(&transition.target)
            )
            .unwrap();
            writeln!(out, "            }}").unwrap();
        }
        writeln!(out, "            (state, _) => state,").unwrap();
        writeln!(out, "        }}").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
    }
    /// One transition of a state machine, when both its ends are states
    /// of the machine.
    fn transition(&self, usage: ElementId, states: &[(ElementId, &str)]) -> Option<Transition> {
        let model = self.model;
        if model.kind(usage) != ElementKind::TransitionUsage {
            return None;
        }
        // A transition is not a connector: what it relates it relates
        // through the `Succession` it owns, and that is where its two
        // ends are.
        let relates = model
            .owned(usage)
            .iter()
            .copied()
            .find(|&child| model.kind(child).is_a(ElementKind::SuccessionAsUsage))
            .unwrap_or(usage);
        // the two unnamed chaining features are the ends, source first
        let ends: Vec<ElementId> = model
            .owned(relates)
            .iter()
            .filter(|&&child| model.kind(child) == ElementKind::Feature)
            .filter_map(|&child| sysml_model::end_reaches(model, child).last().copied())
            .collect();
        let state_name = |id: ElementId| {
            states
                .iter()
                .find(|(state, _)| *state == id)
                .map(|(_, n)| (*n).to_string())
        };
        let (&source, &target) = match ends.as_slice() {
            [source, target] => (source, target),
            _ => return None,
        };
        let (source, target) = (state_name(source)?, state_name(target)?);
        let named = model.name(usage);
        let name = named.map_or_else(|| format!("{source}_to_{target}"), str::to_string);

        // what the transition waits for, and what that carries
        let payload = match model.get(usage, "triggerAction") {
            Some(Value::RefList(triggers)) => triggers.first().and_then(|&accept| {
                // `TriggerAction : AcceptActionUsage = AcceptParameterPart`
                // -- what the transition waits for is the accept
                // action's first parameter, and the action is written
                // with no name of its own
                let waits = sysml_model::payload_parameter(model, accept)?;
                let param = model.name(waits)?;
                let ty = model.type_of(waits)?;
                let rust = if let Some(bound) = binding(model, ty) {
                    bound.get(binding::ITEM)?.clone()
                } else if self.written_as_a_type(ty) {
                    type_ident(model.name(ty)?)
                } else {
                    return None;
                };
                Some((param.to_string(), rust))
            }),
            _ => None,
        };
        let event = type_ident(&camel(&name));
        let guard = match model.get(usage, "guardExpression") {
            Some(Value::RefList(guards)) => guards
                .first()
                .and_then(|&expression| expression_text(model, expression)),
            _ => None,
        };
        let effect = matches!(model.get(usage, "effectAction"), Some(Value::RefList(list)) if !list.is_empty());
        Some(Transition {
            usage,
            named: named.is_some(),
            name,
            source,
            target,
            event,
            payload,
            guard,
            effect,
        })
    }
    /// A `calc def` as a function: `in` parameters become arguments, the
    /// return parameter -- or, failing a declared one, what the result
    /// expression's references agree on -- the return type. A simple
    /// result expression becomes the body; anything richer stays in the
    /// model's own words behind a `todo!`.
    pub(crate) fn calculation(&self, def: ElementId, out: &mut String) {
        let model = self.model;
        let spelled = model.name(def).expect("collected named");
        let name = type_ident(spelled);
        let constraint = model.kind(def) == ElementKind::ConstraintDefinition;
        let keyword = if constraint { "constraint" } else { "calc" };
        let mut params: Vec<(String, String)> = Vec::new();
        // a constraint is a boolean expression by definition, whatever
        // its body turns out to say
        let mut declared = constraint.then(|| "bool".to_string());
        let mut clause = None;
        for &child in model.owned(def) {
            if model.member_role(child) == Some(Role::Return) {
                match self.parameter_type(child) {
                    Some(ty) => declared = Some(ty),
                    None => {
                        self.open(
                            OpenKind::Skipped,
                            name.clone(),
                            None,
                            "its return has no Rust type",
                        );
                        writeln!(
                            out,
                            "\n// not generated: {keyword} def `{name}` -- its return has no Rust type"
                        )
                        .unwrap();
                        return;
                    }
                }
                if let Some(value) = value_clause(model, child) {
                    clause = Some(value);
                }
                continue;
            }
            if model.direction(child) != Some("in") {
                continue;
            }
            let Some(param) = model.name(child) else {
                continue;
            };
            let Some(ty) = self.parameter_type(child) else {
                self.open(
                    OpenKind::Skipped,
                    name.clone(),
                    None,
                    format!("parameter `{param}` has no Rust type"),
                );
                writeln!(
                    out,
                    "\n// not generated: {keyword} def `{name}` -- parameter `{param}` has no Rust type"
                )
                .unwrap();
                return;
            };
            params.push((param.to_string(), ty));
        }
        if let Some(trailing) = result_clause(model, def) {
            clause = Some(trailing);
        }

        // a parameter on its own is a value of the type it is declared
        // with, which is what lets `a * b + 1` over two `Real`s be `+ 1.0`
        let resolve = |chain: &str| match params.iter().find(|(param, _)| param == chain) {
            Some((param, ty)) => Some(expr::Reference::typed(ident(param), ty)),
            None => expr::led_by(chain, |leading: &str| {
                params
                    .iter()
                    .find(|(param, _)| param == leading)
                    .map(|(param, _)| ident(param))
            }),
        };
        let translated = match &clause {
            Some(ValueClause::Text(text)) => {
                let units = crate::quantity::units_under(model, def);
                translate_as(
                    text,
                    numbers_of(declared.as_deref()),
                    &units,
                    &resolve,
                    &|name| self.callable(def, name),
                )
            }
            _ => None,
        };
        // an undeclared return type can still be read off the expression:
        // a comparison is a bool, arithmetic is whatever every referenced
        // parameter is
        let returns = declared.or_else(|| {
            let translated = translated.as_ref()?;
            if translated.boolean {
                return Some("bool".to_string());
            }
            let mut types = translated.references.iter().map(|reference| {
                params
                    .iter()
                    .find(|(param, _)| param == reference)
                    .map(|(_, ty)| ty.clone())
            });
            let first = types.next()??;
            types
                .all(|ty| ty.as_deref() == Some(first.as_str()))
                .then_some(first)
        });
        let Some(returns) = returns else {
            self.open(
                OpenKind::Skipped,
                name.clone(),
                None,
                "its result type is neither declared nor inferable",
            );
            writeln!(
                out,
                "\n// not generated: {keyword} def `{name}` -- its result type is neither declared \
                 nor inferable"
            )
            .unwrap();
            return;
        };
        let arguments = params
            .iter()
            .map(|(param, ty)| format!("{}: {ty}", ident(param)))
            .collect::<Vec<_>>();

        // `abstract` is the model saying it has no formula to give. What
        // Rust has for that is a trait method -- left without a default,
        // so the compiler asks for it -- and not a body that panics when
        // it is finally reached.
        if model.is_abstract(def) {
            let signature = std::iter::once("&self".to_string())
                .chain(arguments)
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(out).unwrap();
            if let Some(doc) = documentation(model, def) {
                doc_comment(&doc, "", out);
            }
            writeln!(out, "/// SysML: `abstract {keyword} def {name}`").unwrap();
            writeln!(
                out,
                "/// The model gives no formula; the implementation is yours."
            )
            .unwrap();
            self.open(
                OpenKind::Trait,
                spelled,
                Some(format!("{name}::{}", ident(spelled))),
                "an abstract definition declares no formula",
            );
            writeln!(out, "pub trait {name} {{").unwrap();
            writeln!(out, "    fn {}({signature}) -> {returns};", ident(spelled)).unwrap();
            writeln!(out, "}}").unwrap();
            return;
        }

        let (body, unfit) = self.formula_body(&clause, translated, name.clone(), ident(spelled));

        writeln!(out).unwrap();
        if let Some(doc) = documentation(model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: `{keyword} def {name}`").unwrap();
        if unfit {
            writeln!(
                out,
                "/// The formula is beyond the simple subset; the body is left to write."
            )
            .unwrap();
        }
        if body.starts_with("todo!") {
            writeln!(out, "#[allow(unused_variables)]").unwrap();
        }
        // A formula over a `Real` and a `Natural` is a formula Rust will
        // not have: the numeric literals were kept as written rather than
        // coerced, so the mixing surfaces as a type error. Saying which
        // parameters disagree puts that where it can be acted on -- in
        // the model -- and not only in a file marked DO NOT EDIT.
        if let Some(mixed) = mixed_numerics(&params) {
            writeln!(out, "/// {mixed}").unwrap();
        }
        let arguments = arguments.join(", ");
        writeln!(
            out,
            "pub fn {}({arguments}) -> {returns} {{",
            ident(spelled)
        )
        .unwrap();
        writeln!(out, "    {body}").unwrap();
        writeln!(out, "}}").unwrap();
    }
    /// A `calc` usage of a struct as a method: its own `in` parameters
    /// as arguments, the struct's fields reachable as `self.field`.
    pub(crate) fn calc_method(&self, usage: ElementId, fields: &[Field]) -> Result<String, String> {
        let model = self.model;
        let name = &self.member_name(usage).expect("named, or it was skipped");

        // typed by a calculation definition and carrying no formula of
        // its own, the usage performs that definition rather than being
        // one: what it says is which of the parameters come off the part
        if let Some(def) = model.type_of(usage) {
            if self.shapes.get(&def) == Some(&Shape::Calculation)
                && result_clause(model, usage).is_none()
                && value_clause(model, usage).is_none()
            {
                return self.calc_delegation(usage, def, fields);
            }
        }

        let Some(returns) = self.parameter_type(usage) else {
            return Err(format!("calc `{name}` -- its result has no Rust type"));
        };
        let mut params: Vec<(String, String)> = Vec::new();
        for &child in model.owned(usage) {
            if model.direction(child) != Some("in") {
                continue;
            }
            let Some(param) = model.name(child) else {
                continue;
            };
            let Some(ty) = self.parameter_type(child) else {
                return Err(format!(
                    "calc `{name}` -- parameter `{param}` has no Rust type"
                ));
            };
            params.push((param.to_string(), ty));
        }

        let resolve = |chain: &str| {
            if let Some((param, ty)) = params.iter().find(|(param, _)| param == chain) {
                return Some(expr::Reference::typed(ident(param), ty));
            }
            let leading = chain.split('.').next().unwrap_or(chain);
            if params.iter().any(|(param, _)| param == leading) {
                return expr::led_by(chain, |leading| Some(ident(leading)));
            }
            self.through_fields(fields, chain)
        };
        let clause = result_clause(model, usage).or_else(|| value_clause(model, usage));
        let translated = match &clause {
            Some(ValueClause::Text(text)) => {
                let units = crate::quantity::units_under(model, usage);
                translate_as(
                    text,
                    numbers_of(Some(&returns)),
                    &units,
                    &resolve,
                    &|name| self.callable(usage, name),
                )
            }
            _ => None,
        };
        let (body, unfit) = self.formula_body(&clause, translated, name, ident(name));

        let mut method = String::new();
        writeln!(method, "    /// SysML: `calc {name}`").unwrap();
        method.push_str(&self.unnamed_note(usage));
        if unfit {
            writeln!(
                method,
                "    /// The formula is beyond the simple subset; the body is left to write."
            )
            .unwrap();
        }
        if body.starts_with("todo!") {
            writeln!(method, "    #[allow(unused_variables)]").unwrap();
        }
        let arguments = params
            .iter()
            .map(|(param, ty)| format!(", {}: {ty}", ident(param)))
            .collect::<Vec<_>>()
            .join("");
        writeln!(
            method,
            "    pub fn {}(&self{arguments}) -> {returns} {{",
            ident(name)
        )
        .unwrap();
        writeln!(method, "        {body}").unwrap();
        writeln!(method, "    }}").unwrap();
        Ok(method)
    }
    /// Whether the machine a `state` usage names declares a state to
    /// start in.
    pub(crate) fn state_starts(&self, usage: ElementId) -> bool {
        self.model.type_of(usage).is_some_and(|def| {
            self.model.owned(def).iter().any(|&child| {
                self.model.kind(child) == ElementKind::StateUsage
                    && self.model.name(child).is_some()
            })
        })
    }
    /// A `state` usage of a part: the state its machine is in, as the
    /// generated enum, under the name the model gave the usage.
    pub(crate) fn state_field(&self, usage: ElementId) -> Option<(String, String)> {
        let def = self.model.type_of(usage)?;
        if self.shapes.get(&def) != Some(&Shape::StateMachine) {
            return None;
        }
        let name = self.model.name(usage)?.to_string();
        Some((name, format!("{}State", type_ident(self.model.name(def)?))))
    }
    /// An action definition's body, compiled out of what the model says its
    /// parts are and how they are wired: the subactions in the order the
    /// successions and flows put them in, each called with what the flows
    /// and bindings feed it, and the result read off the flows into the
    /// definition's own `out` parameters.
    ///
    /// The control flow is the model's too. A `decide` becomes an `if` over
    /// the guards of the ways out of it, so that one branch runs and not
    /// every one; an `if` action an `if`, a `while` or `loop` action a loop;
    /// a `fork`, `join` or `merge` only orders what is around it, since one
    /// order its branches may run in is to run them one after another.
    ///
    /// It is all-or-nothing. A dataflow with a gap in it, or a construct
    /// past what this writes, is not written half-way; the trait method
    /// stays open and what stopped it is named, because a modeller who is
    /// one flow short should not have to guess which.
    fn action_body(&self, def: ElementId) -> Result<Body, String> {
        let model = self.model;
        let mut wiring = Wiring {
            def,
            steps: Vec::new(),
            into: HashMap::new(),
            fanout: HashMap::new(),
            remaining: HashMap::new(),
            scope_of: HashMap::new(),
            blocks: 0,
        };
        self.gather(def, &mut wiring)?;
        if wiring.steps.is_empty() {
            // a leaf behaviour is not a dataflow with a gap in it; there
            // was never anything here to perform, and saying so on every
            // one of them is noise
            return Err(String::new());
        }
        for &source in wiring.into.values() {
            *wiring.fanout.entry(source).or_default() += 1;
        }
        // how many subactions each enclosing parameter is fed to
        let usages: Vec<ElementId> = wiring.steps.iter().map(|&(usage, _)| usage).collect();
        for parameter in self.in_parameters(def) {
            if let Some(name) = model.name(parameter) {
                let count = self.bindings_naming(&usages, name);
                wiring.remaining.insert(ident(name), count);
            }
        }

        let mut lines = self.sequence(def, &[], &mut wiring)?;

        // the result is whatever flows into the definition's own outs
        let mut results = Vec::new();
        for out in model.owned(def).iter().copied() {
            if model.direction(out) != Some("out") {
                continue;
            }
            let named = model.name(out).unwrap_or("?");
            let &(source, parameter) = wiring
                .into
                .get(&(None, out))
                .ok_or_else(|| format!("nothing produces `{named}`"))?;
            if wiring
                .scope_of
                .get(&source)
                .is_some_and(|at| !at.is_empty())
            {
                return Err(format!(
                    "`{named}` is produced only inside a branch or a loop, which leaves it \
                     unproduced on the way that does not run it"
                ));
            }
            results.push(
                self.read_out(source, parameter, &mut wiring.fanout, false)
                    .ok_or_else(|| format!("what flows into `{named}` is no result of its own"))?,
            );
        }
        match results.len() {
            0 => {}
            1 => lines.push(results.join("")),
            _ => lines.push(format!("({})", results.join(", "))),
        }

        let mut supertraits: Vec<String> = Vec::new();
        for &(_, performed) in &wiring.steps {
            if performed == def {
                // a behaviour made of itself is not a supertrait of
                // itself, and Rust reads that as a circle
                return Err("it is made of itself".to_string());
            }
            // and nor is one made of something made of it, however far
            // round: `A1: B1` with `B1: A1` is the same circle
            if self.performs_in_the_end(performed, def) {
                return Err(format!(
                    "it is made of itself, by way of `{}`",
                    model.name(performed).unwrap_or("?")
                ));
            }
            let name = type_ident(
                model
                    .name(performed)
                    .ok_or("a subaction performs something unnamed")?,
            );
            if !supertraits.contains(&name) {
                supertraits.push(name);
            }
        }
        Ok(Body { supertraits, lines })
    }
    /// Whether `from` performs `target`, itself or through what it is
    /// made of: every action usage it owns, at any depth, and the
    /// actions those perform in turn.
    fn performs_in_the_end(&self, from: ElementId, target: ElementId) -> bool {
        let model = self.model;
        let mut seen = HashSet::new();
        let mut waiting = vec![from];
        while let Some(def) = waiting.pop() {
            if !seen.insert(def) {
                continue;
            }
            let mut under = model.owned(def).to_vec();
            while let Some(child) = under.pop() {
                under.extend_from_slice(model.owned(child));
                if !model.kind(child).is_a(ElementKind::ActionUsage) {
                    continue;
                }
                if let Some(performed) = model.type_of(child) {
                    if performed == target {
                        return true;
                    }
                    waiting.push(performed);
                }
            }
        }
        false
    }
    /// Every subaction and flow of a behaviour, into the bodies of its
    /// `if` and loop actions too -- or what stops it from being written,
    /// named, where it holds something no generated body performs.
    fn gather(&self, container: ElementId, wiring: &mut Wiring) -> Result<(), String> {
        let model = self.model;
        for &child in model.owned(container) {
            if model.direction(child).is_some() {
                continue;
            }
            let kind = model.kind(child);
            match kind {
                ElementKind::ActionUsage | ElementKind::PerformActionUsage => {
                    let named = model.name(child).ok_or("a subaction with no name")?;
                    let performed = model
                        .type_of(child)
                        .ok_or_else(|| format!("`{named}` performs nothing"))?;
                    if self.shapes.get(&performed) != Some(&Shape::Action) {
                        return Err(format!("`{named}` performs no generated action"));
                    }
                    wiring.steps.push((child, performed));
                }
                ElementKind::IfActionUsage | ElementKind::WhileLoopActionUsage => {
                    self.gather_clauses(child, wiring)?;
                }
                ElementKind::ForkNode
                | ElementKind::JoinNode
                | ElementKind::MergeNode
                | ElementKind::DecisionNode
                | ElementKind::TransitionUsage => {}
                _ if kind.is_a(ElementKind::FlowUsage) => {
                    let (from, to) = self
                        .end_chains(child)
                        .ok_or("a flow whose ends did not resolve")?;
                    let [source, out] = from.as_slice() else {
                        return Err("a flow that does not come out of a subaction".to_string());
                    };
                    // `subaction.parameter` arrives at that subaction;
                    // anything else names the definition's own parameter,
                    // which is where a result comes from
                    let target = if let [usage, parameter] = to.as_slice() {
                        (Some(*usage), *parameter)
                    } else {
                        (None, *to.last().expect("a resolved end has a segment"))
                    };
                    wiring.into.insert(target, (*source, *out));
                }
                _ if kind.is_a(ElementKind::SuccessionAsUsage) => {}
                _ if kind.is_a(ElementKind::ActionUsage) => {
                    return Err(format!(
                        "{} is {} -- no generated body performs one",
                        model
                            .name(child)
                            .map_or_else(|| "a step of it".to_string(), |n| format!("`{n}`")),
                        construct(kind)
                    ));
                }
                _ => {}
            }
        }
        Ok(())
    }
    /// [`Self::gather`] over the bodies of one `if` or loop action.
    fn gather_clauses(&self, structured: ElementId, wiring: &mut Wiring) -> Result<(), String> {
        for clause in clauses(self.model, structured) {
            if self.model.kind(clause) == ElementKind::IfActionUsage {
                // `else if`: the clause is an `if` of its own
                self.gather_clauses(clause, wiring)?;
            } else if self.model.kind(clause).is_a(ElementKind::ActionUsage) {
                self.gather(clause, wiring)?;
            }
        }
        Ok(())
    }
    /// The statements one behaviour, or one body inside it, comes to: its
    /// steps and control nodes in the order its successions and flows
    /// allow. `scope` is the blocks the statements are written in,
    /// outermost first, each with whether it repeats.
    fn sequence(
        &self,
        container: ElementId,
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
    ) -> Result<Vec<String>, String> {
        let model = self.model;
        let nodes: Vec<ElementId> = model
            .owned(container)
            .iter()
            .copied()
            .filter(|&child| model.direction(child).is_none() && is_node(model.kind(child)))
            .collect();
        let edges = self.edges(container, &nodes, wiring)?;
        self.region(&nodes, &edges, scope, wiring)
    }
    /// The ways from one node of a behaviour to the next: its successions,
    /// guarded or not, and the flows, since what reads a result can only
    /// come after what produces it. Only those between two of `nodes`
    /// order anything here.
    fn edges(
        &self,
        container: ElementId,
        nodes: &[ElementId],
        wiring: &Wiring,
    ) -> Result<Vec<Edge>, String> {
        let model = self.model;
        let mut edges = Vec::new();
        for &child in model.owned(container) {
            let kind = model.kind(child);
            let (relates, guard) = if kind == ElementKind::TransitionUsage {
                // what a guarded succession relates it relates through the
                // succession it owns
                let relates = model
                    .owned(child)
                    .iter()
                    .copied()
                    .find(|&inner| model.kind(inner).is_a(ElementKind::SuccessionAsUsage))
                    .ok_or("a guarded succession whose ends did not resolve")?;
                let guard = match model.get(child, "guardExpression") {
                    Some(Value::RefList(guards)) => guards.first().copied(),
                    _ => None,
                };
                (relates, guard)
            } else if kind.is_a(ElementKind::SuccessionAsUsage) {
                (child, None)
            } else {
                continue;
            };
            let (from, to) = self
                .end_chains(relates)
                .ok_or("a succession whose ends did not resolve")?;
            // `end_chains` answers only for two ends that resolved, so
            // each has at least the one segment it resolved from
            let from = *from.first().expect("a resolved end has a segment");
            let to = *to.first().expect("a resolved end has a segment");
            if !nodes.contains(&from) || !nodes.contains(&to) {
                continue;
            }
            if guard.is_some() && model.kind(from) != ElementKind::DecisionNode {
                return Err(format!(
                    "the succession out of `{}` is guarded, and a guard is honoured here \
                     only on the way out of a `decide`",
                    model.name(from).unwrap_or("?")
                ));
            }
            edges.push(Edge {
                from,
                to,
                guard,
                flows: false,
            });
        }
        for (&(consumer, _), &(source, _)) in &wiring.into {
            if let Some(consumer) = consumer {
                if nodes.contains(&source) && nodes.contains(&consumer) {
                    edges.push(Edge {
                        from: source,
                        to: consumer,
                        guard: None,
                        flows: true,
                    });
                }
            }
        }
        // the flows came out of a map; what is written must not depend
        // on the order it hands them back in
        edges.sort_by_key(|edge| (edge.from, edge.to, edge.flows));
        edges.dedup_by_key(|edge| (edge.from, edge.to, edge.flows));
        Ok(edges)
    }
    /// The statements a set of nodes comes to, in an order the edges
    /// between them allow.
    fn region(
        &self,
        nodes: &[ElementId],
        edges: &[Edge],
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
    ) -> Result<Vec<String>, String> {
        let model = self.model;
        let mut after: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
        for edge in edges {
            if nodes.contains(&edge.from) && nodes.contains(&edge.to) {
                after.entry(edge.from).or_default().push(edge.to);
            }
        }
        let order =
            ordered(nodes, &after).ok_or("its subactions follow one another in a circle")?;
        let mut done: HashSet<ElementId> = HashSet::new();
        let mut lines = Vec::new();
        for node in order {
            if !done.insert(node) {
                continue;
            }
            match model.kind(node) {
                ElementKind::ActionUsage | ElementKind::PerformActionUsage => {
                    lines.push(self.call(node, scope, wiring)?);
                }
                ElementKind::DecisionNode => {
                    lines.extend(self.decision(node, nodes, edges, scope, wiring, &mut done)?);
                }
                ElementKind::IfActionUsage => {
                    lines.extend(self.if_action(node, scope, wiring)?);
                }
                ElementKind::WhileLoopActionUsage => {
                    lines.extend(self.loop_action(node, scope, wiring)?);
                }
                // a fork, a join or a merge orders what is around it and
                // does nothing of its own
                _ => {}
            }
        }
        Ok(lines)
    }
    /// One subaction, called with what feeds it.
    fn call(
        &self,
        usage: ElementId,
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
    ) -> Result<String, String> {
        let model = self.model;
        let performed = wiring
            .steps
            .iter()
            .find(|&&(u, _)| u == usage)
            .map(|&(_, performed)| performed)
            .expect("gathered before it is called");
        let mine = model.name(usage).unwrap_or("?");
        wiring.scope_of.insert(usage, scope.to_vec());
        let mut arguments = Vec::new();
        for parameter in self.in_parameters(performed) {
            let named = model.name(parameter).unwrap_or("?");
            // what a step inside a loop reads from outside it is read
            // again on every pass, so it cannot be taken on the first
            let repeats = match wiring.into.get(&(Some(usage), parameter)) {
                Some(&(source, _)) => {
                    let source_name = model.name(source).unwrap_or("?");
                    let Some(at) = wiring.scope_of.get(&source) else {
                        return Err(format!(
                            "`{mine}.{named}` reads what `{source_name}` produces, which \
                             nothing runs before it"
                        ));
                    };
                    if !scope.starts_with(at) {
                        return Err(format!(
                            "`{mine}.{named}` reads what `{source_name}` produces only inside \
                             a branch or a loop `{mine}` is not in"
                        ));
                    }
                    scope[at.len()..].iter().any(|&(_, repeats)| repeats)
                }
                None => scope.iter().any(|&(_, repeats)| repeats),
            };
            arguments.push(
                self.fed_by(usage, parameter, wiring, repeats)
                    .ok_or_else(|| format!("nothing feeds `{mine}.{named}`"))?,
            );
        }
        Ok(format!(
            "let {} = self.{}({});",
            ident(mine),
            ident(model.name(performed).unwrap_or("?")),
            arguments.join(", ")
        ))
    }
    /// A `decide` as an `if` over the guards of the ways out of it, each
    /// way running what only it leads to, up to a `merge`.
    fn decision(
        &self,
        node: ElementId,
        nodes: &[ElementId],
        every: &[Edge],
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
        done: &mut HashSet<ElementId>,
    ) -> Result<Vec<String>, String> {
        let model = self.model;
        let named = model.name(node).unwrap_or("?");
        // a flow orders what reads after what it reads, and says nothing
        // of which way a decision goes
        let edges: Vec<&Edge> = every.iter().filter(|edge| !edge.flows).collect();
        let ways: Vec<&Edge> = edges
            .iter()
            .copied()
            .filter(|edge| edge.from == node)
            .collect();
        // the guarded ways first, in the order the model wrote them, and
        // the one with no guard -- the `else` -- last
        let mut guarded: Vec<&Edge> = ways.iter().copied().filter(|w| w.guard.is_some()).collect();
        guarded.sort_by_key(|way| way.guard);
        let otherwise: Vec<&Edge> = ways.iter().copied().filter(|w| w.guard.is_none()).collect();
        if otherwise.len() > 1 {
            return Err(format!(
                "`{named}` has more than one way out with no guard, and a decision takes one"
            ));
        }
        let mut claimed: HashSet<ElementId> = HashSet::new();
        let mut branches: Vec<(Option<String>, Vec<ElementId>)> = Vec::new();
        for way in guarded.iter().chain(&otherwise) {
            // what only this way leads to, up to where the ways merge
            let mut reached: Vec<ElementId> = Vec::new();
            let mut stack = vec![way.to];
            while let Some(at) = stack.pop() {
                if reached.contains(&at) || model.kind(at) == ElementKind::MergeNode {
                    continue;
                }
                reached.push(at);
                stack.extend(edges.iter().filter(|e| e.from == at).map(|e| e.to));
            }
            for &at in &reached {
                let joins = edges
                    .iter()
                    .filter(|e| e.to == at)
                    .any(|e| e.from != node && !reached.contains(&e.from));
                if joins || !claimed.insert(at) || done.contains(&at) {
                    return Err(format!(
                        "the ways out of `{named}` meet again other than at a `merge`, \
                         at `{}`",
                        model.name(at).unwrap_or("?")
                    ));
                }
            }
            let condition = match way.guard {
                Some(guard) => Some(self.condition(guard, wiring.def)?),
                None => None,
            };
            let mut ordered_region: Vec<ElementId> = nodes
                .iter()
                .copied()
                .filter(|n| reached.contains(n))
                .collect();
            ordered_region.dedup();
            branches.push((condition, ordered_region));
        }
        let mut lines = Vec::new();
        for (at, (condition, region)) in branches.iter().enumerate() {
            done.extend(region.iter().copied());
            let mut inner = scope.to_vec();
            wiring.blocks += 1;
            inner.push((wiring.blocks, false));
            let body = self.region(region, every, &inner, wiring)?;
            let opening = match (at, condition) {
                (0, Some(condition)) => format!("if {condition} {{"),
                (_, Some(condition)) => format!("}} else if {condition} {{"),
                // nothing but an `else`: the one way there is, always
                (0, None) => "{".to_string(),
                (_, None) => "} else {".to_string(),
            };
            lines.push(opening);
            lines.extend(body.into_iter().map(|line| format!("    {line}")));
        }
        if !branches.is_empty() {
            lines.push("}".to_string());
        }
        Ok(lines)
    }
    /// An `if` action as an `if`, its `else` -- a body or another `if` --
    /// as an `else`.
    fn if_action(
        &self,
        node: ElementId,
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
    ) -> Result<Vec<String>, String> {
        let model = self.model;
        let clauses = clauses(model, node);
        let [condition, then, rest @ ..] = clauses.as_slice() else {
            return Err("an `if` with no condition or no body".to_string());
        };
        if !model.kind(*condition).is_a(ElementKind::Expression) {
            return Err("an `if` with no condition".to_string());
        }
        let condition = self.condition(*condition, wiring.def)?;
        let mut lines = vec![format!("if {condition} {{")];
        lines.extend(self.block(*then, scope, false, wiring)?);
        match rest {
            [] => {}
            [otherwise] if model.kind(*otherwise) == ElementKind::IfActionUsage => {
                let nested = self.if_action(*otherwise, scope, wiring)?;
                lines.push(format!("}} else {}", nested[0]));
                lines.extend(nested[1..nested.len() - 1].iter().cloned());
            }
            [otherwise] => {
                lines.push("} else {".to_string());
                lines.extend(self.block(*otherwise, scope, false, wiring)?);
            }
            _ => return Err("an `if` with more than one `else`".to_string()),
        }
        lines.push("}".to_string());
        Ok(lines)
    }
    /// A `while` action as a `while`, a `loop` as a `loop`, and an `until`
    /// as the `break` at the end of the body.
    fn loop_action(
        &self,
        node: ElementId,
        scope: &[(usize, bool)],
        wiring: &mut Wiring,
    ) -> Result<Vec<String>, String> {
        let model = self.model;
        let clauses = clauses(model, node);
        let [condition, body, rest @ ..] = clauses.as_slice() else {
            return Err("a loop with no body".to_string());
        };
        // `loop` writes no condition, and the model holds an empty
        // parameter where `while` holds one
        let mut lines = vec![if model.kind(*condition).is_a(ElementKind::Expression) {
            format!("while {} {{", self.condition(*condition, wiring.def)?)
        } else {
            "loop {".to_string()
        }];
        lines.extend(self.block(*body, scope, true, wiring)?);
        match rest {
            [] => {}
            [until] if model.kind(*until).is_a(ElementKind::Expression) => {
                lines.push(format!("    if {} {{", self.condition(*until, wiring.def)?));
                lines.push("        break;".to_string());
                lines.push("    }".to_string());
            }
            _ => return Err("a loop whose `until` is no expression".to_string()),
        }
        lines.push("}".to_string());
        Ok(lines)
    }
    /// The statements of one body of an `if` or a loop, indented into it.
    fn block(
        &self,
        body: ElementId,
        scope: &[(usize, bool)],
        repeats: bool,
        wiring: &mut Wiring,
    ) -> Result<Vec<String>, String> {
        let mut inner = scope.to_vec();
        wiring.blocks += 1;
        inner.push((wiring.blocks, repeats));
        Ok(self
            .sequence(body, &inner, wiring)?
            .into_iter()
            .map(|line| format!("    {line}"))
            .collect())
    }
    /// A guard or a condition of a behaviour as the Rust `bool` it is,
    /// over the behaviour's own `in` parameters -- or why it is not one.
    fn condition(&self, expression: ElementId, def: ElementId) -> Result<String, String> {
        let model = self.model;
        let Some(text) = expression_text(model, expression) else {
            return Err("a condition the model holds no text for".to_string());
        };
        let parameters = self.in_parameters(def);
        let resolve = |chain: &str| {
            expr::led_by(chain, |leading: &str| {
                parameters
                    .iter()
                    .find(|&&p| model.name(p) == Some(leading))
                    .map(|_| ident(leading))
            })
        };
        let translated = translate(&text, &resolve, &|name| self.callable(expression, name));
        let flag = |rust: &str| {
            parameters.iter().any(|&p| {
                model.name(p).map(ident).as_deref() == Some(rust)
                    && self.parameter_type(p).as_deref() == Some("bool")
            })
        };
        match translated {
            Some(translated) if translated.boolean || flag(&translated.rust) => Ok(translated.rust),
            _ => Err(format!(
                "the condition `{}` is past what this generator translates into a `bool`",
                text.trim()
            )),
        }
    }
    /// The `in` parameters of an action definition, in declaration order.
    fn in_parameters(&self, def: ElementId) -> Vec<ElementId> {
        self.model
            .owned(def)
            .iter()
            .copied()
            .filter(|&child| self.model.direction(child) == Some("in"))
            .filter(|&child| self.model.name(child).is_some())
            .collect()
    }
    /// What feeds one parameter of one subaction: a flow out of another,
    /// or a value the usage bound it to. `repeats` says the call is read
    /// again on every pass of a loop the value comes from outside of, so
    /// whatever is not a copy is cloned rather than taken.
    fn fed_by(
        &self,
        usage: ElementId,
        parameter: ElementId,
        wiring: &mut Wiring,
        repeats: bool,
    ) -> Option<String> {
        if let Some(&(source, out)) = wiring.into.get(&(Some(usage), parameter)) {
            return self.read_out(source, out, &mut wiring.fanout, repeats);
        }
        // `action wash : Washout { in stream = TrainingRun::stream; }`
        let name = self.model.name(parameter)?;
        let bound = self
            .model
            .owned(usage)
            .iter()
            .copied()
            .find(|&child| self.model.name(child) == Some(name))
            .and_then(|child| value_clause(self.model, child))?;
        let parameters = self.in_parameters(wiring.def);
        let resolve = |chain: &str| {
            expr::led_by(chain, |leading: &str| {
                parameters
                    .iter()
                    .find(|&&p| self.model.name(p) == Some(leading))
                    .map(|_| ident(leading))
            })
        };
        let read = match bound {
            // `TrainingRun::stream` names the enclosing action's own
            // parameter, and the argument is what Rust calls it
            ValueClause::Text(text) => {
                let tail = text.rsplit("::").next()?.trim().to_string();
                translate(&tail, &resolve, &|name| self.callable(usage, name))?.rust
            }
            ValueClause::Literal(rust) => rust,
        };
        // a parameter fed to more than one subaction is cloned until the
        // last of them, which may take it -- unless it is a scalar, which
        // every call copies anyway
        let left = wiring.remaining.entry(read.clone()).or_insert(0);
        *left = left.saturating_sub(1);
        let copyable = self
            .parameter_shape(parameter)
            .is_some_and(|(base, _)| copyable(&base));
        Some(if (*left > 0 || repeats) && !copyable {
            format!("{read}.clone()")
        } else {
            read
        })
    }
    /// How many of the subactions bind one of the enclosing parameters.
    fn bindings_naming(&self, usages: &[ElementId], name: &str) -> usize {
        usages
            .iter()
            .flat_map(|&child| self.model.owned(child).iter().copied())
            .filter(|&bound| match value_clause(self.model, bound) {
                Some(ValueClause::Text(text)) => {
                    text.rsplit("::").next().map(str::trim) == Some(name)
                }
                _ => false,
            })
            .count()
    }
    /// One `out` of an earlier subaction: the call answered with a tuple
    /// where its definition declares several, in declaration order.
    fn read_out(
        &self,
        source: ElementId,
        parameter: ElementId,
        fanout: &mut HashMap<(ElementId, ElementId), usize>,
        repeats: bool,
    ) -> Option<String> {
        let performed = self.model.type_of(source)?;
        let outs: Vec<ElementId> = self
            .model
            .owned(performed)
            .iter()
            .copied()
            .filter(|&child| self.model.direction(child) == Some("out"))
            .collect();
        // A flow comes out of what it names, so where the end resolved to
        // an `in` of the same name -- which an action that transforms its
        // input has -- the `out` is what was meant.
        let at = outs.iter().position(|&out| out == parameter).or_else(|| {
            let named = self.model.name(parameter)?;
            outs.iter()
                .position(|&out| self.model.name(out) == Some(named))
        });
        let read = match outs.len() {
            // a flow comes out of an `out`, so there is at least one
            1 => ident(self.model.name(source)?),
            _ => format!("{}.{}", ident(self.model.name(source)?), at?),
        };
        // one output feeding two consumers is read by clone until the
        // last of them, which may take it -- and a scalar is copied by
        // every call anyway
        let copyable = self
            .parameter_shape(parameter)
            .is_some_and(|(base, _)| copyable(&base));
        let left = fanout.entry((source, parameter)).or_insert(0);
        *left = left.saturating_sub(1);
        Some(if !copyable && (*left > 0 || repeats) {
            format!("{read}.clone()")
        } else {
            read
        })
    }
    /// The two ends of a connector, each as the chain its operand
    /// resolved to: `wash.washedState` is `[wash, washedState]`, and a
    /// bare `weights` is `[weights]`. The final target alone cannot say
    /// which subaction an end belongs to, which is exactly what wiring a
    /// dataflow needs to know.
    fn end_chains(&self, usage: ElementId) -> Option<(Vec<ElementId>, Vec<ElementId>)> {
        let chains: Vec<Vec<ElementId>> = self
            .model
            .owned(usage)
            .iter()
            .filter(|&&child| self.model.flag(child, "isEnd"))
            .map(|&child| sysml_model::end_reaches(self.model, child))
            .filter(|chain| !chain.is_empty())
            .collect();
        match chains.as_slice() {
            [from, to] => Some((from.clone(), to.clone())),
            _ => None,
        }
    }
    /// The two ends of a succession or a flow, as the model resolved
    /// them -- `relatedFeature` is where the semantics pass records what
    /// the operands of `first ... then ...` and `flow ... to ...` point
    /// at.
    pub(crate) fn ends_of(&self, usage: ElementId) -> Option<String> {
        let ends = self.model.related_feature(usage);
        let named: Vec<&str> = ends
            .iter()
            .filter_map(|&end| self.model.name(end))
            .collect();
        (named.len() == 2).then(|| format!("`{}` -> `{}`", named[0], named[1]))
    }
    /// What an action definition's `out` parameters make of its result:
    /// nothing, the one type, or a tuple of them.
    pub(crate) fn action_result(&self, def: ElementId) -> Option<String> {
        let model = self.model;
        let mut outputs = Vec::new();
        for &child in model.owned(def) {
            if model.direction(child) != Some("out") {
                continue;
            }
            outputs.push(self.parameter_type(child)?);
        }
        Some(match outputs.len() {
            0 => "()".to_string(),
            1 => outputs.join(""),
            _ => format!("({})", outputs.join(", ")),
        })
    }
    /// An `action def` as a trait with one method: `in` parameters become
    /// its arguments and `out` parameters its result -- a tuple where
    /// there are several. A behaviour assembled from subactions, flows
    /// and successions is not something this generator can write, so it
    /// asks for it the way it asks for an abstract calculation.
    pub(crate) fn action(&self, def: ElementId, out: &mut String) {
        let model = self.model;
        let spelled = model.name(def).expect("collected named");
        let name = type_ident(spelled);
        let mut inputs: Vec<String> = Vec::new();
        for &child in model.owned(def) {
            if model.direction(child) != Some("in") {
                continue;
            }
            let Some(param) = model.name(child) else {
                continue;
            };
            let Some(ty) = self.parameter_type(child) else {
                self.open(
                    OpenKind::Skipped,
                    name.clone(),
                    None,
                    format!("parameter `{param}` has no Rust type"),
                );
                writeln!(
                    out,
                    "\n// not generated: action def `{name}` -- parameter `{param}` has no \
                     Rust type"
                )
                .unwrap();
                return;
            };
            inputs.push(format!("{}: {ty}", ident(param)));
        }
        let Some(returns) = self.action_result(def) else {
            self.open(
                OpenKind::Skipped,
                name.clone(),
                None,
                "a result of it has no Rust type",
            );
            writeln!(
                out,
                "\n// not generated: action def `{name}` -- a result of it has no Rust type"
            )
            .unwrap();
            return;
        };
        let signature = std::iter::once("&mut self".to_string())
            .chain(inputs)
            .collect::<Vec<_>>()
            .join(", ");

        writeln!(out).unwrap();
        if let Some(doc) = documentation(model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: `action def {name}`").unwrap();
        writeln!(
            out,
            "/// The model names this behaviour without saying how it runs; \
             the implementation is yours."
        )
        .unwrap();
        // What the model does say is what the behaviour is made of. The
        // generator cannot compile a dataflow into a body, but it can
        // hand the implementor the model's own account of one instead of
        // making them go and read it.
        for step in self.decomposition(def) {
            writeln!(out, "/// {step}").unwrap();
        }
        // Where the model wired its parts up completely, that wiring is
        // the body: the implementation owes the parts, not the whole.
        let body = self.action_body(def);
        if let Err(why) = &body {
            if !why.is_empty() {
                writeln!(out, "/// Not performed here: {why}.").unwrap();
            }
        }
        let supertraits = body
            .as_ref()
            .ok()
            .filter(|body| !body.supertraits.is_empty())
            .map(|body| format!(": {}", body.supertraits.join(" + ")))
            .unwrap_or_default();
        self.open(
            OpenKind::Trait,
            self.model.name(def).unwrap_or(&name),
            Some(name.clone()),
            match &body {
                Ok(_) => "the model wired its parts up, so the parts are what is owed",
                Err(_) => "the model names the behaviour without saying how it runs",
            },
        );
        writeln!(out, "pub trait {name}{supertraits} {{").unwrap();
        match &body {
            Ok(body) => {
                writeln!(
                    out,
                    "    fn {}({signature}){} {{",
                    ident(spelled),
                    returns_clause(&returns)
                )
                .unwrap();
                for line in &body.lines {
                    writeln!(out, "        {line}").unwrap();
                }
                writeln!(out, "    }}").unwrap();
            }
            Err(_) => writeln!(
                out,
                "    fn {}({signature}){};",
                ident(spelled),
                returns_clause(&returns)
            )
            .unwrap(),
        }
        writeln!(out, "}}").unwrap();
    }
    /// A `calc` usage typed by a calculation definition, as a method that
    /// performs it: the parameters the usage bound are read off the part,
    /// the rest stay arguments of the method, and the calculation itself
    /// is called outright where the definition had a formula, or arrives
    /// as an implementation of the trait an `abstract` one generated.
    fn calc_delegation(
        &self,
        usage: ElementId,
        def: ElementId,
        fields: &[Field],
    ) -> Result<String, String> {
        let model = self.model;
        let name = &self.member_name(usage).expect("named, or it was skipped");
        let def_name = type_ident(model.name(def).expect("collected named"));
        let returns = model
            .owned(def)
            .iter()
            .copied()
            .find(|&child| model.member_role(child) == Some(Role::Return))
            .and_then(|child| self.parameter_type(child));
        let Some(returns) = returns else {
            return Err(format!(
                "calc `{name}` -- the result of `{def_name}` has no Rust type"
            ));
        };
        // an abstract definition generated a trait, so the caller brings
        // the implementation along; a concrete one generated a function
        let performer = model.is_abstract(def);
        self.delegation(usage, def, fields, "calc", &returns, performer, "&")
    }
    /// The body common to every performance of a definition: pair each of
    /// the definition's `in` parameters with what the usage bound it to,
    /// leave the rest for the caller, and write the method around them.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn delegation(
        &self,
        usage: ElementId,
        def: ElementId,
        fields: &[Field],
        label: &str,
        returns: &str,
        performer: bool,
        provider_ref: &str,
    ) -> Result<String, String> {
        let model = self.model;
        let name = &self.member_name(usage).expect("named, or it was skipped");
        let spelled = model.name(def).expect("collected named");
        let def_name = type_ident(spelled);
        let resolve = |chain: &str| self.through_fields(fields, chain);

        let mut params: Vec<(String, String)> = Vec::new();
        let mut arguments: Vec<String> = Vec::new();
        for &child in model.owned(def) {
            if model.direction(child) != Some("in") {
                continue;
            }
            let Some(param) = model.name(child) else {
                continue;
            };
            let Some(ty) = self.parameter_type(child) else {
                return Err(format!(
                    "{label} `{name}` -- parameter `{param}` of `{def_name}` has no Rust type"
                ));
            };
            // whatever the usage did not bind, the caller still owes
            let bound = model
                .owned(usage)
                .iter()
                .copied()
                .find(|&sibling| model.name(sibling) == Some(param))
                .and_then(|sibling| value_clause(model, sibling));
            let Some(clause) = bound else {
                arguments.push(ident(param));
                params.push((param.to_string(), ty));
                continue;
            };
            let read = match clause {
                ValueClause::Literal(rust) => rust,
                // A parameter bound to a feature of the very part that
                // owns it has to be spelled `Part::feature`, since the
                // bare name would be the parameter itself. What Rust
                // reads is the feature, so the qualifier is dropped where
                // the tail is one this part has.
                ValueClause::Text(text) => {
                    let tail = text.rsplit("::").next().unwrap_or(&text).trim();
                    let read = resolve(tail)
                        .map(|reached| Translated::plain(reached.rust))
                        .or_else(|| translate(&text, &resolve, &|name| self.callable(usage, name)));
                    match read {
                        Some(translated) => translated.rust,
                        None => {
                            return Err(format!(
                                "{label} `{name}` -- `{param}` is bound to `{text}`, beyond the \
                             simple subset"
                            ))
                        }
                    }
                }
            };
            let Some(argument) = self.owned_argument(read, &ty, child) else {
                return Err(format!(
                    "{label} `{name}` -- `{param}` is read off the part, and `{ty}` cannot be \
                     cloned into the call"
                ));
            };
            arguments.push(argument);
        }

        let provider = if params.iter().any(|(param, _)| param == "with") {
            "with_"
        } else {
            "with"
        };
        let mut signature = String::from("&self");
        if performer {
            write!(signature, ", {provider}: {provider_ref}impl {def_name}").unwrap();
        }
        for (param, ty) in &params {
            write!(signature, ", {}: {ty}", ident(param)).unwrap();
        }
        let arguments = arguments.join(", ");
        let call = if performer {
            format!("{provider}.{}({arguments})", ident(spelled))
        } else {
            format!("{}({arguments})", ident(spelled))
        };

        let mut method = String::new();
        writeln!(method, "    /// SysML: `{label} {name} : {def_name}`").unwrap();
        method.push_str(&self.unnamed_note(usage));
        writeln!(
            method,
            "    pub fn {}({signature}){} {{",
            ident(name),
            returns_clause(returns)
        )
        .unwrap();
        writeln!(method, "        {call}").unwrap();
        writeln!(method, "    }}").unwrap();
        Ok(method)
    }
    /// A bound argument as something the call can take by value: reading
    /// a field has to clone it, since the method only borrows the part.
    fn owned_argument(&self, read: String, ty: &str, param: ElementId) -> Option<String> {
        let copyable = copyable(ty);
        let field_read = read.strip_prefix("self.").is_some_and(|rest| {
            rest.chars()
                .all(|ch| ch.is_alphanumeric() || ch == '_' || ch == '#')
        });
        if copyable || !field_read {
            return Some(read);
        }
        let clonable = ty == "String"
            || self
                .model
                .type_of(param)
                .is_some_and(|target| self.derivable.contains(&target));
        clonable.then(|| format!("{read}.clone()"))
    }
}

/// What an action body is being compiled from, and what has been written
/// of it so far.
struct Wiring {
    /// The definition whose body this is.
    def: ElementId,
    /// Every subaction, nested ones too, and what it performs.
    steps: Vec<(ElementId, ElementId)>,
    /// Every flow, by the (subaction, parameter) it arrives at.
    into: HashMap<(Option<ElementId>, ElementId), (ElementId, ElementId)>,
    /// How many reads each output has left.
    fanout: HashMap<(ElementId, ElementId), usize>,
    /// How many subactions each enclosing parameter is still fed to.
    remaining: HashMap<String, usize>,
    /// The blocks each written subaction sits in, outermost first, each
    /// with whether it repeats: what it produces is in Rust's scope only
    /// inside them.
    scope_of: HashMap<ElementId, Vec<(usize, bool)>>,
    /// Blocks opened so far, to tell them apart.
    blocks: usize,
}

/// One way from a node of a behaviour to the next.
struct Edge {
    from: ElementId,
    to: ElementId,
    /// The guard on it, where it is a guarded succession.
    guard: Option<ElementId>,
    /// Whether it is a flow, which orders but does not branch.
    flows: bool,
}

/// What can stand in a behaviour's order: a subaction, a control node, or
/// an `if` or loop action.
fn is_node(kind: ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::ActionUsage
            | ElementKind::PerformActionUsage
            | ElementKind::ForkNode
            | ElementKind::JoinNode
            | ElementKind::MergeNode
            | ElementKind::DecisionNode
            | ElementKind::IfActionUsage
            | ElementKind::WhileLoopActionUsage
    )
}

/// The clauses of an `if` or loop action, in the order the model holds
/// them: the condition, the body, and the `else` or the `until`.
fn clauses(model: &Model, structured: ElementId) -> Vec<ElementId> {
    model
        .owned(structured)
        .iter()
        .copied()
        .filter(|&child| model.direction(child) == Some("in"))
        .collect()
}

/// A step of a behaviour no generated body performs, as a reader would
/// call it.
fn construct(kind: ElementKind) -> String {
    match kind {
        ElementKind::SendActionUsage => "a `send`".to_string(),
        ElementKind::AcceptActionUsage => "an `accept`".to_string(),
        ElementKind::AssignmentActionUsage => "an assignment".to_string(),
        ElementKind::ForLoopActionUsage => "a `for` loop".to_string(),
        ElementKind::TerminateActionUsage => "a `terminate`".to_string(),
        other => format!("a {other:?}"),
    }
}
