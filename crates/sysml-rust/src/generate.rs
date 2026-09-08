//! Rust from a resolved SysML model.
//!
//! Two kinds of thing come out of one pass over the model:
//!
//! **Data and structure.** A `part def`, `item def`, `attribute def` or
//! `port def` becomes a struct: attributes and composed parts become
//! fields (multiplicities as containers -- `[*]` a `Vec`, `[0..1]` an
//! `Option`, `[n]` an array), inherited features are flattened in along
//! the reified specializations with redefinitions shadowing what they
//! redefine, and a composition cycle is broken with a `Box` at the edge
//! that closes it. Declared values become a `Default` implementation
//! where every field has one. An `enum def` -- and a variation with
//! `variant` members -- becomes an enum. An `abstract` definition
//! contributes its features to its subtypes but gets no struct of its
//! own.
//!
//! **Calculations.** A `calc def` becomes a function over its `in`
//! parameters and a `calc` usage of a struct a method over its fields.
//! Result expressions in the simple subset -- literals, references,
//! arithmetic, comparisons, logic, `if c ? a else b` -- translate as
//! written (numeric literals keep their spelling, so mixed-type
//! arithmetic surfaces as a Rust type error, not a coercion); anything
//! richer keeps its SysML text behind a `todo!`. An `abstract calc def`
//! declares no formula at all, so it becomes a trait with one method
//! and no default body: what the model leaves open the compiler asks
//! for, rather than a `todo!` waiting to be reached. The same translation
//! gives state-machine guards over their event payload a real default
//! body, and closed expressions (`= 2.0 * 3.0`) their place in
//! `Default`.
//!
//! **Behaviour against existing APIs.** A port typed by a `port def`
//! carrying a `@rust { ... }` binding (what [`crate::import`] writes)
//! becomes a generic parameter bound to the real Rust trait, and each
//! `perform`ed bound action becomes a method delegating through that
//! port, with the API's own signature: `async`, `Result` and the
//! receiver the binding states. A `state def` becomes a state machine:
//! an enum of states, an enum of the events its transitions accept, a
//! hooks trait carrying guards (their SysML text in the docs), effects
//! and entry/exit notifications, and a `step` function walking the
//! transition table.
//!
//! What has no generated shape is written into the output as a comment
//! rather than dropped. The output is deterministic -- declaration order
//! in, declaration order out -- and intended to be committed next to the
//! model; regenerating and diffing is the drift check.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use sysml_model::{ElementId, ElementKind, Model, Role, Value};

use crate::binding;

/// What every generated struct and enum derives, where its fields let
/// it. `Default` joins them when the model gave every field a value.
const DERIVED: [&str; 3] = ["Debug", "Clone", "PartialEq"];
use crate::expr::{self, translate, translate_as, Numbers, Translated};

/// The Rust a model implies, and what the model left for a person to
/// write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Generated {
    pub rust: String,
    /// Every place the generator stopped short, in the order it met
    /// them. The output says each of these in a comment too -- that is
    /// for whoever reads the file; this is for whoever has to fill the
    /// gap, and a work list is a poor thing to have to find by reading.
    pub open: Vec<Open>,
}

/// One thing the generator did not write, and why.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Open {
    pub kind: OpenKind,
    /// What in the model it is about, by the name the model gives it.
    pub sysml: String,
    /// What has to be written in Rust, where there is a name for it: the
    /// trait whose method is yours to implement.
    pub rust: Option<String>,
    pub why: String,
}

/// What kind of gap it is -- which decides who fills it and how.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenKind {
    /// A definition the model left abstract became a trait with no
    /// default body: the compiler will ask for it.
    Trait,
    /// An expression outside the translated subset kept the model's own
    /// words behind a `todo!`.
    Todo,
    /// A state machine's guards, effects and entry/exit notifications.
    Hooks,
    /// Something with no generated shape at all, said in a comment.
    Skipped,
}

impl OpenKind {
    /// The word for it, for an answer written as JSON.
    pub fn name(self) -> &'static str {
        match self {
            OpenKind::Trait => "trait",
            OpenKind::Todo => "todo",
            OpenKind::Hooks => "hooks",
            OpenKind::Skipped => "skipped",
        }
    }
}

/// What stops code generation outright (a model this generator cannot
/// write faithfully); everything smaller is a comment in the output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RustgenError {
    /// A performed action's binding names no port of the part.
    NoPortForAction {
        part: String,
        action: String,
        path: String,
    },
}

impl std::fmt::Display for RustgenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RustgenError::NoPortForAction { part, action, path } => write!(
                f,
                "`{part}` performs `{action}` bound to `{path}`, but no port of the part \
                 provides that API"
            ),
        }
    }
}

impl std::error::Error for RustgenError {}

/// Rust source for the definitions under `roots`.
pub fn generate(model: &Model, roots: &[ElementId]) -> Result<Generated, RustgenError> {
    let mut out = String::new();
    writeln!(
        out,
        "// generated by `sysml rustgen` -- DO NOT EDIT\n//\n\
         // Definitions of the model become structs and enums; ports bound to\n\
         // existing Rust APIs become generic parameters, `perform`ed actions\n\
         // methods, and state definitions state machines. What the model\n\
         // leaves abstract becomes a trait for you to implement -- alongside\n\
         // this file, not in it.\n\
         //\n\
         // An expression is spelled the way the model spells it, and a\n\
         // signature carries the parameters the model declares, so the lints\n\
         // that ask for a different spelling have nothing to say about a file\n\
         // nobody edits. What the model declares is written whether or not\n\
         // the caller reaches for it, which is the other thing a lint would\n\
         // otherwise ask about.\n\
         #![allow(\n\
         \x20   dead_code,\n\
         \x20   clippy::manual_range_contains,\n\
         \x20   clippy::too_many_arguments\n\
         )]"
    )
    .expect("writing to a String cannot fail");

    let generator = Generator::collect(model, roots);
    for &def in &generator.order {
        generator.definition(def, &mut out)?;
    }
    for (def, why) in &generator.skipped {
        let name = model.name(*def).expect("collected named");
        generator.open(OpenKind::Skipped, name, None, why);
        writeln!(out, "\n// not generated: `{name}` -- {why}")
            .expect("writing to a String cannot fail");
    }
    generator.requirements(roots, &mut out);
    Ok(Generated {
        rust: out,
        open: generator.open.into_inner(),
    })
}

/// How one definition will be generated.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Struct,
    Enum,
    Variation,
    StateMachine,
    /// A `calc def` -- or a `constraint def`, which is one returning a
    /// boolean: a function over its `in` parameters.
    Calculation,
    /// An `action def`: a trait, since a behaviour is not something this
    /// generator can write.
    Action,
    /// Abstract: features flatten into subtypes, no item of its own.
    Flattened,
}

struct Generator<'a> {
    model: &'a Model,
    order: Vec<ElementId>,
    /// Definitions this generator has no shape for, in declaration
    /// order, and why, so that the output says so instead of passing
    /// over them.
    skipped: Vec<(ElementId, String)>,
    shapes: HashMap<ElementId, Shape>,
    /// Generated types that can answer `Default::default()`.
    defaultable: HashSet<ElementId>,
    /// Structs whose fields all support `Debug`/`Clone`/`PartialEq`.
    derivable: HashSet<ElementId>,
    /// Composition edges broken with a `Box` to keep structs finite.
    boxed: HashSet<(ElementId, ElementId)>,
    /// Per struct: its generic parameters and what each composed field
    /// passes along, so parts with API ports compose.
    plans: HashMap<ElementId, Plan>,
    /// Every generated type's fields, worked out once they stop
    /// changing. Empty until then.
    fields: HashMap<ElementId, Option<Vec<Field>>>,
    /// What the generator stopped short of writing. A cell because the
    /// pass writes through `&self`: the alternative is to thread a
    /// second `&mut` beside the output through every one of these
    /// methods, which says nothing the name of this field does not.
    open: RefCell<Vec<Open>>,
}

impl Generator<'_> {
    /// Remember a gap the output is about to say in a comment. The two
    /// are written together at every site so that neither can be added
    /// without the other.
    fn open(
        &self,
        kind: OpenKind,
        sysml: impl Into<String>,
        rust: Option<String>,
        why: impl Into<String>,
    ) {
        self.open.borrow_mut().push(Open {
            kind,
            sysml: sysml.into(),
            rust,
            why: why.into(),
        });
    }
}

/// The generic signature of one struct and how its fields use it.
#[derive(Default)]
struct Plan {
    /// Parameters in declaration order: `(name, trait bound)`.
    params: Vec<(String, String)>,
    /// The parameter names each composed usage instantiates its type with.
    arguments: HashMap<ElementId, Vec<String>>,
    /// Fields dropped because a composition cycle would need endless
    /// parameters, by name and note.
    dropped: HashMap<ElementId, String>,
}

impl<'a> Generator<'a> {
    fn collect(model: &'a Model, roots: &[ElementId]) -> Generator<'a> {
        let mut order = Vec::new();
        let mut shapes = HashMap::new();
        let mut skipped: Vec<(ElementId, String)> = Vec::new();
        for &root in roots {
            for id in model.descendants(root) {
                if model.name(id).is_none() || binding(model, id).is_some() {
                    continue;
                }
                let shape = match model.kind(id) {
                    ElementKind::EnumerationDefinition => Shape::Enum,
                    ElementKind::StateDefinition => Shape::StateMachine,
                    ElementKind::CalculationDefinition => Shape::Calculation,
                    ElementKind::ConstraintDefinition => Shape::Calculation,
                    ElementKind::ActionDefinition => Shape::Action,
                    // an interface or a connection definition is a pair
                    // of ends and whatever they carry -- a struct like
                    // any other definition with features
                    ElementKind::PartDefinition
                    | ElementKind::ItemDefinition
                    | ElementKind::AttributeDefinition
                    | ElementKind::PortDefinition
                    | ElementKind::InterfaceDefinition
                    | ElementKind::ConnectionDefinition => {
                        if model
                            .owned(id)
                            .iter()
                            .any(|&child| model.member_role(child) == Some(Role::Variant))
                        {
                            Shape::Variation
                        } else if model.is_abstract(id) {
                            Shape::Flattened
                        } else {
                            Shape::Struct
                        }
                    }
                    // what has no shape is still said out loud, since
                    // silence reads as "there was nothing here"
                    // a requirement has a shape of its own -- the ignored
                    // test each one becomes -- so it is not skipped. Only
                    // that exact kind: a viewpoint is a requirement by
                    // specialization and gets no test.
                    kind if kind.is_a(ElementKind::Definition)
                        && kind != ElementKind::RequirementDefinition =>
                    {
                        if !skipped.iter().any(|(seen, _)| *seen == id) {
                            skipped.push((id, format!("{} has no Rust shape", kind.name())));
                        }
                        continue;
                    }
                    _ => continue,
                };
                if let std::collections::hash_map::Entry::Vacant(slot) = shapes.entry(id) {
                    slot.insert(shape);
                    order.push(id);
                }
            }
        }
        let mut generator = Generator {
            model,
            order,
            skipped,
            shapes,
            defaultable: HashSet::new(),
            derivable: HashSet::new(),
            boxed: HashSet::new(),
            plans: HashMap::new(),
            fields: HashMap::new(),
            open: RefCell::new(Vec::new()),
        };
        generator.break_cycles();
        generator.settle_unbuildable();
        generator.settle_collisions();
        generator.settle_fields();
        generator.settle_plans();
        generator.settle_defaults();
        generator.settle_derives();
        generator
    }

    /// Depth-first over the composition graph; the edge that would close
    /// a cycle is remembered and later held behind a `Box`.
    ///
    /// The graph runs over the flattened fields rather than the declared
    /// members, since a field a general hands down is held inline just
    /// as tightly as one declared here, and over the payloads of a
    /// variation's variants, which an enum holds inline the same way.
    /// A cycle closing through either of those went unboxed, and the
    /// struct it closed on had no size Rust could work out.
    fn break_cycles(&mut self) {
        let mut done: HashSet<ElementId> = HashSet::new();
        let mut boxed = HashSet::new();
        for &def in &self.order {
            self.walk_composition(def, &mut Vec::new(), &mut done, &mut boxed);
        }
        self.boxed = boxed;
    }

    fn walk_composition(
        &self,
        def: ElementId,
        path: &mut Vec<ElementId>,
        done: &mut HashSet<ElementId>,
        boxed: &mut HashSet<(ElementId, ElementId)>,
    ) {
        if done.contains(&def) {
            return;
        }
        path.push(def);
        for (holder, target) in self.composed(def) {
            if path.contains(&target) {
                boxed.insert((holder, target));
            } else {
                self.walk_composition(target, path, done, boxed);
            }
        }
        path.pop();
        done.insert(def);
    }

    /// Every generated type `def` holds inside itself, each paired with
    /// the element a `Box` would go on -- for an inherited field, the
    /// general that declared it, since that is where the field is
    /// written. A `Vec` is left out: it already breaks the recursion.
    fn composed(&self, def: ElementId) -> Vec<(ElementId, ElementId)> {
        let mut edges = Vec::new();
        if self.shapes.get(&def) == Some(&Shape::Variation) {
            // only a variant carrying a payload holds anything inline
            edges.extend(
                self.variants(def)
                    .into_iter()
                    .filter_map(|(_, payload)| Some((def, payload?))),
            );
            return edges;
        }
        let held = self
            .fields(def)
            .unwrap_or_default()
            .into_iter()
            .map(|field| (self.model.owner(field.usage).unwrap_or(def), field))
            .chain(self.plain_ports(def).into_iter().map(|port| (def, port)));
        for (holder, field) in held {
            if field.container == Container::Many {
                continue;
            }
            if let FieldType::Generated(target) = field.ty {
                if matches!(
                    self.shapes.get(&target),
                    Some(Shape::Struct | Shape::Variation)
                ) {
                    edges.push((holder, target));
                }
            }
        }
        edges
    }

    /// The generic plan of every struct: its own API ports first, then
    /// the parameters every composed generic part needs, renamed after
    /// the field that carries it.
    fn settle_plans(&mut self) {
        // whether a struct's composition subtree touches an API port --
        // a cycle through such a struct cannot be written with finitely
        // many parameters
        let mut reaches: HashMap<ElementId, bool> = HashMap::new();
        let order = self.order.clone();
        for &def in &order {
            self.reaches_ports(def, &mut reaches, &mut HashSet::new());
        }
        for &def in &order {
            if self.shapes[&def] == Shape::Struct {
                let mut visiting = HashSet::new();
                self.plan_of(def, &reaches, &mut visiting);
            }
        }
    }

    fn reaches_ports(
        &self,
        def: ElementId,
        reaches: &mut HashMap<ElementId, bool>,
        visiting: &mut HashSet<ElementId>,
    ) -> bool {
        if let Some(&known) = reaches.get(&def) {
            return known;
        }
        if !visiting.insert(def) {
            return false;
        }
        let mut found = self.has_api_ports(def);
        if !found {
            for field in self.fields(def).unwrap_or_default() {
                if let FieldType::Generated(target) = field.ty {
                    if self.shapes.get(&target) == Some(&Shape::Struct)
                        && self.reaches_ports(target, reaches, visiting)
                    {
                        found = true;
                        break;
                    }
                }
            }
        }
        visiting.remove(&def);
        reaches.insert(def, found);
        found
    }

    fn plan_of(
        &mut self,
        def: ElementId,
        reaches: &HashMap<ElementId, bool>,
        visiting: &mut HashSet<ElementId>,
    ) {
        if self.plans.contains_key(&def) || !visiting.insert(def) {
            return;
        }
        let def_name = type_ident(self.model.name(def).expect("collected named"));
        let mut plan = Plan::default();
        let mut taken: HashSet<String> = HashSet::new();

        // the struct's own API ports come first
        let mut at = 0;
        for &child in self.model.owned(def) {
            if self.model.kind(child) != ElementKind::PortUsage {
                continue;
            }
            let Some(port) = self.port_of(child) else {
                continue;
            };
            let mut parameter = type_ident(&camel(&port.name));
            if parameter == def_name || !taken.insert(parameter.clone()) {
                parameter = format!("P{at}");
                taken.insert(parameter.clone());
            }
            plan.params.push((parameter, port.trait_path));
            at += 1;
        }

        // then whatever the composed generic parts need, renamed after
        // the field carrying them
        for field in self.fields(def).unwrap_or_default() {
            let FieldType::Generated(target) = field.ty else {
                continue;
            };
            if self.shapes.get(&target) != Some(&Shape::Struct) {
                continue;
            }
            if visiting.contains(&target) {
                if reaches.get(&target) == Some(&true) {
                    plan.dropped.insert(
                        field.usage,
                        format!(
                            "`{}` -- a composition cycle through API ports has no finite \
                             generic signature",
                            field.name
                        ),
                    );
                }
                continue;
            }
            self.plan_of(target, reaches, visiting);
            let target_params: Vec<(String, String)> = self
                .plans
                .get(&target)
                .map(|plan| plan.params.clone())
                .unwrap_or_default();
            if target_params.is_empty() {
                continue;
            }
            let mut arguments = Vec::new();
            for (theirs, bound) in target_params {
                let after = type_ident(&camel(&field.name));
                let mut local = format!("{after}{theirs}");
                let mut bump = 0;
                while !taken.insert(local.clone()) {
                    bump += 1;
                    local = format!("{after}{theirs}{bump}");
                }
                arguments.push(local.clone());
                plan.params.push((local, bound));
            }
            plan.arguments.insert(field.usage, arguments);
        }
        visiting.remove(&def);
        self.plans.insert(def, plan);
    }

    /// Which generated types can implement `Default`: enums always (their
    /// first variant), structs when every field can -- computed to a fixed
    /// point because structs compose each other.
    fn settle_defaults(&mut self) {
        for (&def, &shape) in &self.shapes {
            // An enumeration with no variants has no value to be, so
            // nothing holding one can start either. What counts as a
            // variant is asked of the same function that writes them,
            // because two answers to that question is how a field comes
            // to start at a value its type does not have.
            let has_values = match shape {
                Shape::Enum => !self.enum_values(def).is_empty(),
                // a variation starts at the first variant that carries
                // nothing, since `#[default]` is only for such a variant
                Shape::Variation => self
                    .variants(def)
                    .iter()
                    .any(|(_, payload)| payload.is_none()),
                _ => false,
            };
            if has_values {
                self.defaultable.insert(def);
            }
        }
        loop {
            let mut grew = false;
            for &def in &self.order {
                if self.shapes[&def] != Shape::Struct || self.defaultable.contains(&def) {
                    continue;
                }
                // a port bound to someone else's trait becomes a
                // generic parameter, and the `Default` impl asks that
                // parameter for a `Default` of its own -- so it is no
                // reason for the definition to have none
                //
                // an unbound port is a field of the struct like any
                // other, so `Default` has to be able to start it too
                let ports_fine = self
                    .plain_ports(def)
                    .iter()
                    .all(|field| self.field_defaultable(field));
                // a `state def` with no states generates an enum with no
                // values and no `initial()` to call, so a part holding
                // one has nothing to start it at
                let states_fine = self
                    .model
                    .owned(def)
                    .iter()
                    .filter(|&&child| self.model.kind(child) == ElementKind::StateUsage)
                    .all(|&child| self.state_field(child).is_none() || self.state_starts(child));
                if ports_fine
                    && states_fine
                    && self
                        .fields(def)
                        .is_some_and(|fields| fields.iter().all(|f| self.field_defaultable(f)))
                {
                    self.defaultable.insert(def);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
    }

    /// Two definitions can land on the same Rust name -- one `Driver` in
    /// a package and another nested in a part, or two names an escaped
    /// spelling joins up the same way. Only the first can have it; the
    /// rest are named rather than written twice over.
    fn settle_collisions(&mut self) {
        // Rust keeps types and values in namespaces of their own, so a
        // struct and a function may share a name and two functions may
        // not. The key says which namespace the name is claimed in.
        let mut taken: HashMap<(bool, String), ElementId> = HashMap::new();
        let mut clashing = Vec::new();
        for &def in &self.order {
            let spelled = self.model.name(def).expect("the unnamed never got a shape");
            let name = type_ident(spelled);
            // a definition may write more than one name: a state machine
            // writes three, and none of them is the name of the state
            // definition itself
            let claims = match self.shapes[&def] {
                Shape::StateMachine => vec![
                    (true, format!("{name}State")),
                    (true, format!("{name}Event")),
                    (true, format!("{name}Hooks")),
                ],
                // a calculation with a formula is a function; one without
                // is a trait, and traits are types
                Shape::Calculation if !self.model.is_abstract(def) => {
                    vec![(false, ident(spelled))]
                }
                _ => vec![(true, name)],
            };
            match claims
                .iter()
                .find_map(|claim| Some((claim, *taken.get(claim)?)))
            {
                Some(((_, claimed), first)) => clashing.push((
                    def,
                    format!(
                        "`{claimed}` is already the Rust name of `{}`",
                        qualified_of(self.model, first)
                    ),
                )),
                None => taken.extend(claims.into_iter().map(|claim| (claim, def))),
            }
        }
        for (def, why) in clashing {
            self.shapes.remove(&def);
            self.order.retain(|&kept| kept != def);
            self.skipped.push((def, why));
        }
    }

    /// A struct whose specializations run in a circle has no fields this
    /// generator can settle, and so no struct at all. Deciding that here
    /// rather than when it is written keeps everything that *refers* to
    /// it honest: a field of a type that was never generated would name
    /// something the file does not define.
    fn settle_unbuildable(&mut self) {
        let unbuildable: Vec<ElementId> = self
            .order
            .iter()
            .copied()
            .filter(|&def| self.shapes[&def] == Shape::Struct && self.fields(def).is_none())
            .collect();
        for def in unbuildable {
            self.shapes.remove(&def);
            self.order.retain(|&kept| kept != def);
            self.skipped
                .push((def, "its specializations form a circle".to_string()));
        }
    }

    /// Whether the type of `field` says it already has `trait_name`.
    /// Only a bound type is asked: what this generator wrote, it knows.
    fn field_type_claims(&self, field: &Field, trait_name: &str) -> bool {
        self.model
            .type_of(field.usage)
            .or_else(|| self.model.type_of(redefined(self.model, field.usage)?))
            .and_then(|ty| binding(self.model, ty))
            .is_some_and(|bound| binding::claims(&bound, trait_name))
    }

    /// Which structs can carry `#[derive(Debug, Clone, PartialEq)]`:
    /// those whose fields are scalars or other such structs and enums --
    /// nothing is claimed of external API types.
    fn settle_derives(&mut self) {
        for (&def, &shape) in &self.shapes {
            if matches!(shape, Shape::Enum) {
                self.derivable.insert(def);
            }
        }
        loop {
            let mut grew = false;
            for &def in &self.order {
                if self.shapes[&def] != Shape::Struct || self.derivable.contains(&def) {
                    continue;
                }
                let ports = self
                    .model
                    .owned(def)
                    .iter()
                    .any(|&child| self.model.kind(child) == ElementKind::PortUsage);
                let fields_fine = self.fields(def).is_some_and(|fields| {
                    fields.iter().all(|field| match &field.ty {
                        FieldType::Scalar(_) => true,
                        FieldType::Generated(target) => self.derivable.contains(target),
                        // a type this generator did not write says for
                        // itself what it can do, or nothing is claimed
                        FieldType::External(_) => DERIVED
                            .iter()
                            .all(|wanted| self.field_type_claims(field, wanted)),
                    })
                });
                if !ports && !self.generic(def) && fields_fine {
                    self.derivable.insert(def);
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
    }

    /// Does this struct carry generic parameters, its own or inherited
    /// from what it composes?
    fn generic(&self, def: ElementId) -> bool {
        self.plans
            .get(&def)
            .is_some_and(|plan| !plan.params.is_empty())
    }

    fn field_defaultable(&self, field: &Field) -> bool {
        if field.default.is_some() {
            return true;
        }
        match field.container {
            // a Vec or an Option is empty by default, whatever it holds
            Container::Many | Container::Optional => true,
            // an array is as startable as the element it repeats:
            // `from_fn` builds one without asking the element to be
            // `Copy`, which `[T::default(); N]` would
            Container::Array(_) => match &field.ty {
                FieldType::Scalar(_) => true,
                FieldType::Generated(target) => self.defaultable.contains(target),
                FieldType::External(_) => self.field_type_claims(field, "Default"),
            },
            Container::One => match &field.ty {
                FieldType::Scalar(_) => true,
                FieldType::Generated(target) => self.defaultable.contains(target),
                FieldType::External(_) => self.field_type_claims(field, "Default"),
            },
        }
    }

    /// One definition, whatever its shape.
    fn definition(&self, def: ElementId, out: &mut String) -> Result<(), RustgenError> {
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

    /// Work every definition's fields out once. Six callers ask for a
    /// definition's fields, two of them inside a loop that runs until
    /// nothing changes, so one answer was being walked out over and
    /// over. Nothing after `settle_collisions` changes what the answer
    /// is, which is why this is where it is settled.
    fn settle_fields(&mut self) {
        let settled = self
            .order
            .iter()
            .map(|&def| (def, self.walk_fields(def)))
            .collect();
        self.fields = settled;
    }

    /// The flattened data fields of a struct definition, settled where
    /// they have been and worked out where they have not -- the phases
    /// that run before [`Generator::settle_fields`] ask too.
    fn fields(&self, def: ElementId) -> Option<Vec<Field>> {
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
    fn walk_fields(&self, def: ElementId) -> Option<Vec<Field>> {
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
                    .get(child, "superclassifier")
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
    fn written_as_a_type(&self, def: ElementId) -> bool {
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
        if self.model.name(usage).is_none()
            && self.model.get(usage, "isEnd") == Some(&Value::Bool(true))
        {
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
            FieldType::External(bound.get(binding::PATH)?.clone())
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
    fn enum_values(&self, def: ElementId) -> Vec<&str> {
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
    fn variants(&self, def: ElementId) -> Vec<(&str, Option<ElementId>)> {
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

    /// A `state def` as a runnable machine: states, events, hooks and a
    /// `step` over the transition table.
    fn state_machine(&self, def: ElementId, out: &mut String) {
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
        let transitions: Vec<Transition> = model
            .owned(def)
            .iter()
            .filter_map(|&child| self.transition(child, &states))
            .collect();

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
        if let Some((_, first)) = states.first() {
            writeln!(out, "\nimpl {name}State {{").unwrap();
            writeln!(
                out,
                "    /// The first state the definition declares.\n    #[must_use]"
            )
            .unwrap();
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
        writeln!(out, "#[allow(unused_variables)]").unwrap();
        writeln!(out, "pub trait {name}Hooks {{").unwrap();
        for transition in &transitions {
            let payload = transition
                .payload
                .as_ref()
                .map(|(param, ty)| format!(", {}: &{ty}", ident(param)))
                .unwrap_or_default();
            if let Some(guard) = &transition.guard {
                // a guard over the event's payload translates into the
                // default body; anything else stays an open decision
                let resolve = |leading: &str| {
                    transition
                        .payload
                        .as_ref()
                        .filter(|(param, _)| param == leading)
                        .map(|(param, _)| ident(param))
                };
                let body = translate(guard, &resolve, &|name| self.callable(name))
                    .filter(|translated| translated.boolean)
                    .map(|translated| translated.rust)
                    .unwrap_or_else(|| "true".to_string());
                writeln!(out, "    /// SysML guard: `[{guard}]`").unwrap();
                writeln!(
                    out,
                    "    fn guard_{}(&self{payload}) -> bool {{\n        {body}\n    }}",
                    ident(&transition.name)
                )
                .unwrap();
            }
            if transition.effect {
                writeln!(out, "    /// SysML effect of `{}`.", transition.name).unwrap();
                writeln!(
                    out,
                    "    fn effect_{}(&mut self{payload}) {{}}",
                    ident(&transition.name)
                )
                .unwrap();
            }
        }
        for (_, state) in &states {
            writeln!(out, "    fn on_entry_{}(&mut self) {{}}", ident(state)).unwrap();
            writeln!(out, "    fn on_exit_{}(&mut self) {{}}", ident(state)).unwrap();
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
        for (transition, event) in transitions.iter().zip(&events) {
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
                    format!(
                        " if hooks.guard_{}({})",
                        ident(&transition.name),
                        ident(param)
                    )
                }
                (Some(_), None) => format!(" if hooks.guard_{}()", ident(&transition.name)),
                (None, _) => String::new(),
            };
            writeln!(out, "            {pattern}{guard} => {{").unwrap();
            writeln!(
                out,
                "                hooks.on_exit_{}();",
                ident(&transition.source)
            )
            .unwrap();
            if transition.effect {
                let argument = transition
                    .payload
                    .as_ref()
                    .map(|(param, _)| ident(param))
                    .unwrap_or_default();
                writeln!(
                    out,
                    "                hooks.effect_{}({argument});",
                    ident(&transition.name)
                )
                .unwrap();
            }
            writeln!(
                out,
                "                hooks.on_entry_{}();",
                ident(&transition.target)
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
        let name = model.name(usage)?;
        // the two unnamed chaining features are the ends, source first
        let ends: Vec<ElementId> = model
            .owned(usage)
            .iter()
            .filter_map(|&child| {
                if model.kind(child) != ElementKind::Feature {
                    return None;
                }
                sysml_model::end_reaches(model, child).last().copied()
            })
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
                    bound.get(binding::PATH)?.clone()
                } else if self.written_as_a_type(ty) {
                    type_ident(model.name(ty)?)
                } else {
                    return None;
                };
                Some((param.to_string(), rust))
            }),
            _ => None,
        };
        let event = type_ident(&camel(name));
        let guard = match model.get(usage, "guardExpression") {
            Some(Value::RefList(guards)) => guards
                .first()
                .and_then(|&expression| expression_text(model, expression)),
            _ => None,
        };
        let effect = matches!(model.get(usage, "effectAction"), Some(Value::RefList(list)) if !list.is_empty());
        Some(Transition {
            name: name.to_string(),
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
    fn calculation(&self, def: ElementId, out: &mut String) {
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

        let resolve = |leading: &str| {
            params
                .iter()
                .find(|(param, _)| param == leading)
                .map(|(param, _)| ident(param))
        };
        let translated = match &clause {
            Some(ValueClause::Text(text)) => {
                translate_as(text, numbers_of(declared.as_deref()), &resolve, &|name| {
                    self.callable(name)
                })
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

        let (body, unfit) = match (&clause, translated) {
            (_, Some(translated)) => (translated.rust, false),
            (Some(ValueClause::Literal(rust)), None) => (rust.clone(), false),
            // the model's own words, and `{` in them is the model's, not
            // a placeholder for `todo!` to fill
            (Some(ValueClause::Text(text)), None) => (format!("todo!(\"{{}}\", {text:?})"), true),
            (None, None) => ("todo!()".to_string(), false),
        };
        if body.starts_with("todo!") {
            self.open(
                OpenKind::Todo,
                name.clone(),
                Some(ident(spelled)),
                match &clause {
                    Some(ValueClause::Text(text)) => {
                        format!("the formula is beyond the translated subset: `{text}`")
                    }
                    _ => "the model states no formula".to_string(),
                },
            );
        }

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
    fn calc_method(&self, usage: ElementId, fields: &[Field]) -> Result<String, String> {
        let model = self.model;
        let name = model.name(usage).expect("named, or it was skipped");

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

        let resolve = |leading: &str| {
            if let Some((param, _)) = params.iter().find(|(param, _)| param == leading) {
                return Some(ident(param));
            }
            fields
                .iter()
                .find(|field| field.name == leading)
                .map(|field| format!("self.{}", field.spelled))
        };
        let clause = result_clause(model, usage).or_else(|| value_clause(model, usage));
        let translated = match &clause {
            Some(ValueClause::Text(text)) => {
                translate_as(text, numbers_of(Some(&returns)), &resolve, &|name| {
                    self.callable(name)
                })
            }
            _ => None,
        };
        let (body, unfit) = match (&clause, translated) {
            (_, Some(translated)) => (translated.rust, false),
            (Some(ValueClause::Literal(rust)), None) => (rust.clone(), false),
            // the model's own words, and `{` in them is the model's, not
            // a placeholder for `todo!` to fill
            (Some(ValueClause::Text(text)), None) => (format!("todo!(\"{{}}\", {text:?})"), true),
            (None, None) => ("todo!()".to_string(), false),
        };
        if body.starts_with("todo!") {
            self.open(
                OpenKind::Todo,
                name,
                Some(ident(name)),
                match &clause {
                    Some(ValueClause::Text(text)) => {
                        format!("the formula is beyond the translated subset: `{text}`")
                    }
                    _ => "the model states no formula".to_string(),
                },
            );
        }

        let mut method = String::new();
        writeln!(method, "    /// SysML: `calc {name}`").unwrap();
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
    fn state_starts(&self, usage: ElementId) -> bool {
        self.model.type_of(usage).is_some_and(|def| {
            self.model.owned(def).iter().any(|&child| {
                self.model.kind(child) == ElementKind::StateUsage
                    && self.model.name(child).is_some()
            })
        })
    }

    /// A `state` usage of a part: the state its machine is in, as the
    /// generated enum, under the name the model gave the usage.
    fn state_field(&self, usage: ElementId) -> Option<(String, String)> {
        let def = self.model.type_of(usage)?;
        if self.shapes.get(&def) != Some(&Shape::StateMachine) {
            return None;
        }
        let name = self.model.name(usage)?.to_string();
        Some((name, format!("{}State", type_ident(self.model.name(def)?))))
    }

    /// An `assert constraint` usage as the check it stands for. The model
    /// claims the constraint holds; nothing in Rust can hold anyone to
    /// that, so what is generated is the means of asking.
    fn assertion(&self, usage: ElementId, fields: &[Field]) -> Result<String, String> {
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
    fn decomposition(&self, def: ElementId) -> Vec<String> {
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

    /// An action definition's body, compiled out of what the model says
    /// its parts are and how they are wired: the subactions in the order
    /// the successions put them in, each called with what the flows and
    /// bindings feed it, and the result read off the flows into the
    /// definition's own `out` parameters.
    ///
    /// It is all-or-nothing. A dataflow with a gap in it -- an input
    /// nothing feeds, a result nothing produces, an order that does not
    /// exist -- is not written half-way; the trait method stays open,
    /// and what stopped it is named, because a modeller who is one flow
    /// short should not have to guess which one.
    fn action_body(&self, def: ElementId) -> Result<Body, String> {
        let model = self.model;
        let mut steps: Vec<(ElementId, ElementId)> = Vec::new();
        for &child in model.owned(def) {
            if !matches!(
                model.kind(child),
                ElementKind::ActionUsage | ElementKind::PerformActionUsage
            ) {
                continue;
            }
            let named = model.name(child).ok_or("a subaction with no name")?;
            let performed = model
                .type_of(child)
                .ok_or_else(|| format!("`{named}` performs nothing"))?;
            if self.shapes.get(&performed) != Some(&Shape::Action) {
                return Err(format!("`{named}` performs no generated action"));
            }
            steps.push((child, performed));
        }
        if steps.is_empty() {
            // a leaf behaviour is not a dataflow with a gap in it; there
            // was never anything here to perform, and saying so on every
            // one of them is noise
            return Err(String::new());
        }

        // `first a then b` is an edge, and the order they make is the
        // order the calls go in
        let mut after: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
        for &child in model.owned(def) {
            if !model.kind(child).is_a(ElementKind::SuccessionAsUsage) {
                continue;
            }
            let (from, to) = self
                .end_chains(child)
                .ok_or("a succession whose ends did not resolve")?;
            // `end_chains` answers only for two ends that resolved, so
            // each has at least the one segment it resolved from
            after
                .entry(*from.first().expect("a resolved end has a segment"))
                .or_default()
                .push(*to.first().expect("a resolved end has a segment"));
        }
        let order = ordered(&steps.iter().map(|&(u, _)| u).collect::<Vec<_>>(), &after)
            .ok_or("its subactions follow one another in a circle")?;

        // every flow, by the (subaction, parameter) it arrives at
        let mut into: HashMap<(Option<ElementId>, ElementId), (ElementId, ElementId)> =
            HashMap::new();
        for &child in model.owned(def) {
            if !model.kind(child).is_a(ElementKind::FlowUsage) {
                continue;
            }
            let (from, to) = self
                .end_chains(child)
                .ok_or("a flow whose ends did not resolve")?;
            let [source, out] = from.as_slice() else {
                return Err("a flow that does not come out of a subaction".to_string());
            };
            // `subaction.parameter` arrives at that subaction; anything
            // else names the definition's own parameter, which is where
            // a result comes from
            let target = if let [usage, parameter] = to.as_slice() {
                (Some(*usage), *parameter)
            } else {
                (None, *to.last().expect("a resolved end has a segment"))
            };
            into.insert(target, (*source, *out));
        }
        let mut fanout: HashMap<(ElementId, ElementId), usize> = HashMap::new();
        for &source in into.values() {
            *fanout.entry(source).or_default() += 1;
        }

        // how many subactions each enclosing parameter is fed to
        let mut remaining: HashMap<String, usize> = HashMap::new();
        for parameter in self.in_parameters(def) {
            if let Some(name) = model.name(parameter) {
                remaining.insert(ident(name), self.bindings_naming(def, name));
            }
        }

        let mut lines = Vec::new();
        for &usage in &order {
            let (_, performed) = *steps
                .iter()
                .find(|&&(u, _)| u == usage)
                .expect("ordered from the steps");
            let mine = model.name(usage).unwrap_or("?");
            let mut arguments = Vec::new();
            for parameter in self.in_parameters(performed) {
                let named = model.name(parameter).unwrap_or("?");
                arguments.push(
                    self.fed_by(def, usage, parameter, &into, &mut fanout, &mut remaining)
                        .ok_or_else(|| format!("nothing feeds `{mine}.{named}`"))?,
                );
            }
            lines.push(format!(
                "let {} = self.{}({});",
                ident(mine),
                ident(model.name(performed).unwrap_or("?")),
                arguments.join(", ")
            ));
        }

        // the result is whatever flows into the definition's own outs
        let mut results = Vec::new();
        for out in model.owned(def).iter().copied() {
            if model.direction(out) != Some("out") {
                continue;
            }
            let named = model.name(out).unwrap_or("?");
            let &(source, parameter) = into
                .get(&(None, out))
                .ok_or_else(|| format!("nothing produces `{named}`"))?;
            results.push(
                self.read_out(source, parameter, &mut fanout)
                    .ok_or_else(|| format!("what flows into `{named}` is no result of its own"))?,
            );
        }
        match results.len() {
            0 => {}
            1 => lines.push(results.join("")),
            _ => lines.push(format!("({})", results.join(", "))),
        }

        let mut supertraits: Vec<String> = Vec::new();
        for &(_, performed) in &steps {
            if performed == def {
                // a behaviour made of itself is not a supertrait of
                // itself, and Rust reads that as a circle
                return Err("it is made of itself".to_string());
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
    /// or a value the usage bound it to.
    fn fed_by(
        &self,
        def: ElementId,
        usage: ElementId,
        parameter: ElementId,
        into: &HashMap<(Option<ElementId>, ElementId), (ElementId, ElementId)>,
        fanout: &mut HashMap<(ElementId, ElementId), usize>,
        remaining: &mut HashMap<String, usize>,
    ) -> Option<String> {
        if let Some(&(source, out)) = into.get(&(Some(usage), parameter)) {
            return self.read_out(source, out, fanout);
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
        let parameters = self.in_parameters(def);
        let resolve = |leading: &str| {
            parameters
                .iter()
                .find(|&&p| self.model.name(p) == Some(leading))
                .map(|_| ident(leading))
        };
        let read = match bound {
            // `TrainingRun::stream` names the enclosing action's own
            // parameter, and the argument is what Rust calls it
            ValueClause::Text(text) => {
                let tail = text.rsplit("::").next()?.trim().to_string();
                translate(&tail, &resolve, &|name| self.callable(name))?.rust
            }
            ValueClause::Literal(rust) => rust,
        };
        // a parameter fed to more than one subaction is cloned until the
        // last of them, which may take it -- unless it is a scalar, which
        // every call copies anyway
        let left = remaining.entry(read.clone()).or_insert(0);
        *left = left.saturating_sub(1);
        let copyable = self
            .parameter_shape(parameter)
            .is_some_and(|(base, _)| copyable(&base));
        Some(if *left > 0 && !copyable {
            format!("{read}.clone()")
        } else {
            read
        })
    }

    /// How many of the subactions bind one of the enclosing parameters.
    fn bindings_naming(&self, def: ElementId, name: &str) -> usize {
        self.model
            .owned(def)
            .iter()
            .filter(|&&child| {
                matches!(
                    self.model.kind(child),
                    ElementKind::ActionUsage | ElementKind::PerformActionUsage
                )
            })
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
        Some(if !copyable && *left > 0 {
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
            .filter(|&&child| self.model.kind(child) == ElementKind::Feature)
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
    fn ends_of(&self, usage: ElementId) -> Option<String> {
        let ends = self.model.related_feature(usage);
        let named: Vec<&str> = ends
            .iter()
            .filter_map(|&end| self.model.name(end))
            .collect();
        (named.len() == 2).then(|| format!("`{}` -> `{}`", named[0], named[1]))
    }

    /// What an action definition's `out` parameters make of its result:
    /// nothing, the one type, or a tuple of them.
    fn action_result(&self, def: ElementId) -> Option<String> {
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
    fn action(&self, def: ElementId, out: &mut String) {
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
        let name = model.name(usage).expect("named, or it was skipped");
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
    fn delegation(
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
        let name = model.name(usage).expect("named, or it was skipped");
        let spelled = model.name(def).expect("collected named");
        let def_name = type_ident(spelled);
        let resolve = |leading: &str| {
            fields
                .iter()
                .find(|field| field.name == leading)
                .map(|field| format!("self.{}", field.spelled))
        };

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
                        .map(Translated::plain)
                        .or_else(|| translate(&text, &resolve, &|name| self.callable(name)));
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

    /// An unbound port as a plain field of its generated port struct.
    fn plain_port(&self, def: ElementId, usage: ElementId) -> Option<Field> {
        let target = self.model.type_of(usage)?;
        if self.shapes.get(&target) != Some(&Shape::Struct) || self.has_api_ports(target) {
            return None;
        }
        let container = multiplicity(self.model, usage);
        let name = self.model.name(usage)?.to_string();
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
    fn plain_ports(&self, def: ElementId) -> Vec<Field> {
        self.model
            .owned(def)
            .iter()
            .filter(|&&child| self.model.kind(child) == ElementKind::PortUsage)
            .filter_map(|&child| self.plain_port(def, child))
            .collect()
    }

    /// Does this definition hold ports bound to external APIs -- which
    /// generate as generic parameters, not composable fields?
    fn has_api_ports(&self, def: ElementId) -> bool {
        self.model.owned(def).iter().any(|&child| {
            self.model.kind(child) == ElementKind::PortUsage && self.port_of(child).is_some()
        })
    }

    /// The model's requirements, as tests.
    ///
    /// Each names what the model says satisfies it. A requirement a
    /// verification case answers for becomes a test that runs that
    /// case's Rust; one with no verification becomes an ignored stub
    /// that says so, which is a list of what has been promised and not
    /// yet kept.
    fn requirements(&self, roots: &[ElementId], out: &mut String) {
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
    /// with the Rust their `@rust` bindings name.
    fn claims(&self) -> Claims {
        let model = self.model;
        let mut claims = Claims::default();
        for id in model.ids() {
            if model.kind(id).is_a(ElementKind::SatisfyRequirementUsage)
                // `not satisfy r by p;` says p does not, so naming it as
                // what answers for `r` would put the opposite of the
                // model in the doc
                && model.get(id, "isNegated") != Some(&Value::Bool(true))
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
                (model.get(id, "verifiedRequirement"), model.name(id))
            {
                let path = binding(model, id).and_then(|bound| bound.get(binding::PATH).cloned());
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

    fn port_of(&self, usage: ElementId) -> Option<Port> {
        let port_def = self.model.type_of(usage)?;
        let bound = binding(self.model, port_def)?;
        let path = bound.get(binding::PATH)?.clone();
        Some(Port {
            name: self.model.name(usage)?.to_string(),
            type_name: self.model.name(port_def)?.to_string(),
            parameter: String::new(),
            trait_path: path,
        })
    }

    /// One performed API action as a delegating method.
    fn method(
        &self,
        part_name: &str,
        usage: ElementId,
        ports: &[Port],
        fields: &[Field],
    ) -> Result<String, RustgenError> {
        let model = self.model;
        let usage_name = model.name(usage).expect("named, or it was skipped");
        // Every way this method declines says the same two things -- which
        // performed action, and why -- so it says them in one place, to the
        // reader of the file and to whoever has to write what is missing.
        let declined = |why: String| {
            self.open(OpenKind::Skipped, usage_name, None, why.clone());
            Ok(format!(
                "    // not generated: perform `{usage_name}` -- {why}\n"
            ))
        };
        let Some(action) = model.type_of(usage) else {
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
                "`{}` carries no `@rust` binding",
                model.name(action).unwrap_or("?")
            ));
        };
        let path = bound.get(binding::PATH).cloned().unwrap_or_default();
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
        writeln!(body, "{signature} {{").unwrap();
        writeln!(body, "        {call}").unwrap();
        writeln!(body, "    }}").unwrap();
        Ok(body)
    }

    /// The Rust type of an action parameter.
    fn parameter_type(&self, usage: ElementId) -> Option<String> {
        let (base, container) = self.parameter_shape(usage)?;
        Some(contained(base, container))
    }

    /// How Rust spells a call to a named calculation: the function its
    /// definition generated, where the definition generated one. An
    /// `abstract` definition became a trait instead, and a trait method
    /// is not callable out of nowhere.
    fn callable(&self, name: &str) -> Option<expr::Callee> {
        let (&target, _) = self.shapes.iter().find(|(&id, &shape)| {
            shape == Shape::Calculation && self.model.name(id) == Some(name)
        })?;
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
    fn parameter_shape(&self, usage: ElementId) -> Option<(String, Container)> {
        // a parameter bound by a usage -- `in previous = stateVector` --
        // declares no type of its own; it keeps the one it redefines
        let declaring = self
            .model
            .type_of(usage)
            .map(|_| usage)
            .or_else(|| redefined(self.model, usage))?;
        let ty = self.model.type_of(declaring)?;
        // a parameter carries its multiplicity the way a field does: a
        // `[1..*]` of something is a `Vec` of it, not one of it
        let container = multiplicity(self.model, declaring);
        if let Some(bound) = binding(self.model, ty) {
            return bound
                .get(binding::PATH)
                .cloned()
                .map(|path| (path, container));
        }
        if self.written_as_a_type(ty) {
            return self
                .model
                .name(ty)
                .map(|name| (type_ident(name), container));
        }
        Some((scalar_of(self.model.name(ty)?)?.to_string(), container))
    }
}

/// What the model claims about its requirements: which features
/// satisfy each, and which verification cases answer for it, each with
/// the Rust path its binding names where it names one.
#[derive(Default)]
struct Claims {
    satisfiers: HashMap<ElementId, Vec<String>>,
    verifications: HashMap<ElementId, Vec<(String, Option<String>)>>,
}

/// A port of a part, once its type turned out to be a bound API trait.
struct Port {
    name: String,
    type_name: String,
    parameter: String,
    trait_path: String,
}

/// One data field of a generated struct.
#[derive(Clone)]
struct Field {
    name: String,
    /// The name Rust knows it by, which is `ident(name)` unless another
    /// field of the same struct got there first.
    spelled: String,
    usage: ElementId,
    ty: FieldType,
    container: Container,
    /// The element type sits behind a `Box`, breaking a composition cycle.
    boxed: bool,
    /// The declared `= value`, already rendered as Rust.
    default: Option<String>,
}

/// The names taken in one Rust namespace. What SysML keeps apart Rust
/// may not -- `fuelTank`, `fuel_tank` and `'fuel tank'` are three names
/// in a model and one in Rust -- so the second one to want a spelling
/// is numbered rather than written twice over.
#[derive(Default)]
struct Namespace(HashSet<String>);

impl Namespace {
    fn take(&mut self, wanted: String) -> String {
        let mut spelled = wanted.clone();
        let mut at = 1;
        while !self.0.insert(spelled.clone()) {
            at += 1;
            spelled = format!("{wanted}_{at}");
        }
        spelled
    }
}

#[derive(Clone)]
enum FieldType {
    Scalar(&'static str),
    Generated(ElementId),
    External(String),
}

/// What a multiplicity wraps the element type in.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    One,
    Optional,
    Many,
    Array(i64),
}

/// The `= value` clause of a usage: a literal already rendered as Rust,
/// or the SysML text of an expression, still to translate.
enum ValueClause {
    Literal(String),
    Text(String),
}

/// One transition, ready to be written into the table.
struct Transition {
    name: String,
    source: String,
    target: String,
    event: String,
    /// The accepted payload: its parameter name and Rust type.
    payload: Option<(String, String)>,
    /// The guard's SysML text, for the hook's documentation.
    guard: Option<String>,
    effect: bool,
}

/// A compiled action body: the traits its parts demand of whoever
/// implements it, and the statements that perform them in order.
struct Body {
    supertraits: Vec<String>,
    lines: Vec<String>,
}

/// The subactions in an order the successions allow, or nothing where
/// they lead in a circle.
fn ordered(
    steps: &[ElementId],
    after: &HashMap<ElementId, Vec<ElementId>>,
) -> Option<Vec<ElementId>> {
    let mut order: Vec<ElementId> = Vec::new();
    let mut left: Vec<ElementId> = steps.to_vec();
    while !left.is_empty() {
        // whatever nothing left is waiting on can go next
        let next = left.iter().position(|&step| {
            !left
                .iter()
                .any(|&other| after.get(&other).is_some_and(|nexts| nexts.contains(&step)))
        })?;
        order.push(left.remove(next));
    }
    Some(order)
}

/// Which numeric types a calculation's parameters disagree over, if they
/// do. Rust combines none of them without a cast, and this generator
/// writes no cast, so a formula over two of them will not compile.
fn mixed_numerics(params: &[(String, String)]) -> Option<String> {
    let numeric: Vec<&(String, String)> = params
        .iter()
        .filter(|(_, ty)| matches!(ty.as_str(), "f64" | "i64" | "u64"))
        .collect();
    let mut kinds: Vec<&str> = numeric.iter().map(|(_, ty)| ty.as_str()).collect();
    kinds.sort_unstable();
    kinds.dedup();
    if kinds.len() < 2 {
        return None;
    }
    let named: Vec<String> = numeric
        .iter()
        .map(|(name, ty)| format!("`{name}` is {ty}"))
        .collect();
    Some(format!(
        "Mixed numeric parameters ({}). Rust combines none of these without a cast \
         and none is written, so a formula over more than one of them will not \
         compile: say them in one type in the model.",
        named.join(", ")
    ))
}

/// The return clause of a signature -- nothing at all where there is
/// nothing to return, since `-> ()` is noise, and noise clippy objects to.
fn returns_clause(returns: &str) -> String {
    if returns == "()" {
        String::new()
    } else {
        format!(" -> {returns}")
    }
}

/// Whether a Rust type is `Copy`, so that reading a value of it twice
/// costs nothing. What is not gets a `.clone()` wherever it is read
/// more than once. Three places asked this and had to agree.
fn copyable(ty: &str) -> bool {
    matches!(ty, "f64" | "i64" | "u64" | "bool")
}

/// A type under the container its multiplicity asks for.
fn contained(base: String, container: Container) -> String {
    match container {
        Container::One => base,
        Container::Optional => format!("Option<{base}>"),
        Container::Many => format!("Vec<{base}>"),
        Container::Array(n) => format!("[{base}; {n}]"),
    }
}

/// The declared multiplicity of a usage, as the container it implies.
fn multiplicity(model: &Model, usage: ElementId) -> Container {
    let Some(Value::Ref(range)) = model.get(usage, "multiplicity") else {
        return Container::One;
    };
    let bound_value = |name: &str| -> Option<BoundKind> {
        let Some(Value::Ref(bound)) = model.get(*range, name) else {
            return None;
        };
        if model.kind(*bound) == ElementKind::LiteralInfinity {
            return Some(BoundKind::Many);
        }
        match model.get(*bound, "value") {
            Some(Value::Int(int)) => Some(BoundKind::Exactly(*int)),
            _ => Some(BoundKind::Unknown),
        }
    };
    match (
        bound_value("bound"),
        bound_value("lowerBound"),
        bound_value("upperBound"),
    ) {
        (Some(BoundKind::Many), _, _) => Container::Many,
        // `[0]` is a multiplicity of nothing, which Rust spells as an
        // array of nothing; a `Vec` would claim it can hold more
        (Some(BoundKind::Exactly(n)), _, _) if n >= 0 => Container::Array(n),
        (None, Some(BoundKind::Exactly(0)), Some(BoundKind::Exactly(1))) => Container::Optional,
        // `[1..1]` is the multiplicity everything has by default, said
        // out loud -- one of the thing, not a collection of them
        (None, Some(BoundKind::Exactly(1)), Some(BoundKind::Exactly(1))) => Container::One,
        (None, Some(_), Some(_)) => Container::Many,
        _ => Container::Many,
    }
}

enum BoundKind {
    Many,
    Exactly(i64),
    Unknown,
}

/// The `= value` a usage declared, as far as Rust can start from it: a
/// literal as itself, a closed simple expression (`2.0 * 3.0`) as
/// written, a name as whatever the feature it refers to starts from --
/// any other expression over other features has no place in
/// `Default::default()`.
fn default_of(model: &Model, usage: ElementId) -> Option<String> {
    match value_clause(model, usage)? {
        ValueClause::Literal(rust) => Some(rust),
        ValueClause::Text(text) => match translate_as(
            &text,
            match wants_float(model, usage) {
                true => Numbers::AsReals,
                false => Numbers::AsWritten,
            },
            &|_| None,
            &|_| None,
        ) {
            Some(translated) => Some(translated.rust),
            // A value that is only a name refers to a feature, and name
            // resolution wrote down which one, so the value to start
            // from is that feature's own. One step: what the feature it
            // names starts from has to be a value in itself, or there
            // is nothing here to write.
            None => match value_clause(model, referent_of(model, usage)?)? {
                ValueClause::Literal(rust) => Some(rust),
                ValueClause::Text(_) => None,
            },
        },
    }
}

/// The numbers a requirement states about itself, in the order it
/// states them -- the bounds of `attribute lowerBound : Millis = 100;`
/// and its like.
///
/// A verification is handed these rather than repeating them, so that
/// the requirement stays the only place they are written. Changing one
/// in the model changes the call, and a verification that no longer
/// fits it stops compiling, which is the point.
fn declared_values(model: &Model, requirement: ElementId) -> Vec<String> {
    model
        .owned(requirement)
        .iter()
        .filter(|&&child| model.kind(child) == ElementKind::AttributeUsage)
        .filter_map(|&child| default_of(model, child))
        .collect()
}

/// The feature a usage's value refers to, where the whole value is a
/// name -- `attribute pin : PinNumber = ledPinNumber;`.
fn referent_of(model: &Model, usage: ElementId) -> Option<ElementId> {
    let membership = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    let expression = model.get(membership, "value")?.as_id()?;
    model.get(expression, "referent").and_then(Value::as_id)
}

/// The trailing result expression of a calculation body, where one was
/// written -- the last one wins, as the spec has it.
fn result_clause(model: &Model, element: ElementId) -> Option<ValueClause> {
    let mut clause = None;
    for &child in model.owned(element) {
        if model.kind(child) == ElementKind::Expression
            && model.member_role(child) == Some(Role::Result)
        {
            if let Some(text) = expression_text(model, child) {
                clause = Some(ValueClause::Text(text));
            }
        }
    }
    clause
}

/// The Rust scalar a SysML type stands for, where it stands for one.
/// Fields and parameters both ask this, so that one model type cannot
/// be a `f64` in a struct and something else in a signature.
fn scalar_of(name: &str) -> Option<&'static str> {
    Some(match name {
        "Real" => "f64",
        "Integer" => "i64",
        // `Positive` is a `Natural` the model has ruled zero out of;
        // Rust has no such integer that is also `Default`
        "Natural" | "Positive" => "u64",
        "Boolean" => "bool",
        "String" => "String",
        _ => return None,
    })
}

/// Whether a usage is typed by something Rust spells as a float. SysML
/// writes `attribute shift : Real = 1670;` and means 1670.0; Rust reads
/// that literal as an integer and refuses it, so the point has to be
/// put back.
fn wants_float(model: &Model, usage: ElementId) -> bool {
    let Some(target) = model
        .type_of(usage)
        .or_else(|| redefined(model, usage).and_then(|it| model.type_of(it)))
    else {
        return false;
    };
    match binding(model, target).and_then(|bound| bound.get(binding::PATH).cloned()) {
        Some(path) => path == "f32" || path == "f64",
        None => model.name(target).and_then(scalar_of) == Some("f64"),
    }
}

/// A real as Rust spells it. `{:?}` writes `inf` and `NaN`, which are
/// what a model overflowing a `f64` -- `1e999` -- arrives as and which
/// Rust does not read back as anything.
fn real_literal(real: f64) -> String {
    match real {
        _ if real.is_nan() => "f64::NAN".to_string(),
        f64::INFINITY => "f64::INFINITY".to_string(),
        f64::NEG_INFINITY => "f64::NEG_INFINITY".to_string(),
        _ => format!("{real:?}"),
    }
}

/// How a Rust type of `spelled` wants its whole numbers written.
fn numbers_of(spelled: Option<&str>) -> Numbers {
    match spelled {
        Some("f32" | "f64") => Numbers::AsReals,
        _ => Numbers::AsWritten,
    }
}

/// The `= value` clause of a usage, ready for translation.
fn value_clause(model: &Model, usage: ElementId) -> Option<ValueClause> {
    let membership = model
        .owned(usage)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    let Some(Value::Ref(expression)) = model.get(membership, "value") else {
        return None;
    };
    let rust = match model.get(*expression, "value") {
        Some(Value::Real(real)) => real_literal(*real),
        Some(Value::Int(int)) if wants_float(model, usage) => format!("{int}.0"),
        Some(Value::Int(int)) => format!("{int}"),
        Some(Value::Bool(flag)) => format!("{flag}"),
        Some(Value::String(text)) => format!("{text:?}.to_string()"),
        _ => return expression_text(model, *expression).map(ValueClause::Text),
    };
    Some(ValueClause::Literal(rust))
}

/// The written text of an expression element, off the textual
/// representation the builder keeps for what it does not evaluate.
fn expression_text(model: &Model, expression: ElementId) -> Option<String> {
    model.owned(expression).iter().find_map(|&written| {
        (model.kind(written) == ElementKind::TextualRepresentation)
            .then(|| model.get(written, "body"))
            .flatten()?
            .as_str()
            .map(str::to_string)
    })
}

/// What an unnamed redefining usage redefines, so it can borrow the name.
fn redefined(model: &Model, usage: ElementId) -> Option<ElementId> {
    model.owned(usage).iter().find_map(|&child| {
        (model.kind(child) == ElementKind::Redefinition)
            .then(|| model.redefined_feature(child))
            .flatten()
    })
}

/// The `@rust { :>> name = value; ... }` pairs of one element, if it
/// carries a binding.
fn binding(model: &Model, element: ElementId) -> Option<HashMap<String, String>> {
    let mut out = HashMap::new();
    for &child in model.owned(element) {
        if model.kind(child) != ElementKind::MetadataUsage {
            continue;
        }
        // `@Safety { :>> level = "high"; }` has the shape of a binding
        // and means nothing of the sort; only the metadata definition
        // this crate owns says which Rust item an element stands for
        if model.type_of(child).and_then(|def| model.name(def)) != Some(binding::DEF) {
            continue;
        }
        for &setting in model.owned(child) {
            let redefined = model.owned(setting).iter().copied().find_map(|rel| {
                if model.kind(rel) != ElementKind::Redefinition {
                    return None;
                }
                match model.get(rel, "redefinedFeature") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                }
            });
            let value = model.owned(setting).iter().copied().find_map(|part| {
                if model.kind(part) != ElementKind::FeatureValue {
                    return None;
                }
                match model.get(part, "value") {
                    Some(Value::Ref(literal)) => Some(*literal),
                    _ => None,
                }
            });
            let (Some(redefined), Some(value)) = (redefined, value) else {
                continue;
            };
            let Some(name) = model.name(redefined) else {
                continue;
            };
            let rendered = match model.get(value, "value") {
                Some(Value::String(text)) => text.clone(),
                Some(Value::Bool(flag)) => flag.to_string(),
                Some(Value::Int(int)) => int.to_string(),
                _ => continue,
            };
            out.insert(name.to_string(), rendered);
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The element's documentation text, if it has any, laid out as it was
/// written: a `doc /* ... */` spanning several lines keeps its lines, and
/// each of them loses the indentation and the `*` that line it up in the
/// model text -- neither of which belongs in a Rust doc comment.
fn documentation(model: &Model, element: ElementId) -> Option<String> {
    model.owned(element).iter().find_map(|&child| {
        if model.kind(child) != ElementKind::Documentation {
            return None;
        }
        let body = model.get(child, "body")?.as_str()?;
        let lines: Vec<&str> = body
            .lines()
            .map(|line| {
                let line = line.trim_start();
                let line = line.strip_prefix('*').unwrap_or(line);
                // one space after the `*` is the marker's own; any
                // further indentation the writer meant, so it stays
                let line = line.strip_prefix(' ').unwrap_or(line);
                line.trim_end()
            })
            .skip_while(|line| line.is_empty())
            .collect();
        let end = lines.iter().rposition(|line| !line.is_empty())? + 1;
        Some(lines[..end].join("\n"))
    })
}

/// Documentation as Rust doc comments, one line of `///` per line of the
/// original, each behind `indent` so a field's docs sit with the field.
fn doc_comment(doc: &str, indent: &str, out: &mut String) {
    for line in doc.lines() {
        if line.is_empty() {
            writeln!(out, "{indent}///").unwrap();
        } else {
            writeln!(out, "{indent}/// {line}").unwrap();
        }
    }
}

/// `<...>` unless there is nothing to put in it.
fn angle(list: &str) -> String {
    if list.is_empty() {
        String::new()
    } else {
        format!("<{list}>")
    }
}

/// `StockVisibility` -> `stock_visibility`: the case Rust asks of
/// everything but a type, and SysML of nothing.
fn snake(name: &str) -> String {
    let mut out = String::new();
    for (at, ch) in name.chars().enumerate() {
        if ch.is_uppercase() {
            if at > 0 {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// `store` -> `Store`, for parameters, variants and event names.
fn camel(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for ch in name.chars() {
        if ch == '_' {
            upper = true;
        } else if upper {
            out.extend(ch.to_uppercase());
            upper = false;
        } else {
            out.push(ch);
        }
    }
    out
}

/// A definition's name with what owns it in front, for saying which of
/// two things with one name is meant.
fn qualified_of(model: &Model, def: ElementId) -> String {
    let mut parts = Vec::new();
    let mut at = Some(def);
    while let Some(id) = at {
        if let Some(name) = model.name(id) {
            parts.push(name.to_string());
        }
        at = model.owner(id);
    }
    parts.reverse();
    parts.join("::")
}

/// Whether Rust will accept a character inside an identifier.
///
/// Rust's rule is Unicode's XID, which `char::is_alphanumeric` is not:
/// it says yes to `²` and to the Arabic-Indic digits, and `x²` is not a
/// name Rust can spell. Letters and ASCII digits are the part of XID
/// this generator needs, and anything else is dropped.
fn spellable(ch: char) -> bool {
    ch.is_alphabetic() || ch.is_ascii_digit()
}

/// A SysML name as the name of a Rust *type*. An escaped name holds
/// whatever the modeller wrote -- `'Ideal Gas Parcel'` is one name in
/// SysML -- and Rust spells type names out of a much smaller alphabet,
/// so anything it cannot use joins the words up instead of ending the
/// name early.
fn type_ident(name: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for ch in name.chars() {
        if spellable(ch) {
            if upper {
                out.extend(ch.to_uppercase());
                upper = false;
            } else {
                out.push(ch);
            }
        } else {
            // a space or a hyphen joins two words rather than cutting one
            upper = !out.is_empty();
        }
    }
    if out.is_empty() {
        // a name Rust cannot spell one character of -- `'+'`, `''` --
        // still has to be called something, and `_` is not a name
        out.push_str("Unnamed");
    } else if out.starts_with(|ch: char| ch.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if TAKEN_TYPES.contains(&out.as_str()) {
        out.push('_');
    }
    out
}

/// Type names a generated file cannot have, because it uses them for
/// something else. Rust's own keywords are here for the obvious reason;
/// so are the prelude names and primitives this generator writes -- a
/// `part def Vec` taking `Vec` would leave every `Vec<T>` in the file
/// naming a struct that has no parameter, and a `part def Default` the
/// same for every derive.
const TAKEN_TYPES: [&str; 61] = [
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in",
    "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "Self", "static", "struct", "super", "trait", "true", "try", "type",
    "typeof", "unsafe", "unsized", "use", "virtual", "where", "while", "yield", "Box", "Vec",
    "Option", "String", "Default", "Result", "bool", "f64", "i64", "u64",
];

/// A SysML name as the Rust name of a field, parameter or method:
/// `spectralRadius` -> `spectral_radius`, since the model's own
/// convention is the one Rust keeps for types; anything Rust cannot
/// spell in an identifier becomes `_`; and a reserved word is taken raw,
/// or suffixed where no raw form exists.
pub(crate) fn ident(name: &str) -> String {
    /// Everything Rust has taken, including what it has only reserved.
    const RESERVED: [&str; 51] = [
        "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "do",
        "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "if", "impl", "in", "let",
        "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref", "return",
        "static", "struct", "trait", "true", "try", "type", "typeof", "unsafe", "unsized", "use",
        "virtual", "where", "while", "yield",
        // no raw form exists for these, so they take a suffix instead
        "crate", "self", "Self", "super",
    ];
    /// The keywords `r#` cannot rescue.
    const SUFFIXED: [&str; 4] = ["crate", "self", "Self", "super"];

    // an escaped name may hold anything at all -- `'provide transportation'`
    // is one name in SysML -- and Rust spells identifiers out of a much
    // smaller alphabet
    let name: String = snake(name)
        .chars()
        .map(|ch| if spellable(ch) || ch == '_' { ch } else { '_' })
        .collect();
    let name = if name.starts_with(|ch: char| ch.is_ascii_digit()) {
        format!("_{name}")
    } else if name.is_empty() || name == "_" {
        // `_` is a pattern, not a name, and a name Rust can spell no
        // part of still has to be called something
        "unnamed".to_string()
    } else {
        name
    };
    if SUFFIXED.contains(&name.as_str()) {
        format!("{name}_")
    } else if RESERVED.contains(&name.as_str()) {
        format!("r#{name}")
    } else {
        name
    }
}
