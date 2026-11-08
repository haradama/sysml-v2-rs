//! What a quantity is once it leaves the model: the number of SI
//! coherent units it holds.
//!
//! `attribute mass : MassValue` holds a number and a unit, and a type
//! that kept both would make every expression over it a question about
//! units. What is generated holds the number alone, in the units the SI
//! builds everything else from -- kilograms, metres, seconds, and what
//! those make -- so `2500 [g]` is `2.5` and `100 [km/h]` is a speed in
//! metres per second. Nothing then checks that a mass is not added to a
//! length; that is the price of an `f64`, and the unit is written beside
//! each field so that a reader knows what the number counts.
//!
//! Every answer here is read off the model, and a unit is worth what the
//! library says it is worth: `kilogram` is `kilo` times `gram`, `hour`
//! is sixty `min`, `newton` is `kg*m/s^2`. Whatever cannot be read that
//! way -- a unit nothing resolved, an expression this does not evaluate
//! -- has no factor, and the expression it is in stays untranslated
//! rather than being given a wrong number.

use std::collections::{HashMap, HashSet};

use sysml_model::{ElementId, ElementKind, Model, Value};

/// Whether `ty` is a scalar quantity -- a specialization of
/// `Quantities::ScalarQuantityValue`, as `ISQ::MassValue` is.
///
/// Its supertypes do not lead to `Real`: a quantity value *has* a number
/// (`num : Real`) and a unit (`mRef`), and what it specializes is
/// `NumericalValue`, which is above `Real` rather than below it. So
/// this asks for the quantity by name, and a model's own `attribute def
/// Payload :> MassValue` is one as well.
pub(crate) fn is_scalar_quantity(model: &Model, ty: ElementId) -> bool {
    specializes(model, ty, "Quantities", "ScalarQuantityValue")
}

/// What the number a quantity-typed usage holds counts, for the line
/// beside it: an `f64` says nothing about kilograms.
pub(crate) fn counted_in(model: &Model, usage: ElementId) -> Option<String> {
    counted(model, crate::generate::held_type(model, usage)?)
}

/// What the number a scalar quantity `ty` comes to counts.
pub(crate) fn counted(model: &Model, ty: ElementId) -> Option<String> {
    if !is_scalar_quantity(model, ty) {
        return None;
    }
    let name = model.name(ty)?;
    Some(match si_unit(model, ty) {
        Some(unit) => format!("`{name}` in `{unit}`."),
        None => format!("`{name}` in SI coherent units."),
    })
}

/// The SI coherent unit a quantity is counted in, written the way the
/// library writes a unit: `kg`, `m/s`, `kg*m^2/s^3`.
///
/// Read off the dimension the library gives the quantity's unit --
/// `SpeedUnit` is `L^1 T^-1` -- rather than off a list of names, so a
/// quantity a model defines for itself is answered the same way.
/// Nothing where the dimension is not written as powers of the seven
/// base quantities, or is written as no power of any: a characteristic
/// number counts in ones.
pub(crate) fn si_unit(model: &Model, quantity: ElementId) -> Option<String> {
    let unit = lineage(model, quantity).into_iter().find_map(|ty| {
        let reference = member(model, ty, "mRef")?;
        model.type_of(reference)
    })?;
    let mut powers: HashMap<usize, i64> = HashMap::new();
    for factor in lineage(model, unit)
        .into_iter()
        .flat_map(|ty| model.owned(ty).iter().copied())
        .filter(|&child| {
            model.type_of(child).and_then(|ty| model.name(ty)) == Some("QuantityPowerFactor")
        })
    {
        let written = sysml_model::expression_text(
            model,
            value_of(model, member(model, factor, "quantity")?)?,
        )?;
        let symbol = written.rsplit('.').next()?.trim_matches('\'');
        let base = BASE.iter().position(|(quantity, _)| *quantity == symbol)?;
        // a root of a unit has no spelling among powers of whole units
        let exponent = Some(number(
            model,
            value_of(model, member(model, factor, "exponent")?)?,
            &mut Vec::new(),
        )?)
        .filter(|it| it.fract() == 0.0)?;
        *powers.entry(base).or_default() += exponent as i64;
    }
    let spell = |sign: i64| -> Vec<String> {
        (0..BASE.len())
            .filter_map(|base| {
                let power = *powers.get(&base)? * sign;
                (power > 0).then(|| match power {
                    1 => BASE[base].1.to_string(),
                    _ => format!("{}^{power}", BASE[base].1),
                })
            })
            .collect()
    };
    let (over, under) = (spell(1), spell(-1));
    let over = match over.is_empty() {
        true if under.is_empty() => return None,
        true => "1".to_string(),
        false => over.join("*"),
    };
    Some(match under.len() {
        0 => over,
        1 => format!("{over}/{}", under[0]),
        _ => format!("{over}/({})", under.join("*")),
    })
}

/// The ISQ base quantities, by the symbol `isq` declares each under, and
/// the SI base unit that counts it -- in the order a unit is
/// conventionally written.
const BASE: [(&str, &str); 7] = [
    ("M", "kg"),
    ("L", "m"),
    ("T", "s"),
    ("I", "A"),
    ("Θ", "K"),
    ("N", "mol"),
    ("J", "cd"),
];

/// How many SI coherent units one of each unit written in brackets under
/// `root` is, keyed by the unit as written with its spaces taken out --
/// which is how the expression translator, reading the same text, asks.
///
/// `2500 [g]` and `100 [km/h]` are each an operator `[` over a number
/// and a unit expression, and the unit expression's names were resolved
/// where it is written, so what `g` means here is what the model's own
/// imports made it mean.
pub(crate) fn units_under(model: &Model, root: ElementId) -> HashMap<String, f64> {
    model
        .descendants(root)
        .into_iter()
        .filter(|&expression| {
            model.maybe(expression, "operator").and_then(Value::as_str) == Some("[")
        })
        .filter_map(|expression| {
            let unit = *operands(model, expression).get(1)?;
            let factor = bracket_factor(model, unit)?;
            Some((
                spaceless(&sysml_model::expression_text(model, unit)?),
                factor,
            ))
        })
        .collect()
}

/// The unit text the translator and [`units_under`] agree to look up by.
pub(crate) fn spaceless(written: &str) -> String {
    written.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// The factor of a unit expression written in brackets.
///
/// The library's degree Celsius and degree Fahrenheit are units of
/// temperature *difference*, and the temperature itself is on a scale
/// with an offset rather than a unit. `20 ['°C']` written where a
/// temperature is meant would come out as twenty kelvin above absolute
/// zero, so a bracket naming one of those alone is not translated at
/// all. Kelvin is both kinds of unit, and is counted as one.
fn bracket_factor(model: &Model, unit: ElementId) -> Option<f64> {
    if model.kind(unit) == ElementKind::FeatureReferenceExpression {
        let named = referent(model, unit)?;
        let kinds: Vec<&str> = model
            .types_of(named)
            .filter_map(|ty| model.name(ty))
            .collect();
        if kinds.contains(&"TemperatureDifferenceUnit")
            && !kinds.contains(&"ThermodynamicTemperatureUnit")
        {
            return None;
        }
    }
    number(model, unit, &mut Vec::new())
}

/// The value of an expression over numbers and units, a unit counting
/// as its factor.
fn number(model: &Model, expression: ElementId, seen: &mut Vec<ElementId>) -> Option<f64> {
    match model.maybe(expression, "value") {
        Some(Value::Int(int)) => return Some(*int as f64),
        Some(Value::Real(real)) => return Some(*real),
        _ => {}
    }
    match model.kind(expression) {
        ElementKind::FeatureReferenceExpression => {
            unit_factor(model, referent(model, expression)?, seen)
        }
        ElementKind::OperatorExpression => {
            let operator = model
                .maybe(expression, "operator")
                .and_then(Value::as_str)?;
            let operands = operands(model, expression);
            let mut value = |at: usize| number(model, operands[at], seen);
            match (operator, operands.len()) {
                ("*", 2) => Some(value(0)? * value(1)?),
                ("/", 2) => {
                    let (over, under) = (value(0)?, value(1)?);
                    (under != 0.0).then(|| over / under)
                }
                ("^" | "**", 2) => Some(value(0)?.powf(value(1)?)),
                ("-", 1) => Some(-value(0)?),
                _ => None,
            }
        }
        _ => None,
    }
}

/// How many SI coherent units one `unit` is.
///
/// A unit converted from another is its factor times that other one's;
/// a unit defined by an expression is that expression's value; a unit
/// that says neither is where the library starts, and counts one --
/// except `gram`, which the library starts from so that `kilogram` can
/// be a prefixed unit like any other, and which is the one base unit the
/// SI counts in thousands.
fn unit_factor(model: &Model, unit: ElementId, seen: &mut Vec<ElementId>) -> Option<f64> {
    if seen.contains(&unit)
        || !model
            .types_of(unit)
            .any(|ty| specializes(model, ty, "MeasurementReferences", "MeasurementUnit"))
    {
        return None;
    }
    seen.push(unit);
    let factor = match member(model, unit, "unitConversion") {
        Some(conversion) => conversion_factor(model, conversion, seen),
        None => match value_of(model, unit) {
            Some(defined) => number(model, defined, seen),
            None if is_named(model, unit, "SI", "gram") => Some(1e-3),
            None => Some(1.0),
        },
    };
    seen.pop();
    factor
}

/// A `ConversionByConvention` states its factor; a `ConversionByPrefix`
/// takes it from the prefix it names.
fn conversion_factor(
    model: &Model,
    conversion: ElementId,
    seen: &mut Vec<ElementId>,
) -> Option<f64> {
    let reference = number(
        model,
        value_of(model, member(model, conversion, "referenceUnit")?)?,
        seen,
    )?;
    let stated = match member(model, conversion, "conversionFactor") {
        Some(factor) => value_of(model, factor)?,
        None => {
            let prefix = referent(
                model,
                value_of(model, member(model, conversion, "prefix")?)?,
            )?;
            value_of(model, member(model, prefix, "conversionFactor")?)?
        }
    };
    Some(number(model, stated, seen)? * reference)
}

/// Whether `ty` is, or specializes, the definition `package::name`.
fn specializes(model: &Model, ty: ElementId, package: &str, name: &str) -> bool {
    lineage(model, ty)
        .into_iter()
        .any(|it| is_named(model, it, package, name))
}

/// `ty` and everything it specializes, nearest first.
fn lineage(model: &Model, ty: ElementId) -> Vec<ElementId> {
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    let mut queue = std::collections::VecDeque::from([ty]);
    while let Some(next) = queue.pop_front() {
        if !seen.insert(next) {
            continue;
        }
        order.push(next);
        queue.extend(model.owned(next).iter().filter_map(|&child| {
            (model.kind(child) == ElementKind::Subclassification)
                .then(|| model.maybe(child, "superclassifier").and_then(Value::as_id))
                .flatten()
        }));
    }
    order
}

fn is_named(model: &Model, id: ElementId, package: &str, name: &str) -> bool {
    model.name(id) == Some(name)
        && model.owner(id).and_then(|owner| model.name(owner)) == Some(package)
}

/// The feature `owner` declares or redefines under `name`.
fn member(model: &Model, owner: ElementId, name: &str) -> Option<ElementId> {
    model
        .owned(owner)
        .iter()
        .copied()
        .find(|&child| model.effective_name(child) == Some(name))
}

/// The expression a feature's `= value` holds.
fn value_of(model: &Model, feature: ElementId) -> Option<ElementId> {
    let membership = model
        .owned(feature)
        .iter()
        .copied()
        .find(|&child| model.kind(child) == ElementKind::FeatureValue)?;
    model.maybe(membership, "value")?.as_id()
}

fn referent(model: &Model, expression: ElementId) -> Option<ElementId> {
    model.maybe(expression, "referent").and_then(Value::as_id)
}

/// What an operator expression hands over, in order: each operand is a
/// parameter feature whose value is the operand's expression.
fn operands(model: &Model, expression: ElementId) -> Vec<ElementId> {
    model
        .owned(expression)
        .iter()
        .filter(|&&child| model.kind(child) == ElementKind::Feature)
        .filter_map(|&parameter| value_of(model, parameter))
        .collect()
}
