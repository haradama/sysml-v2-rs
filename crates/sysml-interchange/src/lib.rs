//! Standard JSON interchange for [`sysml_model::Model`].
//!
//! Follows the serialization style of the SysML v2 API & Services standard:
//! every element is a JSON object with `"@type"` (metaclass name), `"@id"`
//! (UUID) and its properties, where element references are `{"@id": ...}`
//! objects. Element UUIDs are deterministic (UUIDv5 over the element's
//! ownership path), so exporting the same model twice yields identical JSON.
//!
//! Every element is serialized with the complete property set its
//! metaclass declares, the shape the standard's API serializes: stored
//! properties as they are, derivable ones derived, and the rest at their
//! defaults (`null`, `[]`, `false`). Derived here are identity and naming
//! (`elementId`, `name`, `shortName`, `qualifiedName`), the whole
//! ownership web (`owner`/`ownedElement`, `ownedRelationship`,
//! `owningRelationship`, `owningMembership`, `owningNamespace`), a
//! relationship's related elements (`relatedElement`, `source`, `target`,
//! `ownedRelatedElement`, `owningRelatedElement`), a membership's member
//! (`memberElement`, `memberName` and their owned forms), and annotation
//! bindings (`documentation`, `textualRepresentation`).
//!
//! The inheritance closure is derived too, from the relationships name
//! resolution reified: `feature`, `inheritedFeature` and
//! `inheritedMembership` walk the resolved specializations and typings
//! (redefined features excepted), `input`/`output`/`parameter` read the
//! declared directions, and a feature's `type` is its typings' targets.
//! An unresolved model derives empty closures -- which is what it knows.
//!
//! Ownership is reified the way the abstract syntax has it: a membership
//! bridges a namespace and each ordinary element it owns, while a pure
//! relationship owns its elements directly as `ownedRelatedElement`. The
//! membership's metaclass follows the member: `FeatureMembership` for a
//! feature of a type, `EndFeatureMembership` for a connector end,
//! `ParameterMembership`/`ReturnParameterMembership` for a directed
//! feature of a behavior, `SubjectMembership`, `ActorMembership`,
//! `StakeholderMembership`, `ObjectiveMembership` and `VariantMembership`
//! for members declared in those roles, `TransitionFeatureMembership`
//! (with its `kind`) for a transition's trigger, guard and effect,
//! `StateSubactionMembership` (kind `entry`/`do`/`exit`) for a state's
//! subactions, `RequirementConstraintMembership` (kind `assumption` or
//! `requirement`) and `FramedConcernMembership` for a requirement's
//! constraints and concerns, and `OwningMembership` otherwise. Each carries the visibility the member
//! was declared with. Bridging memberships are synthesized on export with
//! deterministic UUIDs and folded back on import -- role, visibility and
//! all -- so either shape, this crate's or another tool's, reads back
//! into the same model.
//!
//! What only a resolver can know -- the members imports bring in, what
//! each import resolved to, and which elements belong to a library model
//! -- comes in through [`Extras`]: `sysml-cli`'s `export` builds it from
//! `sysml-semantics` (`imported_members`, `import_of`) and the files
//! loaded as `--library`, and [`to_json_with`] folds it into `member`,
//! `membership`, `importedMembership`, `importedNamespace` and
//! `isLibraryElement`. Plain [`to_json`] leaves those at what the model
//! alone can say.
//!
//! `name`, `shortName`, `qualifiedName` and a membership's `memberName`
//! follow KerML's effective-name rule: a feature declared without a name
//! -- `attribute :>> mass;` -- answers to the name of what it redefines
//! (or, failing that, references). The implied specializations resolution
//! reasons with can be made part of the model itself with
//! `sysml-semantics`' `materialize_implied` -- each becomes an owned
//! relationship with `isImplied` set, which this serialization then
//! carries like any other; `sysml-cli`'s `export` runs it.
//!
//! Remaining simplification: derived properties beyond the ones named
//! here are emitted at their defaults.

use std::collections::HashMap;

use serde_json::{json, Map, Value as Json};
use sysml_model::{ElementId, ElementKind, FeatureType, Model, PrimitiveType, Value};
use uuid::Uuid;

/// Errors produced when reading interchange JSON.
#[derive(Debug)]
pub enum ImportError {
    NotAnArray,
    MissingType(usize),
    UnknownType(String),
    MissingId(usize),
    UnknownReference(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotAnArray => write!(f, "expected a JSON array of elements"),
            ImportError::MissingType(i) => write!(f, "element {i} has no \"@type\""),
            ImportError::UnknownType(t) => write!(f, "unknown metaclass {t:?}"),
            ImportError::MissingId(i) => write!(f, "element {i} has no \"@id\""),
            ImportError::UnknownReference(id) => write!(f, "reference to unknown element {id:?}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// Deterministic UUID for an element: v5 over its ownership path. Each
/// segment includes the sibling index (so same-named siblings stay unique)
/// plus the member name where available.
pub fn element_uuid(model: &Model, id: ElementId) -> Uuid {
    let mut segments = Vec::new();
    let mut current = Some(id);
    while let Some(elem) = current {
        let index = model
            .owner(elem)
            .map(|o| {
                model
                    .owned(o)
                    .iter()
                    .position(|c| *c == elem)
                    .unwrap_or_default()
            })
            .unwrap_or(elem.index());
        let segment = match model.name(elem) {
            Some(name) => format!("{index}:{name}"),
            None => format!("{index}"),
        };
        segments.push(segment);
        current = model.owner(elem);
    }
    segments.reverse();
    let path = format!("sysml-v2-rs:{}", segments.join("/"));
    Uuid::new_v5(&Uuid::NAMESPACE_URL, path.as_bytes())
}

/// The membership metaclasses that only carry ownership -- and, for some,
/// a role or a visibility the owned element keeps -- so they fold into
/// edges on import and are synthesized back on export. `FeatureValue` and
/// friends stay real elements: they carry state of their own.
const FOLDED: [ElementKind; 14] = [
    ElementKind::OwningMembership,
    ElementKind::FeatureMembership,
    ElementKind::EndFeatureMembership,
    ElementKind::ParameterMembership,
    ElementKind::ReturnParameterMembership,
    ElementKind::SubjectMembership,
    ElementKind::ActorMembership,
    ElementKind::StakeholderMembership,
    ElementKind::ObjectiveMembership,
    ElementKind::VariantMembership,
    ElementKind::TransitionFeatureMembership,
    ElementKind::StateSubactionMembership,
    ElementKind::RequirementConstraintMembership,
    ElementKind::FramedConcernMembership,
];

/// Property names the exporter computes rather than reads, skipped on
/// import wherever the metaclass marks them derived. A metaclass that
/// declares one of these as a stored fact of its own keeps it.
const SYNTHESIZED: [&str; 38] = [
    "relatedElement",
    "source",
    "target",
    "memberElement",
    "ownedMemberElement",
    "ownedMemberFeature",
    "memberElementId",
    "ownedMemberElementId",
    "feature",
    "ownedFeature",
    "inheritedFeature",
    "inheritedMembership",
    "endFeature",
    "ownedEndFeature",
    "input",
    "output",
    "directedFeature",
    "parameter",
    "membership",
    "ownedMembership",
    "member",
    "ownedMember",
    "ownedSpecialization",
    "ownedSubclassification",
    "ownedTyping",
    "ownedSubsetting",
    "ownedRedefinition",
    "ownedReferenceSubsetting",
    "ownedImport",
    "type",
    "owningType",
    "annotatedElement",
    "nestedUsage",
    "ownedUsage",
    "importedMembership",
    "isLibraryElement",
    "featuringType",
    "owningFeatureMembership",
];

/// The role a folded membership gives back to its member, so that what
/// picked the membership's metaclass survives the round trip. A state
/// subaction's and a requirement constraint's metaclass alone does not
/// say which role it was: their `kind` does.
fn folded_role(bridge: &Json) -> Option<&'static str> {
    let roles: [(ElementKind, &str); 6] = [
        (ElementKind::SubjectMembership, "subject"),
        (ElementKind::ActorMembership, "actor"),
        (ElementKind::StakeholderMembership, "stakeholder"),
        (ElementKind::ObjectiveMembership, "objective"),
        (ElementKind::VariantMembership, "variant"),
        (ElementKind::ReturnParameterMembership, "return"),
    ];
    let written = bridge["@type"].as_str();
    if let Some((_, role)) = roles.iter().find(|(kind, _)| Some(kind.name()) == written) {
        return Some(role);
    }
    match written {
        Some("StateSubactionMembership") => match bridge["kind"].as_str() {
            Some("entry") => Some("entry"),
            Some("do") => Some("do"),
            Some("exit") => Some("exit"),
            _ => None,
        },
        Some("FramedConcernMembership") => Some("frame"),
        Some("RequirementConstraintMembership") => match bridge["kind"].as_str() {
            Some("assumption") => Some("assume"),
            _ => Some("require"),
        },
        _ => None,
    }
}

/// The inheritance closure of the model's reified relationships, memoized:
/// what a type's features are once everything its specializations reach is
/// counted in, redefined features excepted.
///
/// Only relationships name resolution reified take part, so an unresolved
/// model derives empty closures -- which is what it knows.
struct Closures<'a> {
    model: &'a Model,
    features: std::cell::RefCell<HashMap<ElementId, std::rc::Rc<Vec<ElementId>>>>,
}

impl<'a> Closures<'a> {
    fn new(model: &'a Model) -> Closures<'a> {
        Closures {
            model,
            features: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// The features an element owns directly.
    fn owned_features(&self, id: ElementId) -> Vec<ElementId> {
        self.model
            .owned(id)
            .iter()
            .copied()
            .filter(|&child| self.model.kind(child).is_a(ElementKind::Feature))
            .filter(|&child| !self.model.kind(child).is_a(ElementKind::Relationship))
            .collect()
    }

    /// What an element specializes: the resolved targets of the
    /// specialization relationships it owns, its types included -- a
    /// feature inherits through its typing the way a subclass does
    /// through its subclassification.
    fn supertypes(&self, id: ElementId) -> Vec<ElementId> {
        self.model
            .owned(id)
            .iter()
            .filter_map(|&child| {
                let target = match self.model.kind(child) {
                    ElementKind::Subclassification => "superclassifier",
                    ElementKind::Subsetting => "subsettedFeature",
                    ElementKind::Redefinition => "redefinedFeature",
                    ElementKind::FeatureTyping => "type",
                    ElementKind::ReferenceSubsetting => "referencedFeature",
                    _ => return None,
                };
                match self.model.get(child, target) {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                }
            })
            .collect()
    }

    /// Everything `id` makes a feature of itself, owned or inherited.
    fn features(&self, id: ElementId) -> std::rc::Rc<Vec<ElementId>> {
        if let Some(known) = self.features.borrow().get(&id) {
            return known.clone();
        }
        // a cycle (`part p :> p;` is legal) ends at what is already known:
        // publishing the owned features first keeps the walk finite
        let owned = self.owned_features(id);
        self.features
            .borrow_mut()
            .insert(id, std::rc::Rc::new(owned.clone()));

        let mut out = owned.clone();
        let mut seen: std::collections::HashSet<ElementId> = out.iter().copied().collect();
        // an inherited feature a nearer one redefines is not inherited
        let redefined: std::collections::HashSet<ElementId> = owned
            .iter()
            .flat_map(|&feature| self.model.owned(feature))
            .filter(|&&child| self.model.kind(child) == ElementKind::Redefinition)
            .filter_map(|&child| match self.model.get(child, "redefinedFeature") {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            })
            .collect();
        for supertype in self.supertypes(id) {
            for &inherited in self.features(supertype).iter() {
                if !redefined.contains(&inherited) && seen.insert(inherited) {
                    out.push(inherited);
                }
            }
        }
        let out = std::rc::Rc::new(out);
        self.features.borrow_mut().insert(id, out.clone());
        out
    }

    /// The features of `id` whose stored direction is one of `wanted`.
    fn directed(&self, id: ElementId, wanted: &[&str]) -> Vec<ElementId> {
        self.features(id)
            .iter()
            .copied()
            .filter(|&feature| {
                self.model
                    .get(feature, "direction")
                    .and_then(Value::as_str)
                    .is_some_and(|direction| wanted.contains(&direction))
            })
            .collect()
    }
}

/// The UUID of the `OwningMembership` synthesized between an element and
/// its owner: the element's own path with a marker, so it is as stable as
/// the element it brings in.
fn membership_uuid(model: &Model, owned: ElementId) -> Uuid {
    let of = element_uuid(model, owned);
    Uuid::new_v5(
        &Uuid::NAMESPACE_URL,
        format!("sysml-v2-rs:{of}#owningMembership").as_bytes(),
    )
}

/// Does ownership of this element pass through a synthesized membership?
///
/// A relationship needs none at either end: the standard has an element
/// own its relationships directly, and a pure relationship own its
/// elements the same way -- a `FeatureValue` holds the expression it sets
/// without a membership between them. A relationship that is also a
/// namespace, though -- a connection definition, an association -- owns
/// its members the way any namespace does, membership and all.
fn bridged(model: &Model, owned: ElementId) -> bool {
    if model.kind(owned).is_a(ElementKind::Relationship) {
        return false;
    }
    !model.owner(owned).is_some_and(|owner| {
        let kind = model.kind(owner);
        kind.is_a(ElementKind::Relationship) && !kind.is_a(ElementKind::Namespace)
    })
}

/// The membership a bridged element is owned through.
///
/// The standard's abstract syntax picks a metaclass by what the member is
/// to its owner: a declared role names it outright, a connector end gets an
/// `EndFeatureMembership`, a directed feature of a behavior is a parameter,
/// a trigger/guard/effect of a transition is a transition feature, any
/// other feature of a type sits behind a `FeatureMembership`, and anything
/// else behind a plain `OwningMembership`.
fn membership_kind(model: &Model, owned: ElementId) -> ElementKind {
    if let Some(role) = model.member_role(owned) {
        return match role {
            "subject" => ElementKind::SubjectMembership,
            "actor" => ElementKind::ActorMembership,
            "stakeholder" => ElementKind::StakeholderMembership,
            "objective" => ElementKind::ObjectiveMembership,
            "variant" => ElementKind::VariantMembership,
            "return" => ElementKind::ReturnParameterMembership,
            "entry" | "do" | "exit" => ElementKind::StateSubactionMembership,
            "assume" | "require" => ElementKind::RequirementConstraintMembership,
            "frame" => ElementKind::FramedConcernMembership,
            _ => ElementKind::OwningMembership,
        };
    }
    let owner_kind = match model.owner(owned) {
        Some(owner) => model.kind(owner),
        None => return ElementKind::OwningMembership,
    };
    if !model.kind(owned).is_a(ElementKind::Feature) || !owner_kind.is_a(ElementKind::Type) {
        return ElementKind::OwningMembership;
    }
    if model.get(owned, "isEnd") == Some(&Value::Bool(true)) {
        return ElementKind::EndFeatureMembership;
    }
    if transition_role(model, owned).is_some() {
        return ElementKind::TransitionFeatureMembership;
    }
    let behavioral = [
        ElementKind::Behavior,
        ElementKind::Step,
        ElementKind::Function,
        ElementKind::Expression,
    ];
    if model.get(owned, "direction").is_some()
        && behavioral.iter().any(|&kind| owner_kind.is_a(kind))
    {
        return ElementKind::ParameterMembership;
    }
    ElementKind::FeatureMembership
}

/// What a transition feature is to its transition -- the `kind` its
/// membership must state -- read off the references the transition stores.
fn transition_role(model: &Model, owned: ElementId) -> Option<&'static str> {
    let transition = model.owner(owned)?;
    if model.kind(transition) != ElementKind::TransitionUsage {
        return None;
    }
    let holds = |name: &str| match model.get(transition, name) {
        Some(Value::Ref(target)) => *target == owned,
        Some(Value::RefList(targets)) => targets.contains(&owned),
        _ => false,
    };
    if holds("triggerAction") {
        Some("trigger")
    } else if holds("guardExpression") {
        Some("guard")
    } else if holds("effectAction") {
        Some("effect")
    } else {
        None
    }
}

/// Every structural feature a metaclass carries, its own and the ones it
/// inherits, first declaration of a name winning.
fn all_features(kind: ElementKind) -> Vec<&'static sysml_model::FeatureMeta> {
    let mut seen = std::collections::HashSet::new();
    std::iter::once(kind)
        .chain(kind.ancestors().iter().copied())
        .flat_map(|k| k.own_features())
        .filter(|meta| seen.insert(meta.name))
        .collect()
}

/// The name an element is known by: its declared name or, for a feature
/// declared without one, the name it takes from what it redefines or
/// references -- KerML's effective-name rule, so `attribute :>> mass;`
/// is a feature named `mass`.
fn effective_name(model: &Model, id: ElementId) -> Option<String> {
    named_after(
        model,
        id,
        "declaredName",
        &mut std::collections::HashSet::new(),
    )
}

/// [`effective_name`], for the short name.
fn effective_short_name(model: &Model, id: ElementId) -> Option<String> {
    named_after(
        model,
        id,
        "declaredShortName",
        &mut std::collections::HashSet::new(),
    )
}

/// The declared property, or the naming feature's, redefinitions before
/// reference subsettings. The guard keeps a redefinition cycle -- illegal,
/// but representable -- finite.
fn named_after(
    model: &Model,
    id: ElementId,
    declared: &str,
    visiting: &mut std::collections::HashSet<ElementId>,
) -> Option<String> {
    if let Some(name) = model.get(id, declared).and_then(Value::as_str) {
        return Some(name.to_string());
    }
    if !visiting.insert(id) {
        return None;
    }
    let naming = model.owned(id).iter().find_map(|&child| {
        let target = match model.kind(child) {
            ElementKind::Redefinition => "redefinedFeature",
            ElementKind::ReferenceSubsetting => "referencedFeature",
            _ => return None,
        };
        match model.get(child, target) {
            Some(Value::Ref(target)) => Some(*target),
            _ => None,
        }
    })?;
    named_after(model, naming, declared, visiting)
}

/// A name the way `qualifiedName` writes it: as it is when it is a basic
/// name, quoted when it needs to be.
fn quoted(name: &str) -> String {
    let basic = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if basic {
        name.to_string()
    } else {
        format!("'{name}'")
    }
}

/// The dotted path of names from the root, or nothing as soon as one
/// element on the way has no name to write.
fn qualified_name(model: &Model, id: ElementId) -> Option<String> {
    let mut segments = Vec::new();
    let mut current = Some(id);
    while let Some(elem) = current {
        let owner = model.owner(elem);
        match effective_name(model, elem) {
            Some(name) => segments.push(quoted(&name)),
            // the root namespace has no name and contributes no segment;
            // anything unnamed below it interrupts the path
            None if owner.is_none() => {}
            None => return None,
        }
        current = owner;
    }
    if segments.is_empty() {
        return None;
    }
    segments.reverse();
    Some(segments.join("::"))
}

/// What a resolver knows and a serializer alone cannot: the standard's
/// derived properties that reach through imports, and which elements live
/// in a library model. `sysml-semantics` computes all of it; the CLI wires
/// the two together.
#[derive(Clone, Debug, Default)]
pub struct Extras {
    /// Namespace -> the members its imports bring in, in import order.
    pub imported: HashMap<ElementId, Vec<ElementId>>,
    /// Import element -> the member or namespace it resolved to.
    pub import_targets: HashMap<ElementId, ElementId>,
    /// Elements that belong to a library model (`isLibraryElement`).
    pub library: std::collections::HashSet<ElementId>,
}

/// Serialize the whole model as an array of element objects (stable order):
/// every element in arena order, then the memberships synthesized between
/// each owner and the owned elements that need one. Each object carries
/// the complete property set of its metaclass.
///
/// Derived properties that need the resolver -- imported memberships,
/// `isLibraryElement` -- stay at their defaults here; [`to_json_with`]
/// takes them as [`Extras`].
pub fn to_json(model: &Model) -> Json {
    to_json_with(model, &Extras::default())
}

/// [`to_json`], with the resolver-derived facts filled in.
pub fn to_json_with(model: &Model, extras: &Extras) -> Json {
    let uuids: HashMap<ElementId, Uuid> = model
        .ids()
        .map(|id| (id, element_uuid(model, id)))
        .collect();
    let reference = |id: &ElementId| json!({ "@id": uuids[id].to_string() });
    let membership = |id: ElementId| json!({ "@id": membership_uuid(model, id).to_string() });
    let references = |ids: &[ElementId]| Json::Array(ids.iter().map(reference).collect());
    let annotations = |id: ElementId, kind: ElementKind| {
        Json::Array(
            model
                .owned(id)
                .iter()
                .filter(|&&child| model.kind(child).is_a(kind))
                .map(reference)
                .collect(),
        )
    };

    let closures = Closures::new(model);
    // a membership reference for each element a namespace reaches: its
    // bridge when ownership is bridged, nothing when it is not
    let membership_of =
        |id: ElementId| -> Option<Json> { bridged(model, id).then(|| membership(id)) };
    let memberships = |ids: &[ElementId]| -> Json {
        Json::Array(ids.iter().copied().filter_map(membership_of).collect())
    };
    let owned_of_kind = |id: ElementId, wanted: ElementKind| -> Json {
        Json::Array(
            model
                .owned(id)
                .iter()
                .filter(|&&child| model.kind(child).is_a(wanted))
                .map(reference)
                .collect(),
        )
    };

    // what a name-driven derived property holds, when the model can say
    let derived = |id: ElementId, name: &str| -> Option<Json> {
        let kind = model.kind(id);
        let is_relationship = kind.is_a(ElementKind::Relationship);
        let is_type = kind.is_a(ElementKind::Type);
        match name {
            // the inheritance closure, from the reified specializations
            "feature" if is_type => Some(references(&closures.features(id))),
            "ownedFeature" if is_type => Some(references(&closures.owned_features(id))),
            "inheritedFeature" if is_type => {
                let owned: std::collections::HashSet<ElementId> =
                    closures.owned_features(id).into_iter().collect();
                Some(references(
                    &closures
                        .features(id)
                        .iter()
                        .copied()
                        .filter(|feature| !owned.contains(feature))
                        .collect::<Vec<_>>(),
                ))
            }
            "inheritedMembership" if is_type => {
                let owned: std::collections::HashSet<ElementId> =
                    closures.owned_features(id).into_iter().collect();
                let inherited: Vec<ElementId> = closures
                    .features(id)
                    .iter()
                    .copied()
                    .filter(|feature| !owned.contains(feature))
                    .collect();
                Some(memberships(&inherited))
            }
            "endFeature" if is_type => Some(references(
                &closures
                    .features(id)
                    .iter()
                    .copied()
                    .filter(|&feature| model.get(feature, "isEnd") == Some(&Value::Bool(true)))
                    .collect::<Vec<_>>(),
            )),
            "ownedEndFeature" if is_type => Some(references(
                &closures
                    .owned_features(id)
                    .into_iter()
                    .filter(|&feature| model.get(feature, "isEnd") == Some(&Value::Bool(true)))
                    .collect::<Vec<_>>(),
            )),
            "input" if is_type => Some(references(&closures.directed(id, &["in", "inout"]))),
            "output" if is_type => Some(references(&closures.directed(id, &["out", "inout"]))),
            "directedFeature" if is_type => {
                Some(references(&closures.directed(id, &["in", "out", "inout"])))
            }
            "parameter" if is_type => {
                Some(references(&closures.directed(id, &["in", "out", "inout"])))
            }
            // what a namespace holds, membership by membership
            "ownedMembership" => Some(Json::Array(
                model
                    .owned(id)
                    .iter()
                    .filter_map(|&child| {
                        if model.kind(child).is_a(ElementKind::Membership) {
                            Some(reference(&child))
                        } else {
                            membership_of(child)
                        }
                    })
                    .collect(),
            )),
            "membership" => {
                let mut all: Vec<Json> = model
                    .owned(id)
                    .iter()
                    .filter_map(|&child| {
                        if model.kind(child).is_a(ElementKind::Membership) {
                            Some(reference(&child))
                        } else {
                            membership_of(child)
                        }
                    })
                    .collect();
                for &imported in extras.imported.get(&id).into_iter().flatten() {
                    all.extend(membership_of(imported));
                }
                if is_type {
                    let owned: std::collections::HashSet<ElementId> =
                        closures.owned_features(id).into_iter().collect();
                    for &feature in closures.features(id).iter() {
                        if !owned.contains(&feature) {
                            all.extend(membership_of(feature));
                        }
                    }
                }
                Some(Json::Array(all))
            }
            "ownedMember" => Some(references(
                &model
                    .owned(id)
                    .iter()
                    .filter(|&&child| bridged(model, child))
                    .copied()
                    .collect::<Vec<_>>(),
            )),
            "member" => {
                let mut members: Vec<ElementId> = model
                    .owned(id)
                    .iter()
                    .copied()
                    .filter(|&child| bridged(model, child))
                    .collect();
                let mut seen: std::collections::HashSet<ElementId> =
                    members.iter().copied().collect();
                for &imported in extras.imported.get(&id).into_iter().flatten() {
                    if seen.insert(imported) {
                        members.push(imported);
                    }
                }
                if is_type {
                    members.extend(
                        closures
                            .features(id)
                            .iter()
                            .copied()
                            .filter(|&feature| seen.insert(feature)),
                    );
                }
                Some(references(&members))
            }
            "importedMembership" if kind == ElementKind::MembershipImport => Some(
                extras
                    .import_targets
                    .get(&id)
                    .and_then(|&target| membership_of(target))
                    .unwrap_or(Json::Null),
            ),
            "importedMembership" => Some(memberships(
                extras.imported.get(&id).map_or(&[][..], Vec::as_slice),
            )),
            "importedNamespace" if kind == ElementKind::NamespaceImport => {
                Some(extras.import_targets.get(&id).map_or(Json::Null, reference))
            }
            "isLibraryElement" => Some(extras.library.contains(&id).into()),
            // the specializations an element owns, by their metaclass
            "ownedSpecialization" if is_type => {
                Some(owned_of_kind(id, ElementKind::Specialization))
            }
            "ownedSubclassification" => Some(owned_of_kind(id, ElementKind::Subclassification)),
            "ownedTyping" => Some(owned_of_kind(id, ElementKind::FeatureTyping)),
            "ownedSubsetting" => Some(owned_of_kind(id, ElementKind::Subsetting)),
            "ownedRedefinition" => Some(owned_of_kind(id, ElementKind::Redefinition)),
            "ownedReferenceSubsetting" => model
                .owned(id)
                .iter()
                .find(|&&child| model.kind(child) == ElementKind::ReferenceSubsetting)
                .map(reference)
                .or(Some(Json::Null)),
            "ownedImport" => Some(owned_of_kind(id, ElementKind::Import)),
            // a feature is typed by what its reified typings resolved to
            "type" if kind.is_a(ElementKind::Feature) => Some(Json::Array(
                model
                    .owned(id)
                    .iter()
                    .filter(|&&child| model.kind(child) == ElementKind::FeatureTyping)
                    .filter_map(|&child| match model.get(child, "type") {
                        Some(Value::Ref(target)) => Some(reference(target)),
                        _ => None,
                    })
                    .collect(),
            )),
            "owningType" if kind.is_a(ElementKind::Feature) => Some(
                model
                    .owner(id)
                    .filter(|&owner| model.kind(owner).is_a(ElementKind::Type))
                    .map_or(Json::Null, |owner| reference(&owner)),
            ),
            // an owned feature is featured by -- and its membership is --
            // its owning type's
            "featuringType" if kind.is_a(ElementKind::Feature) => Some(Json::Array(
                model
                    .owner(id)
                    .filter(|&owner| model.kind(owner).is_a(ElementKind::Type))
                    .iter()
                    .map(reference)
                    .collect(),
            )),
            "owningFeatureMembership" if kind.is_a(ElementKind::Feature) => {
                let of_a_type = bridged(model, id)
                    && model.owner(id).is_some()
                    && membership_kind(model, id).is_a(ElementKind::FeatureMembership);
                Some(if of_a_type {
                    membership(id)
                } else {
                    Json::Null
                })
            }
            // an annotating element is about the element it sits on
            "annotatedElement" if kind.is_a(ElementKind::AnnotatingElement) => {
                Some(match model.get(id, "representedElement") {
                    Some(Value::Ref(target)) => Json::Array(vec![reference(target)]),
                    _ => Json::Array(model.owner(id).iter().map(reference).collect()),
                })
            }
            "nestedUsage" if kind.is_a(ElementKind::Usage) => {
                Some(owned_of_kind(id, ElementKind::Usage))
            }
            "ownedUsage" if kind.is_a(ElementKind::Definition) => {
                Some(owned_of_kind(id, ElementKind::Usage))
            }
            "elementId" => Some(uuids[&id].to_string().into()),
            "name" => Some(effective_name(model, id).map_or(Json::Null, Json::from)),
            "shortName" => Some(effective_short_name(model, id).map_or(Json::Null, Json::from)),
            "qualifiedName" => Some(qualified_name(model, id).map_or(Json::Null, Json::from)),
            "owner" => Some(
                model
                    .owner(id)
                    .map_or(Json::Null, |owner| reference(&owner)),
            ),
            "ownedElement" => Some(references(model.owned(id))),
            // the reified shape: a membership bridges the way down to an
            // ordinary element, a relationship is owned as itself, and
            // what a relationship owns appears only as related elements
            "ownedRelationship" => Some(Json::Array(
                model
                    .owned(id)
                    .iter()
                    .filter_map(|&child| {
                        if model.kind(child).is_a(ElementKind::Relationship) {
                            Some(reference(&child))
                        } else if bridged(model, child) {
                            Some(membership(child))
                        } else {
                            None
                        }
                    })
                    .collect(),
            )),
            "owningRelationship" => {
                let owner = model.owner(id)?;
                Some(if bridged(model, id) {
                    membership(id)
                } else if model.kind(owner).is_a(ElementKind::Relationship) {
                    reference(&owner)
                } else {
                    Json::Null
                })
            }
            "owningMembership" => Some(if bridged(model, id) && model.owner(id).is_some() {
                membership(id)
            } else {
                Json::Null
            }),
            "owningNamespace" => Some(if bridged(model, id) {
                model
                    .owner(id)
                    .map_or(Json::Null, |owner| reference(&owner))
            } else {
                Json::Null
            }),
            "documentation" => Some(annotations(id, ElementKind::Documentation)),
            "textualRepresentation" => Some(annotations(id, ElementKind::TextualRepresentation)),
            // a relationship that is also a namespace holds its members
            // through memberships; only the rest is directly related
            "ownedRelatedElement" if is_relationship => Some(references(
                &model
                    .owned(id)
                    .iter()
                    .copied()
                    .filter(|&child| !bridged(model, child))
                    .collect::<Vec<_>>(),
            )),
            "owningRelatedElement" if is_relationship => Some(
                model
                    .owner(id)
                    .map_or(Json::Null, |owner| reference(&owner)),
            ),
            "relatedElement" if is_relationship => {
                let mut related: Vec<ElementId> = model.owner(id).into_iter().collect();
                related.extend(
                    model
                        .owned(id)
                        .iter()
                        .copied()
                        .filter(|&child| !bridged(model, child)),
                );
                Some(references(&related))
            }
            // a membership's member: for the owning kind, what it owns
            "memberElement" | "ownedMemberElement" if kind.is_a(ElementKind::OwningMembership) => {
                Some(model.owned(id).first().map_or(Json::Null, &reference))
            }
            "memberName" | "ownedMemberName" if kind.is_a(ElementKind::OwningMembership) => Some(
                model
                    .owned(id)
                    .first()
                    .and_then(|&member| model.name(member))
                    .map_or(Json::Null, Json::from),
            ),
            "memberElementId" | "ownedMemberElementId"
                if kind.is_a(ElementKind::OwningMembership) =>
            {
                Some(
                    model
                        .owned(id)
                        .first()
                        .map_or(Json::Null, |member| uuids[member].to_string().into()),
                )
            }
            _ => None,
        }
    };

    let mut elements: Vec<Json> = model
        .ids()
        .map(|id| {
            let stored: HashMap<&str, &Value> = model.props(id).collect();
            let mut object = Map::new();
            object.insert("@type".into(), model.kind(id).name().into());
            object.insert("@id".into(), uuids[&id].to_string().into());
            for meta in all_features(model.kind(id)) {
                let value = match stored.get(meta.name) {
                    Some(value) => match value {
                        Value::Bool(b) => Json::from(*b),
                        Value::Int(i) => Json::from(*i),
                        Value::Real(r) => Json::from(*r),
                        Value::String(text) => Json::from(text.clone()),
                        Value::EnumLit(lit) => Json::from(*lit),
                        Value::Ref(r) => reference(r),
                        Value::RefList(rs) => references(rs),
                    },
                    None => derived(id, meta.name).unwrap_or_else(|| default_for(meta)),
                };
                object.insert(meta.name.into(), value);
            }
            Json::Object(object)
        })
        .collect();

    // the memberships themselves, in the order of what they bring in,
    // carrying their whole property set like any other element
    for id in model.ids() {
        let Some(owner) = model.owner(id) else {
            continue;
        };
        if !bridged(model, id) {
            continue;
        }
        let kind = membership_kind(model, id);
        let uuid = membership_uuid(model, id).to_string();
        let mut object = Map::new();
        object.insert("@type".into(), kind.name().into());
        object.insert("@id".into(), uuid.clone().into());
        for meta in all_features(kind) {
            let value = match meta.name {
                "elementId" => uuid.clone().into(),
                "owner" | "owningRelatedElement" | "membershipOwningNamespace" => reference(&owner),
                "ownedElement" | "ownedRelatedElement" => Json::Array(vec![reference(&id)]),
                "relatedElement" => Json::Array(vec![reference(&owner), reference(&id)]),
                "source" => Json::Array(vec![reference(&owner)]),
                "target" => Json::Array(vec![reference(&id)]),
                // each subtype names the member again in its own terms
                "memberElement"
                | "ownedMemberElement"
                | "ownedMemberFeature"
                | "ownedMemberParameter"
                | "ownedSubjectParameter"
                | "ownedActorParameter"
                | "ownedStakeholderParameter"
                | "ownedObjectiveRequirement"
                | "ownedVariantUsage"
                | "transitionFeature"
                | "ownedEndFeature"
                | "ownedConstraint"
                | "ownedConcern" => reference(&id),
                "memberElementId" | "ownedMemberElementId" => uuids[&id].to_string().into(),
                "memberName" | "ownedMemberName" => {
                    effective_name(model, id).map_or(Json::Null, Json::from)
                }
                "memberShortName" | "ownedMemberShortName" => {
                    effective_short_name(model, id).map_or(Json::Null, Json::from)
                }
                "owningType" => reference(&owner),
                "visibility" => model.member_visibility(id).unwrap_or("public").into(),
                // a transition feature's membership says which it is,
                // and so do a state's subactions and a requirement's
                // constraints, in their own vocabularies
                "kind" if kind == ElementKind::TransitionFeatureMembership => {
                    transition_role(model, id).map_or(Json::Null, Json::from)
                }
                "kind" if kind == ElementKind::StateSubactionMembership => {
                    model.member_role(id).map_or(Json::Null, Json::from)
                }
                "kind" if kind.is_a(ElementKind::RequirementConstraintMembership) => {
                    // an assumption says so; a required constraint and a
                    // framed concern are both requirements
                    if model.member_role(id) == Some("assume") {
                        "assumption".into()
                    } else {
                        "requirement".into()
                    }
                }
                _ => default_for(meta),
            };
            object.insert(meta.name.into(), value);
        }
        elements.push(Json::Object(object));
    }
    Json::Array(elements)
}

/// What a property nothing sets or derives reads as: absent, empty or
/// plainly false -- except a visibility, which the metamodel defaults to
/// `public`.
fn default_for(meta: &sysml_model::FeatureMeta) -> Json {
    if meta.name == "visibility" {
        return "public".into();
    }
    if meta.many {
        return Json::Array(Vec::new());
    }
    match meta.ty {
        FeatureType::Data(PrimitiveType::Boolean) => Json::from(false),
        _ => Json::Null,
    }
}

/// Rebuild a model from interchange JSON. Returns the model and the root
/// elements (those without an owner). Unknown properties are ignored;
/// unknown metaclasses and dangling references are errors.
pub fn from_json(json: &Json) -> Result<(Model, Vec<ElementId>), ImportError> {
    let array = json.as_array().ok_or(ImportError::NotAnArray)?;
    let mut model = Model::new();
    let mut by_uuid: HashMap<&str, ElementId> = HashMap::new();
    // a plain `OwningMembership` only carries ownership, which the model
    // holds directly: it becomes an edge rather than an element, whichever
    // tool wrote it
    let mut bridges: HashMap<&str, &Json> = HashMap::new();

    // pass 1: create all elements
    let mut ids = Vec::new();
    let mut created = Vec::new();
    for (index, object) in array.iter().enumerate() {
        let type_name = object["@type"]
            .as_str()
            .ok_or(ImportError::MissingType(index))?;
        let kind = ElementKind::from_name(type_name)
            .ok_or_else(|| ImportError::UnknownType(type_name.to_string()))?;
        let uuid = object["@id"]
            .as_str()
            .ok_or(ImportError::MissingId(index))?;
        if FOLDED.contains(&kind) && object.get("ownedRelatedElement").is_some() {
            bridges.insert(uuid, object);
            continue;
        }
        let id = model.create(kind);
        by_uuid.insert(uuid, id);
        ids.push(id);
        created.push(object);
    }

    let resolve = |value: &Json| -> Result<ElementId, ImportError> {
        let uuid = value["@id"].as_str().unwrap_or_default();
        by_uuid
            .get(uuid)
            .copied()
            .ok_or_else(|| ImportError::UnknownReference(uuid.to_string()))
    };
    // what a membership reference brings in: the elements on its far
    // side, each keeping what the membership said about it
    let through = |model: &mut Model, value: &Json| -> Result<Vec<ElementId>, ImportError> {
        let uuid = value["@id"].as_str().unwrap_or_default();
        let Some(bridge) = bridges.get(uuid) else {
            return Ok(vec![resolve(value)?]);
        };
        let members: Vec<ElementId> = bridge["ownedRelatedElement"]
            .as_array()
            .into_iter()
            .flatten()
            .map(resolve)
            .collect::<Result<_, _>>()?;
        for &member in &members {
            match bridge["visibility"].as_str() {
                Some("private") => model.set_member_visibility(member, "private"),
                Some("protected") => model.set_member_visibility(member, "protected"),
                _ => {}
            }
            if let Some(role) = folded_role(bridge) {
                model.set_member_role(member, role);
            }
        }
        Ok(members)
    };

    // pass 2: properties and ownership. Ownership may be written as the
    // derived `ownedElement`, as memberships, or as a relationship's own
    // related elements -- often all at once, so each pair counts once, in
    // the order it is first given.
    let mut owned = std::collections::HashSet::new();
    for (object, id) in created.iter().zip(&ids) {
        let kind = model.kind(*id);
        let object = object.as_object().expect("validated in pass 1");
        let mut own = |model: &mut Model, child: ElementId| {
            if owned.insert((*id, child)) {
                model.add_owned(*id, child);
            }
        };
        for (key, value) in object {
            match key.as_str() {
                // identity, and the inverse half of the ownership web:
                // derived without exception, and the membership ones may
                // point at a bridge that became an edge, not an element
                "@type"
                | "@id"
                | "elementId"
                | "owner"
                | "owningRelationship"
                | "owningMembership"
                | "owningNamespace"
                | "owningRelatedElement" => continue,
                "ownedElement" | "ownedRelatedElement" => {
                    for child in value.as_array().into_iter().flatten() {
                        let child = resolve(child)?;
                        own(&mut model, child);
                    }
                }
                "ownedRelationship" => {
                    for related in value.as_array().into_iter().flatten() {
                        for child in through(&mut model, related)? {
                            own(&mut model, child);
                        }
                    }
                }
                _ => {
                    let Some(meta) = kind.feature(key) else {
                        continue; // tolerate foreign properties
                    };
                    // what the exporter derives is not read back -- except
                    // where this metaclass declares the property as its
                    // own stored fact, the way `FeatureTyping` holds the
                    // `type` a feature's typing resolved to
                    if meta.derived && SYNTHESIZED.contains(&key.as_str()) {
                        continue;
                    }
                    let converted = match convert_value(meta.ty, value, &resolve) {
                        // an import may name the membership an element was
                        // reached through, which folded into an edge here
                        Err(ImportError::UnknownReference(_))
                            if matches!(
                                key.as_str(),
                                "importedMembership" | "importedNamespace"
                            ) =>
                        {
                            None
                        }
                        other => other?,
                    };
                    if let Some(converted) = converted {
                        model.set(*id, key, converted);
                    }
                }
            }
        }
    }

    let roots = ids
        .iter()
        .copied()
        .filter(|id| model.owner(*id).is_none())
        .collect();
    Ok((model, roots))
}

fn convert_value(
    ty: FeatureType,
    value: &Json,
    resolve: &dyn Fn(&Json) -> Result<ElementId, ImportError>,
) -> Result<Option<Value>, ImportError> {
    let converted = match (ty, value) {
        (_, Json::Null) => None,
        (FeatureType::Data(PrimitiveType::Boolean), Json::Bool(b)) => Some(Value::Bool(*b)),
        (FeatureType::Data(PrimitiveType::Real), Json::Number(n)) => n.as_f64().map(Value::Real),
        (FeatureType::Data(_), Json::Number(n)) => n
            .as_i64()
            .map(Value::Int)
            .or_else(|| n.as_f64().map(Value::Real)),
        (FeatureType::Data(_), Json::String(s)) => Some(Value::String(s.clone())),
        (FeatureType::Enumeration(_), Json::String(s)) => Some(Value::String(s.clone())),
        (FeatureType::Class(_), Json::Object(_)) => Some(Value::Ref(resolve(value)?)),
        (FeatureType::Class(_), Json::Array(items)) => {
            let refs: Result<Vec<_>, _> = items.iter().map(resolve).collect();
            Some(Value::RefList(refs?))
        }
        _ => None,
    };
    Ok(converted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_model::build_model;

    fn example_model() -> Model {
        let parse = sysml_syntax::parse(
            "package P {\n  doc /* docs */\n  abstract part def Vehicle {\n    attribute mass : Real = 10.0;\n  }\n  part car : Vehicle;\n}",
        );
        assert!(parse.ok());
        build_model(&parse).0
    }

    #[test]
    fn export_is_deterministic_and_typed() {
        let model = example_model();
        let a = to_json(&model);
        let b = to_json(&model);
        assert_eq!(a, b);
        let first = &a.as_array().unwrap()[0];
        assert_eq!(first["@type"], "Package");
        assert_eq!(first["declaredName"], "P");
        assert!(first["@id"].as_str().unwrap().len() == 36);
    }

    #[test]
    fn round_trip_preserves_structure() {
        let model = example_model();
        let json = to_json(&model);
        let (rebuilt, roots) = from_json(&json).unwrap();

        assert_eq!(rebuilt.len(), model.len());
        assert_eq!(roots.len(), 1);
        let pkg = roots[0];
        assert_eq!(rebuilt.kind(pkg), ElementKind::Package);
        assert_eq!(rebuilt.name(pkg), Some("P"));
        let members = rebuilt.owned(pkg);
        assert_eq!(rebuilt.kind(members[0]), ElementKind::Documentation);
        let vehicle = members[1];
        assert_eq!(rebuilt.kind(vehicle), ElementKind::PartDefinition);
        assert_eq!(rebuilt.get(vehicle, "isAbstract"), Some(&Value::Bool(true)));
        assert_eq!(rebuilt.name(rebuilt.owned(vehicle)[0]), Some("mass"));

        // and the re-export matches the first export exactly
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn import_errors_display() {
        for (error, needle) in [
            (ImportError::NotAnArray, "array"),
            (ImportError::MissingType(3), "no \"@type\""),
            (ImportError::UnknownType("X".into()), "unknown metaclass"),
            (ImportError::MissingId(1), "no \"@id\""),
            (ImportError::UnknownReference("u".into()), "unknown element"),
        ] {
            assert!(error.to_string().contains(needle), "{error}");
            assert!(!format!("{error:?}").is_empty());
        }
    }

    #[test]
    fn value_variants_and_unnamed_roots_round_trip() {
        use sysml_model::ElementKind;
        let mut model = Model::new();
        // unnamed root element: uuid falls back to the arena index
        let root = model.create(ElementKind::Package);
        let a = model.create(ElementKind::LiteralInteger);
        let b = model.create(ElementKind::LiteralRational);
        let m = model.create(ElementKind::MembershipImport);
        model.add_owned(root, a);
        model.add_owned(root, b);
        model.add_owned(root, m);
        model.set(a, "value", Value::Int(42));
        model.set(b, "value", Value::Real(2.5));
        model.set(m, "visibility", Value::EnumLit("private"));
        model.set(m, "isImportAll", Value::Bool(true));
        model.set(m, "importedMembership", Value::Ref(a));
        model.set(root, "filterCondition", Value::RefList(vec![a, b]));

        let json = to_json(&model);
        let (rebuilt, roots) = from_json(&json).unwrap();
        assert_eq!(roots.len(), 1);
        let ra = rebuilt.owned(roots[0])[0];
        let rb = rebuilt.owned(roots[0])[1];
        let rm = rebuilt.owned(roots[0])[2];
        assert_eq!(rebuilt.get(ra, "value"), Some(&Value::Int(42)));
        assert_eq!(rebuilt.get(rb, "value"), Some(&Value::Real(2.5)));
        // enum literals come back as strings
        assert_eq!(
            rebuilt.get(rm, "visibility").and_then(Value::as_str),
            Some("private")
        );
        assert_eq!(rebuilt.get(rm, "isImportAll"), Some(&Value::Bool(true)));
        assert_eq!(rebuilt.get(rm, "importedMembership"), Some(&Value::Ref(ra)));
        assert_eq!(
            rebuilt.get(roots[0], "filterCondition"),
            Some(&Value::RefList(vec![ra, rb]))
        );
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn ownership_is_reified_the_standard_way() {
        let model = example_model();
        let json = to_json(&model);
        let objects = json.as_array().unwrap();

        // the package reaches its documentation through a membership...
        let package = &objects[0];
        let bridge_id = package["ownedRelationship"][0]["@id"].as_str().unwrap();
        let bridge = objects
            .iter()
            .find(|object| object["@id"] == bridge_id)
            .expect("the membership is in the array");
        assert_eq!(bridge["@type"], "OwningMembership");
        assert_eq!(bridge["owningRelatedElement"]["@id"], package["@id"]);
        assert_eq!(
            bridge["ownedRelatedElement"][0]["@id"],
            package["ownedElement"][0]["@id"]
        );

        // ...while a relationship owns its expression directly, with no
        // membership between them
        let value = objects
            .iter()
            .find(|object| object["@type"] == "FeatureValue")
            .expect("the value was reified");
        assert_eq!(value["ownedRelationship"], serde_json::json!([]));
        let literal_id = value["ownedRelatedElement"][0]["@id"].as_str().unwrap();
        let literal = objects
            .iter()
            .find(|object| object["@id"] == literal_id)
            .unwrap();
        assert_eq!(literal["@type"], "LiteralRational");
        assert_eq!(literal["owningRelationship"]["@id"], value["@id"]);
    }

    #[test]
    fn a_membership_only_file_reads_back_as_ownership() {
        // the shape another tool writes: no derived `ownedElement`,
        // ownership only through the reified memberships
        let json = serde_json::json!([
            { "@type": "Package", "@id": "p", "declaredName": "P",
              "ownedRelationship": [{ "@id": "m1" }] },
            { "@type": "OwningMembership", "@id": "m1",
              "owningRelatedElement": { "@id": "p" },
              "ownedRelatedElement": [{ "@id": "v" }] },
            { "@type": "PartDefinition", "@id": "v", "declaredName": "Vehicle",
              "owningRelationship": { "@id": "m1" } },
        ]);
        let (model, roots) = from_json(&json).unwrap();
        assert_eq!(model.len(), 2, "the membership became an edge");
        assert_eq!(roots.len(), 1);
        let package = roots[0];
        assert_eq!(model.name(package), Some("P"));
        let vehicle = model.owned(package)[0];
        assert_eq!(model.kind(vehicle), ElementKind::PartDefinition);
        assert_eq!(model.owner(vehicle), Some(package));

        // and writing it back out synthesizes an equivalent membership
        let out = to_json(&model);
        assert_eq!(out.as_array().unwrap().len(), 3);
    }

    #[test]
    fn ownership_stated_twice_counts_once() {
        // both the derived and the reified shape at the same time -- what
        // this crate itself writes
        let model = example_model();
        let json = to_json(&model);
        let (rebuilt, _) = from_json(&json).unwrap();
        for id in rebuilt.ids() {
            let children = rebuilt.owned(id);
            let distinct: std::collections::HashSet<_> = children.iter().collect();
            assert_eq!(distinct.len(), children.len(), "a child was added twice");
        }
    }

    #[test]
    fn every_object_carries_its_whole_metaclass() {
        let model = example_model();
        let json = to_json(&model);
        for object in json.as_array().unwrap() {
            let kind = ElementKind::from_name(object["@type"].as_str().unwrap()).unwrap();
            let declared: std::collections::BTreeSet<&str> =
                all_features(kind).iter().map(|meta| meta.name).collect();
            let written: std::collections::BTreeSet<&str> = object
                .as_object()
                .unwrap()
                .keys()
                .filter(|key| !key.starts_with('@'))
                .map(String::as_str)
                .collect();
            assert_eq!(written, declared, "for a {}", kind.name());
        }
    }

    #[test]
    fn derived_properties_read_off_the_model() {
        let model = example_model();
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|object| object["@type"] == name)
                .unwrap()
        };

        let mass = objects
            .iter()
            .find(|object| object["declaredName"] == "mass")
            .unwrap();
        assert_eq!(mass["name"], "mass");
        assert_eq!(mass["qualifiedName"], "P::Vehicle::mass");
        assert_eq!(mass["elementId"], mass["@id"]);
        // an unnamed element on the path leaves the name unqualified
        let literal = of_type("LiteralRational");
        assert_eq!(literal["qualifiedName"], Json::Null);
        assert_eq!(literal["owningNamespace"], Json::Null, "owned by a value");

        // a feature of a type is owned through a FeatureMembership, an
        // element of a package through a plain OwningMembership
        let bridge_of = |member: &Json| {
            objects
                .iter()
                .find(|object| {
                    object["ownedRelatedElement"][0]["@id"] == member["@id"]
                        && object["@id"] != member["@id"]
                        && object["memberElement"].is_object()
                })
                .unwrap()
        };
        assert_eq!(bridge_of(mass)["@type"], "FeatureMembership");
        assert_eq!(bridge_of(mass)["memberName"], "mass");
        assert_eq!(bridge_of(mass)["visibility"], "public");
        let vehicle = objects
            .iter()
            .find(|object| object["declaredName"] == "Vehicle")
            .unwrap();
        assert_eq!(bridge_of(vehicle)["@type"], "OwningMembership");
        assert_eq!(vehicle["owningMembership"], vehicle["owningRelationship"]);

        // a relationship relates its owner to what it owns
        let value = of_type("FeatureValue");
        assert_eq!(value["owningRelatedElement"]["@id"], mass["@id"]);
        assert_eq!(value["relatedElement"][0]["@id"], mass["@id"]);
        assert_eq!(value["relatedElement"][1]["@id"], literal["@id"]);
        assert_eq!(value["memberElement"]["@id"], literal["@id"]);

        // documentation is bound to the element it documents
        let package = of_type("Package");
        let doc = of_type("Documentation");
        assert_eq!(package["documentation"][0]["@id"], doc["@id"]);
    }

    /// A resolved model: the workspace reifies typings and
    /// specializations with their targets, which the closures read.
    fn resolved(source: &str) -> (sysml_semantics::Workspace, Model) {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("test.sysml", source);
        ws.resolve_all();
        let model = ws.model().clone();
        (ws, model)
    }

    #[test]
    fn a_member_keeps_its_visibility_across_the_round_trip() {
        let (_, model) =
            resolved("package P {\n\tprivate part def Hidden;\n\tpart def Shown;\n}\n");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let bridge_of = |name: &str| {
            let member = objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap();
            objects
                .iter()
                .find(|object| {
                    object["ownedRelatedElement"][0]["@id"] == member["@id"]
                        && object["@id"] != member["@id"]
                        && object["memberElement"].is_object()
                })
                .unwrap()
        };
        assert_eq!(bridge_of("Hidden")["visibility"], "private");
        assert_eq!(bridge_of("Shown")["visibility"], "public");

        let (rebuilt, _) = from_json(&json).unwrap();
        let hidden = rebuilt
            .ids()
            .find(|&id| rebuilt.name(id) == Some("Hidden"))
            .unwrap();
        assert_eq!(rebuilt.member_visibility(hidden), Some("private"));
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_role_picks_the_membership_and_survives_the_round_trip() {
        let (_, model) = resolved(
            "requirement def R {\n\tsubject veh;\n\tactor driver;\n\tstakeholder owner1;\n\
             \tobjective obj1;\n}\n\
             action def A {\n\tin item x;\n\tout item y;\n\treturn z;\n}\n\
             part def Choice {\n\tvariant part optA;\n}\n\
             connection def C {\n\tend a;\n\tend b;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let membership_of = |name: &str| {
            let member = objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap();
            objects
                .iter()
                .find(|object| {
                    object["ownedRelatedElement"][0]["@id"] == member["@id"]
                        && object["@id"] != member["@id"]
                        && object["memberElement"].is_object()
                })
                .unwrap()["@type"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(membership_of("veh"), "SubjectMembership");
        assert_eq!(membership_of("driver"), "ActorMembership");
        assert_eq!(membership_of("owner1"), "StakeholderMembership");
        assert_eq!(membership_of("obj1"), "ObjectiveMembership");
        assert_eq!(membership_of("x"), "ParameterMembership");
        assert_eq!(membership_of("y"), "ParameterMembership");
        assert_eq!(membership_of("z"), "ReturnParameterMembership");
        assert_eq!(membership_of("optA"), "VariantMembership");
        assert_eq!(membership_of("a"), "EndFeatureMembership");

        // an actor is a part and an objective a requirement, per the spec
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        assert_eq!(of("driver")["@type"], "PartUsage");
        assert_eq!(of("obj1")["@type"], "RequirementUsage");
        // directions flow into the parameters
        assert_eq!(of("x")["direction"], "in");
        let action = of("A");
        assert_eq!(action["input"][0]["@id"], of("x")["@id"]);
        assert_eq!(action["output"][0]["@id"], of("y")["@id"]);

        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json, "the roles survived");
    }

    #[test]
    fn a_singly_referenced_transition_feature_is_recognized_too() {
        // an imported model may hold `guardExpression` as a single
        // reference rather than a list; both spellings name the guard
        let mut model = Model::new();
        let transition = model.create(ElementKind::TransitionUsage);
        let guard = model.create(ElementKind::Expression);
        model.add_owned(transition, guard);
        model.set(transition, "guardExpression", Value::Ref(guard));
        assert_eq!(transition_role(&model, guard), Some("guard"));
    }

    #[test]
    fn a_feature_outside_a_transition_has_no_transition_role() {
        let (_, model) = resolved("part def P {\n\tattribute a;\n}\n");
        let attribute = model.ids().find(|&id| model.name(id) == Some("a")).unwrap();
        assert_eq!(transition_role(&model, attribute), None);
    }

    #[test]
    fn a_transition_feature_membership_states_its_kind() {
        let (_, model) = resolved(
            "state def S {\n\tstate a;\n\tstate b;\n\
             \ttransition t1 first a if x > 0 then b;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let membership = objects
            .iter()
            .find(|object| object["@type"] == "TransitionFeatureMembership")
            .expect("the guard is owned through one");
        assert_eq!(membership["kind"], "guard");
        assert!(membership["transitionFeature"].is_object());
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn the_inheritance_closure_is_derived_from_the_reified_model() {
        let (_, model) = resolved(
            "part def A {\n\tattribute x;\n\tattribute q;\n}\n\
             part def B :> A {\n\tattribute y;\n\tattribute q :>> q;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| {
                    object["declaredName"] == name && object["@type"] == "PartDefinition"
                })
                .unwrap()
        };
        let ids = |value: &Json| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|reference| reference["@id"].as_str().unwrap().to_string())
                .collect()
        };
        let b = of("B");
        let named = |name: &str| {
            objects
                .iter()
                .find(|object| {
                    object["declaredName"] == name && object["@type"] == "AttributeUsage"
                })
                .unwrap()["@id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        // B's own features come first, then what A hands down -- except
        // `q`, which B redefines
        let features = ids(&b["feature"]);
        assert!(features.contains(&named("y")));
        assert!(features.contains(&named("x")));
        assert_eq!(
            features.len(),
            3,
            "q must be inherited only once: {features:?}"
        );
        assert_eq!(ids(&b["inheritedFeature"]), vec![named("x")]);
        assert_eq!(b["inheritedMembership"].as_array().unwrap().len(), 1);
        // the specialization itself is reachable as an owned relationship
        assert_eq!(b["ownedSubclassification"].as_array().unwrap().len(), 1);
        assert_eq!(ids(&b["member"]).len(), ids(&b["ownedMember"]).len() + 1);
        // and a typed usage derives its type from the reified typing
        let (_, model) = resolved("part def T;\npart u : T;\n");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let usage = objects
            .iter()
            .find(|object| object["declaredName"] == "u")
            .unwrap();
        let ty = objects
            .iter()
            .find(|object| object["declaredName"] == "T")
            .unwrap();
        assert_eq!(usage["type"][0]["@id"], ty["@id"]);
        assert_eq!(usage["ownedTyping"].as_array().unwrap().len(), 1);
    }

    /// The extras a resolver provides, computed from a workspace the way
    /// the CLI computes them.
    fn extras_of(ws: &mut sysml_semantics::Workspace) -> Extras {
        let mut extras = Extras::default();
        let ids: Vec<_> = ws.model().ids().collect();
        for id in ids {
            let imported = ws.imported_members(id);
            if !imported.is_empty() {
                extras.imported.insert(id, imported);
            }
            if let Some(target) = ws.import_of(id) {
                extras.import_targets.insert(id, target);
            }
        }
        extras
    }

    #[test]
    fn state_subactions_and_requirement_constraints_state_their_kind() {
        let (_, model) = resolved(
            "state def Heating {\n\tentry action a;\n\tdo action b;\n\texit action c;\n}\n\
             requirement def R {\n\tassume constraint { true }\n\trequire constraint { true }\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let kinds: Vec<(String, String)> = objects
            .iter()
            .filter(|object| {
                matches!(
                    object["@type"].as_str(),
                    Some("StateSubactionMembership" | "RequirementConstraintMembership")
                )
            })
            .map(|object| {
                (
                    object["@type"].as_str().unwrap().to_string(),
                    object["kind"].as_str().unwrap_or("?").to_string(),
                )
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("StateSubactionMembership".to_string(), "entry".to_string()),
                ("StateSubactionMembership".to_string(), "do".to_string()),
                ("StateSubactionMembership".to_string(), "exit".to_string()),
                (
                    "RequirementConstraintMembership".to_string(),
                    "assumption".to_string()
                ),
                (
                    "RequirementConstraintMembership".to_string(),
                    "requirement".to_string()
                ),
            ]
        );
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json, "the kinds survived");
    }

    #[test]
    fn imports_reach_the_member_lists_through_extras() {
        let (mut ws, model) = resolved(
            "package A {\n\tpart def X;\n\tprivate part def Hidden;\n}\n\
             package B {\n\timport A::*;\n}\n\
             package C {\n\timport A::X;\n}\n",
        );
        let extras = extras_of(&mut ws);
        let json = to_json_with(&model, &extras);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        let (a, b, c, x) = (of("A"), of("B"), of("C"), of("X"));

        // `import A::*;` brings X -- and not the private member -- into B
        assert_eq!(b["member"].as_array().unwrap().len(), 1);
        assert_eq!(b["member"][0]["@id"], x["@id"]);
        assert_eq!(b["importedMembership"].as_array().unwrap().len(), 1);
        // the membership imported is X's own owning membership
        assert_eq!(
            b["importedMembership"][0]["@id"],
            x["owningMembership"]["@id"]
        );
        // A holds both of its members and imports nothing
        assert_eq!(a["member"].as_array().unwrap().len(), 2);
        assert_eq!(a["importedMembership"], json!([]));

        // the imports themselves say what they resolved to
        let namespace_import = objects
            .iter()
            .find(|object| object["@type"] == "NamespaceImport")
            .unwrap();
        assert_eq!(namespace_import["importedNamespace"]["@id"], a["@id"]);
        let membership_import = objects
            .iter()
            .find(|object| object["@type"] == "MembershipImport")
            .unwrap();
        assert_eq!(
            membership_import["importedMembership"]["@id"],
            x["owningMembership"]["@id"]
        );
        assert_eq!(c["member"][0]["@id"], x["@id"]);

        // and the whole shape reads back and re-exports identically
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json_with(&rebuilt, &extras), json);
    }

    #[test]
    fn library_elements_say_they_are_library_elements() {
        let (_, model) = resolved("part def Local;\n");
        let root = model.ids().next().unwrap();
        let mut extras = Extras::default();
        let json = to_json_with(&model, &extras);
        assert_eq!(json[0]["isLibraryElement"], false);
        extras.library.insert(root);
        let json = to_json_with(&model, &extras);
        assert_eq!(json[0]["isLibraryElement"], true);
    }

    #[test]
    fn unresolved_relationships_derive_nothing() {
        // reified relationships whose targets never resolved: the closure
        // walks past them instead of tripping over them
        let mut model = Model::new();
        let b = model.create(ElementKind::PartDefinition);
        model.set(b, "declaredName", Value::String("B".to_string()));
        let dangling = model.create(ElementKind::Subclassification);
        model.add_owned(b, dangling);
        let feature = model.create(ElementKind::AttributeUsage);
        model.set(feature, "declaredName", Value::String("x".to_string()));
        model.add_owned(b, feature);
        let untyped = model.create(ElementKind::FeatureTyping);
        model.add_owned(feature, untyped);
        let unredefined = model.create(ElementKind::Redefinition);
        model.add_owned(feature, unredefined);

        let json = to_json(&model);
        let of = |name: &str| {
            json.as_array()
                .unwrap()
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(of("B")["feature"].as_array().unwrap().len(), 1);
        assert_eq!(of("x")["type"], json!([]));
    }

    #[test]
    fn a_trigger_and_an_effect_are_transition_features_too() {
        let (_, model) = resolved(
            "item def Sig;\npart def B { port pt; }\n\
             state def S {\n\tstate a;\n\tstate b;\n\tpart sink : B;\n\
             \ttransition t1 first a accept s1 : Sig do send 1 to sink.pt then b;\n}\n",
        );
        let json = to_json(&model);
        let kinds: Vec<String> = json
            .as_array()
            .unwrap()
            .iter()
            .filter(|object| object["@type"] == "TransitionFeatureMembership")
            .map(|object| object["kind"].as_str().unwrap_or("?").to_string())
            .collect();
        assert!(kinds.contains(&"trigger".to_string()), "{kinds:?}");
        assert!(kinds.contains(&"effect".to_string()), "{kinds:?}");
    }

    #[test]
    fn a_framed_concern_is_a_required_concern() {
        let (_, model) =
            resolved("concern def C1;\nrequirement def R {\n\tframe concern c : C1;\n}\n");
        let json = to_json(&model);
        let membership = json
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["@type"] == "FramedConcernMembership")
            .expect("the concern is owned through one");
        assert_eq!(membership["kind"], "requirement");
        assert!(membership["ownedConcern"].is_object());
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn an_unheard_of_role_is_owned_like_any_member() {
        // a role this crate does not know keeps plain ownership rather
        // than inventing a membership for it
        let mut model = Model::new();
        let package = model.create(ElementKind::Package);
        let part = model.create(ElementKind::PartUsage);
        model.add_owned(package, part);
        model.set_member_role(part, "mystery");
        assert_eq!(membership_kind(&model, part), ElementKind::OwningMembership);
        // and an element with no owner at all needs no membership either
        let loose = model.create(ElementKind::PartUsage);
        assert_eq!(
            membership_kind(&model, loose),
            ElementKind::OwningMembership
        );
    }

    #[test]
    fn a_foreign_subaction_membership_without_a_kind_has_no_role() {
        assert_eq!(
            folded_role(&json!({ "@type": "StateSubactionMembership" })),
            None
        );
        assert_eq!(folded_role(&json!({ "@type": "Whatever" })), None);
    }

    #[test]
    fn an_unnamed_redefinition_takes_the_name_it_redefines() {
        let (_, model) = resolved(
            "part def Vehicle {\n\tattribute mass = 10.0;\n}\n\
             part myCar : Vehicle {\n\tattribute :>> mass = 20.0;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        // two features answer to `mass` now: the declared one and the
        // redefinition named after it
        let named: Vec<&Json> = objects
            .iter()
            .filter(|object| object["name"] == "mass")
            .collect();
        assert_eq!(named.len(), 2);
        let redefining = named
            .iter()
            .find(|object| object["declaredName"].is_null())
            .expect("the redefinition declares no name of its own");
        assert_eq!(redefining["qualifiedName"], "myCar::mass");
        // its membership knows the name too
        let membership = objects
            .iter()
            .find(|object| {
                object["ownedRelatedElement"][0]["@id"] == redefining["@id"]
                    && object["memberElement"].is_object()
                    && object["@id"] != redefining["@id"]
            })
            .unwrap();
        assert_eq!(membership["memberName"], "mass");
        // features featured by their owning type say so
        assert_eq!(
            redefining["featuringType"][0]["@id"],
            objects
                .iter()
                .find(|object| object["declaredName"] == "myCar")
                .unwrap()["@id"]
        );
        assert_eq!(
            redefining["owningFeatureMembership"]["@id"],
            membership["@id"]
        );
    }

    #[test]
    fn a_naming_cycle_ends_in_no_name() {
        // two unnamed features redefining each other -- illegal, but the
        // walk must end rather than recurse forever
        let mut model = Model::new();
        let a = model.create(ElementKind::AttributeUsage);
        let b = model.create(ElementKind::AttributeUsage);
        for (from, to) in [(a, b), (b, a)] {
            let redefinition = model.create(ElementKind::Redefinition);
            model.add_owned(from, redefinition);
            model.set(redefinition, "redefinedFeature", Value::Ref(to));
        }
        assert_eq!(effective_name(&model, a), None);
        assert_eq!(effective_short_name(&model, a), None);
    }

    #[test]
    fn implied_specializations_flow_into_the_closure() {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file(
            "mini.kerml",
            "package Base {\n\tabstract feature things;\n\tabstract datatype DataValue;\n}\n",
        );
        ws.add_file(
            "parts.sysml",
            "package Parts {\n\tabstract part def Part {\n\tattribute portion;\n}\n}\n",
        );
        ws.add_file("model.sysml", "part def Vehicle;\n");
        ws.resolve_all();
        assert!(ws.materialize_implied() > 0);
        let model = ws.model().clone();
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        // Vehicle now inherits Part's features through the implied
        // subclassification, and says the relationship is implied
        let vehicle = of("Vehicle");
        assert_eq!(vehicle["isImpliedIncluded"], true);
        let implied = objects
            .iter()
            .find(|object| {
                object["@type"] == "Subclassification"
                    && object["subclassifier"]["@id"] == vehicle["@id"]
            })
            .unwrap();
        assert_eq!(implied["isImplied"], true);
        assert!(vehicle["inheritedFeature"]
            .as_array()
            .unwrap()
            .iter()
            .any(|feature| feature["@id"] == of("portion")["@id"]));
        // and the whole thing round-trips
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_name_that_is_not_basic_is_quoted() {
        assert_eq!(quoted("mass"), "mass");
        assert_eq!(quoted("wheel 1"), "'wheel 1'");
        assert_eq!(quoted("1st"), "'1st'");
        assert_eq!(quoted(""), "''");
    }

    #[test]
    fn tolerates_nulls_and_foreign_properties() {
        let json = serde_json::json!([
            { "@type": "Package", "@id": "00000000-0000-0000-0000-000000000001",
              "declaredName": null,
              "someToolSpecificThing": 5,
              "isImpliedIncluded": [1, 2] }
        ]);
        let (model, roots) = from_json(&json).unwrap();
        assert_eq!(model.props(roots[0]).count(), 0);
    }

    #[test]
    fn rejects_unknown_types_and_dangling_refs() {
        let bad = serde_json::json!([{ "@type": "NotAClass", "@id": "x" }]);
        assert!(matches!(from_json(&bad), Err(ImportError::UnknownType(_))));
        let dangling = serde_json::json!([
            { "@type": "Package", "@id": "11111111-1111-1111-1111-111111111111",
              "ownedElement": [{ "@id": "22222222-2222-2222-2222-222222222222" }] }
        ]);
        assert!(matches!(
            from_json(&dangling),
            Err(ImportError::UnknownReference(_))
        ));
    }
}
