//! Typed AST views over the lossless syntax tree.
//!
//! Each type is a thin wrapper around a [`SyntaxNode`] of a specific
//! [`SyntaxKind`]; accessors walk the children on demand. Nothing here
//! copies or owns the tree.

use crate::{SyntaxKind, SyntaxKind::*, SyntaxNode, SyntaxToken};

/// A typed view of one node of the syntax tree.
///
/// Every view here is the node itself, of a kind this type stands for:
/// casting is a check and a move, not a copy, and the tree a view came
/// from is still reachable through [`AstNode::syntax`].
pub trait AstNode: Sized {
    /// Whether a node of this kind is one of these.
    fn can_cast(kind: SyntaxKind) -> bool;
    /// This view of `node`, or nothing where `node` is something else.
    fn cast(node: SyntaxNode) -> Option<Self>;
    /// The node itself, for reaching what this view does not offer:
    /// the text, the trivia around it, what encloses it.
    fn syntax(&self) -> &SyntaxNode;
}

macro_rules! ast_node {
    ($(#[$attr:meta])* $name:ident, $kind:ident) => {
        $(#[$attr])*
        #[derive(Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(SyntaxNode);

        impl AstNode for $name {
            fn can_cast(kind: SyntaxKind) -> bool {
                kind == $kind
            }
            fn cast(node: SyntaxNode) -> Option<Self> {
                Self::can_cast(node.kind()).then(|| $name(node))
            }
            fn syntax(&self) -> &SyntaxNode {
                &self.0
            }
        }
    };
}

ast_node!(
    /// A whole file: whatever it declares at its top level.
    SourceFile,
    SOURCE_FILE
);
ast_node!(
    /// `package P { ... }`, and the `library package` the standard
    /// library is written in.
    Package,
    PACKAGE
);
ast_node!(
    /// What a declaration holds between its braces.
    Body,
    BODY
);
ast_node!(
    /// `import P::*;` -- the path it names, however it ends.
    Import,
    IMPORT
);
ast_node!(
    /// `alias N for P::Q;` -- a second name for something declared
    /// elsewhere.
    Alias,
    ALIAS
);
ast_node!(
    /// `doc /* ... */`, which is documentation of whatever owns it.
    Documentation,
    DOCUMENTATION
);
ast_node!(
    /// `comment /* ... */`, which is a note about the model rather than
    /// documentation of a part of it.
    CommentElem,
    COMMENT_ELEM
);
ast_node!(
    /// `part def Vehicle { ... }` -- a definition of any kind. Which
    /// kind is the keyword, read through [`Definition::kind_token`].
    Definition,
    DEFINITION
);
ast_node!(
    /// `part engine : Engine[1];` -- a usage of any kind, which is a
    /// definition used rather than declared.
    Usage,
    USAGE
);
ast_node!(
    /// A declared name, as written: quoted where the notation had to
    /// quote it.
    Name,
    NAME
);
ast_node!(
    /// A name reached through the namespaces above it: `ISQ::MassValue`.
    QualifiedName,
    QUALIFIED_NAME
);
ast_node!(
    /// `: Engine` -- what a usage is typed by.
    Typing,
    TYPING
);
ast_node!(
    /// `:> PowerSource` -- what a feature specializes.
    Subsetting,
    SUBSETTING
);
ast_node!(
    /// `:>> mass` -- what a feature redefines.
    Redefinition,
    REDEFINITION
);
ast_node!(
    /// `[1..*]` -- how many of something there are.
    Multiplicity,
    MULTIPLICITY
);
ast_node!(
    /// `= 4` or `:= expr` -- the value a usage is given.
    Value,
    VALUE
);
ast_node!(
    /// One type named in a typing, a specialization or a redefinition,
    /// with the `~` of a conjugated one.
    TypeRef,
    TYPE_REF
);

/// Any namespace member.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Member {
    /// A nested package.
    Package(Package),
    /// An import.
    Import(Import),
    /// An alias.
    Alias(Alias),
    /// Documentation of whatever this is a member of.
    Documentation(Documentation),
    /// A comment.
    CommentElem(CommentElem),
    /// A definition of any kind.
    Definition(Definition),
    /// A usage of any kind.
    Usage(Usage),
}

impl AstNode for Member {
    fn can_cast(kind: SyntaxKind) -> bool {
        matches!(
            kind,
            PACKAGE | IMPORT | ALIAS | DOCUMENTATION | COMMENT_ELEM | DEFINITION | USAGE
        )
    }

    fn cast(node: SyntaxNode) -> Option<Self> {
        let member = match node.kind() {
            PACKAGE => Member::Package(Package(node)),
            IMPORT => Member::Import(Import(node)),
            ALIAS => Member::Alias(Alias(node)),
            DOCUMENTATION => Member::Documentation(Documentation(node)),
            COMMENT_ELEM => Member::CommentElem(CommentElem(node)),
            DEFINITION => Member::Definition(Definition(node)),
            USAGE => Member::Usage(Usage(node)),
            _ => return None,
        };
        Some(member)
    }

    fn syntax(&self) -> &SyntaxNode {
        match self {
            Member::Package(n) => n.syntax(),
            Member::Import(n) => n.syntax(),
            Member::Alias(n) => n.syntax(),
            Member::Documentation(n) => n.syntax(),
            Member::CommentElem(n) => n.syntax(),
            Member::Definition(n) => n.syntax(),
            Member::Usage(n) => n.syntax(),
        }
    }
}

fn child<N: AstNode>(node: &SyntaxNode) -> Option<N> {
    node.children().find_map(N::cast)
}

fn children<N: AstNode>(node: &SyntaxNode) -> impl Iterator<Item = N> {
    node.children().filter_map(N::cast)
}

impl SourceFile {
    /// What the file declares at its top level, in order.
    pub fn members(&self) -> impl Iterator<Item = Member> {
        children(self.syntax())
    }
}

impl Package {
    /// Its declared name, where it has one.
    pub fn name(&self) -> Option<Name> {
        child(self.syntax())
    }

    /// What it holds, where it holds anything.
    pub fn body(&self) -> Option<Body> {
        child(self.syntax())
    }

    /// Whether it is a `library package`, whose members are visible to
    /// a model that never imported them.
    pub fn is_library(&self) -> bool {
        self.token(LIBRARY_KW).is_some()
    }

    /// Whether it is a `standard library package`, which only the
    /// standard library itself declares.
    pub fn is_standard(&self) -> bool {
        self.token(STANDARD_KW).is_some()
    }
}

impl Body {
    /// What is declared inside, in order.
    pub fn members(&self) -> impl Iterator<Item = Member> {
        children(self.syntax())
    }
}

impl Import {
    /// The path it imports, `::*` and `::**` included.
    pub fn target(&self) -> Option<QualifiedName> {
        child(self.syntax())
    }
}

impl Alias {
    /// The name it declares.
    pub fn name(&self) -> Option<Name> {
        child(self.syntax())
    }

    /// What that name is a name for.
    pub fn target(&self) -> Option<QualifiedName> {
        child(self.syntax())
    }
}

impl Definition {
    /// The keyword identifying the definition kind (`part`, `attribute`, ...).
    pub fn kind_token(&self) -> Option<SyntaxToken> {
        first_token_matching(self.syntax(), SyntaxKind::is_def_kind_kw)
    }

    /// Its declared name, where it has one.
    pub fn name(&self) -> Option<Name> {
        child(self.syntax())
    }

    /// What it holds, where it holds anything.
    pub fn body(&self) -> Option<Body> {
        child(self.syntax())
    }

    /// Whether it is `abstract`, so that nothing is ever one of these
    /// and not also one of something more particular.
    pub fn is_abstract(&self) -> bool {
        self.token(ABSTRACT_KW).is_some()
    }

    /// Explicit specializations (`:>` / `specializes`).
    pub fn specializations(&self) -> impl Iterator<Item = Subsetting> {
        children(self.syntax())
    }
}

impl Usage {
    /// The keyword identifying the usage kind (`part`, `attribute`, ...).
    pub fn kind_token(&self) -> Option<SyntaxToken> {
        first_token_matching(self.syntax(), SyntaxKind::is_def_kind_kw)
    }

    /// Its declared name, where it has one -- a usage need not be named.
    pub fn name(&self) -> Option<Name> {
        child(self.syntax())
    }

    /// What it is typed by, where the notation says.
    pub fn typing(&self) -> Option<Typing> {
        child(self.syntax())
    }

    /// How many of it there are, where the notation says.
    pub fn multiplicity(&self) -> Option<Multiplicity> {
        child(self.syntax())
    }

    /// What it is given as a value, where it is given one.
    pub fn value(&self) -> Option<Value> {
        child(self.syntax())
    }

    /// What it holds, where it holds anything.
    pub fn body(&self) -> Option<Body> {
        child(self.syntax())
    }
}

impl TypeRef {
    /// The name of the type it points at.
    pub fn name(&self) -> Option<QualifiedName> {
        child(self.syntax())
    }

    /// `~T` conjugated type reference?
    pub fn is_conjugated(&self) -> bool {
        self.token(crate::SyntaxKind::TILDE).is_some()
    }
}

impl Typing {
    /// Every type named, since a feature may be typed by more than one.
    pub fn targets(&self) -> impl Iterator<Item = QualifiedName> {
        children::<TypeRef>(self.syntax()).filter_map(|t| t.name())
    }
}

impl Subsetting {
    /// Every type specialized, since a feature may specialize more than
    /// one.
    pub fn targets(&self) -> impl Iterator<Item = QualifiedName> {
        children::<TypeRef>(self.syntax()).filter_map(|t| t.name())
    }
}

impl Redefinition {
    /// Every feature redefined, since one declaration may redefine more
    /// than one.
    pub fn targets(&self) -> impl Iterator<Item = QualifiedName> {
        children::<TypeRef>(self.syntax()).filter_map(|t| t.name())
    }
}

impl Name {
    /// Declared name with `'...'` quoting stripped.
    pub fn text(&self) -> String {
        self.syntax()
            .first_token()
            .map(|t| crate::unquote(t.text()))
            .unwrap_or_default()
    }
}

impl QualifiedName {
    /// Name segments with quoting stripped (wildcard segments included as-is).
    pub fn segments(&self) -> Vec<String> {
        self.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .filter(|t| matches!(t.kind(), IDENT | UNRESTRICTED_NAME | STAR | STAR_STAR))
            .map(|t| crate::unquote(t.text()))
            .collect()
    }
}

fn first_token_matching(node: &SyntaxNode, pred: fn(SyntaxKind) -> bool) -> Option<SyntaxToken> {
    node.children_with_tokens()
        .filter_map(|e| e.into_token())
        .find(|t| pred(t.kind()))
}

trait HasToken: AstNode {
    fn token(&self, kind: SyntaxKind) -> Option<SyntaxToken> {
        self.syntax()
            .children_with_tokens()
            .filter_map(|e| e.into_token())
            .find(|t| t.kind() == kind)
    }
}

impl<N: AstNode> HasToken for N {}

/// Whether an expression is nothing but a name: `x`, `A::B`, `a.b.c`.
///
/// Both what the model builds from an expression and what resolves the
/// names in it need this answer, and they must not answer it
/// differently: the model reifies exactly this shape as a
/// `FeatureReferenceExpression`, whose `referent` is what resolution
/// then fills in. A step into a body (`x.?{in p; ...}`) or off a call
/// (`f(x).b`) is not one -- there is no single feature it names.
pub fn is_name_chain(node: &SyntaxNode) -> bool {
    match node.kind() {
        SyntaxKind::NAME_REF => true,
        SyntaxKind::PATH_EXPR => {
            let mut children = node.children();
            match (children.next(), children.next()) {
                (Some(first), None) => is_name_chain(&first),
                _ => false,
            }
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse;

    #[test]
    fn every_accessor_walks() {
        let text = "doc /* file header */\nlibrary package L {\n  package Inner { }\n  doc /* body */\n  alias A for B;\n  import Q::X;\n  comment about B /* c */\n  abstract part def B :> C, D { }\n  part u : ~P [2] :> s :>> r = 1 { }\n}\n";
        let parse = crate::parse(text);
        let file = SourceFile::cast(parse.syntax()).unwrap();
        assert!(SourceFile::cast(parse.syntax().children().next().unwrap()).is_none());

        assert!(Member::cast(parse.syntax()).is_none());
        assert!(!Member::can_cast(crate::SyntaxKind::SOURCE_FILE));
        let pkg = file
            .members()
            .find_map(|m| match m {
                Member::Package(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert!(pkg.is_library());
        assert!(!pkg.is_standard());
        assert_eq!(pkg.name().unwrap().text(), "L");

        for member in pkg.body().unwrap().members() {
            // Member::syntax + can_cast for every variant
            let node = member.syntax().clone();
            assert!(Member::can_cast(node.kind()));
            match &member {
                Member::Documentation(_) | Member::CommentElem(_) => {}
                Member::Alias(alias) => {
                    assert_eq!(alias.name().unwrap().text(), "A");
                    assert_eq!(alias.target().unwrap().segments(), ["B"]);
                }
                Member::Import(import) => {
                    assert_eq!(import.target().unwrap().segments(), ["Q", "X"]);
                }
                Member::Definition(def) => {
                    assert!(def.is_abstract());
                    assert_eq!(def.kind_token().unwrap().text(), "part");
                    assert!(def.body().is_some());
                    let spec: Vec<Subsetting> = def.specializations().collect();
                    let targets: Vec<_> = spec[0].targets().collect();
                    assert_eq!(targets.len(), 2);
                }
                Member::Usage(usage) => {
                    assert_eq!(usage.name().unwrap().text(), "u");
                    assert_eq!(usage.kind_token().unwrap().text(), "part");
                    let typing = usage.typing().unwrap();
                    let type_ref: TypeRef =
                        typing.syntax().children().find_map(TypeRef::cast).unwrap();
                    assert!(type_ref.is_conjugated());
                    assert_eq!(type_ref.name().unwrap().segments(), ["P"]);
                    assert_eq!(typing.targets().count(), 1);
                    assert!(usage.multiplicity().is_some());
                    assert!(usage.value().is_some());
                    assert!(usage.body().is_some());
                    let subsetting: Subsetting = usage
                        .syntax()
                        .children()
                        .find_map(Subsetting::cast)
                        .unwrap();
                    assert_eq!(subsetting.targets().next().unwrap().segments(), ["s"]);
                    let redef: Redefinition = usage
                        .syntax()
                        .children()
                        .find_map(Redefinition::cast)
                        .unwrap();
                    assert_eq!(redef.targets().next().unwrap().segments(), ["r"]);
                }
                Member::Package(nested) => {
                    assert_eq!(nested.name().unwrap().text(), "Inner");
                }
            }
        }

        // quoted names and empty-name fallback
        let parse = crate::parse("doc /* d */ part 'the car';");
        let file = SourceFile::cast(parse.syntax()).unwrap();
        let usage = file
            .members()
            .find_map(|m| match m {
                Member::Usage(u) => Some(u),
                _ => None,
            })
            .unwrap();
        assert_eq!(usage.name().unwrap().text(), "the car");
    }

    #[test]
    fn walk_a_small_model() {
        let text = "doc /* file */\npackage P {\n  doc /* pkg */\n  import Q::*;\n  abstract part def Vehicle :> Thing {\n    doc /* v */\n    attribute mass : Real = 10.0;\n  }\n}\n";
        let parse = parse(text);
        assert!(parse.ok());

        let file = SourceFile::cast(parse.syntax()).unwrap();
        let package = file
            .members()
            .find_map(|m| match m {
                Member::Package(p) => Some(p),
                _ => None,
            })
            .unwrap();
        assert_eq!(package.name().unwrap().text(), "P");

        let members: Vec<_> = package.body().unwrap().members().collect();
        assert_eq!(members.len(), 3);

        let import = members
            .iter()
            .find_map(|m| match m {
                Member::Import(i) => Some(i),
                _ => None,
            })
            .unwrap();
        assert_eq!(import.target().unwrap().segments(), ["Q", "*"]);

        let def = members
            .iter()
            .find_map(|m| match m {
                Member::Definition(d) => Some(d),
                _ => None,
            })
            .unwrap();
        assert!(def.is_abstract());
        assert_eq!(def.kind_token().unwrap().text(), "part");
        assert_eq!(def.name().unwrap().text(), "Vehicle");
        let spec: Vec<_> = def.specializations().collect();
        assert_eq!(spec[0].targets().next().unwrap().segments(), ["Thing"]);

        let attr = def
            .body()
            .unwrap()
            .members()
            .find_map(|m| match m {
                Member::Usage(u) => Some(u),
                _ => None,
            })
            .unwrap();
        assert_eq!(attr.name().unwrap().text(), "mass");
        assert_eq!(
            attr.typing().unwrap().targets().next().unwrap().segments(),
            ["Real"]
        );
        assert!(attr.value().is_some());
    }
}
