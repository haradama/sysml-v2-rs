//! Where an element is named, and what renaming it rewrites.
//!
//! Two front ends ask this -- the language server, for go-to-definition,
//! find-references and rename, and the MCP server, for the same three
//! asked by an agent -- and what one of them answers the other must too:
//! an agent told that a rename touches six places, by a server that then
//! rewrites seven in the editor, has been told something false. So the
//! walk and every refusal live here, and each front end only turns byte
//! ranges into the coordinates its client counts in.

use super::*;
use sysml_syntax::TextSize;

/// What a rename is of, once the new name has been found to be one and
/// the element asked about to have a name of its own to give up.
///
/// [`Workspace::prepare_rename`] makes one and
/// [`Workspace::rename_edits`] finishes it. The two are apart because a
/// front end decides between them whether the declaration is somewhere
/// it may write -- the standard library is not, and which files are the
/// project's is something only the front end knows.
#[derive(Clone, Debug)]
pub struct Renaming {
    /// The element whose name is given up.
    pub target: ElementId,
    /// The file it is declared in, and the range of the name given up.
    pub declared: (usize, TextRange),
    /// Every name the element answers to: the one its declaration spells,
    /// then its short name.
    names: Vec<String>,
    /// Which of them is renamed.
    renamed: String,
    /// The new name as it was asked for.
    new_name: String,
    /// The new name as the name it denotes, `'Other'` being `Other`.
    denoted: String,
}

/// One rewrite a rename makes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenameEdit {
    /// The range replaced, in bytes of that file.
    pub range: TextRange,
    /// What replaces it: the new name, quoted where that file's notation
    /// would not read it as one name otherwise.
    pub new_text: String,
}

/// Whether `text` is one name and nothing else to a file of `dialect`.
///
/// What may stand there is what the lexer reads as one name. Spelling the
/// rule again elsewhere got it wrong three ways: `\u{540d}\u{524d}` passed
/// a Unicode alphabetic test that the ASCII lexer refuses, a quoted `'two
/// words'` was refused although it is a name, and `frame` was refused in
/// KerML, where it is not a keyword.
pub fn is_one_name(text: &str, dialect: Dialect) -> bool {
    let (tokens, complaints) = sysml_syntax::lex_dialect(text, dialect);
    complaints.is_empty()
        && matches!(
            tokens.as_slice(),
            [token] if matches!(token.kind, SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME)
        )
}

/// Why `new_name` cannot be a name in a file of `dialect`, or nothing
/// where it can.
pub fn refuse_name(new_name: &str, dialect: Dialect) -> Option<String> {
    if !is_one_name(new_name, dialect) {
        let (tokens, _) = sysml_syntax::lex_dialect(new_name, dialect);
        return Some(match tokens.as_slice() {
            [token] if dialect.is_keyword(token.kind) => format!("`{new_name}` is a keyword"),
            _ => format!("`{new_name}` is not a name"),
        });
    }
    // `''` lexes as a name and denotes none: what was renamed to it would
    // read as unnamed, and the refusal that caught it said the name was
    // already taken
    if sysml_syntax::unquote(new_name).is_empty() {
        return Some(format!("`{new_name}` is an empty name"));
    }
    None
}

impl Workspace {
    /// The element a position points at: a resolved reference's target, or
    /// the declaration whose name covers the position.
    pub fn target_at(&self, file: usize, offset: TextSize) -> Option<ElementId> {
        self.reference_at(file, offset)
            .map(|r| r.target)
            .or_else(|| self.definition_at(file, offset))
    }

    /// The name written at `offset`, as the name it denotes.
    pub fn spelled_at(&self, file: usize, offset: TextSize) -> Option<String> {
        let syntax = self.file_parse(file).syntax();
        let mut around = syntax.token_at_offset(offset);
        around
            .find(|token| {
                matches!(
                    token.kind(),
                    SyntaxKind::IDENT | SyntaxKind::UNRESTRICTED_NAME
                )
            })
            .map(|token| sysml_syntax::unquote(token.text()))
    }

    /// Every place `target` is named, as the name alone: what resolved to
    /// it, and what resolved to a feature that borrows its name --
    /// `l.component` where `l` holds a `:>> component`. What references
    /// reports and what a rename rewrites, so the two cannot disagree.
    pub fn mentions(&self, target: ElementId) -> Vec<(usize, TextRange)> {
        std::iter::once(target)
            .chain(self.named_after(target))
            .flat_map(|named| self.references_to(named))
            .map(|r| (r.file, r.name_range))
            .collect()
    }

    /// Where `target` is declared, as the name it is declared with. A
    /// declaration recorded as the whole of itself -- `then fork F { ...
    /// }` -- is narrowed to the name in it.
    pub fn declared_at(&self, target: ElementId) -> Option<(usize, TextRange)> {
        let file = self.element_file(target)?;
        let (_, range) = self.element_ranges(target)?;
        let name = self
            .model
            .effective_name(target)
            .or(self.model.effective_short_name(target))
            .unwrap_or_default();
        Some((file, self.name_token(file, range, name).unwrap_or(range)))
    }

    /// Where inside a recorded range the name `name` is written.
    ///
    /// What was recorded is not always the name alone. A mention can be a
    /// feature chain -- `system.sub1`, ending on the step that names what is
    /// being renamed -- and a declaration whose tree holds no name node of
    /// its own is recorded as the whole of it: `then fork F { ... }`, which
    /// the new name would replace, body and all.
    pub fn name_token(&self, file: usize, range: TextRange, name: &str) -> Option<TextRange> {
        let syntax = self.file_parse(file).syntax();
        let first = syntax.token_at_offset(range.start()).right_biased();
        std::iter::successors(first, sysml_syntax::SyntaxToken::next_token)
            .take_while(|token| range.contains_range(token.text_range()))
            .find(|token| sysml_syntax::unquote(token.text()) == name)
            .map(|token| token.text_range())
    }

    /// The first half of a rename: is `new_name` a name in a file of
    /// `dialect`, the notation of `file`, and is there something at
    /// `offset` with a name of its own to give up?
    ///
    /// Renaming touches every file the workspace knows, so what makes it
    /// safe is checked across all of them before a single edit is
    /// offered. The front end asks next whether [`Renaming::declared`] is
    /// somewhere it may write, then [`Workspace::rename_edits`].
    pub fn prepare_rename(
        &self,
        file: usize,
        offset: TextSize,
        new_name: &str,
        dialect: Dialect,
    ) -> Result<Renaming, String> {
        if let Some(why) = refuse_name(new_name, dialect) {
            return Err(why);
        }
        let denoted = sysml_syntax::unquote(new_name);
        let Some(found) = self.target_at(file, offset) else {
            return Err("there is nothing to rename here".to_string());
        };
        let asked = self.spelled_at(file, offset);
        // A feature that declares no name answers to the name of what it
        // redefines or references, and there is nothing in it to rewrite.
        // Renaming is asked of the declaration that name came from, which
        // is what every mention of it -- this one included -- follows.
        let Some(target) = self.model.naming_element(found) else {
            return Err("that element declares no name to rename".to_string());
        };
        // A name reached through `alias X for Y;` resolves to Y, and
        // nothing records that X was the way in. Renaming X would move
        // the declaration and leave every use of it behind, so it is
        // refused rather than half done.
        if self.is_alias(target) {
            return Err("renaming an alias would leave what uses it behind".to_string());
        }
        // Every name this element answers to. The first is the one its
        // declaration spells; the other is its short name. Which of them
        // is renamed is the one the cursor is on: renaming at a `W` that
        // stands for `<W> Wheel` rewrote `Wheel`, a name nobody pointed
        // at, and left the `W` that was.
        let names: Vec<String> = self
            .model
            .effective_name(target)
            .into_iter()
            .chain(self.model.effective_short_name(target))
            .map(String::from)
            .collect();
        let by_short = names.len() == 2 && asked.as_ref() == names.get(1) && names[0] != names[1];
        let renamed = names
            .get(usize::from(by_short))
            .cloned()
            .unwrap_or_default();
        let decl_file = self
            .element_file(target)
            .ok_or("that element belongs to no file".to_string())?;
        let decl_range = match by_short {
            true => self.short_name_range(target),
            false => self.element_ranges(target).map(|(_, name)| name),
        }
        .ok_or("that element declares no name to rename".to_string())?;
        Ok(Renaming {
            target,
            declared: (decl_file, decl_range),
            names,
            renamed,
            new_name: new_name.to_string(),
            denoted,
        })
    }

    /// The second half of a rename: every rewrite it makes, file by file,
    /// in the order the files are first touched -- or why it would not be
    /// a rename at all.
    ///
    /// `dialect` says which notation a file is written in, given its index
    /// and its name: the name was read as a name in the file the rename
    /// was asked in, and `part` is one in KerML, while written unquoted
    /// into a SysML file that names the same element it is a keyword and
    /// the file stops parsing.
    pub fn rename_edits(
        &mut self,
        renaming: &Renaming,
        dialect: impl Fn(usize, &str) -> Dialect,
    ) -> Result<Vec<(usize, Vec<RenameEdit>)>, String> {
        let Renaming {
            target,
            declared: (decl_file, decl_range),
            names,
            renamed,
            new_name,
            denoted,
        } = renaming;
        let target = *target;
        // What names it, and what names a feature that borrows its name:
        // renaming the declaration without those is a model that no
        // longer resolves.
        let mut edits: Vec<(usize, TextRange)> = vec![(*decl_file, *decl_range)];
        edits.extend(self.mentions(target));

        // a name already visible where the declaration stands would
        // capture, or be captured by, the renamed one -- compared as the
        // name it denotes: `'Other'` is `Other`, and compared as written
        // it slipped past and declared a second one
        let taken = self
            .visible_names(*decl_file, decl_range.start())
            .into_iter()
            .any(|(name, _)| name == *denoted);
        if taken {
            return Err(format!("`{new_name}` is already visible there"));
        }
        // The declaration's own scope is not the only place a capture
        // can happen. Every mention of the element stands in a scope of
        // its own, and one that already reaches something under the new
        // name binds to that instead the moment it is rewritten --
        // quietly, because the file still parses and still resolves.
        let wanted = [denoted.clone()];
        for &(file, range) in &edits {
            let at = range.start();
            if !self.looked_up_in_place(file, at) {
                continue;
            }
            let scope = self.innermost_element(file, at);
            if self
                .resolve_from(scope, &wanted)
                .is_some_and(|other| other != target)
            {
                return Err(format!(
                    "`{new_name}` is already visible where this one is used"
                ));
            }
        }

        // grouped by file, each file's in the order they were found
        let mut by_file: Vec<(usize, Vec<TextRange>)> = Vec::new();
        for (file, range) in edits {
            match by_file.iter_mut().find(|(held, _)| *held == file) {
                Some((_, ranges)) => ranges.push(range),
                None => by_file.push((file, vec![range])),
            }
        }
        let mut changes = Vec::new();
        for (file, ranges) in by_file {
            // each file in its own notation
            let written_as = match is_one_name(new_name, dialect(file, self.file_name(file))) {
                true => new_name.clone(),
                false => sysml_syntax::quote(denoted),
            };
            let text = self.file_parse(file).syntax().text().to_string();
            let mut written = Vec::new();
            for range in ranges {
                let Some(at) = self.name_token(file, range, renamed) else {
                    // Nothing there spells the name, so the model
                    // reaches this element some other way: by a short
                    // name, which this rename leaves standing, or
                    // through an `alias B for A;` whose own `for A`
                    // nothing records and no rename can follow.
                    let at = usize::from(range.start())..usize::from(range.end());
                    let spelled = sysml_syntax::unquote(&text[at]);
                    if !names.contains(&spelled) {
                        return Err(format!(
                            "`{spelled}` stands for this one by a way no rename can follow"
                        ));
                    }
                    continue;
                };
                written.push(RenameEdit {
                    range: at,
                    new_text: written_as.clone(),
                });
            }
            changes.push((file, written));
        }
        Ok(changes)
    }

    /// Whether the name written at `at` in `file` is looked up in the scope
    /// that surrounds it.
    ///
    /// A segment that follows `::` or `.` is looked up inside whatever the
    /// segment before it named, so nothing about the scope it is written in
    /// decides where it lands.
    fn looked_up_in_place(&self, file: usize, at: TextSize) -> bool {
        let syntax = self.file_parse(file).syntax();
        let mut token = syntax.token_at_offset(at).left_biased();
        while token.as_ref().is_some_and(|t| t.kind().is_trivia()) {
            token = token.and_then(|t| t.prev_token());
        }
        !token.is_some_and(|t| matches!(t.kind(), SyntaxKind::COLON_COLON | SyntaxKind::DOT))
    }
}
