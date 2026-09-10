//! What a definition becomes: a struct, an enum, or a trait.
//!
//! A `part def` is a struct whose fields are the usages it declares,
//! with multiplicities as containers, declared values as `Default` and
//! inheritance flattened -- Rust has no subtyping, so what a definition
//! specializes is written out into it. An enumeration is an enum, and a
//! `variation` is the enum of its variants. What the model left abstract
//! becomes a trait instead, since the implementation belongs beside the
//! generated file rather than in it.

use std::collections::HashSet;

use sysml_model::{ElementId, ElementKind, Value};

use super::*;

impl<'a> Generator<'a> {
    /// Does this struct carry generic parameters, its own or inherited
    /// from what it composes?
    pub(crate) fn generic(&self, def: ElementId) -> bool {
        self.plans
            .get(&def)
            .is_some_and(|plan| !plan.params.is_empty())
    }
    /// One definition, whatever its shape.
    pub(crate) fn definition(&self, def: ElementId, out: &mut String) -> Result<(), RustgenError> {
        match self.shapes[&def] {
            Shape::Struct => self.structure(def, out),
            Shape::Enum => {
                self.enumeration(def, out);
                Ok(())
            }
            Shape::Variation => {
                self.variation(def, out);
                Ok(())
            }
            Shape::StateMachine => {
                self.state_machine(def, out);
                Ok(())
            }
            Shape::Calculation => {
                self.calculation(def, out);
                Ok(())
            }
            Shape::Action => {
                self.action(def, out);
                Ok(())
            }
            Shape::Flattened => {
                let name = type_ident(self.model.name(def).expect("collected named"));
                writeln!(
                    out,
                    "\n// `{name}` is abstract: its features flatten into its subtypes."
                )
                .unwrap();
                Ok(())
            }
        }
    }
    /// The flattened data fields of a struct definition, settled where
    /// they have been and worked out where they have not -- the phases
    /// that run before [`Generator::settle_fields`] ask too.
    pub(crate) fn fields(&self, def: ElementId) -> Option<Vec<Field>> {
        match self.fields.get(&def) {
            Some(settled) => settled.clone(),
            None => self.walk_fields(def),
        }
    }
    /// A definition's own attributes and compositions, then what its
    /// specializations hand down, redefinitions and same names
    /// shadowing outward.
    ///
    /// `None` marks a specialization cycle, which cannot flatten.
    pub(crate) fn walk_fields(&self, def: ElementId) -> Option<Vec<Field>> {
        if self.specializes_itself(def, &mut Vec::new(), &mut HashSet::new()) {
            return None;
        }
        let mut fields = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        let mut chain = vec![def];
        let mut visited = HashSet::new();
        while let Some(level) = chain.pop() {
            // diamond inheritance reaches one general along two paths;
            // it hands its fields down once, not twice
            if !visited.insert(level) {
                continue;
            }
            for &child in self.model.owned(level) {
                if !matches!(
                    self.model.kind(child),
                    ElementKind::AttributeUsage
                        | ElementKind::PartUsage
                        | ElementKind::ItemUsage
                        // the `end`s an interface or connection definition
                        // declares are references, and they are what the
                        // definition is made of
                        | ElementKind::ReferenceUsage
                ) {
                    continue;
                }
                let Some(field) = self.field(def, level, child) else {
                    continue;
                };
                if taken.insert(field.name.clone()) {
                    fields.push(field);
                }
            }
            chain.extend(self.generals(level));
        }
        Some(fields)
    }
    /// What `def` names as its immediate generals, off the reified
    /// subclassifications.
    fn generals(&self, def: ElementId) -> Vec<ElementId> {
        self.model
            .owned(def)
            .iter()
            .filter(|&&child| self.model.kind(child) == ElementKind::Subclassification)
            .filter_map(|&child| {
                self.model
                    .maybe(child, "superclassifier")
                    .and_then(Value::as_id)
            })
            .collect()
    }
    /// Whether specializing leads back to something already on the way
    /// here. Only a circle stops a definition flattening: a diamond
    /// arrives at one general twice but never at itself, and `fields`
    /// takes what it hands down the first time it gets there.
    fn specializes_itself(
        &self,
        def: ElementId,
        path: &mut Vec<ElementId>,
        settled: &mut HashSet<ElementId>,
    ) -> bool {
        if path.contains(&def) {
            return true;
        }
        if !settled.insert(def) {
            return false;
        }
        path.push(def);
        let circular = self
            .generals(def)
            .into_iter()
            .any(|general| self.specializes_itself(general, path, settled));
        path.pop();
        circular
    }
    /// Whether a definition is written as something a signature can
    /// name. An abstract definition flattens into its subtypes and is
    /// never written; an action or an abstract calculation becomes a
    /// trait, which is not a type; a state definition becomes three
    /// things, none of them called what the definition is called.
    pub(crate) fn written_as_a_type(&self, def: ElementId) -> bool {
        matches!(
            self.shapes.get(&def),
            Some(Shape::Struct | Shape::Enum | Shape::Variation)
        )
    }
    /// One attribute or composed part as a field, if it has a Rust type.
    fn field(&self, def: ElementId, level: ElementId, usage: ElementId) -> Option<Field> {
        // An end name resolution reified stands for what a connector
        // reaches, not for anything the source declared. It carries no
        // name of its own -- what it reaches lends it one -- so what
        // tells it from a member written as a reference is that it is
        // an end.
        if self.model.name(usage).is_none() && self.model.flag(usage, "isEnd") {
            return None;
        }
        // an unnamed redefinition answers to the name it redefines, and
        // KerML's rule for that -- which follows references as well as
        // redefinitions, and a chain of either -- is the model's own
        let name = self.model.effective_name(usage)?.to_string();
        let target = self
            .model
            .type_of(usage)
            .or_else(|| self.model.type_of(redefined(self.model, usage)?))?;
        let ty = if let Some(bound) = binding(self.model, target) {
            FieldType::External(bound.get(binding::ITEM)?.clone())
        } else if matches!(
            self.shapes.get(&target),
            Some(Shape::Struct | Shape::Enum | Shape::Variation)
        ) {
            // a state machine or an abstract definition is not a value type
            FieldType::Generated(target)
        } else {
            FieldType::Scalar(scalar_of(self.model.name(target)?)?)
        };
        let container = multiplicity(self.model, usage);
        // anything but a Vec keeps the value inline, so a cycle needs a Box
        let boxed = container != Container::Many
            && (self.boxed.contains(&(level, target))
                || matches!(ty, FieldType::Generated(t) if t == def));
        Some(Field {
            spelled: ident(&name),
            name,
            usage,
            ty,
            container,
            boxed,
            default: default_of(self.model, usage),
        })
    }
    /// A `part def` and friends as a struct, its API ports as generics
    /// and its performed bound actions as methods.
    fn structure(&self, def: ElementId, out: &mut String) -> Result<(), RustgenError> {
        let model = self.model;
        let def_name = type_ident(model.name(def).expect("collected named"));
        let mut ports: Vec<Port> = Vec::new();
        let mut plain_ports: Vec<Field> = Vec::new();
        let mut performs = Vec::new();
        let mut calcs = Vec::new();
        let mut asserts = Vec::new();
        let mut states: Vec<(String, String)> = Vec::new();
        let mut notes: Vec<String> = Vec::new();

        let mut fields = self.fields(def).expect("settled as buildable");
        for &child in model.owned(def) {
            if model.kind(child).is_a(ElementKind::Relationship)
                || model.kind(child) == ElementKind::MetadataUsage
            {
                continue;
            }
            let Some(child_name) = model.name(child) else {
                continue;
            };
            match model.kind(child) {
                ElementKind::PortUsage => {
                    if let Some(port) = self.port_of(child) {
                        ports.push(port);
                    } else if let Some(field) = self.plain_port(def, child) {
                        // an unbound port holds the generated struct of
                        // its port definition
                        plain_ports.push(field);
                    } else {
                        notes.push(format!(
                            "port `{child_name}` -- its type is neither bound nor generated"
                        ));
                    }
                }
                ElementKind::PerformActionUsage => performs.push(child),
                ElementKind::CalculationUsage => calcs.push(child),
                ElementKind::AssertConstraintUsage => asserts.push(child),
                ElementKind::StateUsage => match self.state_field(child) {
                    Some(state) => states.push(state),
                    None => notes.push(format!(
                        "state `{child_name}` -- its definition is no generated state machine"
                    )),
                },
                // a nested definition is generated at the top level
                kind if kind.is_a(ElementKind::Definition) => {}
                ElementKind::AttributeUsage
                | ElementKind::PartUsage
                | ElementKind::ItemUsage
                | ElementKind::ReferenceUsage => {
                    if !fields.iter().any(|field| {
                        field.usage == child || Some(field.usage) == redefined(model, child)
                    }) && self.field(def, def, child).is_none()
                    {
                        notes.push(format!("`{child_name}` -- no Rust type for its SysML type"));
                    }
                }
                other => notes.push(format!("`{child_name}` -- {} not generated", other.name())),
            }
        }
        // Every member of the struct is spelled once here, in the order
        // they are written, so that three model names arriving at one
        // Rust name become three fields rather than one declared thrice.
        let mut spelling = Namespace::default();
        let port_names: Vec<String> = ports
            .iter()
            .map(|port| spelling.take(ident(&port.name)))
            .collect();
        for field in plain_ports.iter_mut().chain(fields.iter_mut()) {
            field.spelled = spelling.take(std::mem::take(&mut field.spelled));
        }
        let state_names: Vec<String> = states
            .iter()
            .map(|(state, _)| spelling.take(ident(state)))
            .collect();

        // the plan already named the parameters, own ports first
        let plan = self.plans.get(&def).expect("planned in collect");
        for (port, (parameter, _)) in ports.iter_mut().zip(&plan.params) {
            port.parameter = parameter.clone();
        }
        let mut methods = Vec::new();
        for usage in performs {
            methods.push(self.method(&def_name, usage, &ports, &fields)?);
        }
        for usage in calcs {
            match self.calc_method(usage, &fields) {
                Ok(method) => methods.push(method),
                Err(note) => notes.push(note),
            }
        }
        for usage in asserts {
            match self.assertion(usage, &fields) {
                Ok(method) => methods.push(method),
                Err(note) => notes.push(note),
            }
        }

        writeln!(out).unwrap();
        if let Some(doc) = documentation(model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: `part def {def_name}`").unwrap();
        for note in &notes {
            self.open(OpenKind::Skipped, def_name.clone(), None, note);
            writeln!(out, "// not generated: {note}").unwrap();
        }
        // Where every field would start with `Default::default()`, the
        // derive says it in one word. Writing the impl out is what
        // clippy calls `derivable_impls`, and a reader of a file marked
        // DO NOT EDIT cannot act on that.
        let derived_default = plan.params.is_empty()
            && self.defaultable.contains(&def)
            && plain_ports
                .iter()
                .chain(&fields)
                .all(|field| self.starts_as_its_type_does(field))
            && states.is_empty();
        if self.derivable.contains(&def) {
            let default = if derived_default { ", Default" } else { "" };
            writeln!(out, "#[derive({}{default})]", DERIVED.join(", ")).unwrap();
        }
        let generics = plan
            .params
            .iter()
            .map(|(parameter, bound)| format!("{parameter}: {bound}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(out, "pub struct {def_name}{} {{", angle(&generics)).unwrap();
        for (port, spelled) in ports.iter().zip(&port_names) {
            writeln!(
                out,
                "    /// SysML: `port {} : {}`",
                port.name, port.type_name
            )
            .unwrap();
            writeln!(out, "    pub {spelled}: {},", port.parameter).unwrap();
        }
        for field in plain_ports.iter().chain(&fields) {
            if let Some(dropped) = plan.dropped.get(&field.usage) {
                self.open(OpenKind::Skipped, def_name.clone(), None, dropped);
                writeln!(out, "    // not generated: {dropped}").unwrap();
                continue;
            }
            if let Some(doc) = documentation(model, field.usage) {
                doc_comment(&doc, "    ", out);
            }
            writeln!(
                out,
                "    pub {}: {},",
                field.spelled,
                self.rust_type(plan, field)
            )
            .unwrap();
        }
        for ((state, machine), spelled) in states.iter().zip(&state_names) {
            writeln!(out, "    /// SysML: `state {state}`").unwrap();
            writeln!(out, "    pub {spelled}: {machine},").unwrap();
        }
        writeln!(out, "}}").unwrap();

        // the declared values, where every field has something to start
        // from -- the unbound ports among them, or the struct would name
        // fields its `Default` never fills
        let started = !fields.is_empty() || !plain_ports.is_empty() || !states.is_empty();
        let spelled_out = !derived_default || !self.derivable.contains(&def);
        if self.defaultable.contains(&def) && started && spelled_out {
            // A generic parameter stands for a port bound to someone
            // else's trait, and nothing says that trait's implementors
            // know where to start. The impl asks for it rather than
            // assuming it, so that what the model declared a field
            // starts as reaches Rust even where a port does not.
            let bounded = plan
                .params
                .iter()
                .map(|(parameter, bound)| format!("{parameter}: {bound} + Default"))
                .collect::<Vec<_>>()
                .join(", ");
            let named = plan
                .params
                .iter()
                .map(|(parameter, _)| parameter.clone())
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "\nimpl{} Default for {def_name}{} {{",
                angle(&bounded),
                angle(&named)
            )
            .unwrap();
            writeln!(out, "    fn default() -> Self {{").unwrap();
            writeln!(out, "        Self {{").unwrap();
            for spelled in &port_names {
                writeln!(out, "            {spelled}: Default::default(),").unwrap();
            }
            for field in plain_ports.iter().chain(&fields) {
                writeln!(
                    out,
                    "            {}: {},",
                    field.spelled,
                    self.default_value(field)
                )
                .unwrap();
            }
            for ((_, machine), spelled) in states.iter().zip(&state_names) {
                writeln!(out, "            {spelled}: {machine}::initial(),").unwrap();
            }
            writeln!(out, "        }}").unwrap();
            writeln!(out, "    }}").unwrap();
            writeln!(out, "}}").unwrap();
        }

        if !methods.is_empty() {
            let names = plan
                .params
                .iter()
                .map(|(parameter, _)| parameter.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            writeln!(
                out,
                "\nimpl{} {def_name}{} {{",
                angle(&generics),
                angle(&names)
            )
            .unwrap();
            for (at, method) in methods.iter().enumerate() {
                if at > 0 {
                    writeln!(out).unwrap();
                }
                out.push_str(method);
            }
            writeln!(out, "}}").unwrap();
        }
        Ok(())
    }
    fn rust_type(&self, plan: &Plan, field: &Field) -> String {
        let mut base = match &field.ty {
            FieldType::Scalar(name) => (*name).to_string(),
            FieldType::External(path) => path.clone(),
            FieldType::Generated(target) => {
                let mut name = type_ident(self.model.name(*target).expect("collected named"));
                // a generic part takes the parameters this struct carries
                // on its behalf
                if let Some(arguments) = plan.arguments.get(&field.usage) {
                    name = format!("{name}<{}>", arguments.join(", "));
                }
                name
            }
        };
        if field.boxed {
            base = format!("Box<{base}>");
        }
        contained(base, field.container)
    }
    /// Would this field start where its type starts anyway? A declared
    /// `false` on a `Boolean` is `bool::default()` spelled out, and a
    /// struct all of whose fields do that needs no `impl` of its own.
    fn starts_as_its_type_does(&self, field: &Field) -> bool {
        let written = self.default_value(field);
        if written == "Default::default()" {
            return true;
        }
        match (&field.ty, field.container) {
            (FieldType::Scalar(name), Container::One) => matches!(
                (*name, written.as_str()),
                ("bool", "false")
                    | ("i64", "0")
                    | ("u64", "0")
                    | ("f64", "0.0")
                    | ("String", "\"\".to_string()")
            ),
            _ => false,
        }
    }
    fn default_value(&self, field: &Field) -> String {
        match (&field.default, &field.container) {
            (Some(rust), Container::One) if !field.boxed => rust.clone(),
            // `Default` is implemented for arrays only up to a length the
            // standard library stops at; `from_fn` has no such limit
            (_, Container::Array(_)) => "std::array::from_fn(|_| Default::default())".to_string(),
            _ => "Default::default()".to_string(),
        }
    }
    /// An `enum def` as an enum, its first value the default.
    /// The values of an enumeration definition. Asked here and nowhere
    /// else, so that what is written and what is thought startable
    /// cannot disagree.
    pub(crate) fn enum_values(&self, def: ElementId) -> Vec<&str> {
        self.model
            .owned(def)
            .iter()
            .filter(|&&child| self.model.kind(child) == ElementKind::EnumerationUsage)
            .filter_map(|&child| self.model.name(child))
            .collect()
    }
    /// The variants of a variation, in declaration order, each with the
    /// struct it carries as a payload where it has one. What counts as a
    /// variant is asked here by everything that needs to know, so that
    /// what is written and what is claimed about it cannot disagree.
    pub(crate) fn variants(&self, def: ElementId) -> Vec<(&str, Option<ElementId>)> {
        self.model
            .owned(def)
            .iter()
            .filter(|&&child| self.model.member_role(child) == Some(Role::Variant))
            .filter_map(|&child| {
                let payload = self
                    .model
                    .type_of(child)
                    .filter(|target| self.shapes.get(target) == Some(&Shape::Struct));
                Some((self.model.name(child)?, payload))
            })
            .collect()
    }
    fn enumeration(&self, def: ElementId, out: &mut String) {
        let name = type_ident(self.model.name(def).expect("collected named"));
        let values = self.enum_values(def);
        let mut notes = Vec::new();
        for &child in self.model.owned(def) {
            let kind = self.model.kind(child);
            let Some(child_name) = self.model.name(child) else {
                continue;
            };
            if kind != ElementKind::EnumerationUsage && !kind.is_a(ElementKind::Relationship) {
                notes.push(format!(
                    "`{child_name}` -- only `enum` values become variants"
                ));
            }
        }
        writeln!(out).unwrap();
        if let Some(doc) = documentation(self.model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: `enum def {name}`").unwrap();
        for note in &notes {
            self.open(OpenKind::Skipped, name.clone(), None, note);
            writeln!(out, "// not generated: {note}").unwrap();
        }
        // the first value is the default, which an attribute on that
        // variant says in place of an impl -- an enum with no values at
        // all has nothing to be
        let default = if values.is_empty() { "" } else { ", Default" };
        writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq{default})]").unwrap();
        writeln!(out, "pub enum {name} {{").unwrap();
        let mut spelling = Namespace::default();
        for (at, value) in values.iter().enumerate() {
            writeln!(out, "    /// SysML: `enum {value}`").unwrap();
            if at == 0 {
                writeln!(out, "    #[default]").unwrap();
            }
            let variant = spelling.take(type_ident(&camel(value)));
            writeln!(out, "    {variant},").unwrap();
        }
        writeln!(out, "}}").unwrap();
    }
    /// A variation: one enum variant per `variant`, carrying the
    /// variant's type where it has one.
    fn variation(&self, def: ElementId, out: &mut String) {
        let name = type_ident(self.model.name(def).expect("collected named"));
        writeln!(out).unwrap();
        if let Some(doc) = documentation(self.model, def) {
            doc_comment(&doc, "", out);
        }
        writeln!(out, "/// SysML: variation `{name}`").unwrap();
        let variants = self.variants(def);
        // whatever holds a variation asks it for a `Default`, and only a
        // variant carrying no payload can be one
        let starts_at = variants
            .iter()
            .position(|(_, payload)| payload.is_none())
            .filter(|_| self.defaultable.contains(&def));
        if starts_at.is_some() {
            writeln!(out, "#[derive(Default)]").unwrap();
        }
        writeln!(out, "pub enum {name} {{").unwrap();
        let mut spelling = Namespace::default();
        for (at, (variant, payload)) in variants.iter().enumerate() {
            writeln!(out, "    /// SysML: `variant {variant}`").unwrap();
            if starts_at == Some(at) {
                writeln!(out, "    #[default]").unwrap();
            }
            let payload = payload.and_then(|target| {
                let held = type_ident(self.model.name(target)?);
                // a variant carries its payload inline, so a cycle
                // closing through one needs the same `Box` a field does
                Some(match self.boxed.contains(&(def, target)) {
                    true => format!("Box<{held}>"),
                    false => held,
                })
            });
            let variant = spelling.take(type_ident(&camel(variant)));
            match payload {
                Some(payload) => writeln!(out, "    {variant}({payload}),").unwrap(),
                None => writeln!(out, "    {variant},").unwrap(),
            }
        }
        writeln!(out, "}}").unwrap();
    }
}
