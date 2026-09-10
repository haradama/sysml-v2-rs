//! Deciding what to write, before any of it is written.
//!
//! A definition's Rust shape depends on the shapes of everything it
//! composes: whether a struct can derive `Debug`, whether it can answer
//! `Default::default()`, which generic parameters it needs so that a
//! part with API-bound ports composes, and where a composition cycle has
//! to be broken with a `Box` to keep the type finite. None of those can
//! be settled for one definition on its own, so they are settled for all
//! of them at once, by passes that run until they stop changing.
//!
//! Nothing here writes a character of Rust. It works out the answers the
//! writing then reads off.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use sysml_model::{ElementId, ElementKind};

use super::*;

impl<'a> Generator<'a> {
    pub(crate) fn collect(model: &'a Model, roots: &[ElementId]) -> Generator<'a> {
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
}
