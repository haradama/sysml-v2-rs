//! What a drawing looks like, as against what it says.
//!
//! The notation is the standard's: a filled diamond is a composition, a
//! dashed line is a dependency, an italic name is abstract. None of that
//! is here, because a skin that could change it would make drawings that
//! read clearly and say something else. What is here is colour, face and
//! weight -- what a reader can want otherwise without the drawing
//! meaning anything different.
//!
//! Colour carries no meaning in SysML v2, which draws in line and shape.
//! Painting a requirement red says nothing a reader of the standard can
//! rely on; it says something to the people who agreed on it, which is
//! what a skin is for.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use serde_json::Value;

/// A colour, as `0xRRGGBB`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour(pub u32);

impl Colour {
    /// The colour `#rrggbb` names, or nothing where it names none.
    ///
    /// `#abc` is those three digits doubled, as everywhere else that
    /// reads a colour. The hash is optional, since a shell eats an
    /// unquoted one. A name like `rebeccapurple` is refused: SVG's list
    /// of them is long, and renderers disagree about its tail.
    pub fn parse(said: &str) -> Option<Colour> {
        let digits = said.strip_prefix('#').unwrap_or(said);
        if !digits.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return None;
        }
        let full: String = match digits.len() {
            3 => digits.chars().flat_map(|ch| [ch, ch]).collect(),
            6 => digits.to_string(),
            _ => return None,
        };
        u32::from_str_radix(&full, 16).ok().map(Colour)
    }

    /// As a stylesheet writes it.
    fn hex(self) -> String {
        format!("#{:06x}", self.0)
    }
}

/// What one kind of box is painted in, where it is not painted like the
/// rest.
///
/// Each is the palette's own colour where it says nothing, so naming a
/// fill and leaving the rest is the common case: a pale box with the
/// drawing's own lines and letters.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tint {
    /// Inside the box.
    pub fill: Option<Colour>,
    /// Its border.
    pub line: Option<Colour>,
    /// The words in it.
    pub text: Option<Colour>,
}

/// The colours a drawing is painted in, and the exceptions by kind.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    /// What the drawing is read against. A name written over a line is
    /// haloed in this, so it wants to be the colour of whatever the
    /// drawing is put on rather than the colour of a box.
    pub page: Colour,
    /// The inside of a box, of an arrowhead, of a port's square.
    pub fill: Colour,
    /// Every line, and every letter.
    pub ink: Colour,
    /// What the canvas itself is painted, where it is painted at all.
    ///
    /// Nothing leaves it transparent, which is what a drawing meant to
    /// be dropped into a page wants and what this has always written. A
    /// drawing meant to stand on its own -- saved, printed, pasted into
    /// something whose colour nobody chose -- wants this, and usually
    /// wants it the same colour as `page`.
    pub background: Option<Colour>,
    /// Kinds painted otherwise, under the word the drawing writes in
    /// guillemets: `part def`, `requirement def`, `state`. `package` is
    /// the frame drawn round what a package holds, and `comment` the
    /// folded-corner note, neither of which carries a keyword.
    pub kinds: BTreeMap<String, Tint>,
    /// Lines painted otherwise, under the word the notation calls the
    /// relation by: `satisfy`, `composition`, `dependency`. The marker
    /// on the line is painted with it -- a red line with a black
    /// arrowhead is a drawing that looks broken.
    pub relations: BTreeMap<String, Colour>,
}

impl Palette {
    /// A palette of three colours, a transparent canvas and no
    /// exceptions.
    pub fn of(page: Colour, fill: Colour, ink: Colour) -> Palette {
        Palette {
            page,
            fill,
            ink,
            background: None,
            kinds: BTreeMap::new(),
            relations: BTreeMap::new(),
        }
    }
}

/// How a drawing is painted: a palette to read it by, another for a
/// viewer that prefers the dark, the face the text is set in and the
/// weight the lines are drawn at.
#[derive(Clone, Debug, PartialEq)]
pub struct Skin {
    /// What the drawing is painted in.
    pub light: Palette,
    /// The palette a viewer asking for dark is answered with. Nothing
    /// where the drawing is one thing everywhere -- which is what paper
    /// wants, and what every renderer that reads no media query sees
    /// anyway.
    pub dark: Option<Palette>,
    /// The `font-family` the text is set in, as CSS writes one.
    pub font: String,
    /// How thick a line is drawn, in pixels.
    pub stroke: f64,
}

/// White paper and black ink, and the same the other way round for a
/// viewer that asks for it.
impl Default for Skin {
    fn default() -> Skin {
        Skin {
            light: Palette::of(Colour(0xffffff), Colour(0xffffff), Colour(0x000000)),
            dark: Some(Palette::of(
                Colour(0x1e1e1e),
                Colour(0x1e1e1e),
                Colour(0xd4d4d4),
            )),
            font: "Arial, Helvetica, sans-serif".to_string(),
            stroke: 1.0,
        }
    }
}

impl Skin {
    /// One of the skins that ship, by name.
    ///
    /// `default` is what a screen gets. `mono` is the same drawing with
    /// nothing said about the dark, which is what paper and every
    /// renderer that reads no media query see regardless. `contrast`
    /// draws it in a heavier line.
    pub fn named(name: &str) -> Option<Skin> {
        match name {
            "default" => Some(Skin::default()),
            "mono" => Some(Skin {
                dark: None,
                ..Skin::default()
            }),
            "contrast" => Some(Skin {
                dark: None,
                stroke: 2.0,
                ..Skin::default()
            }),
            _ => None,
        }
    }

    /// The names [`Skin::named`] answers to, for a front end that lists
    /// them or says what it did not recognise.
    pub const NAMES: &'static [&'static str] = &["default", "mono", "contrast"];

    /// The colour any palette paints `relation`, where one does. The
    /// drawing asks before it writes a line, since a line that says
    /// which relation it is, is a line some skin paints.
    pub(crate) fn paints(&self, relation: crate::Relation) -> bool {
        let asked = |palette: &Palette| palette.relations.contains_key(relation.name());
        asked(&self.light) || self.dark.as_ref().is_some_and(asked)
    }

    /// Every relation any palette paints, in one list, so the drawing
    /// can carry a marker for each.
    pub(crate) fn painted_relations(&self) -> Vec<crate::Relation> {
        crate::Relation::ALL
            .iter()
            .copied()
            .filter(|relation| self.paints(*relation))
            .collect()
    }

    /// Whether any palette paints the canvas, which is what decides
    /// whether the drawing carries one to paint.
    pub(crate) fn grounded(&self) -> bool {
        self.light.background.is_some()
            || self
                .dark
                .as_ref()
                .is_some_and(|dark| dark.background.is_some())
    }

    /// Whether any palette paints `kind` otherwise, which is what
    /// decides whether the drawing says which kind a box is at all.
    pub(crate) fn tints(&self, kind: &str) -> bool {
        let asked = |palette: &Palette| palette.kinds.contains_key(kind);
        asked(&self.light) || self.dark.as_ref().is_some_and(asked)
    }

    /// The stylesheet a document carries.
    ///
    /// Written out in full rather than through custom properties:
    /// librsvg and resvg -- between them, most of what opens a saved
    /// drawing that is not a browser -- resolve no `var()`, and a
    /// declaration they cannot resolve leaves the shape in its initial
    /// paint, which is a black box with no outline.
    pub fn stylesheet(&self) -> String {
        let grounded = self.grounded();
        let mut out = self.light.rules(self.stroke);
        if grounded {
            out.push_str(&self.light.ground());
        }
        out.push_str(&self.light.tint_rules());
        out.push_str(&self.light.relation_rules());
        if let Some(dark) = &self.dark {
            // only what differs from the light half, so that a reader of
            // the sheet sees what the dark changes
            let mut inside = dark.overrides();
            if grounded {
                inside.push_str(&dark.ground());
            }
            inside.push_str(&dark.tint_rules());
            inside.push_str(&dark.relation_rules());
            let _ = write!(out, "@media (prefers-color-scheme: dark) {{\n{inside}}}\n");
        }
        out
    }
}

/// The class a kind is written under, from the word the drawing writes:
/// `part def` is `kind-part-def`.
///
/// Prefixed because the words collide with the classes the drawing
/// already has: a `port` is a kind of box and `.port` is the little
/// square on a border.
pub(crate) fn class_of(kind: &str) -> String {
    let mut out = String::from("kind-");
    for ch in kind.chars() {
        out.push(if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '-'
        });
    }
    out
}

impl Palette {
    /// Every rule, in the order a reader of the sheet meets the drawing:
    /// the shapes, then the lines, then the words.
    fn rules(&self, stroke: f64) -> String {
        let (page, fill, ink) = (self.page.hex(), self.fill.hex(), self.ink.hex());
        let mut out = String::new();
        let _ = write!(
            out,
            ".box {{ fill: {fill}; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .rule, .edge {{ stroke: {ink}; stroke-width: {stroke}; fill: none; }}\n\
             .arrow {{ fill: {fill}; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .diamond {{ fill: {ink}; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .hollow {{ fill: {fill}; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .tip {{ fill: none; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .initial {{ fill: {ink}; }}\n\
             .port {{ fill: {fill}; stroke: {ink}; stroke-width: {stroke}; }}\n\
             .guide {{ stroke: {ink}; stroke-width: {stroke}; opacity: 0.4; }}\n"
        );
        // The dashes are the notation's and not the skin's: a dependency
        // is the one dashed line the standard draws, and a succession
        // and a lifeline are told from it by patterns of their own.
        let _ = write!(
            out,
            ".dependency {{ stroke: {ink}; stroke-width: {stroke}; fill: none; \
             stroke-dasharray: 6 4; }}\n\
             .succession {{ stroke: {ink}; stroke-width: {stroke}; fill: none; \
             stroke-dasharray: 4 3; }}\n\
             .lifeline {{ stroke: {ink}; stroke-width: {stroke}; fill: none; \
             stroke-dasharray: 3 4; }}\n\
             .lane {{ fill: none; stroke: {ink}; stroke-width: {stroke}; }}\n"
        );
        // A name over a line is haloed in the page's own colour, which is
        // what keeps it readable where it crosses one.
        let _ = write!(
            out,
            ".name {{ fill: {ink}; font-weight: bold; }}\n\
             .abstract {{ font-style: italic; }}\n\
             .keyword, .feature {{ fill: {ink}; }}\n\
             .aside {{ fill: {ink}; paint-order: stroke; stroke: {page}; stroke-width: 3; \
             stroke-linejoin: round; }}\n\
             .compartment {{ fill: {ink}; font-style: italic; }}\n"
        );
        out
    }

    /// The canvas, where this palette paints one. A palette that paints
    /// none still says so, since the other half of the sheet may paint
    /// one and a drawing is one file.
    fn ground(&self) -> String {
        match self.background {
            Some(colour) => format!(".ground {{ fill: {}; }}\n", colour.hex()),
            None => ".ground { fill: none; }\n".to_string(),
        }
    }

    /// What the dark half has to say over the light one: the colours,
    /// and nothing of weight or face, which it does not change.
    fn overrides(&self) -> String {
        let (page, fill, ink) = (self.page.hex(), self.fill.hex(), self.ink.hex());
        let mut out = String::new();
        let _ = write!(
            out,
            ".box, .arrow, .hollow, .port {{ fill: {fill}; stroke: {ink}; }}\n\
             .rule, .edge, .tip, .guide, .dependency, .succession, .lifeline, .lane \
             {{ stroke: {ink}; }}\n\
             .diamond {{ fill: {ink}; stroke: {ink}; }}\n\
             .initial, .name, .keyword, .feature, .compartment {{ fill: {ink}; }}\n\
             .aside {{ fill: {ink}; stroke: {page}; }}\n"
        );
        out
    }

    /// The lines painted otherwise, and the markers on them.
    ///
    /// Two classes rather than one, so that these beat the rule the
    /// shape carries: a line is `.edge`, and this is `.edge.rel-satisfy`.
    /// A marker's shape is filled or hollow according to what it means,
    /// so the filled ones are painted through and the hollow ones only
    /// outlined.
    fn relation_rules(&self) -> String {
        let mut out = String::new();
        for (relation, colour) in &self.relations {
            let class = format!("rel-{}", class_of(relation).trim_start_matches("kind-"));
            let colour = colour.hex();
            let _ = writeln!(
                out,
                ".edge.{class}, .succession.{class}, .dependency.{class}, \
                 .lifeline.{class} {{ stroke: {colour}; }}"
            );
            let _ = writeln!(
                out,
                ".{class}.tip, .{class}.arrow, .{class}.hollow {{ stroke: {colour}; }}"
            );
            let _ = writeln!(
                out,
                ".{class}.diamond {{ fill: {colour}; stroke: {colour}; }}"
            );
        }
        out
    }

    /// The kinds painted otherwise. A box carries its kind's class, so
    /// what is written here reaches its border, its fill and the words
    /// inside it without the drawing having to know any of this.
    fn tint_rules(&self) -> String {
        let mut out = String::new();
        for (kind, tint) in &self.kinds {
            let class = class_of(kind);
            if let Some(fill) = tint.fill {
                let _ = writeln!(out, ".{class} .box {{ fill: {}; }}", fill.hex());
            }
            if let Some(line) = tint.line {
                let _ = writeln!(
                    out,
                    ".{class} .box, .{class} .rule {{ stroke: {}; }}",
                    line.hex()
                );
            }
            if let Some(text) = tint.text {
                let _ = writeln!(
                    out,
                    ".{class} .name, .{class} .keyword, .{class} .feature, \
                     .{class} .compartment {{ fill: {}; }}",
                    text.hex()
                );
            }
        }
        out
    }
}

/// The skin `said` writes down.
///
/// ```json
/// {
///   "light": {
///     "page": "#ffffff", "fill": "#ffffff", "ink": "#000000",
///     "kinds": { "part def": { "fill": "#e8f0fe", "line": "#1a3a6b" } }
///   },
///   "dark": { "page": "#1e1e1e", "fill": "#1e1e1e", "ink": "#d4d4d4" },
///   "font": "Inter, sans-serif",
///   "stroke": 1
/// }
/// ```
///
/// A name of one of the skins that ship stands for it, so a setting can
/// hold either: `"mono"` and the object above are both skins.
pub fn read(said: &Value) -> Result<Skin, String> {
    if let Some(name) = said.as_str() {
        return Skin::named(name).ok_or_else(|| {
            format!(
                "no skin `{name}`; the ones that ship are {}",
                Skin::NAMES.join(", ")
            )
        });
    }
    let Some(object) = said.as_object() else {
        return Err("a skin is an object, or the name of one".to_string());
    };
    let base = Skin::default();
    known(object.keys(), &["light", "dark", "font", "stroke"], "skin")?;
    let light = match object.get("light") {
        Some(said) => palette(said, "light")?,
        None => base.light,
    };
    // `"dark": null` is a drawing that is one thing everywhere, which is
    // not the same as saying nothing about the dark
    let dark = match object.get("dark") {
        Some(Value::Null) => None,
        Some(said) => Some(palette(said, "dark")?),
        None => base.dark,
    };
    let font = match object.get("font") {
        Some(said) => said
            .as_str()
            .ok_or_else(|| "`font` is a font-family, as CSS writes one".to_string())?
            .to_string(),
        None => base.font,
    };
    let stroke = match object.get("stroke") {
        Some(said) => said
            .as_f64()
            .filter(|weight| *weight >= 0.0)
            .ok_or_else(|| "`stroke` is how many pixels thick a line is".to_string())?,
        None => base.stroke,
    };
    Ok(Skin {
        light,
        dark,
        font,
        stroke,
    })
}

/// One palette: three colours, and the kinds painted otherwise.
fn palette(said: &Value, whose: &str) -> Result<Palette, String> {
    let object = said
        .as_object()
        .ok_or_else(|| format!("`{whose}` is an object of colours"))?;
    known(
        object.keys(),
        &["page", "fill", "ink", "background", "kinds", "relations"],
        whose,
    )?;
    let asked = |name: &str| -> Result<Option<Colour>, String> {
        object.get(name).map(|said| colour(said, name)).transpose()
    };
    let base = Skin::default().light;
    let kinds = match object.get("kinds") {
        Some(said) => painted(said, whose)?,
        None => BTreeMap::new(),
    };
    Ok(Palette {
        page: asked("page")?.unwrap_or(base.page),
        fill: asked("fill")?.unwrap_or(base.fill),
        ink: asked("ink")?.unwrap_or(base.ink),
        background: asked("background")?,
        kinds,
        relations: match object.get("relations") {
            Some(said) => lines(said, whose)?,
            None => BTreeMap::new(),
        },
    })
}

/// The relations a palette paints otherwise, one colour each.
fn lines(said: &Value, whose: &str) -> Result<BTreeMap<String, Colour>, String> {
    let object = said
        .as_object()
        .ok_or_else(|| format!("`{whose}.relations` is an object, one colour per relation"))?;
    let known: Vec<&str> = crate::Relation::ALL
        .iter()
        .map(|relation| relation.name())
        .collect();
    object
        .iter()
        .map(|(relation, said)| {
            if !known.contains(&relation.as_str()) {
                return Err(format!(
                    "there is no `{relation}` to paint; the lines a drawing has are {}",
                    known.join(", ")
                ));
            }
            Ok((relation.clone(), colour(said, relation)?))
        })
        .collect()
}

/// The kinds a palette paints otherwise, one entry each.
fn painted(said: &Value, whose: &str) -> Result<BTreeMap<String, Tint>, String> {
    said.as_object()
        .ok_or_else(|| format!("`{whose}.kinds` is an object, one entry per kind"))?
        .iter()
        .map(|(kind, said)| Ok((kind.clone(), tint(said, kind)?)))
        .collect()
}

/// What one kind is painted in: any of a fill, a line and the words.
fn tint(said: &Value, kind: &str) -> Result<Tint, String> {
    // the common case is a fill and nothing else, so a bare colour says
    // exactly that
    if said.is_string() {
        return Ok(Tint {
            fill: Some(colour(said, kind)?),
            ..Tint::default()
        });
    }
    let object = said
        .as_object()
        .ok_or_else(|| format!("`{kind}` is a colour, or an object of them"))?;
    known(object.keys(), &["fill", "line", "text"], kind)?;
    let asked = |name: &str| -> Result<Option<Colour>, String> {
        object
            .get(name)
            .map(|said| colour(said, &format!("{kind}.{name}")))
            .transpose()
    };
    Ok(Tint {
        fill: asked("fill")?,
        line: asked("line")?,
        text: asked("text")?,
    })
}

/// One colour, as `#rrggbb`.
fn colour(said: &Value, whose: &str) -> Result<Colour, String> {
    said.as_str()
        .and_then(Colour::parse)
        .ok_or_else(|| format!("`{whose}` is a colour like `#e8f0fe`, and {said} is not one"))
}

/// Refuse a word a skin is not written with, and say what was meant
/// where something near it is.
fn known<'a>(
    written: impl Iterator<Item = &'a String>,
    allowed: &[&str],
    whose: &str,
) -> Result<(), String> {
    for word in written {
        if allowed.contains(&word.as_str()) {
            continue;
        }
        let near = allowed
            .iter()
            .find(|allowed| allowed.eq_ignore_ascii_case(word));
        return Err(match near {
            Some(meant) => format!("`{whose}` has no `{word}`; did you mean `{meant}`?"),
            None => format!("`{whose}` has no `{word}`; it has {}", allowed.join(", ")),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_is_read_the_way_a_reader_writes_one() {
        assert_eq!(Colour::parse("#ff8800"), Some(Colour(0xff8800)));
        // the hash is optional, since a shell eats an unquoted one
        assert_eq!(Colour::parse("ff8800"), Some(Colour(0xff8800)));
        // three digits are those digits doubled, as everywhere else
        assert_eq!(Colour::parse("#f80"), Some(Colour(0xff8800)));
        assert_eq!(Colour::parse("#FFF"), Some(Colour(0xffffff)));
        for said in ["", "#", "#ff88", "#ff88000", "#gggggg", "rebeccapurple"] {
            assert_eq!(Colour::parse(said), None, "{said}");
        }
        assert_eq!(Colour(0x00ff00).hex(), "#00ff00");
    }

    #[test]
    fn the_skins_that_ship_are_the_ones_it_answers_to() {
        for name in Skin::NAMES {
            let skin = Skin::named(name).unwrap_or_else(|| panic!("no skin named {name}"));
            assert!(!skin.stylesheet().is_empty());
        }
        assert_eq!(Skin::named("chartreuse"), None);
        assert!(Skin::named("default").unwrap().dark.is_some());
        assert!(Skin::named("mono").unwrap().dark.is_none());
        assert_eq!(Skin::named("contrast").unwrap().stroke, 2.0);
    }

    /// The word the drawing writes is the word a skin is written with,
    /// and what stands between them is spelling it as a class.
    #[test]
    fn a_kind_is_written_under_the_word_the_drawing_writes() {
        assert_eq!(class_of("part def"), "kind-part-def");
        assert_eq!(class_of("assoc struct"), "kind-assoc-struct");
        assert_eq!(class_of("PortDef"), "kind-portdef");
        // `port` is a kind of box and `.port` is the square on a border,
        // which is why the prefix is there
        assert_eq!(class_of("port"), "kind-port");
    }

    /// A skin of one's own paints the drawing and nothing else: the
    /// dashes that tell a dependency from a succession still stand.
    #[test]
    fn a_skin_of_ones_own_says_only_what_a_skin_says() {
        let mut light = Palette::of(
            Colour::parse("#fffdf7").unwrap(),
            Colour::parse("#ffffff").unwrap(),
            Colour::parse("#123456").unwrap(),
        );
        light.kinds.insert(
            "part def".to_string(),
            Tint {
                fill: Colour::parse("#e8f0fe"),
                line: Colour::parse("#1a3a6b"),
                text: None,
            },
        );
        light.kinds.insert(
            "requirement def".to_string(),
            Tint {
                fill: None,
                line: None,
                text: Colour::parse("#8b1a10"),
            },
        );
        let skin = Skin {
            light,
            dark: None,
            font: "Inter, sans-serif".to_string(),
            stroke: 1.5,
        };
        let sheet = skin.stylesheet();
        assert!(sheet.contains(".box { fill: #ffffff; stroke: #123456; stroke-width: 1.5; }"));
        assert!(sheet.contains("stroke: #fffdf7"), "the halo is the page's");
        assert!(
            sheet.contains(".kind-part-def .box { fill: #e8f0fe; }"),
            "{sheet}"
        );
        assert!(
            sheet.contains(".kind-part-def .box, .kind-part-def .rule { stroke: #1a3a6b; }"),
            "{sheet}"
        );
        assert!(
            sheet.contains(".kind-requirement-def .name, .kind-requirement-def .keyword"),
            "{sheet}"
        );
        // a kind that says nothing about a colour is not written about it
        assert!(!sheet.contains(".kind-requirement-def .box"), "{sheet}");
        assert!(!sheet.contains("prefers-color-scheme"), "{sheet}");
        assert!(
            sheet.contains("stroke-dasharray: 6 4"),
            "the notation stands"
        );

        // and the drawing is told to say which kind a box is, for those
        // kinds and no others
        assert!(skin.tints("part def"));
        assert!(!skin.tints("state def"));
    }

    /// What a skin paints reaches the drawing, and what it does not
    /// paint is left as it was.
    #[test]
    fn a_painted_kind_is_a_box_that_says_which_kind_it_is() {
        use crate::tests::resolved;
        let ws = resolved("package P { part def A; state def M { state s; } }\n");
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let painted = |kinds: &[(&str, &str)]| {
            let mut light = Skin::default().light;
            for (kind, fill) in kinds {
                light.kinds.insert(
                    (*kind).to_string(),
                    Tint {
                        fill: Colour::parse(fill),
                        ..Tint::default()
                    },
                );
            }
            let style = crate::Style {
                skin: Skin {
                    light,
                    ..Skin::default()
                },
                ..Default::default()
            };
            crate::render(&diagram, &style)
        };

        // painted: the box says which kind it is, and the sheet says
        // what that kind is painted in
        let svg = painted(&[("part def", "#e8f0fe")]);
        assert!(svg.contains("<g class=\"kind-part-def\">"), "{svg}");
        assert!(
            svg.contains(".kind-part-def .box { fill: #e8f0fe; }"),
            "{svg}"
        );
        // and the kind nobody painted is drawn as it always was
        assert!(!svg.contains("kind-state-def"), "{svg}");
        assert_eq!(svg.matches("<g>").count(), 1, "{svg}");

        // the package frame is painted under a word of its own
        let svg = painted(&[("package", "#f6f6f6")]);
        assert!(svg.contains("<g class=\"kind-package\">"), "{svg}");

        // and a drawing nobody painted is the drawing there was
        assert_eq!(
            painted(&[]),
            crate::render(&diagram, &crate::Style::default())
        );
    }

    /// The folded note writes no keyword, so a skin paints it under the
    /// word a reader would use for it.
    #[test]
    fn the_note_is_painted_under_the_word_for_it() {
        use crate::tests::resolved;
        let ws = resolved("package P {\n\tpart def A;\n\tcomment about A /* Said. */\n}\n");
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        assert!(diagram
            .nodes
            .iter()
            .any(|node| node.shape == crate::Shape::Note));
        let mut light = Skin::default().light;
        light.kinds.insert(
            "comment".to_string(),
            Tint {
                fill: Colour::parse("#fffbe6"),
                ..Tint::default()
            },
        );
        let style = crate::Style {
            skin: Skin {
                light,
                ..Skin::default()
            },
            ..Default::default()
        };
        let svg = crate::render(&diagram, &style);
        assert!(svg.contains("<g class=\"kind-comment\">"), "{svg}");
        assert!(
            svg.contains(".kind-comment .box { fill: #fffbe6; }"),
            "{svg}"
        );
        // and it closes what it opened
        assert_eq!(
            svg.matches("<g class=\"kind-comment\">").count(),
            1,
            "{svg}"
        );
    }

    /// A painted relation colours its line and the marker on it, and
    /// the marker is a copy: one is referred to by name and takes no
    /// colour from the line that refers to it.
    #[test]
    fn a_painted_relation_takes_its_marker_with_it() {
        use crate::tests::resolved;
        let ws = resolved("package P { part def A; part def B :> A; }\n");
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let mut light = Skin::default().light;
        light.relations.insert(
            "specialization".to_string(),
            Colour::parse("#1a7f37").unwrap(),
        );
        let style = crate::Style {
            skin: Skin {
                light,
                ..Skin::default()
            },
            ..Default::default()
        };
        let svg = crate::render(&diagram, &style);

        // the line says which relation it is, and points at the copy
        assert!(svg.contains("class=\"edge rel-specialization\""), "{svg}");
        assert!(
            svg.contains("marker-end=\"url(#specialization--specialization)\""),
            "{svg}"
        );
        // the copy is there, drawn as the notation draws it and told
        // which relation it is drawn for
        assert!(
            svg.contains("<marker id=\"specialization--specialization\""),
            "{svg}"
        );
        assert!(svg.contains("class=\"rel-specialization arrow\""), "{svg}");
        // the one it was copied from is still there for everything else
        assert!(svg.contains("<marker id=\"specialization\""), "{svg}");
        assert!(
            svg.contains(".rel-specialization.tip, .rel-specialization.arrow"),
            "{svg}"
        );
    }

    /// A relation the notation draws with no marker -- a connection, an
    /// interface, a binding, all of which are undirected -- is painted
    /// on the line and carries no copy of anything.
    #[test]
    fn a_relation_with_no_marker_is_painted_on_the_line() {
        use crate::tests::resolved;
        let ws = resolved(
            "package P {\n\
             \tpart def W { port hub; }\n\
             \tpart def X { port mount; }\n\
             \tpart def C { part w : W; part x : X; connect w.hub to x.mount; }\n\
             }\n",
        );
        let model = ws.model();
        let inside = model
            .descendants(ws.root())
            .into_iter()
            .find(|&id| model.name(id) == Some("C"))
            .expect("the part that wires the two");
        let diagram = crate::interconnection_diagram(model, inside);
        let mut light = Skin::default().light;
        light
            .relations
            .insert("connection".to_string(), Colour::parse("#8b1a10").unwrap());
        let style = crate::Style {
            skin: Skin {
                light,
                ..Skin::default()
            },
            ..Default::default()
        };
        let svg = crate::render(&diagram, &style);
        assert!(svg.contains("rel-connection"), "{svg}");
        assert!(svg.contains(".edge.rel-connection"), "{svg}");
        // nothing was copied, since there is nothing on the end to copy
        assert!(!svg.contains("--connection\""), "{svg}");
    }

    /// The canvas is painted where a skin paints one, and left alone
    /// where none does -- which is what a drawing dropped into a page
    /// wants, and what this wrote before there were skins.
    #[test]
    fn the_canvas_is_painted_only_where_a_skin_paints_one() {
        use crate::tests::resolved;
        let ws = resolved("package P { part def A; }\n");
        let diagram = crate::definition_diagram(ws.model(), &[ws.root()]);
        let plain = crate::render(&diagram, &crate::Style::default());
        assert!(!plain.contains("class=\"ground\""), "{plain}");
        assert!(!plain.contains(".ground"), "{plain}");

        let style = crate::Style {
            skin: Skin {
                light: Palette {
                    background: Colour::parse("#fffdf7"),
                    ..Skin::default().light
                },
                ..Skin::default()
            },
            ..Default::default()
        };
        let painted = crate::render(&diagram, &style);
        assert!(painted.contains(".ground { fill: #fffdf7; }"), "{painted}");
        assert!(
            painted.contains("<rect class=\"ground\" x=\"0\" y=\"0\""),
            "{painted}"
        );
        // the dark half paints none, and says so rather than inheriting
        let dark = painted
            .find("prefers-color-scheme")
            .expect("the default skin has a dark half");
        assert!(
            painted[dark..].contains(".ground { fill: none; }"),
            "{painted}"
        );
    }

    /// A dark palette paints its own kinds, inside the query.
    #[test]
    fn the_dark_half_paints_its_own_kinds() {
        let mut dark = Palette::of(Colour(0x1e1e1e), Colour(0x1e1e1e), Colour(0xd4d4d4));
        dark.kinds.insert(
            "part def".to_string(),
            Tint {
                fill: Colour::parse("#243447"),
                ..Tint::default()
            },
        );
        let skin = Skin {
            dark: Some(dark),
            ..Skin::default()
        };
        let sheet = skin.stylesheet();
        let query = sheet.find("prefers-color-scheme").expect("a dark half");
        assert!(
            sheet[query..].contains(".kind-part-def .box { fill: #243447; }"),
            "{sheet}"
        );
        assert!(skin.tints("part def"));
    }
    use serde_json::json;

    #[test]
    fn a_name_stands_for_the_skin_it_names() {
        assert_eq!(read(&json!("mono")).unwrap(), Skin::named("mono").unwrap());
        assert!(read(&json!("chartreuse"))
            .unwrap_err()
            .contains("the ones that ship are default, mono, contrast"));
    }

    #[test]
    fn a_skin_written_down_is_the_skin_it_describes() {
        let skin = read(&json!({
            "light": {
                "page": "#fffdf7",
                "ink": "#123456",
                "kinds": {
                    "part def": { "fill": "#e8f0fe", "line": "#1a3a6b" },
                    "requirement def": "#fdecea"
                }
            },
            "dark": null,
            "font": "Inter, sans-serif",
            "stroke": 1.5
        }))
        .unwrap();
        assert_eq!(skin.light.page, Colour(0xfffdf7));
        assert_eq!(skin.light.ink, Colour(0x123456));
        // what it said nothing about is what the default says
        assert_eq!(skin.light.fill, Skin::default().light.fill);
        assert_eq!(skin.dark, None);
        assert_eq!(skin.font, "Inter, sans-serif");
        assert_eq!(skin.stroke, 1.5);
        let part = &skin.light.kinds["part def"];
        assert_eq!(part.fill, Some(Colour(0xe8f0fe)));
        assert_eq!(part.line, Some(Colour(0x1a3a6b)));
        assert_eq!(part.text, None);
        // a bare colour is a fill, which is what a reader means by one
        assert_eq!(
            skin.light.kinds["requirement def"].fill,
            Some(Colour(0xfdecea))
        );

        // and a skin that says nothing is the one that ships
        assert_eq!(read(&json!({})).unwrap(), Skin::default());

        // a dark half written out is read like the light one
        let both = read(&json!({
            "dark": { "page": "#101010", "kinds": { "part def": "#243447" } }
        }))
        .unwrap();
        let dark = both.dark.expect("a dark half");
        assert_eq!(dark.page, Colour(0x101010));
        assert_eq!(dark.kinds["part def"].fill, Some(Colour(0x243447)));
    }

    #[test]
    fn a_relation_nobody_draws_is_refused() {
        let refused = |said: Value| read(&said).unwrap_err();
        let said = refused(json!({ "light": { "relations": { "inheritance": "#000" } } }));
        assert!(said.contains("no `inheritance` to paint"), "{said}");
        assert!(said.contains("specialization"), "{said}");
        assert!(
            refused(json!({ "light": { "relations": [] } })).contains("one colour per relation")
        );
        assert!(
            refused(json!({ "light": { "relations": { "satisfy": 7 } } })).contains("is a colour")
        );

        // and one it does draw is read
        let skin = read(&json!({
            "light": { "relations": { "satisfy": "#8b1a10", "succession flow": "#123456" } }
        }))
        .unwrap();
        assert_eq!(skin.light.relations["satisfy"], Colour(0x8b1a10));
        assert!(skin.paints(crate::Relation::Satisfy));
        assert!(skin.paints(crate::Relation::SuccessionFlow));
        assert!(!skin.paints(crate::Relation::Composition));
        assert_eq!(skin.painted_relations().len(), 2);
    }

    #[test]
    fn the_canvas_is_read_like_any_other_colour() {
        let skin = read(&json!({ "light": { "background": "#fffdf7" }, "dark": null })).unwrap();
        assert_eq!(skin.light.background, Colour::parse("#fffdf7"));
        assert!(skin.grounded());
        assert!(!Skin::default().grounded());
    }

    #[test]
    fn a_word_a_skin_is_not_written_with_is_refused() {
        let refused = |said: Value| read(&said).unwrap_err();
        assert!(refused(json!({ "colour": {} })).contains("no `colour`"));
        assert!(refused(json!({ "Light": {} })).contains("did you mean `light`?"));
        assert!(refused(json!({ "light": { "paper": "#fff" } })).contains("no `paper`"));
        assert!(
            refused(json!({ "light": { "kinds": { "part def": { "border": "#000" } } } }))
                .contains("no `border`")
        );
        assert!(refused(json!({ "light": { "ink": "rebeccapurple" } })).contains("like `#e8f0fe`"));
        assert!(refused(json!({ "light": [] })).contains("`light` is an object"));
        assert!(refused(json!({ "light": { "kinds": [] } })).contains("one entry per kind"));
        assert!(
            refused(json!({ "light": { "kinds": { "part def": 4 } } })).contains("or an object")
        );
        assert!(refused(json!({ "font": 12 })).contains("font-family"));
        assert!(refused(json!({ "stroke": "thick" })).contains("how many pixels"));
        assert!(refused(json!({ "stroke": -1 })).contains("how many pixels"));
        assert!(refused(json!(4)).contains("an object, or the name of one"));
    }
}
