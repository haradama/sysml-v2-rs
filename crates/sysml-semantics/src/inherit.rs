//! What a type reaches through what it specializes.
//!
//! A namespace's members are not only the ones it declares: it inherits
//! whatever its supertypes have, and a feature inherits through the types
//! it is declared with, which is what makes `engine.mass` resolve. Working
//! out the supertypes brings in the implicit ones too -- the
//! semantic-library type the standard maps each metaclass to, and the
//! `baseType` a user-defined keyword names through its metadata.
//!
//! The answers are cached: a lookup asks once per namespace per segment,
//! and the standard library specializes deeply.

use sysml_model::{ElementId, ElementKind, Role, Value};
use sysml_syntax::{is_name_chain, SyntaxKind, SyntaxNode};

use crate::lookup::Reach;
use crate::syntax::*;
use crate::{push_supertype, reaches, Access, Workspace};

impl Workspace {
    /// Whether a feature specializes another, however far up.
    pub(crate) fn reaches(&mut self, feature: ElementId, other: ElementId) -> bool {
        let mut queue = vec![feature];
        let mut seen = Vec::new();
        let mut at = 0;
        while at < queue.len() {
            let up = queue[at];
            at += 1;
            if up == other {
                return true;
            }
            if seen.contains(&up) {
                continue;
            }
            seen.push(up);
            queue.extend(self.supertypes_of(up));
        }
        false
    }
    /// What an element specializes, as the resolver worked it out.
    ///
    /// The text of a model does not say this: a definition's shape is
    /// what it inherits as much as what it declares, and only a resolved
    /// workspace knows which is which.
    pub fn supertypes(&mut self, elem: ElementId) -> Vec<ElementId> {
        self.supertypes_of(elem)
    }
    /// Supertypes of an element for inherited-member lookup: resolved
    /// targets of its typings, specializations, subsettings, redefinitions,
    /// plus the implicit base type from the semantic libraries.
    pub(crate) fn supertypes_of(&mut self, elem: ElementId) -> Vec<ElementId> {
        if let Some(cached) = self.supertypes.get(&elem) {
            return cached.clone();
        }
        if !self.in_progress.insert(elem) {
            // this element's supertypes are already being computed —
            // break the specialization cycle
            return Vec::new();
        }
        let cut = self.misses;
        let refused = self.blocked;
        let mut supers = Vec::new();
        if let Some(node) = self.source.get(&elem).cloned() {
            // an `@name` metadata usage inherits the definition's members
            if let Some(target) = metadata_target(&node) {
                if let Some(def) = self.resolve_from(elem, &target.segments) {
                    push_supertype(&mut supers, elem, def);
                }
            }
            for (part, targets) in relationship_parts(&node) {
                // `classifier U unions A, B;` says which things a `U`
                // is one of; it does not say a `U` is an `A`.
                // `unions`, `intersects` and `differences` narrow an
                // extent rather than specialize a type, and nothing is
                // inherited through them: read as specializations, a
                // connector typed by a union of two associations had
                // the ends of both.
                if matches!(
                    part,
                    SyntaxKind::UNIONS_KW | SyntaxKind::INTERSECTS_KW | SyntaxKind::DIFFERENCES_KW
                ) {
                    continue;
                }
                for t in targets {
                    if let Some(target) = self.resolve_from(elem, &t.segments) {
                        push_supertype(&mut supers, elem, target);
                    }
                }
            }
            // `perform vehicleMassTest.collectData { :>> param }` — the
            // performed/exhibited/included target contributes its members
            if let Some(segments) = adapter_target_segments(&node) {
                if let Some(target) = self.resolve_from(elem, &segments) {
                    push_supertype(&mut supers, elem, target);
                }
            }
            // `#cause 'battery old' { ... }` and `class C1 { @B; }` --
            // semantic metadata makes what it annotates specialize the
            // metaclass's `baseType`
            for base in self.semantic_bases_of(elem) {
                push_supertype(&mut supers, elem, base);
            }
            // `feature b = a#(1).b;` is one of the `b`s `a#(1)` has
            if let Some(target) = self.valued_by(elem, &node) {
                push_supertype(&mut supers, elem, target);
            }
        }
        // `port def P` defines its conjugate as well, and `~P` has
        // what `P` has: conjugating a type reverses the direction of
        // its features, not which features it has. The conjugate is
        // reified and has no syntax to read, so what it is the
        // conjugate of is read off the conjugation it owns -- and
        // without it `apsc.subscr` names nothing where `apsc` is a
        // port typed `~SubscriptionPort`.
        let conjugated: Vec<ElementId> = self
            .model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&owned| self.model.kind(owned).is_a(ElementKind::Conjugation))
            .filter_map(|owned| match self.model.get(owned, "originalType") {
                Some(Value::Ref(target)) => Some(*target),
                _ => None,
            })
            .collect();
        for target in conjugated {
            push_supertype(&mut supers, elem, target);
        }
        // A relationship the standard implies is written into the model
        // and nowhere else, so the source cannot answer for it -- an
        // owned cross feature subsets the cross feature of the end its
        // owner redefines, and nothing in the notation says so. See
        // `imply_cross_subsettings`.
        let implied: Vec<ElementId> = self
            .model
            .owned(elem)
            .iter()
            .copied()
            .filter(|&owned| {
                self.model.kind(owned).is_a(ElementKind::Specialization)
                    && self.model.flag(owned, "isImplied")
            })
            .filter_map(|owned| {
                // a specialization of any kind: a cast is typed by what
                // it casts to, and a typing is a specialization like a
                // subsetting is
                [
                    "redefinedFeature",
                    "subsettedFeature",
                    "type",
                    "superclassifier",
                    "general",
                ]
                .iter()
                .find_map(|named| self.model.maybe(owned, named).and_then(Value::as_id))
            })
            .collect();
        for target in implied {
            push_supertype(&mut supers, elem, target);
        }
        // An element the builder reified has no syntax of its own. An
        // accept node's payload is one: `accept cl : Cmd` declares it on
        // the statement, and the typing written there is attached to the
        // payload once it resolves. Read back from the model it is a
        // supertype like any other, and without it `cl.itms` names
        // nothing.
        if supers.is_empty() {
            let typed: Vec<ElementId> = self
                .model
                .owned(elem)
                .iter()
                .copied()
                .filter(|&owned| self.model.kind(owned) == ElementKind::FeatureTyping)
                .filter_map(|owned| match self.model.get(owned, "type") {
                    Some(Value::Ref(target)) => Some(*target),
                    _ => None,
                })
                .collect();
            for target in typed {
                push_supertype(&mut supers, elem, target);
            }
        }
        // A feature can redeclare an inherited one by naming it the
        // same: `in p { ... }` inside a specialization stands for the
        // `p` its supertype declares, and inherits what that one has.
        // Only where nothing else was written about it -- an explicit
        // typing or subsetting is the whole story.
        if supers.is_empty() {
            if let (Some(name), Some(owner)) = (
                self.model.name(elem).map(String::from),
                self.model.owner(elem),
            ) {
                for sup in self.supertypes_of(owner) {
                    for sibling in self.model.owned(sup).to_vec() {
                        if self.model.name(sibling) == Some(name.as_str()) {
                            push_supertype(&mut supers, elem, sibling);
                        }
                    }
                }
            }
        }
        // `variant manualTransmission;` names one of the usages the model already
        // has; `variant part v;` declares a new one, and the difference is
        // whether a kind keyword was written.
        // An enumeration value is a variant that *declares* the value rather than
        // naming one written elsewhere: `enum def E1 { a; b; c; }` has no `a`
        // anywhere else, and looking for one reaches past the enumeration to
        // whatever else the workspace calls `a`.
        let enumerated = self.model.owner(elem).is_some_and(|owner| {
            self.model
                .kind(owner)
                .is_a(ElementKind::EnumerationDefinition)
        });
        if supers.is_empty() && !enumerated && self.model.member_role(elem) == Some(Role::Variant) {
            let bare = self.source.get(&elem).is_some_and(|node| {
                !node
                    .children_with_tokens()
                    .filter_map(|e| e.into_token())
                    .any(|t| t.kind().is_def_kind_kw())
            });
            if let (true, Some(name)) = (bare, self.model.name(elem).map(String::from)) {
                if let Some(target) = self.resolve_from(elem, &[name]) {
                    push_supertype(&mut supers, elem, target);
                }
            }
        }
        // A usage has at most one `subject` and one `objective`, so the
        // one it writes stands for the one its type declares -- which is
        // what `objective { verify x :>> massRequirement; }` redefines a
        // member of. Nothing says so in the text; the roles do.
        if let Some(role) = self.model.member_role(elem) {
            if matches!(role, Role::Subject | Role::Objective) {
                let owners: Vec<ElementId> = self.model.owner(elem).into_iter().collect();
                for owner in owners {
                    for sup in self.supertypes_of(owner) {
                        for sibling in self.model.owned(sup).to_vec() {
                            if self.model.member_role(sibling) == Some(role) {
                                push_supertype(&mut supers, elem, sibling);
                            }
                        }
                    }
                }
            }
        }
        for path in self.implied_bases_of(elem) {
            // From the root: these are the standard library's own
            // names, and a model is free to declare a package called
            // `Requirements` of its own -- `SimpleVehicleModel` does --
            // which would otherwise stand in front of the library's and
            // leave the usage specializing nothing.
            let segments: Vec<String> = std::iter::once(String::new())
                .chain(path.split("::").map(String::from))
                .collect();
            if let Some(target) = self.resolve_from(elem, &segments) {
                if target != elem
                    && !supers.contains(&target)
                    && !reaches(&self.model, target, elem)
                {
                    supers.push(target);
                }
            }
        }
        self.in_progress.remove(&elem);
        if self.misses != cut {
            self.incomplete.insert(elem);
        }
        // A name this walk asked for was refused by the guard that
        // stops an import from resolving itself, so a supertype may be
        // missing from the list for no reason but the order things
        // were asked in. Kept for the reference under way and worked
        // out again for the next one.
        if self.blocked != refused {
            self.provisional.insert(elem);
        }
        self.supertypes.insert(elem, supers.clone());
        supers
    }
    /// What the semantic metadata annotating an element makes it specialize.
    ///
    /// `checkMetadataFeatureSemanticSpecialization`: a metadata feature whose
    /// metaclass has a `baseType` annotates a type that must specialize it,
    /// written either as a keyword in front of the declaration or as a member
    /// of its body. The base is taken as the standard maps it: a classifier
    /// annotated with a feature base specializes the *types* of that feature,
    /// since a classifier subsets nothing; the other three pairings take the
    /// base as it is.
    pub(crate) fn semantic_bases_of(&mut self, elem: ElementId) -> Vec<ElementId> {
        let Some(node) = self.source.get(&elem).cloned() else {
            return Vec::new();
        };
        let mut annotations: Vec<(ElementId, Vec<String>)> = prefix_metadata_segments(&node)
            .into_iter()
            .map(|segments| (elem, segments))
            .collect();
        for child in self.model.owned(elem).to_vec() {
            if !self.model.kind(child).is_a(ElementKind::MetadataFeature) {
                continue;
            }
            if let Some(target) = self.source.get(&child).and_then(metadata_target) {
                annotations.push((child, target.segments));
            }
        }
        let a_classifier = self.model.kind(elem).is_a(ElementKind::Classifier);
        let mut bases = Vec::new();
        for (from, segments) in annotations {
            let Some(meta_def) = self.resolve_from(from, &segments) else {
                continue;
            };
            for base in self.semantic_bases(meta_def) {
                let a_feature = self.model.kind(base).is_a(ElementKind::Feature);
                if a_classifier && a_feature {
                    let types = self.supertypes_of(base);
                    bases.extend(
                        types
                            .into_iter()
                            .filter(|&it| self.model.kind(it).is_a(ElementKind::Classifier)),
                    );
                } else {
                    bases.push(base);
                }
            }
        }
        bases
    }
    /// What a feature's value makes it subset.
    ///
    /// `checkFeatureValuationSpecialization`: a feature with a value that is
    /// not a default, no direction and no specialization of its own subsets
    /// the result of the expression it is bound to. That result has nothing to
    /// say for itself until the expression tree has been walked, and a name
    /// written after the feature -- `feature c = b.c;` -- is looked up before
    /// then, so what the value comes to is read off its text here.
    fn valued_by(&mut self, elem: ElementId, node: &SyntaxNode) -> Option<ElementId> {
        if !self.model.kind(elem).is_a(ElementKind::Feature)
            || self.model.maybe(elem, "direction").is_some()
            || !relationship_parts(node).is_empty()
            // `binding a = b;` writes its second end where a value goes
            || binds_an_end(node)
        {
            return None;
        }
        let value = node.children().find(|it| it.kind() == SyntaxKind::VALUE)?;
        let is_default = value
            .children_with_tokens()
            .filter_map(|it| it.into_token())
            .any(|it| it.kind() == SyntaxKind::DEFAULT_KW);
        if is_default {
            return None;
        }
        let written = value.children().find(|it| it.kind() != SyntaxKind::BODY)?;
        self.comes_to(elem, &written)
    }
    /// The feature an expression comes to, where its text says which.
    ///
    /// A name comes to what it names; `a#(1).b` chains `b` from what `a#(1)`
    /// comes to; and `a#(1)` is one of the `a`s, unless `a` is a collection,
    /// whose `#` picks an element out of it. Anything else comes to nothing a
    /// name could be looked up in.
    fn comes_to(&mut self, elem: ElementId, written: &SyntaxNode) -> Option<ElementId> {
        let found = match written.kind() {
            SyntaxKind::NAME_REF => self.resolve_operand(elem, &operand_segments(written)),
            SyntaxKind::PATH_EXPR if is_name_chain(written) => {
                self.resolve_operand(elem, &operand_segments(written))
            }
            SyntaxKind::PATH_EXPR => {
                let head = written.children().next()?;
                let name = written
                    .children_with_tokens()
                    .filter_map(|it| it.into_token())
                    .filter(|it| {
                        matches!(it.kind(), SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME)
                    })
                    .last()?;
                let from = self.comes_to(elem, &head)?;
                let name = sysml_syntax::unquote(name.text());
                self.lookup(from, &name, Reach::all(Access::External))
            }
            SyntaxKind::INDEX_EXPR => {
                let sequence = written.children().next()?;
                let from = self.comes_to(elem, &sequence)?;
                let collection = self.named_globally("Collections::Collection");
                match collection {
                    Some(collection) if self.reaches(from, collection) => None,
                    _ => Some(from),
                }
            }
            _ => None,
        }?;
        self.model
            .kind(found)
            .is_a(ElementKind::Feature)
            .then_some(found)
    }
    /// The base type referenced by a SemanticMetadata definition's
    /// `:>> baseType = <ref> meta ...` member, if any.
    fn semantic_bases(&mut self, meta_def: ElementId) -> Vec<ElementId> {
        if let Some(cached) = self.semantic_bases.get(&meta_def) {
            return cached.clone();
        }
        if !self.in_progress.insert(meta_def) {
            return Vec::new();
        }
        let cut = self.misses;
        let refused = self.blocked;
        let mut found = Vec::new();
        for child in self.model.owned(meta_def).to_vec() {
            if !self.member_name_matches(child, "baseType") {
                continue;
            }
            // `= if p ? A meta T else B meta T` names two bases, and
            // which one a given element gets is decided by an expression
            // this resolver does not evaluate. Both are taken: a name
            // that either of them declares is one the model can mean.
            let operands: Vec<SyntaxNode> = self
                .source
                .get(&child)
                .cloned()
                .into_iter()
                .flat_map(|node| node.children())
                .filter(|c| c.kind() == SyntaxKind::VALUE)
                .flat_map(|value| value.descendants())
                .filter(|c| matches!(c.kind(), SyntaxKind::NAME_REF | SyntaxKind::PATH_EXPR))
                .collect();
            for operand in operands {
                let segments = operand_segments(&operand);
                if segments.is_empty() {
                    continue;
                }
                found.extend(self.resolve_from(child, &segments));
            }
            break;
        }
        self.in_progress.remove(&meta_def);
        if self.misses != cut {
            self.incomplete.insert(meta_def);
        }
        if self.blocked != refused {
            self.provisional.insert(meta_def);
        }
        self.semantic_bases.insert(meta_def, found.clone());
        found
    }
}
