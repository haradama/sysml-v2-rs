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
            Shape::Quantity => {
                let spelled = self.model.name(def).expect("collected named");
                writeln!(out, "\n/// SysML: `attribute def {spelled}`").unwrap();
                if let Some(doc) = documentation(self.model, def) {
                    doc_comment(&doc, "", out);
                }
                if let Some(counted) = crate::quantity::counted(self.model, def) {
                    writeln!(out, "/// {counted}").unwrap();
                }
                let scalar = scalar_type(self.model, def).unwrap_or("f64");
                writeln!(out, "pub type {} = {scalar};", type_ident(spelled)).unwrap();
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
    /// A reference chain through the fields of a generated struct, from
    /// `fields` on: `line.amount` is `self.line.amount` where every step
    /// holds one value, and nothing Rust can spell where one is an
    /// `Option` or a `Vec`, which has no `amount` -- or where the chain
    /// ends on one, which is no value a formula can work with.
    pub(crate) fn through_fields(&self, fields: &[Field], chain: &str) -> Option<expr::Reference> {
        let mut names = chain.split('.');
        let first = names.next()?;
        let mut field = fields.iter().find(|field| field.name == first)?.clone();
        let mut rust = format!("self.{}", field.spelled);
        for name in names {
            let FieldType::Generated(def) = field.ty else {
                return None;
            };
            if field.container != Container::One {
                return None;
            }
            field = self.fields(def)?.into_iter().find(|it| it.name == name)?;
            rust = format!("{rust}.{}", field.spelled);
        }
        if field.container != Container::One {
            return None;
        }
        Some(match field.ty {
            FieldType::Scalar(ty) => expr::Reference::typed(rust, ty),
            _ => rust.into(),
        })
    }
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
        self.walk_fields_and_clashes(def).map(|(fields, _)| fields)
    }
    /// The names two generals of `def` both hand down, neither of them
    /// redefining the other's: only one of them can be the field, and
    /// the first written is. The model is not well formed -- the names
    /// of a namespace's members are to be distinguishable -- and a struct
    /// that quietly lost one says nothing of it.
    pub(crate) fn clashing_fields(&self, def: ElementId) -> Vec<String> {
        self.walk_fields_and_clashes(def)
            .map(|(_, clashes)| clashes)
            .unwrap_or_default()
    }
    fn walk_fields_and_clashes(&self, def: ElementId) -> Option<(Vec<Field>, Vec<String>)> {
        if self.specializes_itself(def, &mut Vec::new(), &mut HashSet::new()) {
            return None;
        }
        let mut clashes = Vec::new();
        // which level each name was taken at
        let mut taken_at: std::collections::HashMap<String, ElementId> = Default::default();
        let mut fields = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        // what a redefinition below has replaced, under whatever name: `y
        // :>> x` stands in the place of `x`, which is no field beside it
        let mut replaced: HashSet<ElementId> = HashSet::new();
        // breadth first and in the order the generals are written, so the
        // nearer and the first-written answer for a name: read as a stack,
        // the last general written won a name two of them declare
        let mut chain = std::collections::VecDeque::from([def]);
        let mut visited = HashSet::new();
        while let Some(level) = chain.pop_front() {
            // diamond inheritance reaches one general along two paths;
            // it hands its fields down once, not twice
            if !visited.insert(level) {
                continue;
            }
            for &child in self.model.owned(level) {
                for &relationship in self.model.owned(child) {
                    if self.model.kind(relationship) == ElementKind::Redefinition {
                        replaced.extend(
                            self.model
                                .maybe(relationship, "redefinedFeature")
                                .and_then(Value::as_id),
                        );
                    }
                }
                if replaced.contains(&child) {
                    continue;
                }
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
                    taken_at.insert(field.name.clone(), level);
                    fields.push(field);
                } else if taken_at
                    .get(&field.name)
                    .is_some_and(|&nearer| !self.is_below(nearer, level))
                    && !clashes.contains(&field.name)
                {
                    // shadowed by a level that does not specialize this one
                    clashes.push(field.name.clone());
                }
            }
            chain.extend(self.generals(level));
        }
        Some((fields, clashes))
    }
    /// The members a struct is made of beyond its data fields -- ports,
    /// performed actions, calculations, assertions, states and whatever
    /// is not generated -- its own first, then what its generals hand
    /// down. Rust has no subtyping, so a `calc` or a `perform` declared on
    /// a general is a method of every struct below it, as its attributes
    /// are fields there; a member redefined below, or one whose name a
    /// nearer member already answers to, is that nearer member's.
    ///
    /// The data fields are [`Generator::fields`]'s, and the generals'
    /// nested definitions are written at the top level, so neither is
    /// handed down here.
    pub(crate) fn members(&self, def: ElementId) -> Vec<ElementId> {
        let model = self.model;
        let mut members = Vec::new();
        let mut taken: HashSet<String> = HashSet::new();
        let mut replaced: HashSet<ElementId> = HashSet::new();
        let mut chain = std::collections::VecDeque::from([def]);
        let mut visited = HashSet::new();
        while let Some(level) = chain.pop_front() {
            if !visited.insert(level) {
                continue;
            }
            let inherited = level != def;
            // a name shadows what is further up, not what stands beside
            // it: `action inner;` and `perform inner;` are two members
            let mut at_level = Vec::new();
            for &child in model.owned(level) {
                let kind = model.kind(child);
                if kind.is_a(ElementKind::Relationship) || kind == ElementKind::MetadataUsage {
                    continue;
                }
                replaced.extend(
                    model
                        .owned(child)
                        .iter()
                        .filter(|&&it| model.kind(it) == ElementKind::Redefinition)
                        .filter_map(|&it| {
                            model.maybe(it, "redefinedFeature").and_then(Value::as_id)
                        }),
                );
                if replaced.contains(&child) {
                    continue;
                }
                if inherited
                    && (kind.is_a(ElementKind::Definition)
                        || matches!(
                            kind,
                            ElementKind::AttributeUsage
                                | ElementKind::PartUsage
                                | ElementKind::ItemUsage
                                | ElementKind::ReferenceUsage
                        ))
                {
                    continue;
                }
                let name = self.member_name(child);
                // a nearer member answers to this name
                if inherited && name.as_ref().is_some_and(|name| taken.contains(name)) {
                    continue;
                }
                at_level.extend(name);
                members.push(child);
            }
            taken.extend(at_level);
            if self.specializes_itself(level, &mut Vec::new(), &mut HashSet::new()) {
                break;
            }
            chain.extend(self.model_generals(level));
        }
        members
    }
    /// The generals a definition hands members down from: the ones the
    /// model wrote, not the library's own -- every `part def` implicitly
    /// specializes `Parts::Part`, whose `ownedPorts` and
    /// `performedActions` are what a part is, not members anyone wrote.
    fn model_generals(&self, def: ElementId) -> Vec<ElementId> {
        let model = self.model;
        model
            .owned(def)
            .iter()
            .filter(|&&child| model.kind(child) == ElementKind::Subclassification)
            .filter(|&&child| !model.flag(child, "isImplied"))
            .filter_map(|&child| model.maybe(child, "superclassifier").and_then(Value::as_id))
            .filter(|&general| {
                let mut at = Some(general);
                while let Some(here) = at {
                    if model.is_standard(here) {
                        return false;
                    }
                    at = model.owner(here);
                }
                true
            })
            .collect()
    }
    /// The name a member is generated under: the one it declares, or the
    /// one it borrows from what it redefines or refers to -- `perform
    /// inner;` is `inner` -- or, for a performance declaring and
    /// borrowing none, the name of what it performs, as `perform action :
    /// Steer;` performs a `Steer` and is nothing else the model calls it.
    pub(crate) fn member_name(&self, usage: ElementId) -> Option<String> {
        if let Some(name) = self.model.effective_name(usage) {
            return Some(name.to_string());
        }
        // only a performance is called after what it performs: an
        // unnamed part or connection is one of several a type may have,
        // and its type's name would be a name nobody gave it
        if self.model.kind(usage) != ElementKind::PerformActionUsage {
            return None;
        }
        let ty = self.performed(usage)?;
        let spelled = self.model.name(ty)?;
        let mut letters = spelled.chars();
        let first = letters.next()?;
        Some(first.to_lowercase().chain(letters).collect())
    }
    /// The doc line a member generated under a name the model never
    /// gave it carries, so that the method's name reads as this
    /// generator's choice and not as the model's: empty for a member the
    /// model names, itself or through what it borrows a name from.
    pub(crate) fn unnamed_note(&self, usage: ElementId) -> String {
        if self.model.effective_name(usage).is_some() {
            return String::new();
        }
        "    /// Unnamed in the model: called here after its type.\n".to_string()
    }
    /// What a `perform` performs: the action it is typed by, or the one
    /// typing the action usage it refers to -- `perform inner;` performs
    /// whatever `action inner : Steer;` is.
    pub(crate) fn performed(&self, usage: ElementId) -> Option<ElementId> {
        let model = self.model;
        model.type_of(usage).or_else(|| {
            let referenced = model.owned(usage).iter().find_map(|&child| {
                (model.kind(child) == ElementKind::ReferenceSubsetting)
                    .then(|| {
                        model
                            .maybe(child, "referencedFeature")
                            .and_then(Value::as_id)
                    })
                    .flatten()
            })?;
            held_type(model, referenced)
        })
    }
    /// Whether `low` specializes `high`, directly or not.
    fn is_below(&self, low: ElementId, high: ElementId) -> bool {
        let mut seen = HashSet::new();
        let mut queue = vec![low];
        while let Some(at) = queue.pop() {
            if at == high {
                return true;
            }
            if seen.insert(at) {
                queue.extend(self.generals(at));
            }
        }
        false
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
        let target = held_type(self.model, usage)?;
        let ty = if let Some(bound) = binding(self.model, target) {
            FieldType::External(bound.get(binding::ITEM)?.clone())
        } else if matches!(
            self.shapes.get(&target),
            Some(Shape::Struct | Shape::Enum | Shape::Variation)
        ) {
            // a state machine or an abstract definition is not a value type
            FieldType::Generated(target)
        } else {
            FieldType::Scalar(scalar_type(self.model, target)?)
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
        for name in self.clashing_fields(def) {
            notes.push(format!(
                "`{name}` is handed down by two generals that do not redefine one \
                 another; only the first written is a field"
            ));
        }
        for child in self.members(def) {
            let Some(child_name) = self.member_name(child) else {
                // an end name resolution reified stands for what a
                // connector reaches, and is no member anyone wrote
                if model.flag(child, "isEnd") {
                    continue;
                }
                // a comment or documentation is about the struct, not part
                // of it, and needs no name to be that
                if !model.kind(child).is_a(ElementKind::Usage) {
                    continue;
                }
                // Nothing the model says names it, and nothing it is
                // typed by does either: there is no Rust name to write
                // it under, which is said rather than passed over.
                let note = format!(
                    "an unnamed {} -- nothing names it, so there is no Rust name for it",
                    model.kind(child).name()
                );
                notes.push(note);
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

        // Each method names itself after what it stands for, and two of
        // those may spell alike in Rust -- `calc fooBar` and `calc
        // foo_bar` are both `foo_bar` -- so the second is numbered, as a
        // field would be.
        let mut called = Namespace::default();
        let methods: Vec<String> = methods
            .into_iter()
            .map(|method| named_once(method, &mut called))
            .collect();

        writeln!(out).unwrap();
        if let Some(doc) = documentation(model, def) {
            doc_comment(&doc, "", out);
        }
        // what the model wrote, keyword and name, rather than `part def`
        // and the Rust name for every definition that becomes a struct
        writeln!(
            out,
            "/// SysML: `{} {}`",
            declared_as(model.kind(def)),
            sysml_syntax::quote(model.name(def).expect("collected named"))
        )
        .unwrap();
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
            if let Some(counted) = crate::quantity::counted_in(model, field.usage) {
                writeln!(out, "    /// {counted}").unwrap();
            }
            // A value the model declared and this could not write starts
            // the field at its type's default instead, and saying nothing
            // made a declared `20 ['°C']` or `y * 2` read as a zero the
            // model chose.
            if let (None, Some(ValueClause::Text(declared))) =
                (&field.default, value_clause(model, field.usage))
            {
                let why = format!(
                    "`{}` starts at its type's default: `= {}` is beyond the simple subset",
                    field.name,
                    on_one_line(&declared)
                );
                writeln!(out, "    // left open: {why}").unwrap();
                self.open(OpenKind::Skipped, def_name.clone(), None, why);
            }
            // a value written, and a container it cannot be written into
            // as it stands
            if let Err(why) = self.declared_start(field) {
                writeln!(out, "    // left open: {why}").unwrap();
                self.open(OpenKind::Skipped, def_name.clone(), None, why);
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
        if let Ok(Some(rust)) = self.declared_start(field) {
            return rust;
        }
        match field.container {
            // `Default` is implemented for arrays only up to a length the
            // standard library stops at; `from_fn` has no such limit
            Container::Array(_) => "std::array::from_fn(|_| Default::default())".to_string(),
            _ => "Default::default()".to_string(),
        }
    }
    /// The declared value of a field in the form its container holds
    /// it, or why that form cannot say what the model says.
    ///
    /// A feature value binds the feature to what the expression comes to,
    /// and every value this generator writes is one value: `opt :
    /// Integer[0..1] = 3` holds that one value, `Some(3)`, and `many :
    /// Integer[*] = 3` the sequence of it, `vec![3]`. `three : Real[3] =
    /// 1.5` is a sequence of one where the multiplicity asks for three,
    /// which the model does not make well formed by repeating the value:
    /// `[1.5; 3]` would be a choice the model never made, so the field
    /// is left open and says so.
    fn declared_start(&self, field: &Field) -> Result<Option<String>, String> {
        let Some(rust) = &field.default else {
            return Ok(None);
        };
        let one_too_few = |holds: String| {
            Err(format!(
                "`{}` starts at its type's default: its declared value `{rust}` is one \
                 value, where {holds}",
                field.name
            ))
        };
        match field.container {
            Container::One if !field.boxed => Ok(Some(rust.clone())),
            Container::One => Ok(None),
            Container::Optional if !field.boxed => Ok(Some(format!("Some({rust})"))),
            Container::Optional => Ok(Some(format!("Some(Box::new({rust}))"))),
            Container::Many => match lower_bound(self.model, field.usage) {
                Some(0 | 1) => Ok(Some(format!("vec![{rust}]"))),
                Some(lower) => one_too_few(format!("its multiplicity asks for at least {lower}")),
                None => one_too_few("its multiplicity's lower bound is no number".to_string()),
            },
            Container::Array(n) => one_too_few(format!("its multiplicity asks for exactly {n}")),
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

/// The keywords a definition of `kind` is declared with: `PartDefinition`
/// is `part def`. Only the definitions that become structs ask, and each
/// of those is one word before `Definition`.
fn declared_as(kind: ElementKind) -> String {
    let name = kind.name();
    format!(
        "{} def",
        name.strip_suffix("Definition")
            .unwrap_or(name)
            .to_lowercase()
    )
}

/// `method` with the name it declares taken from `called`: kept where it
/// is the first to want it, numbered where another method has it.
fn named_once(method: String, called: &mut Namespace) -> String {
    let Some((at, length)) = method
        .find("fn ")
        .map(|at| at + 3)
        .and_then(|at| method[at..].find(['(', '<']).map(|length| (at, length)))
    else {
        return method;
    };
    let name = &method[at..at + length];
    // `r#type` is the keyword's name, and a number after it makes it one
    // that needs no `r#`
    let bare = name.trim_start_matches("r#");
    let taken = called.take(bare.to_string());
    if taken == bare {
        return method;
    }
    format!("{}{taken}{}", &method[..at], &method[at + length..])
}
