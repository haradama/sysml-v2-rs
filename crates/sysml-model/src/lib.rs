//! In-memory element model (abstract syntax) for SysML v2 / KerML.
//!
//! The metamodel — [`ElementKind`] (175 metaclasses), their inheritance
//! hierarchy, feature metadata and enumerations — is generated from the
//! official Ecore definition (see `vendor/metamodel/`, regenerate with
//! `cargo run -p sysml-model --features codegen --bin sysml-codegen`).
//!
//! Elements live in an arena ([`Model`]) and reference each other by
//! [`ElementId`], which mirrors the standard API's UUID-per-element design
//! and sidesteps ownership cycles in the highly cyclic model graph.
//!
//! ```
//! use sysml_model::{ElementKind, Model, Value};
//!
//! let mut model = Model::new();
//! let pkg = model.create(ElementKind::Package);
//! model.set(pkg, "declaredName", Value::String("Vehicles".into()));
//! let part = model.create(ElementKind::PartDefinition);
//! model.add_owned(pkg, part);
//!
//! assert!(ElementKind::PartDefinition.is_a(ElementKind::Classifier));
//! assert_eq!(model.owner(part), Some(pkg));
//! ```

mod build;
/// What writes [`generated`] from the vendored metamodel. Behind the
/// `codegen` feature: it is a development tool, not part of the model.
#[cfg(feature = "codegen")]
pub mod codegen;
#[rustfmt::skip]
pub mod generated;

/// How visible a member is from outside what owns it.
///
/// Written from the keyword that declared it, so that the resolver and
/// the interchange read one answer rather than each working it out from
/// the syntax again -- they used to, and looked at different parts of
/// it: one saw a `private` on the wrapper of a declaration, the other
/// only on the declaration itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Vis {
    /// `public`, and what nothing written means for a member
    Public,
    /// `protected` -- inherited, but not reachable from outside
    Protected,
    /// `private`, and what nothing written means for an import
    Private,
}

/// What a membership makes of the element it owns.
///
/// The keyword that declared the member decides it here, and two other
/// crates read it back: the resolver, to know which members stand for
/// the ones their type declares, and the interchange, to write the
/// membership metaclass the standard names for each. It is an enum
/// rather than a string so that adding one is a compile error in both
/// until they say what it means -- the three used to spell the same
/// dozen words separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// `subject x;` -- what a requirement, use case or verification is about
    Subject,
    /// `actor a;`
    Actor,
    /// `stakeholder s;`
    Stakeholder,
    /// `objective o;` -- what a case is trying to establish
    Objective,
    /// `variant v;` -- one alternative of a variation
    Variant,
    /// `return x;` -- a behaviour's result parameter
    Return,
    /// the trailing expression of a calculation body
    Result,
    /// `entry action a;`
    Entry,
    /// `do action a;`
    Do,
    /// `exit action a;`
    Exit,
    /// `assume constraint c;`
    Assume,
    /// `require constraint c;`
    Require,
    /// `frame concern c;`
    Frame,
    /// `verify r;` -- the requirement a verification case answers for
    Verify,
    /// `render asTreeDiagram;` -- how a view is drawn
    Render,
}

/// The membership an owned element is owned through.
///
/// The standard's abstract syntax picks a metaclass by what the member is
/// to its owner: a declared role names it outright, a connector end gets an
/// `EndFeatureMembership`, a directed feature of a behavior is a parameter,
/// a trigger/guard/effect of a transition is a transition feature, any
/// other feature of a type sits behind a `FeatureMembership`, and anything
/// else behind a plain `OwningMembership`.
///
/// The model holds ownership directly and keeps the membership's own
/// facts on the member ([`MemberSide`]), so whatever needs the standard's
/// view of it -- an interchange writer, the constraint checker -- puts
/// the membership back together from here rather than each its own way.
pub fn membership_kind(model: &Model, owned: ElementId) -> ElementKind {
    if let Some(role) = model.member_role(owned) {
        // no catch-all: a role added to the model is a compile error
        // here until it says which membership the standard names for it
        return match role {
            Role::Subject => ElementKind::SubjectMembership,
            Role::Actor => ElementKind::ActorMembership,
            Role::Stakeholder => ElementKind::StakeholderMembership,
            Role::Objective => ElementKind::ObjectiveMembership,
            Role::Variant => ElementKind::VariantMembership,
            Role::Return => ElementKind::ReturnParameterMembership,
            Role::Result => ElementKind::ResultExpressionMembership,
            Role::Entry | Role::Do | Role::Exit => ElementKind::StateSubactionMembership,
            Role::Assume | Role::Require => ElementKind::RequirementConstraintMembership,
            Role::Frame => ElementKind::FramedConcernMembership,
            Role::Verify => ElementKind::RequirementVerificationMembership,
            Role::Render => ElementKind::ViewRenderingMembership,
        };
    }
    let owner_kind = match model.owner(owned) {
        Some(owner) => model.kind(owner),
        None => return ElementKind::OwningMembership,
    };
    // Not everything a type owns is a feature of it. KerML writes the
    // `[0..*]` of a type as `OwnedMultiplicity : OwningMembership` and a
    // `#Name` prefix as `PrefixMetadataMember : OwningMembership`, and a
    // metadata usage annotates what it is written on rather than being
    // part of it either way. That is what
    // `validateDefinitionVariationOwnedFeatureMembership` -- "a
    // variation owns no feature membership" -- says of `#Security enum
    // def C` and of `variation part x[1]`.
    if !model.kind(owned).is_a(ElementKind::Feature)
        || model.kind(owned).is_a(ElementKind::Multiplicity)
        || model.kind(owned).is_a(ElementKind::MetadataUsage)
        || !owner_kind.is_a(ElementKind::Type)
    {
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
pub fn transition_role(model: &Model, owned: ElementId) -> Option<&'static str> {
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

pub use build::BUILT_FLAGS;
pub use build::{build_into, build_model, Built};
pub use generated::{
    ElementKind, EnumType, FeatureMeta, FeatureType, Operation, PrimitiveType, Rule, DERIVATIONS,
    OPERATIONS, RULES,
};

/// Whether a value has the shape the metamodel gives a property.
///
/// A primitive property holds a value of that primitive type and an
/// enumerated one the literal it names. A property whose type is a class
/// holds a reference either way -- a list only where the metamodel gives
/// it an upper bound above one, though a single reference stands for a
/// list of one and every reader takes it as such.
fn fits(meta: &FeatureMeta, value: &Value) -> bool {
    match (meta.ty, value) {
        (FeatureType::Data(PrimitiveType::Boolean), Value::Bool(_)) => true,
        (
            FeatureType::Data(PrimitiveType::Integer | PrimitiveType::UnlimitedNatural),
            Value::Int(_),
        ) => true,
        (FeatureType::Data(PrimitiveType::Real), Value::Real(_)) => true,
        (FeatureType::Data(PrimitiveType::String), Value::String(_)) => true,
        (FeatureType::Enumeration(_), Value::EnumLit(_)) => true,
        (FeatureType::Class(_), Value::Ref(_)) => true,
        (FeatureType::Class(_), Value::RefList(_)) => meta.many,
        _ => false,
    }
}

/// Identifies an element within one [`Model`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ElementId(u32);

impl ElementId {
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A property value on an element.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Real(f64),
    String(String),
    /// A literal of one of the metamodel enumerations (e.g. `"private"`).
    EnumLit(&'static str),
    Ref(ElementId),
    RefList(Vec<ElementId>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            Value::EnumLit(s) => Some(s),
            _ => None,
        }
    }

    /// The element a single reference points at.
    pub fn as_id(&self) -> Option<ElementId> {
        match self {
            Value::Ref(id) => Some(*id),
            _ => None,
        }
    }

    /// The elements a multi-valued reference points at.
    pub fn as_ids(&self) -> Option<&[ElementId]> {
        match self {
            Value::RefList(ids) => Some(ids),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
struct ElementData {
    kind: ElementKind,
    owner: Option<ElementId>,
    owned: Vec<ElementId>,
    props: Vec<(&'static str, Value)>,
    /// What the membership owning this element says, when it says anything.
    ///
    /// The model holds ownership directly; the standard holds it through a
    /// `Membership` that can carry a visibility and be of a more specific
    /// metaclass. Both halves of that information live here, on the owned
    /// element, so a serializer can put the membership back together.
    membership: MemberSide,
}

/// The membership-borne facts about one owned element.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct MemberSide {
    /// What the member was declared with; `None` is nothing written.
    visibility: Option<Vis>,
    /// The syntactic role that picks the membership's metaclass, when
    /// the member was declared in one. See [`Role`] for the roles.
    role: Option<Role>,
}

/// Arena holding every element of one model.
#[derive(Clone, Debug, Default)]
pub struct Model {
    elements: Vec<ElementData>,
}

impl Model {
    pub fn new() -> Model {
        Model::default()
    }

    pub fn create(&mut self, kind: ElementKind) -> ElementId {
        let id = ElementId(u32::try_from(self.elements.len()).expect("model too large"));
        self.elements.push(ElementData {
            kind,
            owner: None,
            owned: Vec::new(),
            props: Vec::new(),
            membership: MemberSide::default(),
        });
        id
    }

    pub fn len(&self) -> usize {
        self.elements.len()
    }

    /// Record the visibility written for the membership owning `id`.
    ///
    /// `public` is the default for a member, so writing it says nothing
    /// new -- but the source did write it, and an interchange that says
    /// so writes what the author wrote.
    pub fn set_member_visibility(&mut self, id: ElementId, visibility: Vis) {
        self.elements[id.index()].membership.visibility = Some(visibility);
    }

    /// The visibility of the membership owning `id`, when one was declared.
    pub fn member_visibility(&self, id: ElementId) -> Option<Vis> {
        self.elements[id.index()].membership.visibility
    }

    /// Record the syntactic role `id` was declared in, which decides the
    /// metaclass of the membership owning it. See [`Role`].
    pub fn set_member_role(&mut self, id: ElementId, role: Role) {
        self.elements[id.index()].membership.role = Some(role);
    }

    /// The declared role of `id`, when it has one.
    pub fn member_role(&self, id: ElementId) -> Option<Role> {
        self.elements[id.index()].membership.role
    }

    pub fn is_empty(&self) -> bool {
        self.elements.is_empty()
    }

    pub fn ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        (0..self.elements.len() as u32).map(ElementId)
    }

    pub fn kind(&self, id: ElementId) -> ElementKind {
        self.elements[id.index()].kind
    }

    pub fn owner(&self, id: ElementId) -> Option<ElementId> {
        self.elements[id.index()].owner
    }

    pub fn owned(&self, id: ElementId) -> &[ElementId] {
        &self.elements[id.index()].owned
    }

    /// Make `child` an owned element of `parent` (removing any prior owner).
    ///
    /// Ownership is a tree. An element made to own itself, or one of its
    /// own ancestors, would send every walk of that tree round for ever,
    /// so closing such a loop is refused here, at the one place that
    /// writes ownership, rather than guarded against by every reader.
    /// What builds a model from foreign data checks for it first.
    pub fn add_owned(&mut self, parent: ElementId, child: ElementId) {
        let mut ancestor = Some(parent);
        while let Some(id) = ancestor {
            assert!(
                id != child,
                "{child:?} would own {parent:?}, which is itself or one of its own ancestors"
            );
            ancestor = self.owner(id);
        }
        if let Some(old) = self.elements[child.index()].owner {
            self.elements[old.index()].owned.retain(|c| *c != child);
        }
        self.elements[child.index()].owner = Some(parent);
        self.elements[parent.index()].owned.push(child);
    }

    /// Set a property. The name is validated against the metamodel; setting
    /// a property the metaclass does not have is an error.
    ///
    /// So is a value of the wrong shape, where the assertions are on: the
    /// metamodel says what each property holds, and until this checked it
    /// `set(part, "isAbstract", Value::String("yes"))` was accepted and
    /// written back out as a string where every reader expects a boolean.
    pub fn set(&mut self, id: ElementId, prop: &str, value: Value) -> &mut Model {
        let kind = self.kind(id);
        let meta = kind
            .feature(prop)
            .unwrap_or_else(|| panic!("{:?} has no property `{prop}`", kind));
        debug_assert!(
            fits(meta, &value),
            "{kind:?}::{prop} holds {meta:?}, not {value:?}"
        );
        let data = &mut self.elements[id.index()];
        if let Some(slot) = data.props.iter_mut().find(|(n, _)| *n == meta.name) {
            slot.1 = value;
        } else {
            data.props.push((meta.name, value));
        }
        self
    }

    pub fn get(&self, id: ElementId, prop: &str) -> Option<&Value> {
        self.elements[id.index()]
            .props
            .iter()
            .find(|(n, _)| *n == prop)
            .map(|(_, v)| v)
    }

    pub fn props(&self, id: ElementId) -> impl Iterator<Item = (&'static str, &Value)> {
        self.elements[id.index()].props.iter().map(|(n, v)| (*n, v))
    }

    /// What a feature is typed by, in the order the typings were
    /// written.
    ///
    /// A typing is not a property on the feature: parsing reifies each
    /// one as an owned [`ElementKind::FeatureTyping`] whose `type` is
    /// filled in once the name resolves, which is where every reader of
    /// a type has to look. A typing that never resolved is skipped, so
    /// this yields fewer answers than the model wrote.
    pub fn types_of(&self, feature: ElementId) -> impl Iterator<Item = ElementId> + '_ {
        self.owned(feature)
            .iter()
            .filter(|&&child| self.kind(child) == ElementKind::FeatureTyping)
            .filter_map(|&child| self.get(child, "type").and_then(Value::as_id))
    }

    /// The first of [`Model::types_of`], which is the only one for
    /// every feature that names a single type.
    pub fn type_of(&self, feature: ElementId) -> Option<ElementId> {
        self.types_of(feature).next()
    }

    /// `declaredName`, the primary name of an element (if any).
    pub fn name(&self, id: ElementId) -> Option<&str> {
        self.get(id, "declaredName").and_then(Value::as_str)
    }

    /// The name an element answers to.
    ///
    /// Its `declaredName`; or, for a feature that declares neither a name
    /// nor a short name, the name of the feature it is named after --
    /// KerML's `effectiveName()`, by which `attribute :>> mass;` is a
    /// feature named `mass`. Declaring only a short name is still
    /// declaring: such a feature borrows nothing and has no name. The
    /// rule follows a chain of borrowed names to the one that finally
    /// declares something, and a chain that comes back round to where it
    /// began -- illegal, but representable -- names nothing.
    ///
    /// KerML names a feature after what it redefines; SysML adds what it
    /// references, for a variant, a requirement's `assume`/`require`, and
    /// a `perform`. This follows any reference subsetting rather than
    /// those three cases, which is what the resolver has always done and
    /// what the whole corpus resolves under -- `part ::> v;` answers to
    /// `v` wherever it is written.
    ///
    /// It reads the reified `Redefinition` and `ReferenceSubsetting`
    /// elements, so it answers only once those have been resolved. The
    /// interchange, the diagram and the Rust generator each kept a copy
    /// of this rule, and no two of them agreed on the reference case.
    pub fn effective_name(&self, id: ElementId) -> Option<&str> {
        self.named_after(id, "declaredName")
    }

    /// [`Model::effective_name`], for the short name.
    pub fn effective_short_name(&self, id: ElementId) -> Option<&str> {
        self.named_after(id, "declaredShortName")
    }

    /// The element whose declaration [`Model::effective_name`] reads: `id`
    /// itself where it declares a name, and otherwise the far end of the
    /// chain of features it borrows one along.
    ///
    /// What renames a feature has to reach this one. `part :>> component;`
    /// declares no name of its own, so there is nothing in it to rewrite:
    /// an edit that treated it as the declaration would replace the whole
    /// line with the new name.
    pub fn naming_element(&self, id: ElementId) -> Option<ElementId> {
        let mut at = id;
        let mut visited = std::collections::HashSet::new();
        loop {
            // one that declared either name is named, and borrows nothing
            let declares = ["declaredName", "declaredShortName"]
                .iter()
                .any(|declared| self.get(at, declared).is_some());
            if declares {
                return Some(at);
            }
            if !visited.insert(at) {
                return None;
            }
            at = self.owned(at).iter().find_map(|&rel| {
                let target = match self.kind(rel) {
                    ElementKind::Redefinition => "redefinedFeature",
                    ElementKind::ReferenceSubsetting => "referencedFeature",
                    _ => return None,
                };
                self.get(rel, target).and_then(Value::as_id)
            })?;
        }
    }

    fn named_after(&self, id: ElementId, prop: &str) -> Option<&str> {
        let names = self.naming_element(id)?;
        self.get(names, prop).and_then(Value::as_str)
    }

    /// Depth-first traversal of the ownership tree from `root`.
    pub fn descendants(&self, root: ElementId) -> Vec<ElementId> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            out.push(id);
            stack.extend(self.owned(id).iter().rev().copied());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_hierarchy() {
        use ElementKind::*;
        assert!(PartDefinition.is_a(ItemDefinition));
        assert!(PartDefinition.is_a(Classifier));
        assert!(PartDefinition.is_a(Element));
        assert!(!Classifier.is_a(PartDefinition));
        assert!(PartUsage.is_a(Usage));
        assert!(PartUsage.is_a(Feature));
        assert!(Element.is_abstract());
        assert_eq!(
            ElementKind::from_name("PartDefinition"),
            Some(PartDefinition)
        );
    }

    #[test]
    fn feature_metadata() {
        use ElementKind::*;
        let f = Element.feature("declaredName").unwrap();
        assert_eq!(f.ty, FeatureType::Data(PrimitiveType::String));
        assert!(!f.many);
        // inherited lookup
        assert!(PartDefinition.feature("declaredName").is_some());
        // memberships are containments of relationships
        let f = Namespace.feature("ownedMembership").unwrap();
        assert!(f.many);
        assert!(f.derived);
    }

    #[test]
    fn a_type_is_read_off_the_typing_that_resolved() {
        let mut model = Model::new();
        let part = model.create(ElementKind::PartUsage);
        let definition = model.create(ElementKind::PartDefinition);
        assert_eq!(model.type_of(part), None);

        // a typing whose name never resolved has no answer to give
        let unresolved = model.create(ElementKind::FeatureTyping);
        model.add_owned(part, unresolved);
        assert_eq!(model.type_of(part), None);

        let typing = model.create(ElementKind::FeatureTyping);
        model.set(typing, "type", Value::Ref(definition));
        model.add_owned(part, typing);
        assert_eq!(model.type_of(part), Some(definition));
        assert_eq!(model.types_of(part).count(), 1);

        // only a reference names an element
        assert_eq!(Value::Ref(definition).as_id(), Some(definition));
        assert_eq!(Value::Bool(true).as_id(), None);
        assert_eq!(
            Value::RefList(vec![definition]).as_ids(),
            Some([definition].as_slice())
        );
        assert_eq!(Value::Ref(definition).as_ids(), None);
    }

    #[test]
    fn arena_ownership() {
        let mut model = Model::new();
        let pkg = model.create(ElementKind::Package);
        let part = model.create(ElementKind::PartDefinition);
        model.add_owned(pkg, part);
        model.set(part, "declaredName", Value::String("Vehicle".into()));
        model.set(part, "isAbstract", Value::Bool(true));

        assert_eq!(model.owner(part), Some(pkg));
        assert_eq!(model.owned(pkg), &[part]);
        assert_eq!(model.name(part), Some("Vehicle"));
        assert_eq!(model.descendants(pkg).len(), 2);
    }

    #[test]
    fn value_helpers_and_reparenting() {
        assert_eq!(Value::Bool(true).as_str(), None);
        assert_eq!(Value::EnumLit("private").as_str(), Some("private"));

        let mut model = Model::new();
        assert!(model.is_empty());
        let a = model.create(ElementKind::Package);
        let b = model.create(ElementKind::Package);
        let child = model.create(ElementKind::PartDefinition);
        assert!(!model.is_empty());
        model.add_owned(a, child);
        model.add_owned(b, child); // re-parent
        assert_eq!(model.owned(a), &[]);
        assert_eq!(model.owner(child), Some(b));
        // overwriting a property keeps a single slot
        model.set(child, "declaredName", Value::String("x".into()));
        model.set(child, "declaredName", Value::String("y".into()));
        assert_eq!(model.name(child), Some("y"));
        assert_eq!(model.props(child).count(), 1);
    }

    #[test]
    #[should_panic(expected = "has no property")]
    fn unknown_property_panics() {
        let mut model = Model::new();
        let pkg = model.create(ElementKind::Package);
        model.set(pkg, "notAProperty", Value::Bool(true));
    }

    #[test]
    #[should_panic(expected = "itself or one of its own ancestors")]
    fn an_element_cannot_own_itself() {
        let mut model = Model::new();
        let pkg = model.create(ElementKind::Package);
        model.add_owned(pkg, pkg);
    }

    #[test]
    #[should_panic(expected = "itself or one of its own ancestors")]
    fn an_element_cannot_own_its_own_ancestor() {
        let mut model = Model::new();
        let outer = model.create(ElementKind::Package);
        let inner = model.create(ElementKind::Package);
        let leaf = model.create(ElementKind::PartDefinition);
        model.add_owned(outer, inner);
        model.add_owned(inner, leaf);
        // closing the loop three levels up is refused the same way
        model.add_owned(leaf, outer);
    }

    fn named_feature(model: &mut Model, name: &str) -> ElementId {
        let feature = model.create(ElementKind::AttributeUsage);
        model.set(feature, "declaredName", Value::String(name.into()));
        feature
    }

    fn borrowing(model: &mut Model, kind: ElementKind, prop: &str, from: ElementId) -> ElementId {
        let feature = model.create(ElementKind::AttributeUsage);
        let rel = model.create(kind);
        model.set(rel, prop, Value::Ref(from));
        model.add_owned(feature, rel);
        feature
    }

    #[test]
    fn an_unnamed_feature_answers_to_the_name_of_what_it_redefines_or_references() {
        let mut model = Model::new();
        let mass = named_feature(&mut model, "mass");
        model.set(mass, "declaredShortName", Value::String("m".into()));
        assert_eq!(model.effective_name(mass), Some("mass"));
        assert_eq!(model.effective_short_name(mass), Some("m"));

        // `attribute :>> mass;`
        let redefining = borrowing(
            &mut model,
            ElementKind::Redefinition,
            "redefinedFeature",
            mass,
        );
        assert_eq!(model.effective_name(redefining), Some("mass"));
        assert_eq!(model.effective_short_name(redefining), Some("m"));

        // `ref ::> <the redefining one>` -- a chain, through a reference
        let referencing = borrowing(
            &mut model,
            ElementKind::ReferenceSubsetting,
            "referencedFeature",
            redefining,
        );
        assert_eq!(model.effective_name(referencing), Some("mass"));

        // a redefinition whose target never resolved names nothing
        let unresolved = model.create(ElementKind::AttributeUsage);
        let rel = model.create(ElementKind::Redefinition);
        model.add_owned(unresolved, rel);
        assert_eq!(model.effective_name(unresolved), None);

        // `attribute :> Mass;` -- a subsetting says what a feature is a
        // kind of, not what it is called, so there is no name to borrow
        let subsetting = borrowing(
            &mut model,
            ElementKind::Subsetting,
            "subsettedFeature",
            mass,
        );
        assert_eq!(model.effective_name(subsetting), None);

        // one that declares only a short name is named, and borrows nothing
        let short = model.create(ElementKind::AttributeUsage);
        model.set(short, "declaredShortName", Value::String("s".into()));
        let rel = model.create(ElementKind::Redefinition);
        model.set(rel, "redefinedFeature", Value::Ref(mass));
        model.add_owned(short, rel);
        assert_eq!(model.effective_name(short), None);
        assert_eq!(model.effective_short_name(short), Some("s"));
    }

    #[test]
    fn a_redefinition_cycle_names_nothing_rather_than_never_answering() {
        let mut model = Model::new();
        let a = model.create(ElementKind::AttributeUsage);
        let b = borrowing(&mut model, ElementKind::Redefinition, "redefinedFeature", a);
        let rel = model.create(ElementKind::Redefinition);
        model.set(rel, "redefinedFeature", Value::Ref(b));
        model.add_owned(a, rel);
        assert_eq!(model.effective_name(a), None);
        assert_eq!(model.effective_name(b), None);
    }

    /// The metamodel says what each property holds. A value of another
    /// shape used to be kept and written back out as itself -- an
    /// `isAbstract` of `"yes"` where every reader expects a boolean.
    ///
    /// Only where the assertions are on: this is a check on the code
    /// that builds a model, not on the models it builds.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "PartDefinition::isAbstract")]
    fn a_property_refuses_a_value_of_the_wrong_shape() {
        let mut model = Model::new();
        let part = model.create(ElementKind::PartDefinition);
        model.set(part, "isAbstract", Value::String("yes".to_string()));
    }

    /// There is no longer a role this does not know: `Role` is an
    /// enum and the match over it has no catch-all, so one added to the
    /// model does not fall through to plain ownership -- it stops the
    /// build until this says which membership the standard names for it.
    /// What is left to check is the element that has no role at all.
    #[test]
    fn a_member_with_no_role_is_owned_plainly() {
        let mut model = Model::new();
        let package = model.create(ElementKind::Package);
        let part = model.create(ElementKind::PartUsage);
        model.add_owned(package, part);
        assert_eq!(membership_kind(&model, part), ElementKind::OwningMembership);
        // and an element with no owner at all needs no membership either
        let loose = model.create(ElementKind::PartUsage);
        assert_eq!(
            membership_kind(&model, loose),
            ElementKind::OwningMembership
        );
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
}
