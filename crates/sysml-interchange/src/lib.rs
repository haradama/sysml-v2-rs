//! Standard JSON interchange for [`sysml_model::Model`].
//!
//! Follows the serialization style of the SysML v2 API & Services
//! standard: every element is a JSON object with `"@type"`, `"@id"` and
//! its properties, where element references are `{"@id": ...}` objects.
//! Element UUIDs are deterministic -- UUIDv5 over the ownership path,
//! where a name places an element among its siblings -- so the same model
//! exports as the same JSON however its files were ordered on the way in.
//!
//! Every element carries the complete property set its metaclass declares:
//! stored properties as they are, derivable ones derived, the rest at
//! their defaults. Derived here are identity and naming, the whole
//! ownership web, a relationship's related elements, a membership's
//! member, and annotation bindings.
//!
//! The inheritance closure is derived from the relationships name
//! resolution reified: `feature`, `inheritedFeature` and
//! `inheritedMembership` walk the resolved specializations and typings,
//! `input`/`output`/`parameter` read the declared directions, and a
//! feature's `type` is its typings' targets. An unresolved model derives
//! empty closures -- which is what it knows.
//!
//! Ownership is reified the way the abstract syntax has it: a membership
//! bridges a namespace and each element it owns, while a pure relationship
//! is owned directly and owns its own elements as `ownedRelatedElement`. A
//! relationship that is also a type or a feature is a member all the same.
//! The membership's metaclass follows the member -- `FeatureMembership`
//! for a feature of a type, `EndFeatureMembership` for a connector end,
//! and so on for each role the notation has a keyword for,
//! `OwningMembership` otherwise -- and each carries the visibility the
//! member was declared with. Bridging memberships are synthesized on
//! export with deterministic UUIDs and folded back on import, so either
//! shape reads back into the same model.
//!
//! What only a resolver can know -- the members imports bring in, what
//! each import resolved to, and which elements belong to a library model
//! -- comes in through [`Extras`], which [`to_json_with`] folds into
//! `member`, `membership`, `importedMembership`, `importedNamespace` and
//! `isLibraryElement`.
//!
//! `name`, `shortName`, `qualifiedName` and a membership's `memberName`
//! follow KerML's effective-name rule: a feature declared without a name
//! answers to the name of what it redefines, or failing that references.
//!
//! Remaining simplification: derived properties beyond the ones named here
//! are emitted at their defaults.

use std::collections::HashMap;

use serde_json::{json, Map, Value as Json};
use sysml_model::{ElementId, ElementKind, FeatureType, Model, PrimitiveType, Role, Value, Vis};
use uuid::Uuid;

/// Errors produced when reading interchange JSON.
#[derive(Debug)]
pub enum ImportError {
    /// The document is not the array of elements the standard writes.
    NotAnArray,
    /// An object at this position declares no `@type`.
    MissingType(usize),
    /// A `@type` no metaclass answers to.
    UnknownType(String),
    /// A `@type` naming a metaclass the standard declares abstract, which no element may be.
    AbstractType(String),
    /// An object at this position declares no `@id`.
    MissingId(usize),
    /// Two objects claim the same `@id`.
    DuplicateId(String),
    /// A reference to an `@id` the document does not declare.
    UnknownReference(String),
    /// Two owners claim the same element.
    SharedOwnership(String),
    /// Ownership that runs in a circle, which no containment can.
    OwnershipCycle(String),
    /// Text that is not JSON, which only [`read_json`] can meet: given a
    /// `Json` the parsing is already somebody else's business.
    NotJson(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::NotAnArray => write!(f, "expected a JSON array of elements"),
            ImportError::MissingType(i) => write!(f, "element {i} has no \"@type\""),
            ImportError::UnknownType(t) => write!(f, "unknown metaclass {t:?}"),
            ImportError::AbstractType(t) => write!(f, "metaclass {t:?} is abstract"),
            ImportError::MissingId(i) => write!(f, "element {i} has no \"@id\""),
            ImportError::DuplicateId(id) => write!(f, "two elements share the id {id:?}"),
            ImportError::UnknownReference(id) => write!(f, "reference to unknown element {id:?}"),
            ImportError::NotJson(why) => write!(f, "not JSON: {why}"),
            ImportError::SharedOwnership(id) => write!(f, "two elements own {id:?}"),
            ImportError::OwnershipCycle(id) => {
                write!(
                    f,
                    "{id:?} is owned by itself, directly or through its owners"
                )
            }
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
        let index = match model.owner(elem) {
            Some(owner) => namesake_index(model, model.owned(owner), elem),
            None => namesake_index(model, &roots(model), elem),
        };
        segments.push(path_segment(model, elem, index));
        current = model.owner(elem);
    }
    segments.reverse();
    uuid_of_path(&format!("sysml-v2-rs:{}", segments.join("/")))
}

/// Where an element sits among the siblings that answer to the same name.
///
/// The plain position among all of them made every UUID in a file depend
/// on which files were loaded before it: a workspace hangs each file's
/// declarations off one root namespace, so `sysml export a.sysml b.sysml`
/// and the same two the other way round moved the package `a.sysml`
/// declares to another slot. Counting only namesakes leaves a name to
/// place its own element, so what a model exports depends on the model
/// alone -- not on the file names, which keying by file would have made it
/// depend on instead.
fn namesake_index(model: &Model, siblings: &[ElementId], id: ElementId) -> usize {
    siblings
        .iter()
        .take_while(|&&sibling| sibling != id)
        .filter(|&&sibling| model.name(sibling) == model.name(id))
        .count()
}

/// The elements no other element owns.
fn roots(model: &Model) -> Vec<ElementId> {
    model
        .ids()
        .filter(|&id| model.owner(id).is_none())
        .collect()
}

/// One step of the path a UUID is built from: where the element sits
/// among its owner's children, and the name it was declared with.
fn path_segment(model: &Model, id: ElementId, index: usize) -> String {
    match model.name(id) {
        Some(name) => format!("{index}:{name}"),
        None => format!("{index}"),
    }
}

fn uuid_of_path(path: &str) -> Uuid {
    Uuid::new_v5(&Uuid::NAMESPACE_URL, path.as_bytes())
}

/// What one walk down the ownership tree settles for every element: the
/// UUID it is written under, the UUID of the membership it is owned
/// through, and the qualified name it answers to.
///
/// Each is a path from the root, and each was once built by walking back
/// up from every element in turn -- the UUID scanning each owner's
/// children to find where the element sits among them, and again for every
/// reference to a synthesized membership, which made an export cost grow
/// with the square of the model's depth.
struct Identities {
    uuids: HashMap<ElementId, Uuid>,
    bridges: HashMap<ElementId, Uuid>,
    qualified: HashMap<ElementId, String>,
}

fn identities(model: &Model) -> Identities {
    let mut out = Identities {
        uuids: HashMap::with_capacity(model.len()),
        bridges: HashMap::new(),
        qualified: HashMap::with_capacity(model.len()),
    };
    let roots = roots(model);
    let mut stack: Vec<(ElementId, String, Option<String>)> = roots
        .iter()
        .copied()
        .map(|id| {
            (
                id,
                format!(
                    "sysml-v2-rs:{}",
                    path_segment(model, id, namesake_index(model, &roots, id))
                ),
                // the root namespace has no name and contributes no
                // segment; what it owns is named from there
                qualified_under(Some(""), model, id).or(Some(String::new())),
            )
        })
        .collect();
    while let Some((id, path, qualified)) = stack.pop() {
        let children = model.owned(id);
        // one pass over the children, counting each name as it goes:
        // asking where each of them sits among its namesakes one at a
        // time would walk the whole list again for every one of them
        let mut namesakes: HashMap<Option<&str>, usize> = HashMap::new();
        for &child in children {
            let seen = namesakes.entry(model.name(child)).or_default();
            let index = *seen;
            *seen += 1;
            stack.push((
                child,
                format!("{path}/{}", path_segment(model, child, index)),
                qualified_under(qualified.as_deref(), model, child),
            ));
        }
        let uuid = uuid_of_path(&path);
        if bridged(model, id) {
            out.bridges.insert(
                id,
                uuid_of_path(&format!("sysml-v2-rs:{uuid}#owningMembership")),
            );
        }
        out.uuids.insert(id, uuid);
        if let Some(qualified) = qualified.filter(|name| !name.is_empty()) {
            out.qualified.insert(id, qualified);
        }
    }
    out
}

/// The qualified name of an element under the name of its owner:
/// nothing, once an element on the way has no name to write.
fn qualified_under(prefix: Option<&str>, model: &Model, id: ElementId) -> Option<String> {
    let prefix = prefix?;
    let name = quoted(model.effective_name(id)?);
    Some(if prefix.is_empty() {
        name
    } else {
        format!("{prefix}::{name}")
    })
}

/// The membership metaclasses that only carry ownership -- and, for some,
/// a role or a visibility the owned element keeps -- so they fold into
/// edges on import and are synthesized back on export. `FeatureValue` and
/// friends stay real elements: they carry state of their own.
const FOLDED: [ElementKind; 17] = [
    ElementKind::OwningMembership,
    ElementKind::FeatureMembership,
    ElementKind::EndFeatureMembership,
    ElementKind::ParameterMembership,
    ElementKind::ReturnParameterMembership,
    ElementKind::ResultExpressionMembership,
    ElementKind::SubjectMembership,
    ElementKind::ActorMembership,
    ElementKind::StakeholderMembership,
    ElementKind::ObjectiveMembership,
    ElementKind::VariantMembership,
    ElementKind::TransitionFeatureMembership,
    ElementKind::StateSubactionMembership,
    ElementKind::RequirementConstraintMembership,
    ElementKind::RequirementVerificationMembership,
    ElementKind::FramedConcernMembership,
    ElementKind::ViewRenderingMembership,
];

/// The derived properties the model stores itself, and so the only ones
/// import reads back: a multiplicity and its bounds, a transition's
/// trigger, guard and effect, and the rest of what the builder writes into
/// a property the metamodel calls derived.
///
/// Everything else derived is computed afresh on export. Reading one of
/// those back would let the copy that came in shadow the answer: an
/// element renamed after import would still export the `name` it arrived
/// with.
///
/// The CLI's corpus sweep round-trips every file and reports the drift
/// when one is missing from this list.
const STORED_DERIVED: [&str; 18] = [
    "bound",
    "chainingFeature",
    "condition",
    "effectAction",
    "featureWithValue",
    "guardExpression",
    "lowerBound",
    "multiplicity",
    "referencingFeature",
    "referent",
    "relatedFeature",
    "representedElement",
    "satisfiedRequirement",
    "satisfyingFeature",
    "triggerAction",
    "upperBound",
    "value",
    "verifiedRequirement",
];

/// The role a folded membership gives back to its member, so that what
/// picked the membership's metaclass survives the round trip. A state
/// subaction's and a requirement constraint's metaclass alone does not
/// say which role it was: their `kind` does.
fn folded_role(bridge: &Json) -> Option<Role> {
    let roles: [(ElementKind, Role); 7] = [
        (ElementKind::SubjectMembership, Role::Subject),
        (ElementKind::ActorMembership, Role::Actor),
        (ElementKind::StakeholderMembership, Role::Stakeholder),
        (ElementKind::ObjectiveMembership, Role::Objective),
        (ElementKind::VariantMembership, Role::Variant),
        (ElementKind::ReturnParameterMembership, Role::Return),
        (ElementKind::ResultExpressionMembership, Role::Result),
    ];
    let written = bridge["@type"].as_str();
    if let Some((_, role)) = roles.iter().find(|(kind, _)| Some(kind.name()) == written) {
        return Some(*role);
    }
    match written {
        Some("StateSubactionMembership") => match bridge["kind"].as_str() {
            Some("entry") => Some(Role::Entry),
            Some("do") => Some(Role::Do),
            Some("exit") => Some(Role::Exit),
            _ => None,
        },
        Some("FramedConcernMembership") => Some(Role::Frame),
        Some("ViewRenderingMembership") => Some(Role::Render),
        Some("RequirementVerificationMembership") => Some(Role::Verify),
        Some("RequirementConstraintMembership") => match bridge["kind"].as_str() {
            Some("assumption") => Some(Role::Assume),
            _ => Some(Role::Require),
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

    /// The features an element owns directly, connectors among them: a
    /// connector is a feature of the type that declares it, whatever
    /// else it relates.
    fn owned_features(&self, id: ElementId) -> Vec<ElementId> {
        self.model
            .owned(id)
            .iter()
            .copied()
            .filter(|&child| self.model.kind(child).is_a(ElementKind::Feature))
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
                match self.model.maybe(child, target) {
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
            .filter_map(|&child| match self.model.maybe(child, "redefinedFeature") {
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
                    .maybe(feature, "direction")
                    .and_then(Value::as_str)
                    .is_some_and(|direction| wanted.contains(&direction))
            })
            .collect()
    }
}

/// Does ownership of this element pass through a synthesized membership?
///
/// A pure relationship needs none at either end: the standard has an
/// element own those directly, so a `FeatureValue` holds the expression it
/// sets without a membership between them. A relationship that is also a
/// type or a feature is a member all the same -- the grammar reaches a
/// connector, an association and a dependency alike through the membership
/// of the namespace that declares them -- and a relationship that is also
/// a namespace owns its own members that way in turn.
fn bridged(model: &Model, owned: ElementId) -> bool {
    if only_a_relationship(model.kind(owned)) {
        return false;
    }
    !model.owner(owned).is_some_and(|owner| {
        let kind = model.kind(owner);
        kind.is_a(ElementKind::Relationship) && !kind.is_a(ElementKind::Namespace)
    })
}

/// Is this metaclass a relationship and nothing else -- a membership, an
/// import, a specialization -- rather than one that is also a type or a
/// dependency?
///
/// What a relationship owns that is only a relationship in turn is an
/// `ownedRelationship` of it and no end of it, while a connector or an
/// association owned by a membership is both.
fn only_a_relationship(kind: ElementKind) -> bool {
    kind.is_a(ElementKind::Relationship)
        && !kind.is_a(ElementKind::Type)
        && !kind.is_a(ElementKind::Dependency)
}

/// What a relationship owns and relates in one: the ends it holds itself,
/// rather than everything under it.
///
/// An `import A::*[@Safety]` owns the filter that narrows it, and a
/// `Membership` owns whatever it brings in; neither is an end of the
/// relationship above it. Counting them as ends made the ends the model
/// states disagree with the ends read back.
fn related_to(model: &Model, id: ElementId) -> Vec<ElementId> {
    model
        .owned(id)
        .iter()
        .copied()
        .filter(|&child| !bridged(model, child) && !only_a_relationship(model.kind(child)))
        .collect()
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

/// A name the way `qualifiedName` writes it: as it is when it is a basic
/// name, quoted when it is not -- a keyword included, since `part`
/// written plainly reads back as the keyword rather than as a name.
/// Inside the quotes a backslash and a quote are escaped again, undoing
/// what [`sysml_syntax::unquote`] did when the name was read.
fn quoted(name: &str) -> String {
    let basic = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && sysml_syntax::SyntaxKind::from_keyword(name).is_none();
    if basic {
        return name.to_string();
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push('\'');
    for ch in name.chars() {
        match ch {
            '\\' | '\'' => {
                out.push('\\');
                out.push(ch);
            }
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

/// What an annotating element's `about` named: the target of each
/// `Annotation` it owns, in the order they were written. A comment that
/// named nothing owns none and is about the element it sits on.
fn annotated(model: &Model, id: ElementId) -> Vec<ElementId> {
    model
        .owned(id)
        .iter()
        .copied()
        .filter(|&child| model.kind(child) == ElementKind::Annotation)
        .filter_map(|child| model.get(child, "annotatedElement").and_then(Value::as_id))
        .collect()
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
    /// Elements to leave out of the document altogether.
    ///
    /// A model is resolved against a library and is not made of one: a
    /// six-element model exported with the standard library beside it writes
    /// ninety-six thousand elements, and `sysml api push` sends them. What the
    /// model refers to across that line is written as the `@id` it always was
    /// -- UUIDv5 over the ownership path, so anybody holding the same library
    /// computes the same ones.
    pub omitted: std::collections::HashSet<ElementId>,
}

/// Serialize the whole model as an array of element objects in stable
/// order: every element in arena order, then the memberships synthesized
/// between each owner and the owned elements that need one.
///
/// Derived properties that need the resolver -- imported memberships,
/// `isLibraryElement` -- stay at their defaults here; [`to_json_with`]
/// takes them as [`Extras`].
pub fn to_json(model: &Model) -> Json {
    to_json_with(model, &Extras::default())
}

/// One model being written out, and everything worked out about it once.
///
/// This was eight closures stacked at the top of a five-hundred-line
/// function, capturing seven things between them. A closure cannot be
/// lifted out of the body that declares it, so the length was not
/// incidental to the design -- it *was* the design.
struct Writing<'a> {
    model: &'a Model,
    extras: &'a Extras,
    /// The identity every element is referred to by.
    uuids: HashMap<ElementId, Uuid>,
    /// And the identity of the membership standing between an owner and
    /// what it owns, where the model bridges that ownership.
    bridge_uuids: HashMap<ElementId, Uuid>,
    qualified: HashMap<ElementId, String>,
    /// The property set each metaclass declares. Working one out means
    /// walking a whole inheritance chain, so it is done once per
    /// metaclass rather than once per element.
    features: HashMap<ElementKind, Vec<&'static sysml_model::FeatureMeta>>,
    /// The inheritance closures the derived properties are read from.
    closures: Closures<'a>,
}

impl Writing<'_> {
    fn reference(&self, id: &ElementId) -> Json {
        json!({ "@id": self.uuids[id].to_string() })
    }

    fn membership(&self, id: ElementId) -> Json {
        json!({ "@id": self.bridge_uuids[&id].to_string() })
    }

    fn references(&self, ids: &[ElementId]) -> Json {
        Json::Array(ids.iter().map(|id| self.reference(id)).collect())
    }

    fn annotations(&self, id: ElementId, kind: ElementKind) -> Json {
        Json::Array(
            self.model
                .owned(id)
                .iter()
                .filter(|&&child| self.model.kind(child).is_a(kind))
                .map(|child| self.reference(child))
                .collect(),
        )
    }

    /// A membership reference for one element a namespace reaches: its
    /// bridge where ownership is bridged, nothing where it is not.
    fn membership_of(&self, id: ElementId) -> Option<Json> {
        bridged(self.model, id).then(|| self.membership(id))
    }

    fn memberships(&self, ids: &[ElementId]) -> Json {
        Json::Array(
            ids.iter()
                .copied()
                .filter_map(|id| self.membership_of(id))
                .collect(),
        )
    }

    /// What a namespace holds, membership by membership: a membership it
    /// owns outright stands for itself, and anything else is reached
    /// through the bridge that owns it.
    fn owned_memberships(&self, id: ElementId) -> Vec<Json> {
        self.model
            .owned(id)
            .iter()
            .filter_map(
                |&child| match self.model.kind(child).is_a(ElementKind::Membership) {
                    true => Some(self.reference(&child)),
                    false => self.membership_of(child),
                },
            )
            .collect()
    }

    /// Every element of the model, in arena order, each carrying the
    /// whole property set its metaclass declares.
    fn elements(&self) -> impl Iterator<Item = Json> + '_ {
        self.model
            .ids()
            .filter(|id| !self.extras.omitted.contains(id))
            .map(|id| {
                let stored: HashMap<&str, &Value> = self.model.props(id).collect();
                let mut object = Map::new();
                object.insert("@type".into(), self.model.kind(id).name().into());
                object.insert("@id".into(), self.uuids[&id].to_string().into());
                for meta in &self.features[&self.model.kind(id)] {
                    let value = match stored.get(meta.name) {
                        Some(value) => match value {
                            Value::Bool(b) => Json::from(*b),
                            Value::Int(i) => Json::from(*i),
                            // JSON has no infinity: a number its syntax
                            // cannot hold is written as the text of it, and
                            // read back from that text
                            Value::Real(r) if !r.is_finite() => Json::from(r.to_string()),
                            Value::Real(r) => Json::from(*r),
                            Value::String(text) => Json::from(text.clone()),
                            Value::EnumLit(lit) => Json::from(*lit),
                            Value::Ref(r) => self.reference(r),
                            Value::RefList(rs) => self.references(rs),
                        },
                        None => self
                            .derived(id, meta.name)
                            .unwrap_or_else(|| default_for(meta)),
                    };
                    object.insert(meta.name.into(), value);
                }
                if self.model.kind(id).is_a(ElementKind::Relationship) {
                    let owner = self
                        .model
                        .owner(id)
                        .filter(|_| !bridged(self.model, id))
                        .map(|owner| self.reference(&owner));
                    let related: Vec<Json> = related_to(self.model, id)
                        .iter()
                        .map(|it| self.reference(it))
                        .collect();
                    fill_ends(&mut object, self.model.kind(id), owner, related);
                }
                Json::Object(object)
            })
    }

    /// And the memberships synthesized between each owner and what it
    /// owns, in the order of what they bring in, carrying their whole
    /// property set like any other element.
    fn bridges(&self) -> impl Iterator<Item = Json> + '_ {
        // the memberships themselves, in the order of what they bring in,
        // carrying their whole property set like any other element
        self.model.ids().filter_map(move |id| {
            let owner = self.model.owner(id)?;
            if !bridged(self.model, id) {
                return None;
            }
            // a membership of an element that is not in the document
            // brings nothing into it
            if self.extras.omitted.contains(&id) {
                return None;
            }
            let kind = sysml_model::membership_kind(self.model, id);
            let uuid = self.bridge_uuids[&id].to_string();
            let mut object = Map::new();
            object.insert("@type".into(), kind.name().into());
            object.insert("@id".into(), uuid.clone().into());
            for meta in &self.features[&kind] {
                let value = match meta.name {
                    "elementId" => uuid.clone().into(),
                    "owner" | "owningRelatedElement" | "membershipOwningNamespace" => {
                        self.reference(&owner)
                    }
                    "ownedElement" | "ownedRelatedElement" => {
                        Json::Array(vec![self.reference(&id)])
                    }
                    "relatedElement" => {
                        Json::Array(vec![self.reference(&owner), self.reference(&id)])
                    }
                    "source" => Json::Array(vec![self.reference(&owner)]),
                    "target" => Json::Array(vec![self.reference(&id)]),
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
                    | "action"
                    | "ownedResultExpression"
                    | "ownedConstraint"
                    | "ownedRequirement"
                    | "ownedConcern"
                    | "ownedRendering" => self.reference(&id),
                    // what the member refers to, rather than the member: a
                    // required constraint, a framed concern, a verified
                    // requirement and the rendering a view is drawn with
                    // each name the element their member references, where
                    // it references one
                    "referencedConstraint"
                    | "referencedConcern"
                    | "verifiedRequirement"
                    | "referencedRendering" => self
                        .model
                        .owned(id)
                        .iter()
                        .find(|&&child| self.model.kind(child) == ElementKind::ReferenceSubsetting)
                        .and_then(|&child| self.model.get(child, "referencedFeature"))
                        .and_then(Value::as_id)
                        .map_or(Json::Null, |target| self.reference(&target)),
                    "memberElementId" | "ownedMemberElementId" => {
                        self.uuids[&id].to_string().into()
                    }
                    "memberName" | "ownedMemberName" => {
                        self.model.effective_name(id).map_or(Json::Null, Json::from)
                    }
                    "memberShortName" | "ownedMemberShortName" => self
                        .model
                        .effective_short_name(id)
                        .map_or(Json::Null, Json::from),
                    "owningType" => self.reference(&owner),
                    // the standard spells it out even where nothing was written
                    "visibility" => self
                        .model
                        .member_visibility(id)
                        .unwrap_or(Vis::Public)
                        .keyword()
                        .into(),
                    // a transition feature's membership says which it is,
                    // and so do a state's subactions and a requirement's
                    // constraints, in their own vocabularies
                    "kind" if kind == ElementKind::TransitionFeatureMembership => {
                        sysml_model::transition_role(self.model, id).map_or(Json::Null, Json::from)
                    }
                    // the standard spells a state subaction's kind with the
                    // keyword that declared it
                    "kind" if kind == ElementKind::StateSubactionMembership => [
                        (Role::Entry, "entry"),
                        (Role::Do, "do"),
                        (Role::Exit, "exit"),
                    ]
                    .iter()
                    .find(|(role, _)| Some(*role) == self.model.member_role(id))
                    .map_or(Json::Null, |(_, word)| Json::from(*word)),
                    "kind" if kind.is_a(ElementKind::RequirementConstraintMembership) => {
                        // an assumption says so; a required constraint and a
                        // framed concern are both requirements
                        if self.model.member_role(id) == Some(Role::Assume) {
                            "assumption".into()
                        } else {
                            "requirement".into()
                        }
                    }
                    _ => default_for(meta),
                };
                object.insert(meta.name.into(), value);
            }
            Some(Json::Object(object))
        })
    }

    /// What a name-driven derived property holds, when the model can say.
    ///
    /// The metamodel declares these and states no value for them: they are
    /// what a reader is expected to work out from what the model keeps. What
    /// this model cannot answer is left at the default, which is the honest
    /// answer for a property it does not build the abstract syntax for.
    fn derived(&self, id: ElementId, name: &str) -> Option<Json> {
        let kind = self.model.kind(id);
        let is_relationship = kind.is_a(ElementKind::Relationship);
        let is_type = kind.is_a(ElementKind::Type);
        match name {
            // the inheritance closure, from the reified specializations
            "feature" if is_type => Some(self.references(&self.closures.features(id))),
            "ownedFeature" if is_type => Some(self.references(&self.closures.owned_features(id))),
            "inheritedFeature" if is_type => {
                let owned: std::collections::HashSet<ElementId> =
                    self.closures.owned_features(id).into_iter().collect();
                Some(
                    self.references(
                        &self
                            .closures
                            .features(id)
                            .iter()
                            .copied()
                            .filter(|feature| !owned.contains(feature))
                            .collect::<Vec<_>>(),
                    ),
                )
            }
            "inheritedMembership" if is_type => {
                let owned: std::collections::HashSet<ElementId> =
                    self.closures.owned_features(id).into_iter().collect();
                let inherited: Vec<ElementId> = self
                    .closures
                    .features(id)
                    .iter()
                    .copied()
                    .filter(|feature| !owned.contains(feature))
                    .collect();
                Some(self.memberships(&inherited))
            }
            "endFeature" if is_type => Some(
                self.references(
                    &self
                        .closures
                        .features(id)
                        .iter()
                        .copied()
                        .filter(|&feature| self.model.flag(feature, "isEnd"))
                        .collect::<Vec<_>>(),
                ),
            ),
            "ownedEndFeature" if is_type => Some(
                self.references(
                    &self
                        .closures
                        .owned_features(id)
                        .into_iter()
                        .filter(|&feature| self.model.flag(feature, "isEnd"))
                        .collect::<Vec<_>>(),
                ),
            ),
            "input" if is_type => {
                Some(self.references(&self.closures.directed(id, &["in", "inout"])))
            }
            "output" if is_type => {
                Some(self.references(&self.closures.directed(id, &["out", "inout"])))
            }
            "directedFeature" if is_type => {
                Some(self.references(&self.closures.directed(id, &["in", "out", "inout"])))
            }
            "parameter" if is_type => {
                Some(self.references(&self.closures.directed(id, &["in", "out", "inout"])))
            }
            "ownedMembership" => Some(Json::Array(self.owned_memberships(id))),
            // everything `ownedMembership` holds, and then what the
            // namespace reaches without owning it
            "membership" => {
                let mut all = self.owned_memberships(id);
                for &imported in self.extras.imported.get(&id).into_iter().flatten() {
                    all.extend(self.membership_of(imported));
                }
                if is_type {
                    let owned: std::collections::HashSet<ElementId> =
                        self.closures.owned_features(id).into_iter().collect();
                    for &feature in self.closures.features(id).iter() {
                        if !owned.contains(&feature) {
                            all.extend(self.membership_of(feature));
                        }
                    }
                }
                Some(Json::Array(all))
            }
            "ownedMember" => Some(
                self.references(
                    &self
                        .model
                        .owned(id)
                        .iter()
                        .filter(|&&child| bridged(self.model, child))
                        .copied()
                        .collect::<Vec<_>>(),
                ),
            ),
            "member" => {
                let mut members: Vec<ElementId> = self
                    .model
                    .owned(id)
                    .iter()
                    .copied()
                    .filter(|&child| bridged(self.model, child))
                    .collect();
                let mut seen: std::collections::HashSet<ElementId> =
                    members.iter().copied().collect();
                for &imported in self.extras.imported.get(&id).into_iter().flatten() {
                    if seen.insert(imported) {
                        members.push(imported);
                    }
                }
                if is_type {
                    members.extend(
                        self.closures
                            .features(id)
                            .iter()
                            .copied()
                            .filter(|&feature| seen.insert(feature)),
                    );
                }
                Some(self.references(&members))
            }
            "importedMembership" if kind == ElementKind::MembershipImport => Some(
                self.extras
                    .import_targets
                    .get(&id)
                    .and_then(|&target| self.membership_of(target))
                    .unwrap_or(Json::Null),
            ),
            "importedMembership" => {
                Some(self.memberships(self.extras.imported.get(&id).map_or(&[][..], Vec::as_slice)))
            }
            "importedNamespace" if kind == ElementKind::NamespaceImport => Some(
                self.extras
                    .import_targets
                    .get(&id)
                    .map_or(Json::Null, |it| self.reference(it)),
            ),
            "isLibraryElement" => Some(self.extras.library.contains(&id).into()),
            // an import says how far what it brings in travels; written
            // without a keyword, the metamodel has it stop where it is
            "visibility" if kind.is_a(ElementKind::Import) => Some(
                self.model
                    .member_visibility(id)
                    .unwrap_or(Vis::Private)
                    .keyword()
                    .into(),
            ),
            // the specializations an element owns, by their metaclass
            "ownedSpecialization" if is_type => {
                Some(self.owned_of_kind(id, ElementKind::Specialization))
            }
            "ownedSubclassification" => {
                Some(self.owned_of_kind(id, ElementKind::Subclassification))
            }
            "ownedTyping" => Some(self.owned_of_kind(id, ElementKind::FeatureTyping)),
            "ownedSubsetting" => Some(self.owned_of_kind(id, ElementKind::Subsetting)),
            "ownedRedefinition" => Some(self.owned_of_kind(id, ElementKind::Redefinition)),
            "ownedReferenceSubsetting" => self
                .model
                .owned(id)
                .iter()
                .find(|&&child| self.model.kind(child) == ElementKind::ReferenceSubsetting)
                .map(|it| self.reference(it))
                .or(Some(Json::Null)),
            "ownedImport" => Some(self.owned_of_kind(id, ElementKind::Import)),
            // a feature is typed by what its reified typings resolved to
            "type" if kind.is_a(ElementKind::Feature) => Some(Json::Array(
                self.model
                    .types_of(id)
                    .map(|target| self.reference(&target))
                    .collect(),
            )),
            "owningType" if kind.is_a(ElementKind::Feature) => Some(
                self.model
                    .owner(id)
                    .filter(|&owner| self.model.kind(owner).is_a(ElementKind::Type))
                    .map_or(Json::Null, |owner| self.reference(&owner)),
            ),
            // an owned feature is featured by -- and its membership is --
            // its owning type's
            "featuringType" if kind.is_a(ElementKind::Feature) => Some(Json::Array(
                self.model
                    .owner(id)
                    .filter(|&owner| self.model.kind(owner).is_a(ElementKind::Type))
                    .iter()
                    .map(|it| self.reference(it))
                    .collect(),
            )),
            "owningFeatureMembership" if kind.is_a(ElementKind::Feature) => {
                let of_a_type = bridged(self.model, id)
                    && self.model.owner(id).is_some()
                    && sysml_model::membership_kind(self.model, id)
                        .is_a(ElementKind::FeatureMembership);
                Some(if of_a_type {
                    self.membership(id)
                } else {
                    Json::Null
                })
            }
            // a relationship owned by one of the elements it relates
            // names that end again in its own vocabulary
            "owningClassifier"
            | "owningFeature"
            | "owningFeatureOfType"
            | "membershipOwningNamespace"
            | "importOwningNamespace"
            | "owningAnnotatingElement"
                if is_relationship =>
            {
                Some(match self.model.owner(id) {
                    Some(owner) if !bridged(self.model, id) => self.reference(&owner),
                    _ => Json::Null,
                })
            }
            // an annotating element is about what its `about` named --
            // one owned Annotation for each -- and, where it named
            // nothing, about the element it sits on
            "annotatedElement" if kind.is_a(ElementKind::AnnotatingElement) => {
                let about = annotated(self.model, id);
                // `representedElement` belongs to a textual representation; a
                // comment or a documentation is an annotating element without
                // one
                Some(match self.model.maybe(id, "representedElement") {
                    Some(Value::Ref(target)) => Json::Array(vec![self.reference(target)]),
                    _ if !about.is_empty() => self.references(&about),
                    _ => Json::Array(
                        self.model
                            .owner(id)
                            .iter()
                            .map(|it| self.reference(it))
                            .collect(),
                    ),
                })
            }
            "annotation" | "ownedAnnotatingRelationship"
                if kind.is_a(ElementKind::AnnotatingElement) =>
            {
                Some(self.owned_of_kind(id, ElementKind::Annotation))
            }
            "nestedUsage" if kind.is_a(ElementKind::Usage) => {
                Some(self.owned_of_kind(id, ElementKind::Usage))
            }
            "ownedUsage" if kind.is_a(ElementKind::Definition) => {
                Some(self.owned_of_kind(id, ElementKind::Usage))
            }
            "elementId" => Some(self.uuids[&id].to_string().into()),
            "name" => Some(self.model.effective_name(id).map_or(Json::Null, Json::from)),
            "shortName" => Some(
                self.model
                    .effective_short_name(id)
                    .map_or(Json::Null, Json::from),
            ),
            "qualifiedName" => Some(
                self.qualified
                    .get(&id)
                    .map_or(Json::Null, |name| Json::from(name.clone())),
            ),
            "owner" => Some(
                self.model
                    .owner(id)
                    .map_or(Json::Null, |owner| self.reference(&owner)),
            ),
            "ownedElement" => Some(self.references(self.model.owned(id))),
            // the reified shape: a membership bridges the way down to an
            // ordinary element, a relationship is owned as itself, and
            // what a relationship owns appears only as related elements
            "ownedRelationship" => Some(Json::Array(
                self.model
                    .owned(id)
                    .iter()
                    .filter_map(|&child| {
                        if bridged(self.model, child) {
                            Some(self.membership(child))
                        } else if self.model.kind(child).is_a(ElementKind::Relationship) {
                            Some(self.reference(&child))
                        } else {
                            None
                        }
                    })
                    .collect(),
            )),
            "owningRelationship" => {
                let owner = self.model.owner(id)?;
                Some(if bridged(self.model, id) {
                    self.membership(id)
                } else if self.model.kind(owner).is_a(ElementKind::Relationship) {
                    self.reference(&owner)
                } else {
                    Json::Null
                })
            }
            "owningMembership" => Some(
                if bridged(self.model, id) && self.model.owner(id).is_some() {
                    self.membership(id)
                } else {
                    Json::Null
                },
            ),
            "owningNamespace" => Some(if bridged(self.model, id) {
                self.model
                    .owner(id)
                    .map_or(Json::Null, |owner| self.reference(&owner))
            } else {
                Json::Null
            }),
            "documentation" => Some(self.annotations(id, ElementKind::Documentation)),
            "textualRepresentation" => {
                Some(self.annotations(id, ElementKind::TextualRepresentation))
            }
            // a relationship that is also a namespace holds its members
            // through memberships; only the rest is directly related
            "ownedRelatedElement" if is_relationship => {
                Some(self.references(&related_to(self.model, id)))
            }
            // a bridged relationship is owned by its membership, which
            // is no end of it
            "owningRelatedElement" if is_relationship => Some(match self.model.owner(id) {
                Some(owner) if !bridged(self.model, id) => self.reference(&owner),
                _ => Json::Null,
            }),
            // an alias names what it brings in itself: `alias Q for P;`
            // is a membership through which P is known as Q. What it
            // stands for is the resolver's to say, and stays null here.
            "memberName" if kind == ElementKind::Membership => {
                Some(self.model.name(id).map_or(Json::Null, Json::from))
            }
            "memberShortName" if kind == ElementKind::Membership => Some(
                self.model
                    .get(id, "declaredShortName")
                    .and_then(Value::as_str)
                    .map_or(Json::Null, Json::from),
            ),
            // a membership's member: for the owning kind, what it owns
            "memberElement" | "ownedMemberElement" if kind.is_a(ElementKind::OwningMembership) => {
                Some(
                    self.model
                        .owned(id)
                        .first()
                        .map_or(Json::Null, |it| self.reference(it)),
                )
            }
            "memberName" | "ownedMemberName" if kind.is_a(ElementKind::OwningMembership) => Some(
                self.model
                    .owned(id)
                    .first()
                    .and_then(|&member| self.model.name(member))
                    .map_or(Json::Null, Json::from),
            ),
            "memberElementId" | "ownedMemberElementId"
                if kind.is_a(ElementKind::OwningMembership) =>
            {
                Some(
                    self.model
                        .owned(id)
                        .first()
                        .map_or(Json::Null, |member| self.uuids[member].to_string().into()),
                )
            }
            _ => None,
        }
    }

    fn owned_of_kind(&self, id: ElementId, wanted: ElementKind) -> Json {
        Json::Array(
            self.model
                .owned(id)
                .iter()
                .filter(|&&child| self.model.kind(child).is_a(wanted))
                .map(|child| self.reference(child))
                .collect(),
        )
    }
}

/// [`to_json`], with the resolver-derived facts filled in from
/// [`Extras`].
pub fn to_json_with(model: &Model, extras: &Extras) -> Json {
    let writing = writing(model, extras);
    Json::Array(writing.elements().chain(writing.bridges()).collect())
}

/// Everything worked out about a model once, before anything is written.
fn writing<'a>(model: &'a Model, extras: &'a Extras) -> Writing<'a> {
    let Identities {
        uuids,
        bridges: bridge_uuids,
        qualified,
    } = identities(model);
    let features: HashMap<ElementKind, Vec<&'static sysml_model::FeatureMeta>> =
        sysml_model::generated::ELEMENT_KINDS
            .iter()
            .map(|&kind| (kind, all_features(kind)))
            .collect();
    Writing {
        model,
        extras,
        uuids,
        bridge_uuids,
        qualified,
        features,
        closures: Closures::new(model),
    }
}

/// Write the document out, an element at a time.
///
/// The same bytes [`to_json_with`] would give [`serde_json::to_writer_pretty`],
/// without the document ever existing whole. That matters more than it
/// sounds: a model of the standard library is 47 MB, the document is 754
/// MB of text, and the `serde_json::Value` between them is about six
/// gigabytes -- a hundred and twenty times the model, built only to be
/// turned into text and dropped. Here each element becomes a `Value`,
/// is written, and goes.
///
/// The array is framed by the serializer rather than by hand, which is
/// what keeps the indentation of what is inside it right.
///
/// # Errors
///
/// Whatever `out` gives back. Nothing here can fail to serialize.
pub fn write_json<W: std::io::Write>(
    model: &Model,
    extras: &Extras,
    out: W,
) -> serde_json::Result<usize> {
    let document = Document {
        writing: &writing(model, extras),
        written: std::cell::Cell::new(0),
    };
    serde_json::to_writer_pretty(out, &document)?;
    Ok(document.written.get())
}

/// The document as a sequence, pulled from the model as it is written.
struct Document<'a> {
    writing: &'a Writing<'a>,
    /// How many have gone by. The count is the walk, so there is nothing
    /// to report until the walk is over -- and a caller that says how
    /// many elements it wrote wants it.
    written: std::cell::Cell<usize>,
}

impl serde::Serialize for Document<'_> {
    fn serialize<S: serde::Serializer>(&self, out: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        // no length, for the same reason
        let mut array = out.serialize_seq(None)?;
        let mut written = 0;
        for element in self.writing.elements().chain(self.writing.bridges()) {
            array.serialize_element(&element)?;
            written += 1;
        }
        self.written.set(written);
        array.end()
    }
}

/// What a property nothing sets or derives reads as: absent, empty or
/// plainly false -- except a membership's visibility, which the
/// metamodel defaults to `public`.
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

/// Where each relationship metaclass writes its ends: the chain of
/// property names from the one it declares down to `source` or `target`,
/// each redefining or subsetting the next. A `Subclassification` writes
/// its specific end as `subclassifier`, which redefines `specific`, which
/// subsets `source`. Read most specific first, and only the links the
/// metaclass declares are written, so one entry serves a whole family.
const ENDS: [(ElementKind, &[&str], &[&str]); 15] = [
    (
        ElementKind::FeatureValue,
        &["featureWithValue", "membershipOwningNamespace", "source"],
        &["value", "memberElement", "target"],
    ),
    (
        ElementKind::Membership,
        &["membershipOwningNamespace", "source"],
        &[
            "ownedMemberFeature",
            "ownedMemberElement",
            "memberElement",
            "target",
        ],
    ),
    (
        ElementKind::Import,
        &["importOwningNamespace", "source"],
        &[
            "importedMembership",
            "importedNamespace",
            "importedElement",
            "target",
        ],
    ),
    (
        ElementKind::Specialization,
        &[
            "subclassifier",
            "typedFeature",
            "redefiningFeature",
            "referencingFeature",
            "subsettingFeature",
            "specific",
            "source",
        ],
        &[
            "superclassifier",
            "type",
            "redefinedFeature",
            "referencedFeature",
            "subsettedFeature",
            "general",
            "target",
        ],
    ),
    (
        ElementKind::Dependency,
        &["client", "source"],
        &["supplier", "target"],
    ),
    (
        ElementKind::Annotation,
        &["annotatingElement", "owningAnnotatingElement", "source"],
        &["annotatedElement", "owningAnnotatedElement", "target"],
    ),
    (
        ElementKind::Conjugation,
        &["conjugatedType", "source"],
        &["originalType", "target"],
    ),
    (
        ElementKind::Disjoining,
        &["typeDisjoined", "source"],
        &["disjoiningType", "target"],
    ),
    (
        ElementKind::TypeFeaturing,
        &["featureOfType", "source"],
        &["featuringType", "target"],
    ),
    (
        ElementKind::FeatureChaining,
        &["featureChained", "source"],
        &["chainingFeature", "target"],
    ),
    (
        ElementKind::FeatureInverting,
        &["featureInverted", "source"],
        &["invertingFeature", "target"],
    ),
    (
        ElementKind::Differencing,
        &["typeDifferenced", "source"],
        &["differencingType", "target"],
    ),
    (
        ElementKind::Unioning,
        &["typeUnioned", "source"],
        &["unioningType", "target"],
    ),
    (
        ElementKind::Intersecting,
        &["typeIntersected", "source"],
        &["intersectingType", "target"],
    ),
    (
        ElementKind::Connector,
        &["sourceFeature", "source"],
        &["targetFeature", "target"],
    ),
];

/// Fill in a relationship's ends, and what its metaclass calls them.
///
/// Where the model wrote no end at all, ownership answers: a
/// specialization is owned by the type it specializes, a feature value
/// by the feature it sets, and each owns the far end unless a membership
/// stands between them.
fn fill_ends(
    object: &mut Map<String, Json>,
    kind: ElementKind,
    owner: Option<Json>,
    owned: Vec<Json>,
) {
    let (sources, targets) = ENDS
        .iter()
        .find(|(of, _, _)| kind.is_a(*of))
        .map_or((&[][..], &[][..]), |(_, sources, targets)| {
            (*sources, *targets)
        });
    let mut related = fill_end(object, kind, sources, owner.into_iter().collect());
    for end in fill_end(object, kind, targets, owned) {
        if !related.contains(&end) {
            related.push(end);
        }
    }
    // a connector whose ends have not resolved still says what it
    // relates, under the name that redefines `relatedElement`
    if related.is_empty() {
        if let Some(Json::Array(features)) = object.get("relatedFeature") {
            related = features.clone();
        }
    }
    object.insert("relatedElement".into(), Json::Array(related));
}

/// One end: the first link of the chain the model filled in, or the
/// fallback where it filled in none, written back into every link.
fn fill_end(
    object: &mut Map<String, Json>,
    kind: ElementKind,
    chain: &[&str],
    fallback: Vec<Json>,
) -> Vec<Json> {
    let found = chain.iter().find_map(|name| match object.get(*name) {
        Some(Json::Array(values)) if !values.is_empty() => Some(values.clone()),
        Some(value @ Json::Object(_)) => Some(vec![value.clone()]),
        _ => None,
    });
    let end = found.unwrap_or(fallback);
    for name in chain {
        let Some(meta) = kind.feature(name) else {
            continue;
        };
        let written = if meta.many {
            Json::Array(end.clone())
        } else {
            end.first().cloned().unwrap_or(Json::Null)
        };
        object.insert((*name).into(), written);
    }
    end
}

/// Rebuild a model from interchange JSON. Returns the model and the root
/// elements (those without an owner). Unknown properties are ignored;
/// unknown metaclasses and dangling references are errors.
pub fn from_json(json: &Json) -> Result<(Model, Vec<ElementId>), ImportError> {
    let array = json.as_array().ok_or(ImportError::NotAnArray)?;
    read_elements(&array.iter().map(Element::Built).collect::<Vec<_>>())
}

/// The same, from the text of a document rather than a document.
///
/// Read this way the elements are never all `Json` at once: each is
/// parsed when it is wanted and dropped when it has been read. A
/// document of the standard library is 754 MB of text, and as a
/// `serde_json::Value` it is about six gigabytes -- for a model that is
/// 47 MB when it is built. What stays here is the text, the borrows into
/// it, and the model.
///
/// # Errors
///
/// [`ImportError::NotJson`] where the bytes are not an array of objects,
/// and whatever [`from_json`] would say about the document they spell.
///
/// The count is how many objects the document holds, which is not how
/// many elements the model holds -- a membership that only carries
/// ownership is an object there and an edge here. [`from_json`] does not
/// report it because its caller is holding the array already.
pub fn read_json(bytes: &[u8]) -> Result<(Model, Vec<ElementId>, usize), ImportError> {
    // Text that is not JSON and JSON that is not a document are not the
    // same mistake, and `serde` says which: a shape it could read but
    // did not expect is `Data`, and anything it could not read at all is
    // the syntax. Told apart here, each reads as what it is.
    let array: Vec<&serde_json::value::RawValue> =
        serde_json::from_slice(bytes).map_err(|why| match why.classify() {
            serde_json::error::Category::Data => ImportError::NotAnArray,
            _ => ImportError::NotJson(why.to_string()),
        })?;
    let objects = array.len();
    let (model, roots) = read_elements(&array.into_iter().map(Element::Text).collect::<Vec<_>>())?;
    Ok((model, roots, objects))
}

/// One element of a document: a `Json` already built, or the text of one
/// that is not built yet and need not stay built.
#[derive(Clone, Copy)]
enum Element<'a> {
    Built(&'a Json),
    Text(&'a serde_json::value::RawValue),
}

/// The three things the first pass asks of an element.
///
/// Asked without building the element, which is the point: text that is
/// skipped is not allocated, so a pass that wants two strings does not
/// pay for the forty properties beside them.
#[derive(serde::Deserialize)]
struct Head<'a> {
    #[serde(rename = "@type", borrow, default)]
    kind: Option<&'a str>,
    #[serde(rename = "@id", borrow, default)]
    uuid: Option<&'a str>,
    #[serde(rename = "ownedRelatedElement", default)]
    owned: Option<serde::de::IgnoredAny>,
}

impl<'a> Element<'a> {
    /// What the first pass needs, and no more.
    fn head(&self, at: usize) -> Result<Head<'a>, ImportError> {
        match self {
            Element::Built(json) => Ok(Head {
                kind: json["@type"].as_str(),
                uuid: json["@id"].as_str(),
                owned: json
                    .get("ownedRelatedElement")
                    .map(|_| serde::de::IgnoredAny),
            }),
            Element::Text(raw) => {
                serde_json::from_str(raw.get()).map_err(|_| ImportError::MissingType(at))
            }
        }
    }

    /// The element as a `Json`, borrowed where there is one to borrow.
    fn read(&self, at: usize) -> Result<std::borrow::Cow<'a, Json>, ImportError> {
        match self {
            Element::Built(json) => Ok(std::borrow::Cow::Borrowed(json)),
            Element::Text(raw) => serde_json::from_str(raw.get())
                .map(std::borrow::Cow::Owned)
                .map_err(|_| ImportError::MissingType(at)),
        }
    }
}

fn read_elements(array: &[Element<'_>]) -> Result<(Model, Vec<ElementId>), ImportError> {
    let mut model = Model::new();
    let mut by_uuid: HashMap<String, ElementId> = HashMap::new();
    // a plain `OwningMembership` only carries ownership, which the model
    // holds directly: it becomes an edge rather than an element, whichever
    // tool wrote it
    // Kept as they arrived rather than as they read. Pass two asks each
    // of these what it brings in, once, so holding the read of it until
    // then holds a third of the document in `Json` for nothing.
    let mut bridges: HashMap<String, Element<'_>> = HashMap::new();

    // pass 1: create all elements
    let mut ids = Vec::new();
    let mut created = Vec::new();
    for (index, source) in array.iter().enumerate() {
        // Three fields, not forty. Only a bridge is built here, because
        // pass two asks it what it brings in; every other element is
        // built there instead, one at a time.
        let head = source.head(index)?;
        let type_name = head.kind.ok_or(ImportError::MissingType(index))?;
        let kind = ElementKind::from_name(type_name)
            .ok_or_else(|| ImportError::UnknownType(type_name.to_string()))?;
        if kind.is_abstract() {
            return Err(ImportError::AbstractType(type_name.to_string()));
        }
        let uuid = head.uuid.ok_or(ImportError::MissingId(index))?.to_string();
        if by_uuid.contains_key(&uuid) || bridges.contains_key(&uuid) {
            return Err(ImportError::DuplicateId(uuid));
        }
        if FOLDED.contains(&kind) && head.owned.is_some() {
            bridges.insert(uuid, *source);
            continue;
        }
        let id = model.create(kind);
        by_uuid.insert(uuid, id);
        ids.push(id);
        created.push((index, source));
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
        let Some(source) = bridges.get(uuid) else {
            return Ok(vec![resolve(value)?]);
        };
        let bridge = source.read(0)?;
        let bridge = &bridge;
        let members: Vec<ElementId> = bridge["ownedRelatedElement"]
            .as_array()
            .into_iter()
            .flatten()
            .map(resolve)
            .collect::<Result<_, _>>()?;
        for &member in &members {
            match bridge["visibility"].as_str() {
                Some("private") => model.set_member_visibility(member, Vis::Private),
                Some("protected") => model.set_member_visibility(member, Vis::Protected),
                _ => {}
            }
            if let Some(role) = folded_role(bridge) {
                model.set_member_role(member, role);
            }
        }
        Ok(members)
    };

    // Pass 2: properties, and the ownership each element states. Ownership
    // may be written as the derived `ownedElement`, as memberships, or as a
    // relationship's own related elements -- often all at once, so each pair
    // counts once, in the order it is first given. None of it is built until
    // all of it is known to be a tree: `Model::add_owned` refuses a cycle
    // rather than let every later walk run for ever, and foreign JSON is
    // where a cycle would come from.
    let mut stated = std::collections::HashSet::new();
    let mut edges: Vec<(ElementId, ElementId)> = Vec::new();
    for (&(index, source), id) in created.iter().zip(&ids) {
        let kind = model.kind(*id);
        let object = source.read(index)?;
        let object = object.as_object().expect("validated in pass 1");
        let mut own = |child: ElementId| {
            if stated.insert((*id, child)) {
                edges.push((*id, child));
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
                | "owningRelatedElement"
                // a relationship's ends are read from the properties
                // its own metaclass names them by; these general
                // spellings say the same thing, and one of them may
                // name a membership that folded into an edge
                | "source"
                | "target"
                | "importedElement" => continue,
                "ownedElement" | "ownedRelatedElement" => {
                    for child in value.as_array().into_iter().flatten() {
                        own(resolve(child)?);
                    }
                }
                "ownedRelationship" => {
                    for related in value.as_array().into_iter().flatten() {
                        for child in through(&mut model, related)? {
                            own(child);
                        }
                    }
                }
                _ => {
                    let Some(meta) = kind.feature(key) else {
                        continue; // tolerate foreign properties
                    };
                    // what the exporter derives is not read back, except
                    // for the handful of derived properties the model
                    // stores itself
                    if meta.derived && !STORED_DERIVED.contains(&key.as_str()) {
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
                        // An import says how far what it brings in
                        // travels, and the model keeps that where it
                        // keeps every membership-borne fact -- beside
                        // the element rather than among its properties,
                        // which is where the exporter reads it from.
                        match (key.as_str(), &converted) {
                            ("visibility", Value::EnumLit(written))
                                if kind.is_a(ElementKind::Import) =>
                            {
                                if let Some(visibility) = Vis::written(written) {
                                    model.set_member_visibility(*id, visibility);
                                }
                            }
                            _ => {
                                model.set(*id, key, converted);
                            }
                        }
                    }
                }
            }
        }
    }

    let spelled: HashMap<ElementId, &str> = by_uuid
        .iter()
        .map(|(uuid, id)| (*id, uuid.as_str()))
        .collect();
    ownership_is_a_tree(&edges, &spelled)?;
    for (owner, child) in edges {
        model.add_owned(owner, child);
    }

    let roots = ids
        .iter()
        .copied()
        .filter(|id| model.owner(*id).is_none())
        .collect();
    Ok((model, roots))
}

/// Is the ownership the JSON stated a tree? Every element owned at most
/// once, and none among its own owners.
fn ownership_is_a_tree(
    edges: &[(ElementId, ElementId)],
    spelled: &HashMap<ElementId, &str>,
) -> Result<(), ImportError> {
    let named = |id: ElementId| spelled[&id].to_string();
    let mut owner: HashMap<ElementId, ElementId> = HashMap::new();
    for &(parent, child) in edges {
        if owner.get(&child).is_some_and(|&first| first != parent) {
            return Err(ImportError::SharedOwnership(named(child)));
        }
        owner.insert(child, parent);
    }
    // climbing from each element to its root settles every element on
    // the way, so the whole web is walked once
    let mut settled: std::collections::HashSet<ElementId> = std::collections::HashSet::new();
    for &(_, start) in edges {
        let mut climbed = Vec::new();
        let mut on_path = std::collections::HashSet::new();
        let mut at = Some(start);
        while let Some(node) = at {
            if settled.contains(&node) {
                break;
            }
            if !on_path.insert(node) {
                return Err(ImportError::OwnershipCycle(named(node)));
            }
            climbed.push(node);
            at = owner.get(&node).copied();
        }
        settled.extend(climbed);
    }
    Ok(())
}

/// The `'static` text the metamodel spells an enumeration literal with.
///
/// A model holds a literal as one of those and its readers match on it;
/// a `Value::String` of the same text would print the same and never
/// compare equal to one. A string naming no literal of the enumeration
/// is not one, and nothing is stored for it.
fn enum_literal(ty: sysml_model::EnumType, text: &str) -> Option<&'static str> {
    use sysml_model::generated::*;
    use sysml_model::EnumType;
    Some(match ty {
        EnumType::FeatureDirectionKind => FeatureDirectionKind::from_literal(text)?.literal(),
        EnumType::PortionKind => PortionKind::from_literal(text)?.literal(),
        EnumType::RequirementConstraintKind => {
            RequirementConstraintKind::from_literal(text)?.literal()
        }
        EnumType::StateSubactionKind => StateSubactionKind::from_literal(text)?.literal(),
        EnumType::TransitionFeatureKind => TransitionFeatureKind::from_literal(text)?.literal(),
        EnumType::TriggerKind => TriggerKind::from_literal(text)?.literal(),
        EnumType::VisibilityKind => VisibilityKind::from_literal(text)?.literal(),
    })
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
        (FeatureType::Data(PrimitiveType::Real), Json::String(text)) => {
            text.parse::<f64>().ok().map(Value::Real)
        }
        (FeatureType::Data(_), Json::Number(n)) => n
            .as_i64()
            .map(Value::Int)
            .or_else(|| n.as_f64().map(Value::Real)),
        (FeatureType::Data(_), Json::String(s)) => Some(Value::String(s.clone())),
        (FeatureType::Enumeration(ty), Json::String(text)) => {
            enum_literal(ty, text).map(Value::EnumLit)
        }
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
    fn a_root_keeps_its_uuid_whatever_order_the_files_came_in() {
        let exported = |first: &str, second: &str| -> std::collections::BTreeMap<String, String> {
            let mut ws = sysml_semantics::Workspace::new();
            ws.add_file("first.sysml", first);
            ws.add_file("second.sysml", second);
            ws.resolve_all();
            to_json(ws.model())
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|object| {
                    Some((
                        object["qualifiedName"].as_str()?.to_string(),
                        object["@id"].as_str()?.to_string(),
                    ))
                })
                .collect()
        };
        let a = "package A {\n\tpart def X;\n}\n";
        let b = "package B {\n\tpart def Y;\n}\n";
        let one_way = exported(a, b);
        assert!(one_way.contains_key("A::X"));
        assert_eq!(one_way, exported(b, a), "the load order moved the UUIDs");
    }

    #[test]
    fn two_roots_of_one_name_are_still_told_apart() {
        let mut model = Model::new();
        for _ in 0..2 {
            let root = model.create(ElementKind::Package);
            model.set(root, "declaredName", Value::String("P".to_string()));
        }
        let roots: Vec<ElementId> = model.ids().collect();
        assert_ne!(
            element_uuid(&model, roots[0]),
            element_uuid(&model, roots[1])
        );
        let json = to_json(&model);
        let ids: std::collections::HashSet<&str> = json
            .as_array()
            .unwrap()
            .iter()
            .map(|object| object["@id"].as_str().unwrap())
            .collect();
        assert_eq!(ids.len(), 2);
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
            (ImportError::AbstractType("X".into()), "abstract"),
            (ImportError::MissingId(1), "no \"@id\""),
            (ImportError::DuplicateId("u".into()), "share the id"),
            (ImportError::UnknownReference("u".into()), "unknown element"),
            (ImportError::SharedOwnership("u".into()), "two elements own"),
            (ImportError::OwnershipCycle("u".into()), "owned by itself"),
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
        // a stored list of references: what a dependency relates
        let d = model.create(ElementKind::Dependency);
        model.add_owned(root, d);
        model.set(d, "client", Value::RefList(vec![a, b]));

        let json = to_json(&model);
        let (rebuilt, roots) = from_json(&json).unwrap();
        assert_eq!(roots.len(), 1);
        let ra = rebuilt.owned(roots[0])[0];
        let rb = rebuilt.owned(roots[0])[1];
        let rm = rebuilt.owned(roots[0])[2];
        assert_eq!(rebuilt.maybe(ra, "value"), Some(&Value::Int(42)));
        assert_eq!(rebuilt.maybe(rb, "value"), Some(&Value::Real(2.5)));
        // enum literals come back as strings, and an import's
        // visibility comes back beside the element rather than among
        // its properties -- which is where the exporter reads it from,
        // so it is what makes the round trip close
        assert_eq!(rebuilt.member_visibility(rm), Some(Vis::Private));
        assert_eq!(rebuilt.get(rm, "isImportAll"), Some(&Value::Bool(true)));
        assert_eq!(rebuilt.get(rm, "importedMembership"), Some(&Value::Ref(ra)));
        let rd = rebuilt.owned(roots[0])[3];
        assert_eq!(
            rebuilt.get(rd, "client"),
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
        assert_eq!(rebuilt.member_visibility(hidden), Some(Vis::Private));
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
        // A declared parameter is written as a `TypeBodyElement`, so its
        // membership is an ordinary feature membership. Only what an invocation
        // hands over is a `ParameterMembership`, whose `parameterDirection()`
        // must be `in`; read the other way round, `out y` would be a parameter
        // whose direction is not what its own membership requires.
        assert_eq!(membership_of("x"), "FeatureMembership");
        assert_eq!(membership_of("y"), "FeatureMembership");
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
    fn a_feature_outside_a_transition_has_no_transition_role() {
        let (_, model) = resolved("part def P {\n\tattribute a;\n}\n");
        let attribute = model.ids().find(|&id| model.name(id) == Some("a")).unwrap();
        assert_eq!(sysml_model::transition_role(&model, attribute), None);
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
    fn an_import_that_filters_keeps_the_filter_out_of_its_ends() {
        // `import Q::*[@Safety][@Approved];`: each bracket is an
        // `ElementFilterMembership` the import owns, and neither is a
        // namespace the import brings in. Written as ends they came
        // back as the imported namespace instead, and the second export
        // disagreed with the first.
        let mut model = Model::new();
        let package = model.create(ElementKind::Package);
        model.set(package, "declaredName", Value::String("P".to_string()));
        let import = model.create(ElementKind::NamespaceImport);
        model.add_owned(package, import);
        for _ in 0..2 {
            let filter = model.create(ElementKind::ElementFilterMembership);
            model.add_owned(import, filter);
            let condition = model.create(ElementKind::Expression);
            model.add_owned(filter, condition);
        }

        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|object| object["@type"] == name)
                .unwrap()
        };
        let written = of_type("NamespaceImport");
        assert_eq!(written["ownedRelationship"].as_array().unwrap().len(), 2);
        assert_eq!(written["ownedRelatedElement"], json!([]));
        assert_eq!(written["importedNamespace"], Json::Null);
        assert_eq!(written["relatedElement"].as_array().unwrap().len(), 1);
        // the filter still relates the expression that says what it keeps
        let filter = of_type("ElementFilterMembership");
        assert_eq!(filter["ownedRelatedElement"].as_array().unwrap().len(), 1);

        let (rebuilt, roots) = from_json(&json).unwrap();
        assert_eq!(rebuilt.len(), model.len());
        let import = rebuilt.owned(roots[0])[0];
        assert_eq!(rebuilt.owned(import).len(), 2, "both filters came back");
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_render_member_is_a_view_rendering_membership() {
        let (_, model) = resolved(
            "rendering def R;\nrendering asTree : R;\nview def V {\n\trender asTree;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let membership = objects
            .iter()
            .find(|object| object["@type"] == "ViewRenderingMembership")
            .expect("`render` names the membership the standard gives it");
        // the membership owns the rendering the view is drawn with, and
        // names again what that rendering refers to
        let rendering = objects
            .iter()
            .find(|object| object["@id"] == membership["ownedRendering"]["@id"])
            .unwrap();
        assert_eq!(rendering["@type"], "RenderingUsage");
        let declared = objects
            .iter()
            .find(|object| {
                object["declaredName"] == "asTree" && object["@type"] == "RenderingUsage"
            })
            .unwrap();
        assert_eq!(membership["referencedRendering"]["@id"], declared["@id"]);
        assert_eq!(membership["memberName"], "asTree");

        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(rebuilt.len(), model.len(), "nothing was owned twice");
        assert_eq!(to_json(&rebuilt), json, "the rendering role survived");
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
        // and one owned by nothing at all, which has no end there either
        let orphan = model.create(ElementKind::FeatureTyping);

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
        let loose = json
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["@id"] == element_uuid(&model, orphan).to_string())
            .unwrap();
        assert_eq!(loose["owningFeature"], Json::Null);
        assert_eq!(loose["source"], json!([]));
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
    fn an_alias_states_the_name_it_gives() {
        let (_, model) = resolved("package P {\n\tpart def A;\n\talias Q for A;\n}\n");
        let json = to_json(&model);
        let alias = json
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["@type"] == "Membership")
            .expect("the alias was reified");
        assert_eq!(alias["declaredName"], "Q");
        assert_eq!(alias["memberName"], "Q");
        assert_eq!(alias["memberShortName"], Json::Null);
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
        assert_eq!(model.effective_name(a), None);
        assert_eq!(model.effective_short_name(a), None);
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
        // a keyword written plainly would read back as the keyword
        assert_eq!(quoted("part"), "'part'");
        assert_eq!(quoted("it's"), "'it\\'s'");
        // whatever the quotes must not end early is escaped again, so
        // that unquoting the result gives the name back
        for name in [
            "it's", "a\\b", "a\nb", "a\rb", "a\tb", "a\u{8}b", "a\u{c}b", "wheel 1",
        ] {
            assert_eq!(sysml_syntax::unquote(&quoted(name)), name, "{name:?}");
        }
    }

    #[test]
    fn a_qualified_name_of_keywords_reads_back() {
        let (_, model) = resolved("package 'part' {\n\tpart def 'in';\n}\n");
        let json = to_json(&model);
        let qualified = json
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["@type"] == "PartDefinition")
            .unwrap()["qualifiedName"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(qualified, "'part'::'in'");
        // and this toolchain's own parser reads that name back
        let parse = sysml_syntax::parse(&format!("part def Copy :> {qualified};\n"));
        assert!(parse.ok(), "{:?}", parse.errors());
    }

    #[test]
    fn the_uuid_an_element_is_exported_under_is_the_published_one() {
        // the walk down the whole model and the walk up from one element
        // build the same path, and must go on agreeing
        let model = example_model();
        let json = to_json(&model);
        for (id, object) in model.ids().zip(json.as_array().unwrap()) {
            assert_eq!(object["@id"], element_uuid(&model, id).to_string());
        }
    }

    #[test]
    fn a_deep_model_is_exported_in_one_walk_of_it() {
        // every UUID used to be computed by walking up to the root, and
        // every membership reference to compute one again: a chain this
        // deep took a minute to export
        let mut model = Model::new();
        let mut owner = model.create(ElementKind::Package);
        for step in 0..2_000 {
            let child = model.create(ElementKind::PartDefinition);
            model.set(child, "declaredName", Value::String(format!("P{step}")));
            model.add_owned(owner, child);
            owner = child;
        }
        let started = std::time::Instant::now();
        let json = to_json(&model);
        let took = started.elapsed();
        assert_eq!(json.as_array().unwrap().len(), 2_001 + 2_000);
        assert!(took < std::time::Duration::from_secs(5), "took {took:?}");
    }

    #[test]
    fn a_cycle_in_imported_ownership_is_refused_rather_than_built() {
        // building it would panic in `add_owned`, and every walk of the
        // ownership web would run for ever if it did not
        let round = json!([
            { "@type": "Package", "@id": "a", "ownedElement": [{ "@id": "b" }] },
            { "@type": "Package", "@id": "b", "ownedElement": [{ "@id": "a" }] },
        ]);
        assert!(matches!(
            from_json(&round),
            Err(ImportError::OwnershipCycle(_))
        ));
        let itself = json!([
            { "@type": "Package", "@id": "a", "ownedElement": [{ "@id": "a" }] },
        ]);
        assert!(matches!(
            from_json(&itself),
            Err(ImportError::OwnershipCycle(_))
        ));
    }

    #[test]
    fn an_element_two_owners_claim_is_refused() {
        let json = json!([
            { "@type": "Package", "@id": "a", "ownedElement": [{ "@id": "c" }] },
            { "@type": "Package", "@id": "b", "ownedElement": [{ "@id": "c" }] },
            { "@type": "Package", "@id": "c" },
        ]);
        assert!(matches!(
            from_json(&json),
            Err(ImportError::SharedOwnership(_))
        ));
    }

    #[test]
    fn a_repeated_id_and_an_abstract_metaclass_are_refused() {
        let twice = json!([
            { "@type": "Package", "@id": "a" },
            { "@type": "Package", "@id": "a" },
        ]);
        assert!(matches!(
            from_json(&twice),
            Err(ImportError::DuplicateId(_))
        ));
        // a membership folded into an edge is claimed by its id too
        let after_a_bridge = json!([
            { "@type": "OwningMembership", "@id": "m",
              "ownedRelatedElement": [{ "@id": "a" }] },
            { "@type": "Package", "@id": "m" },
        ]);
        assert!(matches!(
            from_json(&after_a_bridge),
            Err(ImportError::DuplicateId(_))
        ));
        // and no model holds an element of an abstract metaclass
        let abstract_type = json!([{ "@type": "Relationship", "@id": "r" }]);
        assert!(matches!(
            from_json(&abstract_type),
            Err(ImportError::AbstractType(_))
        ));
    }

    #[test]
    fn a_connector_is_a_feature_of_the_type_that_declares_it() {
        let (_, model) = resolved("part def P {\n\tpart a;\n\tpart b;\n\tconnect a to b;\n}\n");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|object| object["@type"] == name)
                .unwrap_or_else(|| panic!("no {name} was written"))
        };
        let part = of_type("PartDefinition");
        let connector = of_type("ConnectionUsage");
        let ids = |value: &Json| -> Vec<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|reference| reference["@id"].as_str().unwrap().to_string())
                .collect()
        };
        let bridge_id = connector["owningMembership"]["@id"]
            .as_str()
            .expect("the connector is owned through a membership")
            .to_string();
        let bridge = objects
            .iter()
            .find(|object| object["@id"] == bridge_id.as_str())
            .unwrap();
        assert_eq!(bridge["@type"], "FeatureMembership");
        assert_eq!(bridge["owningRelatedElement"]["@id"], part["@id"]);
        assert_eq!(bridge["ownedRelatedElement"][0]["@id"], connector["@id"]);
        // the type reaches the connector as one of its features, and
        // owns the membership rather than the connector itself
        let connector_id = connector["@id"].as_str().unwrap().to_string();
        assert!(ids(&part["ownedFeature"]).contains(&connector_id));
        assert!(ids(&part["feature"]).contains(&connector_id));
        assert!(ids(&part["ownedRelationship"]).contains(&bridge_id));
        assert!(!ids(&part["ownedRelationship"]).contains(&connector_id));
        assert_eq!(connector["owningRelatedElement"], Json::Null);
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_succession_relates_the_features_it_was_resolved_to() {
        let (_, model) =
            resolved("action def A {\n\taction a;\n\taction b;\n\tsuccession first a then b;\n}\n");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let succession = objects
            .iter()
            .find(|object| object["@type"] == "SuccessionAsUsage")
            .expect("the succession was reified");
        // its ends never resolved to a source and a target of their own,
        // but it still says what it relates
        assert_eq!(succession["relatedElement"], succession["relatedFeature"]);
        assert_eq!(succession["relatedElement"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_dependency_is_a_member_of_the_package_that_declares_it() {
        let (_, model) =
            resolved("package P {\n\tpart def A;\n\tpart def B;\n\tdependency from A to B;\n}\n");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        let dependency = objects
            .iter()
            .find(|object| object["@type"] == "Dependency")
            .expect("the dependency was reified");
        let bridge_id = dependency["owningMembership"]["@id"]
            .as_str()
            .expect("a dependency is a member like any other");
        let bridge = objects
            .iter()
            .find(|object| object["@id"] == bridge_id)
            .unwrap();
        assert_eq!(bridge["@type"], "OwningMembership");
        assert_eq!(bridge["owningRelatedElement"]["@id"], of("P")["@id"]);
        // client and supplier are the source and the target
        assert_eq!(dependency["client"][0]["@id"], of("A")["@id"]);
        assert_eq!(dependency["supplier"][0]["@id"], of("B")["@id"]);
        assert_eq!(dependency["source"], dependency["client"]);
        assert_eq!(dependency["target"], dependency["supplier"]);
        assert_eq!(dependency["relatedElement"][0]["@id"], of("A")["@id"]);
        assert_eq!(dependency["relatedElement"][1]["@id"], of("B")["@id"]);
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_reified_relationship_names_the_elements_it_relates() {
        let (_, model) = resolved(
            "attribute def R;\npart def A;\npart def B :> A {\n\tattribute mass : R;\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|object| object["@type"] == name)
                .unwrap()
        };
        // nothing wrote the specific end of a subclassification; the
        // type that owns it is that end
        let subclassification = of_type("Subclassification");
        assert_eq!(subclassification["subclassifier"]["@id"], of("B")["@id"]);
        assert_eq!(subclassification["superclassifier"]["@id"], of("A")["@id"]);
        assert_eq!(
            subclassification["specific"],
            subclassification["subclassifier"]
        );
        assert_eq!(
            subclassification["general"],
            subclassification["superclassifier"]
        );
        assert_eq!(subclassification["source"][0]["@id"], of("B")["@id"]);
        assert_eq!(subclassification["target"][0]["@id"], of("A")["@id"]);
        assert_eq!(
            subclassification["relatedElement"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(subclassification["owningClassifier"]["@id"], of("B")["@id"]);
        // a typing writes its ends itself, and the names it redefines
        // say the same thing
        let typing = of_type("FeatureTyping");
        assert_eq!(typing["typedFeature"]["@id"], of("mass")["@id"]);
        assert_eq!(typing["type"]["@id"], of("R")["@id"]);
        assert_eq!(typing["specific"], typing["typedFeature"]);
        assert_eq!(typing["general"], typing["type"]);
        assert_eq!(typing["owningFeature"]["@id"], of("mass")["@id"]);
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_plain_import_brings_what_it_imports_no_further() {
        let (_, model) = resolved(
            "package A;\npackage B {\n\timport A::*;\n}\n\
             package C {\n\tpublic import A::*;\n}\n\
             package D {\n\tprotected import A::*;\n}\n",
        );
        let json = to_json(&model);
        let visibilities: Vec<&str> = json
            .as_array()
            .unwrap()
            .iter()
            .filter(|object| object["@type"] == "NamespaceImport")
            .map(|object| object["visibility"].as_str().unwrap())
            .collect();
        assert_eq!(visibilities, ["private", "public", "protected"]);
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    #[test]
    fn a_name_changed_after_an_import_is_the_name_exported() {
        let model = example_model();
        let (mut rebuilt, roots) = from_json(&to_json(&model)).unwrap();
        // the derived name arrived as a property of its own before, and
        // shadowed the declared name from then on
        assert_eq!(rebuilt.get(roots[0], "name"), None);
        rebuilt.set(roots[0], "declaredName", Value::String("Q".to_string()));
        let json = to_json(&rebuilt);
        assert_eq!(json[0]["name"], "Q");
        assert_eq!(json[0]["qualifiedName"], "Q");
    }

    #[test]
    fn a_comment_is_about_what_it_named() {
        let (_, model) = resolved(
            "package P {\n\tpart def A;\n\tpart def B;\n\tcomment about A, B /* both */\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        let comment = objects
            .iter()
            .find(|object| object["@type"] == "Comment")
            .expect("the comment was reified");
        let about: Vec<&str> = comment["annotatedElement"]
            .as_array()
            .unwrap()
            .iter()
            .map(|reference| reference["@id"].as_str().unwrap())
            .collect();
        assert_eq!(
            about,
            [
                of("A")["@id"].as_str().unwrap(),
                of("B")["@id"].as_str().unwrap()
            ]
        );
        assert_eq!(comment["annotation"].as_array().unwrap().len(), 2);
        assert_eq!(
            comment["annotation"],
            comment["ownedAnnotatingRelationship"]
        );
        // each annotation relates the comment to what it named
        let annotation = objects
            .iter()
            .find(|object| object["@type"] == "Annotation")
            .unwrap();
        assert_eq!(annotation["source"][0]["@id"], comment["@id"]);
        assert_eq!(annotation["target"][0]["@id"], of("A")["@id"]);
        assert_eq!(annotation["annotatingElement"]["@id"], comment["@id"]);
    }

    #[test]
    fn a_folded_membership_names_its_member_the_way_its_metaclass_does() {
        let (_, model) = resolved(
            "action def Act;\nstate def S {\n\tentry action a : Act;\n}\n\
             constraint def C {\n\ttrue\n}\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|object| object["@type"] == name)
                .unwrap_or_else(|| panic!("no {name} was written"))
        };
        let subaction = of_type("StateSubactionMembership");
        assert_eq!(
            subaction["action"]["@id"],
            subaction["ownedRelatedElement"][0]["@id"]
        );
        let result = of_type("ResultExpressionMembership");
        assert_eq!(
            result["ownedResultExpression"]["@id"],
            result["ownedRelatedElement"][0]["@id"]
        );
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
    fn an_enumeration_reads_back_as_the_literal_the_metamodel_spells() {
        // a model holds a literal as the metamodel's own text, which is
        // what its readers match on
        let json = json!([
            { "@type": "AttributeUsage", "@id": "a", "direction": "in" },
            { "@type": "OccurrenceUsage", "@id": "b", "portionKind": "snapshot" },
            { "@type": "NamespaceImport", "@id": "c", "visibility": "protected" },
            { "@type": "RequirementConstraintMembership", "@id": "d", "kind": "assumption" },
            { "@type": "StateSubactionMembership", "@id": "e", "kind": "do" },
            { "@type": "TransitionFeatureMembership", "@id": "f", "kind": "guard" },
            { "@type": "TriggerInvocationExpression", "@id": "g", "kind": "after" },
            { "@type": "AttributeUsage", "@id": "h", "direction": "sideways" },
        ]);
        let (model, roots) = from_json(&json).unwrap();
        let literals: Vec<Option<&Value>> = [
            "direction",
            "portionKind",
            "visibility",
            "kind",
            "kind",
            "kind",
            "kind",
            "direction",
        ]
        .iter()
        .zip(&roots)
        .map(|(property, &id)| model.get(id, property))
        .collect();
        assert_eq!(model.member_visibility(roots[2]), Some(Vis::Protected));
        assert_eq!(
            literals,
            [
                Some(&Value::EnumLit("in")),
                Some(&Value::EnumLit("snapshot")),
                // an import says how far what it brings in travels, and
                // the model keeps that beside the element rather than
                // among its properties
                None,
                Some(&Value::EnumLit("assumption")),
                Some(&Value::EnumLit("do")),
                Some(&Value::EnumLit("guard")),
                Some(&Value::EnumLit("after")),
                // no literal of the enumeration is spelled that way
                None,
            ]
        );
    }

    #[test]
    fn a_number_json_cannot_hold_is_written_as_text() {
        let mut model = Model::new();
        let literal = model.create(ElementKind::LiteralRational);
        model.set(literal, "value", Value::Real(f64::INFINITY));
        let json = to_json(&model);
        assert_eq!(json[0]["value"], "inf");
        let (rebuilt, roots) = from_json(&json).unwrap();
        assert_eq!(
            rebuilt.maybe(roots[0], "value"),
            Some(&Value::Real(f64::INFINITY))
        );
        assert_eq!(to_json(&rebuilt), json);
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
