//! The sequence view: who takes part in an interaction, and what passes
//! between them in what order.
//!
//! `sequence-view = (sq-graphical-element)*`, drawn as the standard has
//! it: a head node per participant across the top, a dashed lifeline
//! hanging from each, and one arrow per message or succession between
//! them, in the order the model declares.

use std::fmt::Write;

use sysml_model::{ElementId, ElementKind, Model, Value};

use crate::graph::{connector_label, connector_relation, effective_name, keyword, Relation};
use crate::svg::{document, escape, markers};
use crate::Style;

/// One participant, drawn as a head node with a lifeline under it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lifeline {
    /// The feature the lifeline stands for -- the last of the chain that
    /// leads to the event, so `vehicle.cruiseController.sent` belongs to
    /// `cruiseController`.
    pub id: ElementId,
    /// SysML keyword shown in guillemets, e.g. `part`.
    pub keyword: String,
    /// The chain as it was written: `vehicle.cruiseController`.
    pub name: String,
}

impl Lifeline {
    /// The head node's label, as it appears in the drawing.
    pub fn label(&self) -> String {
        format!("\u{ab}{}\u{bb} {}", self.keyword, self.name)
    }
}

/// One thing that passes between two lifelines, in the order the model
/// declares it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Moment {
    /// Indices into [`Sequence::lifelines`].
    pub from: usize,
    pub to: usize,
    /// What the standard writes on the arrow: a message's payload, or a
    /// succession's name.
    pub label: Option<String>,
    /// `sq-succession` is drawn dashed where a `message` is not.
    pub dashed: bool,
}

/// The participants of one interaction and what passes between them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Sequence {
    pub lifelines: Vec<Lifeline>,
    pub moments: Vec<Moment>,
}

/// Read the interaction one definition declares.
///
/// A message names the event at each end by the chain that reaches it
/// (`vehicle.cruiseController.setSpeedReceived`); the participant is that
/// chain without its last step, which is what a lifeline stands for.
pub fn sequence_view(model: &Model, definition: ElementId) -> Sequence {
    let mut sequence = Sequence::default();
    for &child in model.owned(definition) {
        // `sq-graphical-relationship = message | sq-succession`, and both
        // reach an event: `first m1 then m2` orders two messages rather
        // than joining two lifelines, so it is not one of these arrows.
        let relation = connector_relation(model, child);
        if !matches!(relation, Relation::Message | Relation::Succession) {
            continue;
        }
        let ends: Vec<Reach> = model
            .owned(child)
            .iter()
            .filter_map(|&end| reach_of(model, end))
            .filter(|reach| {
                model
                    .kind(reach.event)
                    .is_a(ElementKind::EventOccurrenceUsage)
            })
            .collect();
        let [from, to] = &ends[..] else { continue };
        sequence.moments.push(Moment {
            from: lifeline_at(model, from, &mut sequence.lifelines),
            to: lifeline_at(model, to, &mut sequence.lifelines),
            label: connector_label(model, child, relation),
            dashed: relation == Relation::Succession,
        });
    }
    sequence
}

/// One end of an arrow: the participant it belongs to, and the event it
/// reaches there.
struct Reach {
    /// The chain that names the participant: `vehicle.cruiseController`.
    taking_part: Vec<ElementId>,
    /// Its last step -- what the lifeline stands for.
    id: ElementId,
    /// The event the chain ends at. The standard's note has it: the nodes
    /// at the ends of a message or a succession must refer to one.
    event: ElementId,
}

/// What an end reaches, where it names a chain at all.
///
/// An end naming only the event -- a chain of one -- belongs to that
/// event's own lifeline, since there is no participant above it.
fn reach_of(model: &Model, end: ElementId) -> Option<Reach> {
    let Some(Value::RefList(chain)) = model.get(end, "chainingFeature") else {
        return None;
    };
    let (&event, before) = chain.split_last()?;
    let taking_part = if before.is_empty() {
        chain.clone()
    } else {
        before.to_vec()
    };
    Some(Reach {
        id: *taking_part.last()?,
        taking_part,
        event,
    })
}

/// The lifeline an end belongs to, added to `lifelines` the first time it
/// takes part.
fn lifeline_at(model: &Model, reach: &Reach, lifelines: &mut Vec<Lifeline>) -> usize {
    // `ref part :>> driver;` declares no name of its own and answers to
    // the one it redefines, the way every other view reads it
    let name = reach
        .taking_part
        .iter()
        .filter_map(|&step| effective_name(model, step))
        .collect::<Vec<_>>()
        .join(".");
    if let Some(at) = lifelines.iter().position(|drawn| drawn.name == name) {
        return at;
    }
    lifelines.push(Lifeline {
        id: reach.id,
        keyword: keyword(model.kind(reach.id)),
        name,
    });
    lifelines.len() - 1
}

/// Render a sequence view as a standalone SVG document.
pub fn to_svg(sequence: &Sequence, style: &Style) -> String {
    let head_height = 2.0 * style.padding + style.line_height;
    let column = sequence
        .lifelines
        .iter()
        .map(|line| style.text_width(&line.label()) + 2.0 * style.padding)
        .fold(0.0f64, f64::max)
        .max(style.line_height * 4.0);
    let step = 2.0 * style.line_height;
    let left = |at: usize| style.margin + at as f64 * (column + style.h_gap);
    let centre = |at: usize| left(at) + column / 2.0;

    let width = left(sequence.lifelines.len().max(1)) - style.h_gap + style.margin;
    let height =
        style.margin + head_height + (sequence.moments.len() as f64 + 1.0) * step + style.margin;
    let foot = height - style.margin;

    let mut body = markers();
    for (at, line) in sequence.lifelines.iter().enumerate() {
        writeln!(
            body,
            "<rect class=\"box\" x=\"{:.1}\" y=\"{:.1}\" width=\"{column:.1}\" \
             height=\"{head_height:.1}\" rx=\"{:.1}\"/>",
            left(at),
            style.margin,
            style.line_height / 2.0
        )
        .unwrap();
        writeln!(
            body,
            "<text class=\"keyword\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" \
             dominant-baseline=\"middle\">\u{ab}{}\u{bb} <tspan class=\"name\">{}</tspan></text>",
            centre(at),
            style.margin + head_height / 2.0,
            escape(&line.keyword),
            escape(&line.name),
        )
        .unwrap();
        // the lifeline itself: dashed, and reaching past the last moment
        writeln!(
            body,
            "<line class=\"lifeline\" x1=\"{:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{foot:.1}\"/>",
            centre(at),
            style.margin + head_height,
            centre(at),
        )
        .unwrap();
    }
    for (nth, moment) in sequence.moments.iter().enumerate() {
        let y = style.margin + head_height + (nth as f64 + 1.0) * step;
        let (from, to) = (centre(moment.from), centre(moment.to));
        let class = if moment.dashed { "succession" } else { "edge" };
        writeln!(
            body,
            "<line class=\"{class}\" x1=\"{from:.1}\" y1=\"{y:.1}\" x2=\"{to:.1}\" y2=\"{y:.1}\" \
             marker-end=\"url(#message)\"/>"
        )
        .unwrap();
        if let Some(label) = &moment.label {
            writeln!(
                body,
                "<text class=\"feature\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{}</text>",
                (from + to) / 2.0,
                y - 0.35 * style.line_height,
                escape(label),
            )
            .unwrap();
        }
    }
    document(width, height, style, &body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::resolved;

    fn interaction(source: &str) -> Sequence {
        let ws = resolved(source);
        let target = ws
            .named_elements()
            .find(|(_, name)| *name == "Interaction")
            .map(|(id, _)| id)
            .unwrap();
        sequence_view(ws.model(), target)
    }

    const CRUISE: &str = "item def SetSpeed;\n\
         item def FuelCommand;\n\
         part def Ctl { event occurrence setSpeedReceived; then event occurrence fuelCommandSent; }\n\
         part def Engine { event occurrence fuelCommandReceived; }\n\
         part def Driver { event occurrence setSpeedSent; }\n\
         part def Vehicle { part ctl : Ctl; part engine : Engine; }\n\
         occurrence def Interaction {\n\
         \tref part driver : Driver;\n\
         \tref part vehicle : Vehicle;\n\
         \tmessage setSpeed of SetSpeed from driver.setSpeedSent to vehicle.ctl.setSpeedReceived;\n\
         \tmessage fuelCommand of FuelCommand\n\
         \t\tfrom vehicle.ctl.fuelCommandSent to vehicle.engine.fuelCommandReceived;\n\
         \tfirst setSpeed then fuelCommand;\n\
         }\n";

    #[test]
    fn a_lifeline_stands_for_the_participant_a_message_reaches_through() {
        let sequence = interaction(CRUISE);
        // the participant is the chain without the event it ends at, and
        // each takes its place the first time it takes part
        assert_eq!(
            sequence
                .lifelines
                .iter()
                .map(|line| (line.keyword.as_str(), line.name.as_str()))
                .collect::<Vec<_>>(),
            [
                ("part", "driver"),
                ("part", "vehicle.ctl"),
                ("part", "vehicle.engine"),
            ]
        );
    }

    #[test]
    fn an_arrow_reaches_an_event_or_it_is_no_arrow() {
        let sequence = interaction(CRUISE);
        // `first setSpeed then fuelCommand;` orders two messages rather
        // than joining two lifelines, so it draws no arrow of its own
        assert_eq!(
            sequence
                .moments
                .iter()
                .map(|moment| (moment.from, moment.to, moment.label.as_deref()))
                .collect::<Vec<_>>(),
            [
                (0, 1, Some("setSpeed of SetSpeed")),
                (1, 2, Some("fuelCommand of FuelCommand")),
            ]
        );
        assert!(sequence.moments.iter().all(|moment| !moment.dashed));
    }

    #[test]
    fn a_succession_between_two_events_is_the_dashed_arrow() {
        let sequence = interaction(
            "part def A { event occurrence sent; }\n\
             part def B { event occurrence got; }\n\
             occurrence def Interaction {\n\
             \tref part a : A;\n\
             \tref part b : B;\n\
             \tsuccession first a.sent then b.got;\n\
             }\n",
        );
        assert_eq!(sequence.lifelines.len(), 2);
        assert!(sequence.moments[0].dashed);
    }

    #[test]
    fn an_end_naming_only_the_event_belongs_to_that_events_lifeline() {
        // an interaction whose events are its own: the chain is one step
        // long, and there is no participant above it to belong to
        let sequence = interaction(
            "occurrence def Interaction {\n\
             \tevent occurrence sent;\n\
             \tevent occurrence got;\n\
             \tsuccession first sent then got;\n\
             }\n",
        );
        assert_eq!(
            sequence
                .lifelines
                .iter()
                .map(|line| line.name.as_str())
                .collect::<Vec<_>>(),
            ["sent", "got"]
        );
        assert_eq!(sequence.moments.len(), 1);
    }

    #[test]
    fn a_definition_with_no_interaction_draws_nothing() {
        let sequence = interaction("part def P;\noccurrence def Interaction { part p : P; }\n");
        assert!(sequence.lifelines.is_empty() && sequence.moments.is_empty());
    }

    #[test]
    fn the_document_carries_a_head_a_lifeline_and_an_arrow_for_each() {
        let sequence = interaction(CRUISE);
        let svg = to_svg(&sequence, &Style::default());
        assert_eq!(svg.matches("class=\"lifeline\"").count(), 3);
        assert_eq!(svg.matches("url(#message)").count(), 2);
        assert!(svg.contains(">vehicle.ctl</tspan>"), "{svg}");
        assert!(svg.contains(">setSpeed of SetSpeed</text>"), "{svg}");
    }
}
