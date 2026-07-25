//! In-memory element model (abstract syntax) for SysML v2 / KerML.
//!
//! The metamodel — [`ElementKind`] (175 metaclasses), their inheritance
//! hierarchy, feature metadata and enumerations — is generated from the
//! official Ecore definition (see `vendor/metamodel/`, regenerate with
//! `cargo run -p sysml-codegen`).
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
#[rustfmt::skip]
pub mod generated;

pub use build::{build_into, build_model, Built};
pub use generated::{ElementKind, EnumType, FeatureMeta, FeatureType, PrimitiveType};

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
}

#[derive(Clone, Debug)]
struct ElementData {
    kind: ElementKind,
    owner: Option<ElementId>,
    owned: Vec<ElementId>,
    props: Vec<(&'static str, Value)>,
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
        });
        id
    }

    pub fn len(&self) -> usize {
        self.elements.len()
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
    pub fn add_owned(&mut self, parent: ElementId, child: ElementId) {
        if let Some(old) = self.elements[child.index()].owner {
            self.elements[old.index()].owned.retain(|c| *c != child);
        }
        self.elements[child.index()].owner = Some(parent);
        self.elements[parent.index()].owned.push(child);
    }

    /// Set a property. The name is validated against the metamodel; setting
    /// a property the metaclass does not have is an error.
    pub fn set(&mut self, id: ElementId, prop: &str, value: Value) -> &mut Model {
        let kind = self.kind(id);
        let meta = kind
            .feature(prop)
            .unwrap_or_else(|| panic!("{:?} has no property `{prop}`", kind));
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

    /// `declaredName`, the primary name of an element (if any).
    pub fn name(&self, id: ElementId) -> Option<&str> {
        self.get(id, "declaredName").and_then(Value::as_str)
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
}
