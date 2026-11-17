//! Standard JSON interchange for [`sysml_model::Model`].
//!
//! Follows the serialization style of the SysML v2 API & Services
//! standard: every element is a JSON object with `"@type"`, `"@id"` and
//! its properties, where element references are `{"@id": ...}` objects.
//! Element UUIDs are deterministic -- UUIDv5 over the ownership path,
//! where a name places an element among its siblings, and what an unnamed
//! top-level member says places it among the others every file hangs off
//! the root namespace -- so the same model exports as the same JSON
//! however its files were ordered on the way in.
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
//! and those `Writing::derived` computes are emitted at their defaults.

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
    /// An element at this position that is not a JSON object.
    NotAnObject(usize),
    /// An element at this position that is an object but cannot be read
    /// as one: a number no JSON reader can hold (`1e999`), or an `@id` or
    /// `@type` that is not a string. Said as what it is; reported as a
    /// missing `@type`, it would send a reader looking for something that was
    /// there all along.
    Unreadable(usize, String),
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
            ImportError::NotAnObject(i) => write!(f, "element {i} is not a JSON object"),
            ImportError::Unreadable(i, why) => write!(f, "element {i} cannot be read: {why}"),
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
///
/// What a resolver alone knows about an unnamed top-level member -- what
/// an `import` there imports -- is not in the model, so for those this
/// can disagree with the document [`to_json_with`] writes; ask
/// [`element_uuid_with`] with the same [`Extras`] for the published one.
pub fn element_uuid(model: &Model, id: ElementId) -> Uuid {
    element_uuid_with(model, &Extras::default(), id)
}

/// [`element_uuid`], knowing what the resolver knows.
pub fn element_uuid_with(model: &Model, extras: &Extras, id: ElementId) -> Uuid {
    let mut segments = Vec::new();
    let mut current = Some(id);
    while let Some(elem) = current {
        let key = sibling_key(model, extras, elem);
        let index = match model.owner(elem) {
            Some(owner) => namesake_index(model, extras, model.owned(owner), elem, &key),
            None => namesake_index(model, extras, &roots(model), elem, &key),
        };
        segments.push(path_segment(&key, index));
        current = model.owner(elem);
    }
    segments.reverse();
    uuid_of_path(&format!("sysml-v2-rs:{}", segments.join("/")))
}

/// The UUID every element of `model` is published under, and the UUID of
/// the membership a document stands between each element and its owner
/// where it stands one -- worked out in the one walk down the ownership
/// tree that [`to_json_with`] makes.
///
/// What a reader needs to say what a document refers to and does not
/// carry. A document exported without its library names the library's
/// elements by these, and whoever holds the same library computes the
/// same ones, so an `@id` that is nowhere in the document can still be
/// told to be `ISQBase::MassValue`. Asking [`element_uuid_with`] of every
/// element in turn would cost the square of the model's depth.
pub fn published_uuids(
    model: &Model,
    extras: &Extras,
) -> (HashMap<ElementId, Uuid>, HashMap<ElementId, Uuid>) {
    let Identities { uuids, bridges, .. } = identities(model, extras);
    (uuids, bridges)
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
fn namesake_index(
    model: &Model,
    extras: &Extras,
    siblings: &[ElementId],
    id: ElementId,
    key: &SiblingKey<'_>,
) -> usize {
    siblings
        .iter()
        .take_while(|&&sibling| sibling != id)
        .filter(|&&sibling| sibling_key(model, extras, sibling) == *key)
        .count()
}

/// What tells an element apart from its siblings, short of its position:
/// its name, or -- for an unnamed member of the root namespace -- what it
/// says.
///
/// Every file's top-level declarations hang off that one namespace, and a
/// name only places a named one. The unnamed ones all answered to
/// nothing, so they were counted among each other in load order: two
/// files each opening with a `private import` swapped the two imports'
/// UUIDs when they were exported the other way round. Keyed by their
/// content, each is counted only among siblings that say the same thing,
/// which reordering files cannot change. Deeper down, an element's
/// siblings all come from the one declaration that owns them, in the
/// order it was written, and nothing changes there.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum SiblingKey<'m> {
    Name(&'m str),
    Content(String),
    Nothing,
}

fn sibling_key<'m>(model: &'m Model, extras: &Extras, id: ElementId) -> SiblingKey<'m> {
    if let Some(name) = model.name(id) {
        return SiblingKey::Name(name);
    }
    // the namespace a workspace hangs its files off has no owner and no
    // name; a package that is a root of a document is one declaration
    let top_level = model
        .owner(id)
        .is_some_and(|owner| model.owner(owner).is_none() && model.name(owner).is_none());
    if !top_level {
        return SiblingKey::Nothing;
    }
    let mut text = String::new();
    describe(model, extras, id, &mut text);
    for &child in model.owned(id) {
        text.push('{');
        describe(model, extras, child, &mut text);
        text.push('}');
    }
    SiblingKey::Content(uuid_of_path(&text).simple().to_string())
}

/// One element's own content as text: its metaclass, how it is a member,
/// its name, what it stores, and -- for an import -- what it imports.
/// References are spelled as the names of what they reach, which are the
/// same however the files were ordered.
///
/// Each fact is read the way the document writes it rather than the way
/// the model happens to hold it, so that a model read back from the
/// document describes its elements as the one it was written from did:
/// an import's visibility may be a property or beside the element, and
/// what it imports the resolver's to say or a property.
fn describe(model: &Model, extras: &Extras, id: ElementId, out: &mut String) {
    use std::fmt::Write;
    let stored_visibility = match model.maybe(id, "visibility") {
        Some(Value::EnumLit(written)) => Vis::written(written),
        _ => None,
    };
    let _ = write!(
        out,
        "{} {:?} {:?} {:?};",
        model.kind(id).name(),
        model
            .member_visibility(id)
            .or(stored_visibility)
            .map(Vis::keyword),
        model.member_role(id),
        model.name(id),
    );
    // and a property at the value the document writes when nothing is
    // stored reads back stored: it says nothing either way
    let kind = model.kind(id);
    let mut props: Vec<(&str, &Value)> = model
        .props(id)
        .filter(|(name, _)| {
            !matches!(
                *name,
                "visibility" | "importedMembership" | "importedNamespace"
            )
        })
        .filter(|(name, value)| match (kind.feature(name), value) {
            (Some(meta), Value::Bool(b)) => *b != meta.default.unwrap_or(false),
            (_, Value::RefList(targets)) => !targets.is_empty(),
            _ => true,
        })
        .collect();
    props.sort_by_key(|(name, _)| *name);
    for (name, value) in props {
        let _ = write!(out, "{name}=");
        match value {
            Value::Bool(b) => {
                let _ = write!(out, "{b}");
            }
            Value::Int(i) => {
                let _ = write!(out, "{i}");
            }
            Value::Real(r) => {
                let _ = write!(out, "{r:?}");
            }
            Value::String(text) => {
                let _ = write!(out, "{text:?}");
            }
            Value::EnumLit(lit) => out.push_str(lit),
            Value::Ref(target) => spell_reference(model, *target, out),
            Value::RefList(targets) => {
                for &target in targets {
                    spell_reference(model, target, out);
                    out.push(',');
                }
            }
        }
        out.push(';');
    }
    let imports = extras.import_targets.get(&id).copied().or_else(|| {
        ["importedMembership", "importedNamespace"]
            .iter()
            .find_map(|prop| model.maybe(id, prop).and_then(Value::as_id))
    });
    if let Some(target) = imports {
        out.push_str("imports=");
        spell_reference(model, target, out);
        out.push(';');
    }
}

/// An element as the chain of names down to it, an unnamed step spelled
/// as its metaclass.
fn spell_reference(model: &Model, target: ElementId, out: &mut String) {
    let mut chain = Vec::new();
    let mut at = Some(target);
    while let Some(step) = at {
        // the root namespace every file hangs off is no step of a name
        if model.owner(step).is_some() || model.name(step).is_some() {
            chain.push(match model.effective_name(step) {
                Some(name) => format!("{name:?}"),
                None => model.kind(step).name().to_string(),
            });
        }
        at = model.owner(step);
    }
    chain.reverse();
    out.push_str(&chain.join("::"));
}

/// The elements no other element owns.
fn roots(model: &Model) -> Vec<ElementId> {
    model
        .ids()
        .filter(|&id| model.owner(id).is_none())
        .collect()
}

/// One step of the path a UUID is built from: where the element sits
/// among its owner's children that answer to the same key, and that key.
fn path_segment(key: &SiblingKey<'_>, index: usize) -> String {
    match key {
        SiblingKey::Name(name) => format!("{index}:{}", unmistakable(name)),
        SiblingKey::Content(digest) => format!("{index}~{digest}"),
        SiblingKey::Nothing => format!("{index}"),
    }
}

/// A name as a path segment spells it, so that no name can be mistaken
/// for the segments around it.
///
/// Segments are joined with `/` and each starts with its index, so a name
/// holding what reads as the start of another -- a `/` and digits, then a
/// `:`, a `/` or the end -- read as more than one segment: the package
/// `'a/0:b'` and the `b` inside a package `a` came out the same path, the
/// same UUID, and a document `import` refused as naming one element
/// twice. Only such a name is changed, and one already holding what the
/// change is spelled with, so every other element keeps the UUID it always
/// had -- the library's `'pica (computer) (1/6 in)'` and `'km/h'` among
/// them -- and documents written before still name it.
fn unmistakable(name: &str) -> std::borrow::Cow<'_, str> {
    let bytes = name.as_bytes();
    let reads_as_a_segment = (0..bytes.len()).any(|at| {
        if bytes[at] != b'/' {
            return false;
        }
        let digits = bytes[at + 1..]
            .iter()
            .take_while(|it| it.is_ascii_digit())
            .count();
        // a bare `/` at the end would be read as one before an empty name,
        // and `~` starts what an unnamed top-level member is keyed by
        matches!(bytes.get(at + 1 + digits), None | Some(b':' | b'/' | b'~'))
    });
    let spelled_like_the_change = name.contains("%2F") || name.contains("%25");
    if reads_as_a_segment || spelled_like_the_change {
        name.replace('%', "%25").replace('/', "%2F").into()
    } else {
        name.into()
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

fn identities(model: &Model, extras: &Extras) -> Identities {
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
            let key = sibling_key(model, extras, id);
            (
                id,
                format!(
                    "sysml-v2-rs:{}",
                    path_segment(&key, namesake_index(model, extras, &roots, id, &key))
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
        let mut namesakes: HashMap<SiblingKey<'_>, usize> = HashMap::new();
        for &child in children {
            let key = sibling_key(model, extras, child);
            let segment = {
                let seen = namesakes.entry(key.clone()).or_default();
                let index = *seen;
                *seen += 1;
                path_segment(&key, index)
            };
            stack.push((
                child,
                format!("{path}/{segment}"),
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
    let name = sysml_syntax::quote(model.effective_name(id)?);
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
/// what a type inherits once everything its specializations reach is
/// counted in, redefined features excepted.
///
/// KerML inherits memberships, not features: `inheritedMembership` is
/// `removeRedefinedFeatures(inheritableMemberships())`, which takes the
/// `nonPrivateMemberships` of each supertype -- its public and protected
/// ones and what it inherits in turn -- and a type's inherited features
/// are only those of them that are `FeatureMemberships`. Built from the
/// features alone, a subtype was said to inherit its supertype's private
/// attributes, and never the part definition nested in it.
///
/// Only relationships name resolution reified take part, so an unresolved
/// model derives empty closures -- which is what it knows.
struct Closures<'a> {
    model: &'a Model,
    /// Namespace -> what its public and protected imports bring in, which
    /// a type specializing it inherits with its own memberships.
    imported: &'a HashMap<ElementId, Vec<ElementId>>,
    inherited: std::cell::RefCell<HashMap<ElementId, std::rc::Rc<Vec<ElementId>>>>,
}

impl<'a> Closures<'a> {
    fn new(model: &'a Model, extras: &'a Extras) -> Closures<'a> {
        Closures {
            model,
            imported: &extras.inheritable_imported,
            inherited: std::cell::RefCell::new(HashMap::new()),
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
            // a subsetting written on its own in a feature's body is a
            // member of that feature, and specializes something else
            .filter(|&&child| !bridged(self.model, child))
            .filter_map(|&child| {
                let target = match self.model.kind(child) {
                    ElementKind::Subclassification => "superclassifier",
                    ElementKind::Subsetting => "subsettedFeature",
                    ElementKind::Redefinition => "redefinedFeature",
                    // a `ConjugatedPortTyping` among them
                    kind if kind.is_a(ElementKind::FeatureTyping) => "type",
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

    /// The features `feature` redefines directly.
    fn redefined_by(&self, feature: ElementId) -> impl Iterator<Item = ElementId> + '_ {
        self.model
            .owned(feature)
            .iter()
            .filter(|&&child| self.model.kind(child) == ElementKind::Redefinition)
            .filter_map(|&child| match self.model.maybe(child, "redefinedFeature") {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            })
    }

    /// `allRedefinedFeaturesOf`: the member itself, and everything it
    /// redefines directly or through what it redefines.
    fn all_redefined(&self, member: ElementId) -> std::collections::HashSet<ElementId> {
        let mut out = std::collections::HashSet::from([member]);
        let mut pending = vec![member];
        while let Some(at) = pending.pop() {
            for redefined in self.redefined_by(at) {
                if out.insert(redefined) {
                    pending.push(redefined);
                }
            }
        }
        out
    }

    /// Who a membership stands for, membership by membership: the member
    /// a synthesized one bridges to, or a membership the model holds as
    /// an element of its own -- an alias.
    fn owned_memberships(&self, id: ElementId) -> impl Iterator<Item = ElementId> + '_ {
        self.model.owned(id).iter().copied().filter(move |&child| {
            bridged(self.model, child) || self.model.kind(child) == ElementKind::Membership
        })
    }

    /// The visibility a membership was declared with.
    fn visibility(&self, membership: ElementId) -> Vis {
        self.model
            .member_visibility(membership)
            .or_else(|| match self.model.maybe(membership, "visibility") {
                Some(Value::EnumLit(written)) => Vis::written(written),
                _ => None,
            })
            .unwrap_or(Vis::Public)
    }

    /// `inheritedMemberships()`: each supertype's public and protected
    /// memberships and what it inherits in turn, less those a feature
    /// here -- or another of them -- redefines. Each membership is named
    /// by what [`Closures::owned_memberships`] names it by.
    fn inherited(&self, id: ElementId) -> std::rc::Rc<Vec<ElementId>> {
        if let Some(known) = self.inherited.borrow().get(&id) {
            return known.clone();
        }
        // a cycle (`part p :> p;` is legal) ends at what is already known:
        // publishing nothing first keeps the walk finite
        self.inherited
            .borrow_mut()
            .insert(id, std::rc::Rc::new(Vec::new()));

        let mut candidates = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for supertype in self.supertypes(id) {
            if supertype == id {
                continue;
            }
            let own = self
                .owned_memberships(supertype)
                .filter(|&membership| self.visibility(membership) != Vis::Private);
            // `nonPrivateMemberships` counts what the supertype imports
            // publicly or protectedly among its own: `part def Base {
            // public import Lib::*; }` hands `Lib::X` to every `Derived
            // :> Base`, where a name written in `Derived` finds it
            let imported = self
                .imported
                .get(&supertype)
                .into_iter()
                .flatten()
                .copied()
                .filter(|&member| {
                    bridged(self.model, member)
                        || self.model.kind(member) == ElementKind::Membership
                });
            let inherited = self.inherited(supertype);
            for membership in own.chain(imported).chain(inherited.iter().copied()) {
                if seen.insert(membership) {
                    candidates.push(membership);
                }
            }
        }
        // `removeRedefinedFeatures`: one another of them redefines, and
        // one whose redefined features a feature owned here redefines
        let redefined: Vec<std::collections::HashSet<ElementId>> = candidates
            .iter()
            .map(|&membership| self.all_redefined(membership))
            .collect();
        let redefined_by_another: std::collections::HashSet<ElementId> = candidates
            .iter()
            .zip(&redefined)
            .flat_map(|(&membership, all)| all.iter().copied().filter(move |&it| it != membership))
            .collect();
        let redefined_here: std::collections::HashSet<ElementId> = self
            .owned_features(id)
            .into_iter()
            .flat_map(|feature| self.redefined_by(feature).collect::<Vec<_>>())
            .collect();
        let out: Vec<ElementId> = candidates
            .iter()
            .zip(&redefined)
            .filter(|(membership, all)| {
                !redefined_by_another.contains(membership) && all.is_disjoint(&redefined_here)
            })
            .map(|(&membership, _)| membership)
            .collect();
        let out = std::rc::Rc::new(out);
        self.inherited.borrow_mut().insert(id, out.clone());
        out
    }

    /// `inheritedFeature`: the members of the inherited memberships that
    /// are `FeatureMemberships`.
    fn inherited_features(&self, id: ElementId) -> Vec<ElementId> {
        self.inherited(id)
            .iter()
            .copied()
            .filter(|&membership| {
                bridged(self.model, membership)
                    && sysml_model::membership_kind(self.model, membership)
                        .is_a(ElementKind::FeatureMembership)
            })
            .collect()
    }

    /// `feature`: what `id` owns as features and what it inherits as them.
    fn features(&self, id: ElementId) -> Vec<ElementId> {
        let mut out = self.owned_features(id);
        let mut seen: std::collections::HashSet<ElementId> = out.iter().copied().collect();
        out.extend(
            self.inherited_features(id)
                .into_iter()
                .filter(|&feature| seen.insert(feature)),
        );
        out
    }

    /// The features of `id` whose stored direction is one of `wanted`.
    fn directed(&self, id: ElementId, wanted: &[&str]) -> Vec<ElementId> {
        self.features(id)
            .into_iter()
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
        return a_statement_of_its_own(model, owned);
    }
    !model.owner(owned).is_some_and(|owner| {
        let kind = model.kind(owner);
        kind.is_a(ElementKind::Relationship) && !kind.is_a(ElementKind::Namespace)
    })
}

/// Is this relationship a member of its namespace rather than a part of
/// one of the elements it relates?
///
/// `subset S::f subsets S::g;` written in a package is a statement of its
/// own: KerML parses it as a `NonFeatureElement` the package owns through
/// an `OwningMembership`, exactly as it would a class. A package has no
/// `ownedSpecialization` to hold it -- a namespace owns relationships only
/// as memberships and imports. Written as the package's own relationship,
/// the subsetting came out with the package as its `owningFeature`, which
/// is neither a feature nor anything it relates.
///
/// Inside a type the same statement is told apart by its ends: the type a
/// specialization is written on is its `specific`, so one owned by a type
/// that is not its source end was written on its own. One that names the
/// type it stands in as its source reads the same either way, and is
/// taken as the ordinary kind -- the model does not say which it was.
/// Memberships, imports and annotations are what an element owns as
/// relationships, and never members.
fn a_statement_of_its_own(model: &Model, id: ElementId) -> bool {
    let kind = model.kind(id);
    if kind.is_a(ElementKind::Membership)
        || kind.is_a(ElementKind::Import)
        || kind.is_a(ElementKind::Annotation)
    {
        return false;
    }
    let Some(owner) = model.owner(id) else {
        return false;
    };
    let owner_kind = model.kind(owner);
    if !owner_kind.is_a(ElementKind::Namespace) {
        return false;
    }
    if !owner_kind.is_a(ElementKind::Type) {
        return true;
    }
    stated_source(model, id).is_some_and(|sources| !sources.contains(&owner))
}

/// The source end a relationship states itself, under the most specific
/// name its metaclass gives it -- nothing where it leaves that to its
/// owner.
fn stated_source(model: &Model, id: ElementId) -> Option<Vec<ElementId>> {
    let kind = model.kind(id);
    let (_, sources, _) = ENDS.iter().find(|(of, _, _)| kind.is_a(*of))?;
    sources.iter().find_map(|name| match model.maybe(id, name) {
        Some(Value::Ref(end)) => Some(vec![*end]),
        Some(Value::RefList(ends)) if !ends.is_empty() => Some(ends.clone()),
        _ => None,
    })
}

/// How an element's `owningRelationship` is written: as the membership
/// synthesized for it, or as the relationship that holds it among its
/// `ownedRelatedElement`.
enum Owning {
    Bridge,
    Related(ElementId),
}

/// The relationship an element is owned through, if any. A relationship
/// owned as itself -- a typing on its feature, a membership on its
/// namespace, a filter condition on its import -- is an
/// `ownedRelationship` of its owner and has none.
fn owning_relationship(model: &Model, id: ElementId) -> Option<Owning> {
    let owner = model.owner(id)?;
    if bridged(model, id) {
        Some(Owning::Bridge)
    } else if model.kind(owner).is_a(ElementKind::Relationship)
        && !only_a_relationship(model.kind(id))
    {
        Some(Owning::Related(owner))
    } else {
        None
    }
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
    /// Namespace -> the members its public and protected imports bring
    /// in, in import order: the imported part of KerML's
    /// `nonPrivateMemberships`, which a type specializing the namespace
    /// inherits. [`Extras::imported`] does not say which import brought a
    /// member in, so it cannot say this; a private import's members stay
    /// where they were imported.
    pub inheritable_imported: HashMap<ElementId, Vec<ElementId>>,
    /// Elements that belong to a library model (`isLibraryElement`).
    pub library: std::collections::HashSet<ElementId>,
    /// Elements to leave out of the document altogether.
    ///
    /// A model is resolved against a library and is not made of one: a
    /// small model exported with the standard library beside it writes
    /// the library's elements too, and `sysml api push` sends them. What the
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
    /// Feature -> the types its `TypeFeaturings` name, wherever in the
    /// model each one stands.
    featurings: HashMap<ElementId, Vec<ElementId>>,
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
    /// The element a membership written as `uuid` holds: the one it
    /// bridges to, or what an alias stands for, or -- where `uuid` is an
    /// element rather than a membership -- that element.
    fn member_of(&self, uuid: &str) -> Option<Json> {
        if let Some((&held, _)) = self
            .bridge_uuids
            .iter()
            .find(|(_, it)| it.to_string() == uuid)
        {
            return Some(self.reference(&held));
        }
        let (&named, _) = self.uuids.iter().find(|(_, it)| it.to_string() == uuid)?;
        let held = match self.model.kind(named) == ElementKind::Membership {
            true => self
                .model
                .get(named, "memberElement")
                .and_then(Value::as_id)?,
            false => named,
        };
        Some(self.reference(&held))
    }

    /// What an import imports: what the resolver said, or -- for a model
    /// read from a document rather than resolved from text -- what the
    /// document said, which the model keeps.
    fn import_target(&self, id: ElementId) -> Option<ElementId> {
        self.extras.import_targets.get(&id).copied().or_else(|| {
            ["importedMembership", "importedNamespace"]
                .iter()
                .find_map(|prop| self.model.maybe(id, prop).and_then(Value::as_id))
        })
    }

    /// A membership as [`Closures::owned_memberships`] names it: the
    /// bridge synthesized for a member, or a membership held as itself.
    fn membership_entry(&self, membership: ElementId) -> Json {
        match bridged(self.model, membership) {
            true => self.membership(membership),
            false => self.reference(&membership),
        }
    }

    /// What such a membership brings in: the member a bridge holds, or
    /// what an alias stands for, where that is known.
    fn member_held(&self, membership: ElementId) -> Option<ElementId> {
        match bridged(self.model, membership) {
            true => Some(membership),
            false => self
                .model
                .get(membership, "memberElement")
                .and_then(Value::as_id),
        }
    }

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
                    // What an import imports is a membership, which a model
                    // read from a document keeps as the element it holds:
                    // written as it is stored, `importedMembership` named
                    // the element where the document had named its
                    // membership. The writer knows the way back.
                    let imports = self.model.kind(id).is_a(ElementKind::Import)
                        && matches!(
                            meta.name,
                            "importedMembership" | "importedNamespace" | "importedElement"
                        );
                    if imports {
                        let value = self
                            .derived(id, meta.name)
                            .unwrap_or_else(|| default_for(meta));
                        object.insert(meta.name.into(), value);
                        continue;
                    }
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
                    // The one element an import owns that it also imports is
                    // the filter package `import A::*[@Safety]` is written
                    // as. `expose Nope::*;` owns the text it was written as
                    // too, and with nothing resolved that text stood in for
                    // the namespace exposed.
                    let import = self.model.kind(id).is_a(ElementKind::Import);
                    let owned: Vec<ElementId> = related_to(self.model, id);
                    let related: Vec<Json> = owned
                        .iter()
                        .filter(|&&it| !import || self.model.kind(it).is_a(ElementKind::Namespace))
                        .map(|it| self.reference(it))
                        .collect();
                    let (from, to) = match self.model.kind(id).is_a(ElementKind::Annotation) {
                        true => self.annotation_ends(id, &owned, owner.clone(), related),
                        false => (owner.clone(), related),
                    };
                    fill_ends(&mut object, self.model.kind(id), from, to);
                    // An annotation's ends each have an `owning` name, and
                    // that one says who owns it, not who it relates:
                    // `owningAnnotatedElement` is set only where the
                    // annotated element owns the annotation. Written down
                    // the same chain as the end it subsets, `comment about
                    // C` -- an annotation its comment owns -- said C owned
                    // it as well.
                    if self.model.kind(id).is_a(ElementKind::Annotation) {
                        for (end, owning) in [
                            ("annotatedElement", "owningAnnotatedElement"),
                            ("annotatingElement", "owningAnnotatingElement"),
                        ] {
                            let owns = owner.is_some() && object.get(end) == owner.as_ref();
                            if !owns {
                                object.insert(owning.into(), Json::Null);
                            }
                        }
                    }
                    // and what it owns is related by it all the same:
                    // `ownedRelatedElement` subsets `relatedElement`
                    if let Some(Json::Array(ends)) =
                        object.get_mut("relatedElement").filter(|_| import)
                    {
                        for it in &owned {
                            let it = self.reference(it);
                            if !ends.contains(&it) {
                                ends.push(it);
                            }
                        }
                    }
                    // What an import imports is read off the end it was
                    // written with, once the ends are settled: "for a
                    // MembershipImport, the memberElement of the
                    // importedMembership; for a NamespaceImport, the
                    // importedNamespace". Worked out apart from the ends,
                    // the two could disagree, and a document read back
                    // came out different from the one written.
                    if self.model.kind(id).is_a(ElementKind::Import) {
                        let element = match self.model.kind(id).is_a(ElementKind::NamespaceImport) {
                            true => object.get("importedNamespace").cloned(),
                            false => object
                                .get("importedMembership")
                                .and_then(|it| it["@id"].as_str())
                                .and_then(|uuid| self.member_of(uuid)),
                        };
                        object.insert("importedElement".into(), element.unwrap_or(Json::Null));
                    }
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
                    "owningRelatedElement" | "membershipOwningNamespace" => self.reference(&owner),
                    // a membership is an `ownedRelationship` of its
                    // namespace and owns no relationship of its own, so
                    // KerML's derivations give it no `owner` and no
                    // `ownedElement`: what it owns is related, not owned
                    // through anything
                    "owner" => Json::Null,
                    "ownedElement" => Json::Array(Vec::new()),
                    "ownedRelatedElement" => Json::Array(vec![self.reference(&id)]),
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
                Some(self.references(&self.closures.inherited_features(id)))
            }
            // every inherited membership, whatever it brings in: a
            // nested definition is inherited as surely as a feature is
            "inheritedMembership" if is_type => Some(Json::Array(
                self.closures
                    .inherited(id)
                    .iter()
                    .map(|&membership| self.membership_entry(membership))
                    .collect(),
            )),
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
                    for &membership in self.closures.inherited(id).iter() {
                        all.push(self.membership_entry(membership));
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
                    let inherited = self.closures.inherited(id);
                    members.extend(
                        inherited
                            .iter()
                            .filter_map(|&membership| self.member_held(membership))
                            .filter(|&member| seen.insert(member)),
                    );
                }
                Some(self.references(&members))
            }
            // an alias is a membership of its own, and is what is imported
            "importedMembership" if kind.is_a(ElementKind::MembershipImport) => Some(
                self.import_target(id)
                    .and_then(
                        |target| match self.model.kind(target) == ElementKind::Membership {
                            true => Some(self.reference(&target)),
                            false => self.membership_of(target),
                        },
                    )
                    .unwrap_or(Json::Null),
            ),
            "importedMembership" => {
                Some(self.memberships(self.extras.imported.get(&id).map_or(&[][..], Vec::as_slice)))
            }
            "importedNamespace" if kind.is_a(ElementKind::NamespaceImport) => Some(
                self.import_target(id)
                    .map_or(Json::Null, |it| self.reference(&it)),
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
            // the steps of a chain, each its own relationship, which the
            // `chainingFeature` beside it is read off
            "ownedFeatureChaining" => Some(self.owned_of_kind(id, ElementKind::FeatureChaining)),
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
            "featuringType" if kind.is_a(ElementKind::Feature) => {
                Some(self.references(&self.featuring_types(id)))
            }
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
            // and a specialization's `owningType`, which the end names
            // above redefine, is the type it is written on
            "owningType" if is_relationship => Some(match self.model.owner(id) {
                Some(owner)
                    if !bridged(self.model, id)
                        && self.model.kind(owner).is_a(ElementKind::Type) =>
                {
                    self.reference(&owner)
                }
                _ => Json::Null,
            }),
            // the owner where it is the annotating element, and nothing
            // where the element owning the annotation is the one annotated:
            // `@M;` and a comment owned through an annotation are not their
            // own owner's annotators. Read as the owner either way, it was
            // the first answer the end chain found for `annotatingElement`.
            "owningAnnotatingElement" if is_relationship => Some(match self.model.owner(id) {
                Some(owner)
                    if !bridged(self.model, id)
                        && self.model.kind(owner).is_a(ElementKind::AnnotatingElement) =>
                {
                    self.reference(&owner)
                }
                _ => Json::Null,
            }),
            // `ownedRelatedElement->selectByKind(AnnotatingElement)`, of
            // which there is one at most
            "ownedAnnotatingElement" if is_relationship => Some(
                self.model
                    .owned(id)
                    .iter()
                    .find(|&&it| self.model.kind(it).is_a(ElementKind::AnnotatingElement))
                    .map_or(Json::Null, |it| self.reference(it)),
            ),
            "owningClassifier"
            | "owningFeature"
            | "owningFeatureOfType"
            | "membershipOwningNamespace"
            | "importOwningNamespace"
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
            // `isReference = not isComposite`: a `ref part`, a port and a
            // part at package level are referential, and written at its
            // default every one of them was said to be owned outright
            "isReference" if kind.is_a(ElementKind::Usage) => {
                Some((!self.model.flag(id, "isComposite")).into())
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
            // KerML derives both through the relationship between:
            // `owner = owningRelationship.owningRelatedElement` and
            // `ownedElement = ownedRelationship.ownedRelatedElement`. Written
            // as the model's own edges, `attribute mass = 10.0` listed its
            // FeatureValue, typing and subsetting as what it owns, and the
            // literal named the FeatureValue as its owner -- the shape no
            // other tool derives, and one a document written the standard
            // way contradicted.
            "owner" => Some(
                self.spec_owner(id)
                    .map_or(Json::Null, |owner| self.reference(&owner)),
            ),
            "ownedElement" => Some(self.references(&self.spec_owned_elements(id))),
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
                        } else if only_a_relationship(self.model.kind(child)) {
                            Some(self.reference(&child))
                        } else {
                            None
                        }
                    })
                    .collect(),
            )),
            "owningRelationship" => Some(owning_relationship(self.model, id).map_or(
                Json::Null,
                |owning| match owning {
                    Owning::Bridge => self.membership(id),
                    Owning::Related(relationship) => self.reference(&relationship),
                },
            )),
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

    /// `featuringType`: the owning type a feature membership makes one,
    /// then the `featuringType` of every `TypeFeaturing` the feature is
    /// the `featureOfType` of -- `featured by B` owned by the feature, or
    /// `featuring of S::f by B;` standing in a package -- and, for a
    /// chain, what its first chaining feature is featured by.
    ///
    /// Read off the owner alone, `feature fb : A featured by B` was said
    /// to be featured by the type it sits in and never by B, which a
    /// `TypeFeaturing` in the same document named.
    fn featuring_types(&self, id: ElementId) -> Vec<ElementId> {
        let mut out = Vec::new();
        let mut chained = std::collections::HashSet::from([id]);
        let mut at = id;
        loop {
            let owning = self
                .model
                .owner(at)
                .filter(|&owner| self.model.kind(owner).is_a(ElementKind::Type));
            let featured = self.featurings.get(&at).into_iter().flatten().copied();
            for featuring in owning.into_iter().chain(featured) {
                if !out.contains(&featuring) {
                    out.push(featuring);
                }
            }
            // `chainingFeature->first().featuringType`, which may be a
            // chain in turn
            let first = match self.model.maybe(at, "chainingFeature") {
                Some(Value::RefList(chain)) => chain.first().copied(),
                Some(Value::Ref(only)) => Some(*only),
                _ => None,
            };
            match first {
                Some(next) if chained.insert(next) => at = next,
                _ => return out,
            }
        }
    }

    /// `owningRelationship.owningRelatedElement`, read off the shape the
    /// document is written in: the namespace a membership bridges from,
    /// the element that owns the relationship holding this one -- and
    /// nothing for a relationship owned as itself, which has no owning
    /// relationship to ask.
    fn spec_owner(&self, id: ElementId) -> Option<ElementId> {
        match owning_relationship(self.model, id)? {
            Owning::Bridge => self.model.owner(id),
            Owning::Related(relationship) => self
                .model
                .owner(relationship)
                .filter(|_| !bridged(self.model, relationship)),
        }
    }

    /// `ownedRelationship.ownedRelatedElement`: what each membership
    /// brings in, and what each relationship owned as itself owns in turn
    /// -- the relationships themselves are owned, but not elements of it.
    fn spec_owned_elements(&self, id: ElementId) -> Vec<ElementId> {
        self.model
            .owned(id)
            .iter()
            .flat_map(|&child| {
                if bridged(self.model, child) {
                    vec![child]
                } else if only_a_relationship(self.model.kind(child)) {
                    related_to(self.model, child)
                } else {
                    Vec::new()
                }
            })
            .collect()
    }

    /// Where an annotation's ends are to be found when the model stored
    /// neither: `annotatingElement` is "the ownedAnnotatingElement, if any,
    /// otherwise the owningAnnotatingElement", and the element annotated is
    /// the one owning the annotation where that is not the annotating one.
    ///
    /// `OwnedAnnotation : Annotation = ownedRelatedElement +=
    /// AnnotatingElement` is owned by the element it annotates, and owns the
    /// comment or the metadata it relates. Read owner first, as the ends of
    /// a specialization are, it said the annotated element annotated itself
    /// and was about its own comment.
    fn annotation_ends(
        &self,
        id: ElementId,
        owned: &[ElementId],
        owner: Option<Json>,
        related: Vec<Json>,
    ) -> (Option<Json>, Vec<Json>) {
        let annotating = |it: &ElementId| self.model.kind(*it).is_a(ElementKind::AnnotatingElement);
        if let Some(comment) = owned.iter().find(|it| annotating(it)) {
            return (Some(self.reference(comment)), owner.into_iter().collect());
        }
        let owned_by_annotating = self.model.owner(id).is_some_and(|it| annotating(&it));
        match owned_by_annotating {
            true => (owner, related),
            false => (None, owner.into_iter().chain(related).collect()),
        }
    }

    fn owned_of_kind(&self, id: ElementId, wanted: ElementKind) -> Json {
        Json::Array(
            self.model
                .owned(id)
                .iter()
                .filter(|&&child| self.model.kind(child).is_a(wanted))
                // a relationship that is a member here is owned through
                // its membership, and is none of this element's own
                // specializations
                .filter(|&&child| !only_a_relationship(wanted) || !bridged(self.model, child))
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
    } = identities(model, extras);
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
        closures: Closures::new(model, extras),
        featurings: featurings(model),
    }
}

/// Feature -> the `featuringType` of each `TypeFeaturing` it is the
/// `featureOfType` of, in model order: the feature the relationship
/// names, or where it names none, the feature that owns it.
fn featurings(model: &Model) -> HashMap<ElementId, Vec<ElementId>> {
    let mut out: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
    for id in model.ids() {
        if model.kind(id) != ElementKind::TypeFeaturing {
            continue;
        }
        let Some(Value::Ref(featuring)) = model.maybe(id, "featuringType") else {
            continue;
        };
        let feature = match model.maybe(id, "featureOfType") {
            Some(Value::Ref(feature)) => Some(*feature),
            _ => model.owner(id),
        };
        if let Some(feature) = feature {
            out.entry(feature).or_default().push(*featuring);
        }
    }
    out
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
        // what the specification says the flag is where a model says
        // nothing: `isUnique` is true unless `nonunique` was written, and
        // writing `false` for every flag said every feature was nonunique
        FeatureType::Data(PrimitiveType::Boolean) => Json::from(meta.default.unwrap_or(false)),
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
        // `importedElement` is not on the chain: it is derived as the
        // `memberElement` of what is imported, and copied from the end
        // it held the membership rather than the element
        &["importedMembership", "importedNamespace", "target"],
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
/// document of the standard library is a very large text, and as a
/// `serde_json::Value` it is many times larger -- for a model far smaller
/// than either when it is built. What stays here is the text, the borrows
/// into it, and the model.
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
/// pay for every other property beside them.
///
/// `@type` and `@id` are read as whatever JSON value they are, so that one
/// that is not a string is said to be that rather than missing, and one
/// written with an escape (`"Pack\u0061ge"`) is read at all.
#[derive(serde::Deserialize)]
struct Head {
    #[serde(rename = "@type", default)]
    kind: Option<Json>,
    #[serde(rename = "@id", default)]
    uuid: Option<Json>,
    #[serde(rename = "ownedRelatedElement", default)]
    owned: Option<serde::de::IgnoredAny>,
}

impl Head {
    /// A field of the head that has to be a string, where it is given.
    fn text<'h>(
        field: &'h Option<Json>,
        name: &str,
        at: usize,
    ) -> Result<Option<&'h str>, ImportError> {
        match field {
            None | Some(Json::Null) => Ok(None),
            Some(Json::String(text)) => Ok(Some(text)),
            Some(other) => Err(ImportError::Unreadable(
                at,
                format!("{name:?} is {other}, not a string"),
            )),
        }
    }
}

impl<'a> Element<'a> {
    /// What the first pass needs, and no more.
    fn head(&self, at: usize) -> Result<Head, ImportError> {
        // `Head` reads from a JSON array as readily as from an object --
        // `["Package", "abc"]` is its two strings in order -- and pass two
        // then met an element that was no object
        let object = match self {
            Element::Built(json) => json.is_object(),
            Element::Text(raw) => raw.get().trim_start().starts_with('{'),
        };
        if !object {
            return Err(ImportError::NotAnObject(at));
        }
        match self {
            Element::Built(json) => Ok(Head {
                kind: json.get("@type").cloned(),
                uuid: json.get("@id").cloned(),
                owned: json
                    .get("ownedRelatedElement")
                    .map(|_| serde::de::IgnoredAny),
            }),
            Element::Text(raw) => serde_json::from_str(raw.get())
                .map_err(|why| ImportError::Unreadable(at, why.to_string())),
        }
    }

    /// The element as a `Json`, borrowed where there is one to borrow.
    fn read(&self, at: usize) -> Result<std::borrow::Cow<'a, Json>, ImportError> {
        match self {
            Element::Built(json) => Ok(std::borrow::Cow::Borrowed(json)),
            Element::Text(raw) => serde_json::from_str(raw.get())
                .map(std::borrow::Cow::Owned)
                .map_err(|why| ImportError::Unreadable(at, why.to_string())),
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
        let type_name =
            Head::text(&head.kind, "@type", index)?.ok_or(ImportError::MissingType(index))?;
        let kind = ElementKind::from_name(type_name)
            .ok_or_else(|| ImportError::UnknownType(type_name.to_string()))?;
        if kind.is_abstract() {
            return Err(ImportError::AbstractType(type_name.to_string()));
        }
        let uuid = Head::text(&head.uuid, "@id", index)?
            .ok_or(ImportError::MissingId(index))?
            .to_string();
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
    // the element a membership that folded into an edge holds
    let member_of_bridge = |value: &Json| -> Result<Option<ElementId>, ImportError> {
        let uuid = value["@id"].as_str().unwrap_or_default();
        let Some(source) = bridges.get(uuid) else {
            return Ok(None);
        };
        let bridge = source.read(0)?;
        let held = bridge["memberElement"]
            .as_object()
            .map(|_| &bridge["memberElement"])
            .or_else(|| bridge["ownedRelatedElement"].as_array()?.first());
        Ok(held.and_then(|it| resolve(it).ok()))
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
            // a feature held by a plain `OwningMembership` was declared a
            // member and not a feature of its owner -- `member feature g`,
            // the `[0..1]` of a cross feature -- which the model records
            // rather than works out. Not restored, every such feature came
            // back a `FeatureMembership`'s and inherited as a feature.
            if bridge["@type"] == "OwningMembership"
                && model.kind(member).is_a(ElementKind::Feature)
            {
                model.set_owning_member(member);
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
    //
    // `ownedElement` is the derived one, and not every writer derives it
    // alike: KerML has it reach through the relationship between, so the
    // literal of `attribute a = 3` is an `ownedElement` of `a` and an
    // `ownedRelatedElement` of its FeatureValue at once, so it is not
    // taken to be the FeatureValue's alone. Each pair is kept with
    // whether only `ownedElement` said it, and settled once all are in.
    let mut stated = std::collections::HashSet::new();
    let mut edges: Vec<(ElementId, ElementId, bool)> = Vec::new();
    for (&(index, source), id) in created.iter().zip(&ids) {
        let kind = model.kind(*id);
        let object = source.read(index)?;
        let object = object.as_object().expect("validated in pass 1");
        let mut own = |child: ElementId, derived: bool| {
            if stated.insert((*id, child, derived)) {
                edges.push((*id, child, derived));
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
                        own(resolve(child)?, key == "ownedElement");
                    }
                }
                "ownedRelationship" => {
                    for related in value.as_array().into_iter().flatten() {
                        for child in through(&mut model, related)? {
                            own(child, false);
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
                    let converted = match convert_value(meta, value, &resolve) {
                        // an import may name the membership an element was
                        // reached through, which folded into an edge here:
                        // what it imports is the element on the far side,
                        // kept so that writing the model out again names
                        // the same membership -- dropped, every import of
                        // a member came back importing nothing
                        Err(ImportError::UnknownReference(_))
                            if matches!(
                                key.as_str(),
                                "importedMembership" | "importedNamespace"
                            ) =>
                        {
                            member_of_bridge(value)?.map(Value::Ref)
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
    let edges = settle_derived_ownership(edges, &spelled)?;
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

/// The ownership a document states, with what only `ownedElement` said
/// reconciled against what its relationships say.
///
/// Where a relationship states who owns an element, `ownedElement` may
/// name that owner -- the shape this crate wrote before -- or the owner of
/// that relationship, which is how KerML derives it; either agrees and
/// adds nothing. Any other owner is a second one. Where no relationship
/// says, `ownedElement` is all there is, and is taken as it stands: a
/// document written with nothing else still reads as the tree it spells.
fn settle_derived_ownership(
    edges: Vec<(ElementId, ElementId, bool)>,
    spelled: &HashMap<ElementId, &str>,
) -> Result<Vec<(ElementId, ElementId)>, ImportError> {
    let mut stated: HashMap<ElementId, ElementId> = HashMap::new();
    for &(owner, child, derived) in &edges {
        if !derived {
            // a second owner here is the tree check's to refuse
            stated.entry(child).or_insert(owner);
        }
    }
    // and the owner of the relationship between, however that was said
    let mut above = stated.clone();
    for &(owner, child, derived) in &edges {
        if derived {
            above.entry(child).or_insert(owner);
        }
    }
    let mut kept = Vec::with_capacity(edges.len());
    let mut seen = std::collections::HashSet::new();
    for (owner, child, derived) in edges {
        if derived {
            if let Some(&direct) = stated.get(&child) {
                if direct != owner && above.get(&direct) != Some(&owner) {
                    return Err(ImportError::SharedOwnership(spelled[&child].to_string()));
                }
                continue;
            }
        }
        if seen.insert((owner, child)) {
            kept.push((owner, child));
        }
    }
    Ok(kept)
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

/// The value `value` gives a property shaped as `meta` says, if any.
///
/// A value of another shape is skipped, as foreign JSON always has been
/// here, rather than stored as given: `"isAbstract": 1` or `"value": 1.5`
/// on a `LiteralInteger` went into the model, which panicked where the
/// model checks what it is handed and was written back out wrongly where
/// it does not.
fn convert_value(
    meta: &sysml_model::FeatureMeta,
    value: &Json,
    resolve: &dyn Fn(&Json) -> Result<ElementId, ImportError>,
) -> Result<Option<Value>, ImportError> {
    let converted = match (meta.ty, value) {
        (_, Json::Null) => return Ok(None),
        // `aliasIds: []`: the model holds no list of plain values, and a
        // list of them is skipped as it always was
        (FeatureType::Data(_) | FeatureType::Enumeration(_), Json::Array(_)) if meta.many => {
            return Ok(None);
        }
        (FeatureType::Data(PrimitiveType::Boolean), Json::Bool(b)) => Some(Value::Bool(*b)),
        (FeatureType::Data(PrimitiveType::Real), Json::Number(n)) => n.as_f64().map(Value::Real),
        (FeatureType::Data(PrimitiveType::Real), Json::String(text)) => {
            text.parse::<f64>().ok().map(Value::Real)
        }
        (
            FeatureType::Data(PrimitiveType::Integer | PrimitiveType::UnlimitedNatural),
            Json::Number(n),
        ) => n.as_i64().map(Value::Int),
        (FeatureType::Data(PrimitiveType::String), Json::String(s)) => {
            Some(Value::String(s.clone()))
        }
        (FeatureType::Enumeration(ty), Json::String(text)) => {
            return Ok(enum_literal(ty, text).map(Value::EnumLit));
        }
        (FeatureType::Class(_), Json::Object(_)) => Some(Value::Ref(resolve(value)?)),
        (FeatureType::Class(_), Json::Array(items)) if meta.many => {
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

    /// A name places a top-level member wherever its file was loaded, and
    /// an unnamed one was placed by its position among all the unnamed
    /// ones before it -- from every file. Two files each opening with a
    /// `private import` swapped the two imports' UUIDs when they were
    /// exported the other way round.
    #[test]
    fn an_unnamed_top_level_member_keeps_its_uuid_whatever_order_the_files_came_in() {
        let a = "private import L1::*;\npackage A { attribute x; }\n";
        let b = "private import L2::*;\npackage B { attribute y; }\n";
        let lib = "package L1 { attribute one; }\npackage L2 { attribute two; }\n";
        let exported = |first: &str, second: &str| {
            let mut ws = sysml_semantics::Workspace::new();
            ws.add_file("first.sysml", first);
            ws.add_file("second.sysml", second);
            ws.add_file("lib.sysml", lib);
            ws.resolve_all();
            let extras = extras_of(&mut ws);
            let json = to_json_with(ws.model(), &extras);
            for id in ws.model().ids() {
                let written = json
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|object| {
                        object["@id"] == element_uuid_with(ws.model(), &extras, id).to_string()
                    })
                    .count();
                assert_eq!(written, 1, "the published UUID is the one written");
            }
            // what each import imports, against the UUID it is written under
            json.as_array()
                .unwrap()
                .iter()
                .filter(|object| object["@type"] == "NamespaceImport")
                .map(|object| {
                    (
                        object["importedNamespace"]["@id"].to_string(),
                        object["@id"].to_string(),
                    )
                })
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let one_way = exported(a, b);
        assert_eq!(one_way.len(), 2, "both imports resolved");
        assert!(!one_way.contains_key("null"));
        assert_eq!(
            one_way,
            exported(b, a),
            "the load order moved the imports' UUIDs"
        );

        // and a named element keeps the UUID it always had: the second
        // root-level `A` of nothing named before it
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("first.sysml", a);
        ws.resolve_all();
        let package = ws
            .model()
            .ids()
            .find(|&id| ws.model().name(id) == Some("A"))
            .unwrap();
        assert_eq!(
            element_uuid(ws.model(), package),
            uuid_of_path("sysml-v2-rs:0/0:A")
        );
    }

    /// `isReference = not isComposite`: a `ref part`, a port, an attribute
    /// and a part at package level are referential, and a part owned by
    /// a definition is not. Written at its default, every usage was said
    /// to be owned outright.
    #[test]
    fn a_usage_is_a_reference_unless_it_is_composite() {
        let parse = sysml_syntax::parse(
            "package R { part def A { ref part q; part p; port pt; attribute a; } part top; }",
        );
        let model = build_model(&parse).0;
        let json = to_json(&model);
        let reference = |name: &str| {
            json.as_array()
                .unwrap()
                .iter()
                .find(|it| it["declaredName"] == name)
                .map(|it| it["isReference"].clone())
                .unwrap()
        };
        for (name, is) in [
            ("q", true),
            ("p", false),
            ("pt", true),
            ("a", true),
            ("top", true),
        ] {
            assert_eq!(reference(name), Json::Bool(is), "{name}");
        }
    }

    /// A name holding what reads as another segment of the path its UUID
    /// is made from -- `'a/0:b'` -- must not share a UUID with the element
    /// that path really leads to, or the document does not read back.
    #[test]
    fn a_name_that_reads_as_a_path_is_not_another_element() {
        let parse = sysml_syntax::parse("package a { part def b; }\npackage 'a/0:b';\n");
        let model = build_model(&parse).0;
        let json = to_json(&model);
        let ids: Vec<&str> = json
            .as_array()
            .unwrap()
            .iter()
            .map(|it| it["@id"].as_str().unwrap())
            .collect();
        let distinct: std::collections::HashSet<&str> = ids.iter().copied().collect();
        assert_eq!(distinct.len(), ids.len(), "{json}");

        // only such a name is spelled differently, so every other element
        // keeps the UUID it always had
        for kept in [
            "b",
            "km/h",
            "pica (computer) (1/6 in)",
            "%",
            ",2/3 v",
            "a/b/c",
        ] {
            assert_eq!(unmistakable(kept), kept);
        }
        for (changed, spelled) in [
            ("a/0:b", "a%2F0:b"),
            ("x/12", "x%2F12"),
            // the second segment of `1` with an unnamed element at 2
            ("1/2", "1%2F2"),
            ("x/", "x%2F"),
            ("y/3/z", "y%2F3%2Fz"),
            ("a%2Fb", "a%252Fb"),
            ("100%25", "100%2525"),
        ] {
            assert_eq!(unmistakable(changed), spelled);
        }
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
            (ImportError::NotAnObject(2), "not a JSON object"),
            (
                ImportError::Unreadable(4, "why".into()),
                "element 4 cannot be read: why",
            ),
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

    /// KerML derives `ownedElement` as `ownedRelationship.ownedRelatedElement`
    /// and `owner` as `owningRelationship.owningRelatedElement`: what a
    /// feature owns is the value its FeatureValue holds, not the
    /// FeatureValue, and the literal's owner is the feature. Written as the
    /// model's own edges, the typing, the subsetting and the FeatureValue
    /// were all said to be elements of `mass`, the literal was said to be
    /// owned by the FeatureValue, and a membership was owned by its
    /// namespace -- none of which the standard's derivations give.
    #[test]
    fn owner_and_owned_element_reach_through_the_relationship_between() {
        let (_, model) = resolved("package P { attribute mass : ScalarValues::Real = 10.0; }");
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let by_id = |id: &Json| {
            objects
                .iter()
                .find(|object| object["@id"] == id["@id"])
                .unwrap()
        };
        let mass = objects
            .iter()
            .find(|object| object["declaredName"] == "mass")
            .unwrap();
        let literal = objects
            .iter()
            .find(|object| object["@type"] == "LiteralRational")
            .unwrap();
        let mass_ref = json!({ "@id": mass["@id"] });
        let literal_ref = json!({ "@id": literal["@id"] });
        assert_eq!(mass["ownedElement"], json!([literal_ref]));
        assert_eq!(literal["owner"], mass_ref);

        let value = by_id(&literal["owningRelationship"]);
        assert_eq!(value["@type"], "FeatureValue");
        assert_eq!(value["owner"], Json::Null, "a relationship owned as itself");
        assert_eq!(value["ownedElement"], json!([]));
        assert_eq!(value["owningRelatedElement"], mass_ref);

        let membership = by_id(&mass["owningRelationship"]);
        assert_eq!(membership["owner"], Json::Null);
        assert_eq!(membership["ownedElement"], json!([]));
        assert_eq!(membership["ownedRelatedElement"], json!([mass_ref]));
        let package = by_id(&membership["owningRelatedElement"]);
        assert_eq!(mass["owner"]["@id"], package["@id"]);
        assert_eq!(package["ownedElement"], json!([mass_ref]));

        // and the document reads back as the model it was written from
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json(&rebuilt), json);
    }

    /// The same shape from another writer, with a value in it: `a` owns its
    /// FeatureValue and, derived, the literal that FeatureValue owns. Both
    /// were read as ownership, and the document was refused as giving the
    /// literal two owners.
    #[test]
    fn a_spec_shaped_owned_element_is_no_second_owner() {
        let json = serde_json::json!([
            { "@type": "Package", "@id": "p", "declaredName": "P",
              "ownedRelationship": [{ "@id": "m1" }], "ownedElement": [{ "@id": "a" }] },
            { "@type": "OwningMembership", "@id": "m1",
              "ownedRelatedElement": [{ "@id": "a" }], "memberElement": { "@id": "a" },
              "owningRelatedElement": { "@id": "p" } },
            { "@type": "AttributeUsage", "@id": "a", "declaredName": "x",
              "owner": { "@id": "p" }, "ownedRelationship": [{ "@id": "fv" }],
              "ownedElement": [{ "@id": "lit" }] },
            { "@type": "FeatureValue", "@id": "fv", "owningRelatedElement": { "@id": "a" },
              "ownedRelatedElement": [{ "@id": "lit" }], "value": { "@id": "lit" },
              "owner": null, "ownedElement": [] },
            { "@type": "LiteralInteger", "@id": "lit", "value": 3,
              "owner": { "@id": "a" }, "owningRelationship": { "@id": "fv" } },
        ]);
        let (model, roots) = from_json(&json).unwrap();
        let package = roots[0];
        let attribute = model.owned(package)[0];
        let value = model.owned(attribute)[0];
        assert_eq!(model.kind(value), ElementKind::FeatureValue);
        let literal = model.owned(value)[0];
        assert_eq!(model.kind(literal), ElementKind::LiteralInteger);
        assert_eq!(model.owner(literal), Some(value));

        // an `ownedElement` naming neither the owner nor the one above it
        // is a second owner still
        let mut wrong = json.clone();
        wrong[0]["ownedElement"] = json!([{ "@id": "a" }, { "@id": "lit" }]);
        wrong[2]["ownedElement"] = json!([]);
        assert!(matches!(
            from_json(&wrong),
            Err(ImportError::SharedOwnership(id)) if id == "lit"
        ));
    }

    /// `OwnedAnnotation : Annotation = ownedRelatedElement +=
    /// AnnotatingElement`: an element may own the annotation that annotates
    /// it, and the annotation then owns the comment or the metadata it
    /// relates. A document that writes that shape and leaves the
    /// annotation's ends to be derived -- `annotatingElement` is
    /// `ownedAnnotatingElement` or `owningAnnotatingElement`, and
    /// `annotatedElement` the owner where it owns the annotation -- was read
    /// the way a specialization is, owner first: the package came out
    /// annotating itself, and the comment was what it annotated.
    #[test]
    fn an_annotation_its_annotated_element_owns_relates_the_comment_it_owns() {
        let json = serde_json::json!([
            { "@type": "Package", "@id": "p", "declaredName": "P",
              "ownedRelationship": [{ "@id": "a1" }, { "@id": "a2" }] },
            { "@type": "Annotation", "@id": "a1",
              "owningRelatedElement": { "@id": "p" },
              "ownedRelatedElement": [{ "@id": "c" }] },
            { "@type": "Comment", "@id": "c", "body": "hi",
              "owningRelationship": { "@id": "a1" } },
            { "@type": "Annotation", "@id": "a2",
              "owningRelatedElement": { "@id": "p" },
              "annotatedElement": { "@id": "p" },
              "ownedRelatedElement": [{ "@id": "m" }] },
            { "@type": "MetadataUsage", "@id": "m",
              "owningRelationship": { "@id": "a2" } },
        ]);
        let (model, _) = from_json(&json).unwrap();
        let out = to_json(&model);
        let objects = out.as_array().unwrap();
        let of_type = |name: &str| {
            objects
                .iter()
                .find(|it| it["@type"] == name)
                .unwrap_or_else(|| panic!("no {name} was written"))
        };
        let package = of_type("Package");
        for annotating in [of_type("Comment"), of_type("MetadataUsage")] {
            let annotation = objects
                .iter()
                .find(|it| it["@id"] == annotating["owningRelationship"]["@id"])
                .expect("the annotation owning it is written");
            assert_eq!(annotation["@type"], "Annotation");
            assert_eq!(annotation["annotatingElement"]["@id"], annotating["@id"]);
            assert_eq!(
                annotation["ownedAnnotatingElement"]["@id"],
                annotating["@id"]
            );
            assert_eq!(annotation["owningAnnotatingElement"], Json::Null);
            assert_eq!(annotation["annotatedElement"]["@id"], package["@id"]);
            assert_eq!(annotation["owningAnnotatedElement"]["@id"], package["@id"]);
            assert_eq!(annotation["source"][0]["@id"], annotating["@id"]);
            assert_eq!(annotation["target"][0]["@id"], package["@id"]);
        }
        // and the document read back writes the same ends
        let (rebuilt, _) = from_json(&out).unwrap();
        assert_eq!(to_json(&rebuilt), out);
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

    /// KerML inherits a supertype's `nonPrivateMemberships` -- its public
    /// and protected ones, whatever they bring in -- and the features among
    /// them are what is inherited as features. The closure was built from
    /// features alone: `B :> A` was said to inherit `A::hidden`, which is
    /// private, and never `A::Nested`, which is no feature.
    #[test]
    fn a_subtype_inherits_what_is_not_private_and_not_only_features() {
        let (_, model) = resolved(
            "package T6 {\n\
               part def A { private attribute hidden; protected attribute prot; \
                 attribute pub; part def Nested; }\n\
               part def B :> A { attribute :>> pub; }\n\
               part def C :> B;\n\
             }\n",
        );
        let json = to_json(&model);
        let objects = json.as_array().unwrap();
        let named = |name: &str| {
            objects
                .iter()
                .find(|object| object["declaredName"] == name)
                .unwrap()
        };
        let ids = |value: &Json| -> std::collections::BTreeSet<String> {
            value
                .as_array()
                .unwrap()
                .iter()
                .map(|reference| reference["@id"].as_str().unwrap().to_string())
                .collect()
        };
        let id = |name: &str| named(name)["@id"].as_str().unwrap().to_string();
        let membership = |name: &str| {
            named(name)["owningMembership"]["@id"]
                .as_str()
                .unwrap()
                .to_string()
        };
        let set = |names: &[String]| {
            names
                .iter()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
        };

        let b = named("B");
        let redefining = b["ownedFeature"][0]["@id"].as_str().unwrap().to_string();
        assert_eq!(b["ownedFeature"].as_array().unwrap().len(), 1);
        // `pub` is redefined, `hidden` is private: `prot` alone is
        // inherited as a feature, and `Nested` beside it as a member
        assert_eq!(ids(&b["inheritedFeature"]), set(&[id("prot")]));
        assert_eq!(ids(&b["feature"]), set(&[redefining.clone(), id("prot")]));
        assert_eq!(
            ids(&b["inheritedMembership"]),
            set(&[membership("prot"), membership("Nested")])
        );
        let members = ids(&b["member"]);
        assert!(members.contains(&id("Nested")));
        assert!(!members.contains(&id("hidden")));
        assert!(ids(&b["membership"]).contains(&membership("Nested")));
        assert!(!ids(&b["membership"]).contains(&membership("hidden")));

        // and what B inherits it hands on, its own redefinition with it --
        // while what that redefinition redefines stays redefined
        let c = named("C");
        let handed_on = objects
            .iter()
            .find(|object| object["@id"] == redefining.as_str())
            .unwrap()["owningMembership"]["@id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            ids(&c["inheritedMembership"]),
            set(&[handed_on, membership("prot"), membership("Nested")])
        );
        assert!(!ids(&c["inheritedFeature"]).contains(&id("pub")));
    }

    /// An alias among what a supertype imports is one of its
    /// `nonPrivateMemberships`, and a subtype inherits the alias itself: a
    /// membership of its own kind, not a member reached through one. The
    /// resolver here hands on what an alias stands for, so the extras are
    /// filled in as another caller of [`to_json_with`] may fill them.
    #[test]
    fn an_alias_a_supertype_imports_is_inherited_as_the_alias() {
        let (mut ws, model) = resolved(
            "package L { part def Y; alias X for Y; }\n\
             part def Base;\n\
             part def Derived :> Base;\n",
        );
        let named = |name: &str| {
            model
                .ids()
                .find(|&id| model.name(id) == Some(name))
                .unwrap()
        };
        let mut extras = extras_of(&mut ws);
        extras
            .inheritable_imported
            .insert(named("Base"), vec![named("X")]);
        let json = to_json_with(&model, &extras);
        let objects = json.as_array().unwrap();
        let alias = objects
            .iter()
            .find(|object| object["@type"] == "Membership" && object["memberName"] == "X")
            .expect("the alias");
        let derived = objects
            .iter()
            .find(|object| object["declaredName"] == "Derived")
            .unwrap();
        let inherited = &derived["inheritedMembership"];
        assert!(
            inherited
                .as_array()
                .unwrap()
                .iter()
                .any(|it| it["@id"] == alias["@id"]),
            "{inherited}"
        );
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
            let inheritable = ws.inheritable_imported_members(id);
            if !inheritable.is_empty() {
                extras.inheritable_imported.insert(id, inheritable);
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

        // what each imports is the element, not the membership it came by
        assert_eq!(namespace_import["importedElement"]["@id"], a["@id"]);
        assert_eq!(membership_import["importedElement"]["@id"], x["@id"]);
        // and read back with nothing beside the document, the imports
        // still say what they imported: the membership folded into an
        // edge on the way in, and must not be dropped with it
        let again = to_json(&rebuilt);
        let reimported = again
            .as_array()
            .unwrap()
            .iter()
            .find(|object| object["@type"] == "MembershipImport")
            .unwrap();
        assert_eq!(
            reimported["importedMembership"],
            membership_import["importedMembership"]
        );
        assert_eq!(reimported["importedElement"]["@id"], x["@id"]);

        // a membership the document does not hold is no reason to refuse it
        let mut elsewhere = json.clone();
        for object in elsewhere.as_array_mut().unwrap() {
            if object["@type"] == "MembershipImport" {
                object["importedMembership"] =
                    json!({ "@id": "00000000-0000-0000-0000-000000000000" });
            }
        }
        assert!(from_json(&elsewhere).is_ok());
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
    fn an_expose_names_what_it_exposes_and_not_the_text_it_was_written_as() {
        // `expose T::Veh;` is a `MembershipExpose`, which is a
        // `MembershipImport`: what it imports was only looked up for the
        // import metaclass itself, and every end fell back on what the
        // expose owns -- the text the view compartment shows
        let (mut ws, model) = resolved(
            "package T {\n\tpart def Veh;\n\tpackage Sub { part def Q; }\n\
             \tview v { expose T::Veh; expose Sub::*; expose Nope::*; }\n}\n",
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
        let (veh, sub) = (of("Veh"), of("Sub"));
        let exposes: Vec<&Json> = objects
            .iter()
            .filter(|object| {
                object["@type"]
                    .as_str()
                    .is_some_and(|it| it.ends_with("Expose"))
            })
            .collect();
        assert_eq!(
            exposes[0]["importedMembership"]["@id"],
            veh["owningMembership"]["@id"]
        );
        assert_eq!(exposes[0]["importedElement"]["@id"], veh["@id"]);
        assert_eq!(exposes[1]["importedNamespace"]["@id"], sub["@id"]);
        assert_eq!(exposes[1]["target"][0]["@id"], sub["@id"]);
        // what names nothing imports nothing, rather than its own text
        assert_eq!(exposes[2]["importedNamespace"], Json::Null);
        assert_eq!(exposes[2]["target"], json!([]));
        assert_eq!(exposes[2]["importedElement"], Json::Null);
        for expose in &exposes {
            // the text is still the expose's own, and related by it
            let text = &expose["ownedRelatedElement"][0];
            assert!(expose["relatedElement"].as_array().unwrap().contains(text));
        }
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json_with(&rebuilt, &extras), json);
    }

    #[test]
    fn a_filtered_import_imports_the_package_that_filters() {
        // `NamespaceImport = ... | importedNamespace = FilterPackage {
        // ownedRelatedElement += importedNamespace }`: the import written
        // imports the package it owns, and that package imports `Lib::*`
        // and filters by `@Meta`. Exported as the import of `Lib`, every
        // member of `Lib` arrived, whatever the brackets kept out.
        let (mut ws, model) = resolved(
            "package P {\n\tmetadata def Meta;\n\
             \tpackage Lib { part def W; #Meta part def Z; }\n\
             \tprivate import Lib::*[@Meta];\n}\n",
        );
        let extras = extras_of(&mut ws);
        let json = to_json_with(&model, &extras);
        let objects = json.as_array().unwrap();
        let by_id = |reference: &Json| {
            objects
                .iter()
                .find(|object| object["@id"] == reference["@id"])
                .unwrap()
        };
        let lib = objects
            .iter()
            .find(|object| object["declaredName"] == "Lib")
            .unwrap();
        let written = objects
            .iter()
            .find(|object| {
                object["@type"] == "NamespaceImport" && object["visibility"] == "private"
            })
            .unwrap();
        let package = by_id(&written["importedNamespace"]);
        assert_eq!(package["@type"], "Package");
        assert_eq!(
            written["ownedRelatedElement"],
            json!([written["importedNamespace"]])
        );
        let owned: Vec<&Json> = package["ownedRelationship"]
            .as_array()
            .unwrap()
            .iter()
            .map(by_id)
            .collect();
        assert_eq!(owned[0]["@type"], "NamespaceImport");
        assert_eq!(owned[0]["visibility"], "public");
        assert_eq!(owned[0]["importedNamespace"]["@id"], lib["@id"]);
        assert_eq!(owned[1]["@type"], "ElementFilterMembership");
        assert_ne!(owned[1]["condition"], Json::Null);
        // and only what the filter keeps arrives
        let p = objects
            .iter()
            .find(|object| object["declaredName"] == "P")
            .unwrap();
        let arrived: Vec<&Json> = p["importedMembership"]
            .as_array()
            .unwrap()
            .iter()
            .map(|membership| by_id(&by_id(membership)["memberElement"]))
            .collect();
        assert_eq!(
            arrived
                .iter()
                .map(|it| &it["declaredName"])
                .collect::<Vec<_>>(),
            ["Z"]
        );
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(to_json_with(&rebuilt, &extras), json);
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
        assert_eq!(sysml_syntax::quote("mass"), "mass");
        assert_eq!(sysml_syntax::quote("wheel 1"), "'wheel 1'");
        assert_eq!(sysml_syntax::quote("1st"), "'1st'");
        assert_eq!(sysml_syntax::quote(""), "''");
        // a keyword written plainly would read back as the keyword
        assert_eq!(sysml_syntax::quote("part"), "'part'");
        assert_eq!(sysml_syntax::quote("it's"), "'it\\'s'");
        // whatever the quotes must not end early is escaped again, so
        // that unquoting the result gives the name back
        for name in [
            "it's", "a\\b", "a\nb", "a\rb", "a\tb", "a\u{8}b", "a\u{c}b", "wheel 1",
        ] {
            assert_eq!(
                sysml_syntax::unquote(&sysml_syntax::quote(name)),
                name,
                "{name:?}"
            );
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
        // and the table of them is the same again, so a reader that looks
        // an `@id` up in it finds the element the document meant
        let (uuids, _) = published_uuids(&model, &Extras::default());
        for id in model.ids() {
            assert_eq!(uuids[&id], element_uuid(&model, id));
        }
    }

    #[test]
    fn a_deep_model_is_exported_in_one_walk_of_it() {
        // computing every UUID by walking up to the root, and every
        // membership reference by computing one again, makes a chain this
        // deep take far too long to export
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

    /// Foreign JSON of the wrong shape is never a panic: an element that
    /// is an array was read as `@type` and `@id` in turn and met an
    /// `expect` in the second pass, and a value of the wrong type went
    /// into the model as given.
    #[test]
    fn a_value_of_the_wrong_shape_is_no_panic() {
        let as_text = |json: &Json| read_json(json.to_string().as_bytes()).map(|_| ());
        let not_an_object = json!([["Package", "abc"]]);
        assert!(matches!(
            from_json(&not_an_object),
            Err(ImportError::NotAnObject(0))
        ));
        assert!(matches!(
            as_text(&not_an_object),
            Err(ImportError::NotAnObject(0))
        ));
        let one_of_two = json!([{ "@type": "Package", "@id": "a" }, 5]);
        assert!(matches!(
            from_json(&one_of_two),
            Err(ImportError::NotAnObject(1))
        ));
        for wrong in [
            json!([{ "@type": "PartDefinition", "@id": "a", "isAbstract": 1 }]),
            json!([{ "@type": "PartDefinition", "@id": "a", "isAbstract": "yes" }]),
            json!([{ "@type": "PartDefinition", "@id": "a", "declaredName": 5 }]),
            json!([{ "@type": "LiteralInteger", "@id": "a", "value": 1.5 }]),
            json!([{ "@type": "LiteralInteger", "@id": "a", "value": 18446744073709551615u64 }]),
            json!([
                { "@type": "Membership", "@id": "m", "memberElement": [{ "@id": "a" }] },
                { "@type": "PartDefinition", "@id": "a" },
            ]),
        ] {
            // skipped, as `tolerates_nulls_and_foreign_properties` has a
            // list where a flag belongs skipped
            let (model, roots) = from_json(&wrong).unwrap();
            let property = wrong[0]
                .as_object()
                .unwrap()
                .keys()
                .find(|key| !key.starts_with('@'))
                .unwrap();
            assert_eq!(model.get(roots[0], property), None, "{wrong}");
            assert!(as_text(&wrong).is_ok(), "{wrong}");
        }
        // what is right still reads, `null` and an unknown literal as nothing
        let right = json!([{ "@type": "PartDefinition", "@id": "a",
            "isAbstract": true, "declaredName": null, "visibility": "nowhere" }]);
        assert!(from_json(&right).is_ok());
    }

    /// An element that cannot be read is said to be unreadable, and why.
    ///
    /// No failure to read one may come out as a missing `@type`:
    /// `1e999`, which no JSON reader holds, and an `@id` written as a
    /// number would send the reader looking for a `@type` that was there.
    /// And `@type` is read owned, so one spelled with an escape --
    /// `"Pack\u0061ge"`, which is `Package` -- can still be read.
    #[test]
    fn an_element_that_cannot_be_read_says_why() {
        let said = |text: &str| match read_json(text.as_bytes()) {
            Err(error) => error.to_string(),
            Ok(_) => "read".to_string(),
        };
        let huge = said(r#"[{"@id":"a","@type":"Package","declaredName":1e999}]"#);
        assert!(huge.starts_with("element 0 cannot be read:"), "{huge}");
        assert!(huge.contains("number out of range"), "{huge}");
        assert_eq!(
            said(r#"[{"@id":5,"@type":"Package"}]"#),
            r#"element 0 cannot be read: "@id" is 5, not a string"#
        );
        assert_eq!(
            said(r#"[{"@id":"a","@type":["Package"]}]"#),
            r#"element 0 cannot be read: "@type" is ["Package"], not a string"#
        );
        // a `Json` already built says the same of a field that is no string
        assert!(matches!(
            from_json(&json!([{ "@id": 5, "@type": "Package" }])),
            Err(ImportError::Unreadable(0, _))
        ));
        // what is missing is still said to be missing
        assert_eq!(said(r#"[{"@id":"a"}]"#), r#"element 0 has no "@type""#);
        assert_eq!(
            said(r#"[{"@type":"Package"}]"#),
            r#"element 0 has no "@id""#
        );
        // and an escape is a spelling, not a fault
        assert_eq!(said(r#"[{"@id":"a","@type":"Pack\u0061ge"}]"#), "read");
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

    /// A KerML text, resolved, exported, and the objects it wrote.
    fn kerml_objects(source: &str) -> Vec<Json> {
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("test.kerml", source);
        ws.resolve_all();
        let json = to_json(ws.model());
        json.as_array().unwrap().clone()
    }

    /// `owningAnnotatedElement` subsets `owningRelatedElement`: it names
    /// the annotated element only where that element owns the
    /// annotation. `comment about C` is a comment owning its annotation,
    /// and it said C owned it too -- the end it relates, copied into the
    /// name for who holds it.
    #[test]
    fn an_annotation_its_comment_owns_has_no_owning_annotated_element() {
        let objects = kerml_objects("package P { class C; comment about C /* hi */ }\n");
        let comment = objects.iter().find(|it| it["@type"] == "Comment").unwrap();
        let c = objects.iter().find(|it| it["declaredName"] == "C").unwrap();
        let annotation = objects
            .iter()
            .find(|it| it["@type"] == "Annotation")
            .unwrap();
        assert_eq!(annotation["annotatedElement"]["@id"], c["@id"]);
        assert_eq!(annotation["owningAnnotatedElement"], Json::Null);
        assert_eq!(annotation["owningRelatedElement"]["@id"], comment["@id"]);
        assert_eq!(annotation["owningAnnotatingElement"]["@id"], comment["@id"]);
    }

    /// A relationship written as a statement of its own is a member of
    /// the namespace it stands in, through an `OwningMembership`, and is
    /// owned by none of the elements it relates. `subset S::f subsets
    /// S::g;` in a package came out with the package as its
    /// `owningFeature` and `owningRelatedElement`, and no membership; the
    /// same statement in a feature's body counted as that feature's own
    /// subsetting, and the feature inherited from what it named.
    #[test]
    fn a_relationship_written_on_its_own_is_a_member_of_its_namespace() {
        let source = "package P {\n\
                        class C;\n\
                        struct S { feature f; feature g; feature k subsets g; feature h { subset f subsets g; } }\n\
                        subset S::f subsets S::g;\n\
                        specialization Sp subtype S specializes C;\n\
                      }\n";
        let mut ws = sysml_semantics::Workspace::new();
        ws.add_file("test.kerml", source);
        ws.resolve_all();
        let json = to_json(ws.model());
        let objects = json.as_array().unwrap();
        let by_id = |reference: &Json| {
            objects
                .iter()
                .find(|it| it["@id"] == reference["@id"])
                .unwrap()
        };
        let named = |name: &str| {
            objects
                .iter()
                .find(|it| it["declaredName"] == name)
                .unwrap()
        };
        let package = named("P");
        let (f, g, h) = (named("f"), named("g"), named("h"));
        let in_package = objects
            .iter()
            .find(|it| {
                it["@type"] == "Subsetting"
                    && it["subsettingFeature"]["@id"] == f["@id"]
                    && it["subsettedFeature"]["@id"] == g["@id"]
                    && it["owningNamespace"]["@id"] == package["@id"]
            })
            .expect("the subsetting in the package is a member of it");
        let specialization = named("Sp");
        for relationship in [in_package, specialization] {
            let kind = &relationship["@type"];
            for owning in [
                "owningRelatedElement",
                "owningFeature",
                "owningClassifier",
                "owningType",
            ] {
                if let Some(value) = relationship.get(owning) {
                    assert_eq!(value, &Json::Null, "{owning} of a {kind}");
                }
            }
            let membership = by_id(&relationship["owningMembership"]);
            assert_eq!(membership["@type"], "OwningMembership");
            assert_eq!(membership["ownedMemberElement"]["@id"], relationship["@id"]);
            assert_eq!(
                membership["membershipOwningNamespace"]["@id"],
                package["@id"]
            );
            assert_eq!(relationship["owningNamespace"]["@id"], package["@id"]);
            assert_eq!(relationship["owner"]["@id"], package["@id"]);
            assert!(package["ownedMembership"]
                .as_array()
                .unwrap()
                .contains(&relationship["owningMembership"]));
        }
        // in a feature's body the subsetting is that feature's member,
        // and none of its own specializations
        let in_body = objects
            .iter()
            .find(|it| it["@type"] == "Subsetting" && it["owningNamespace"]["@id"] == h["@id"])
            .expect("the subsetting in h's body is a member of h");
        assert_eq!(in_body["owningFeature"], Json::Null);
        assert!(!h["ownedSubsetting"]
            .as_array()
            .unwrap()
            .iter()
            .any(|it| it["@id"] == in_body["@id"]));
        // and an ordinary one is still owned by the feature it is written on
        let k = named("k");
        let own = by_id(&k["ownedSubsetting"][0]);
        assert_eq!(own["subsettedFeature"]["@id"], g["@id"]);
        assert_eq!(own["owningFeature"]["@id"], k["@id"]);
        assert_eq!(own["owningType"]["@id"], k["@id"]);
        assert_eq!(own["owningRelatedElement"]["@id"], k["@id"]);
        assert_eq!(own["owningMembership"], Json::Null);

        // the membership folds back into ownership on the way in
        let (rebuilt, _) = from_json(&json).unwrap();
        assert_eq!(rebuilt.len(), ws.model().len());
        assert_eq!(to_json(&rebuilt), json);
    }

    /// `featuringType` is every `TypeFeaturing`'s `featuringType`, with
    /// the owning type a feature membership makes one. Read off the owner
    /// alone, `feature fb : A featured by B` would be featured by its
    /// struct and never by B, though the document holds the `TypeFeaturing`
    /// saying so -- and `featuring of S::f by A;` standing in the package
    /// would be lost the same way.
    #[test]
    fn a_feature_is_featured_by_what_its_type_featurings_name() {
        let objects = kerml_objects(
            "package P {\n\
               class A; class B;\n\
               struct S { feature f; feature fb : A featured by B; }\n\
               featuring of S::f by A;\n\
             }\n",
        );
        let id = |name: &str| {
            objects
                .iter()
                .find(|it| it["declaredName"] == name)
                .unwrap()["@id"]
                .clone()
        };
        let featuring = |name: &str| -> Vec<Json> {
            objects
                .iter()
                .find(|it| it["declaredName"] == name)
                .unwrap()["featuringType"]
                .as_array()
                .unwrap()
                .iter()
                .map(|it| it["@id"].clone())
                .collect()
        };
        assert_eq!(featuring("fb"), vec![id("S"), id("B")]);
        assert_eq!(featuring("f"), vec![id("S"), id("A")]);
    }

    /// A supertype's `nonPrivateMemberships` include what its public
    /// imports bring in, so a type specializing it inherits those. The
    /// closure counted only memberships the supertype owned: `Derived :>
    /// Base` never inherited the `Lib::X` that `public import Lib::*;` in
    /// `Base` brings in -- the name a reference written in `Derived`
    /// resolves through -- while what `Base` imports privately stays put.
    #[test]
    fn a_subtype_inherits_what_its_supertype_imports_publicly() {
        let (mut ws, model) = resolved(
            "package Q {\n\
               package Lib { part def X; }\n\
               package Other { part def Y; }\n\
               part def Base { public import Lib::*; private import Other::*; }\n\
               part def Derived :> Base;\n\
               part def Further :> Derived;\n\
             }\n",
        );
        let extras = extras_of(&mut ws);
        let json = to_json_with(&model, &extras);
        let objects = json.as_array().unwrap();
        let named = |name: &str| {
            objects
                .iter()
                .find(|it| it["declaredName"] == name)
                .unwrap()
        };
        let x_membership = named("X")["owningMembership"].clone();
        let y_membership = named("Y")["owningMembership"].clone();
        for subtype in ["Derived", "Further"] {
            let inherited = named(subtype)["inheritedMembership"].as_array().unwrap();
            assert!(
                inherited.contains(&x_membership),
                "{subtype} inherits Lib::X"
            );
            assert!(
                !inherited.contains(&y_membership),
                "{subtype} inherits Other::Y"
            );
            let members = named(subtype)["member"].as_array().unwrap();
            assert!(members.iter().any(|it| it["@id"] == named("X")["@id"]));
        }
    }
}
