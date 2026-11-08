//! A quantity is the `f64` of SI coherent units it holds, and a unit is
//! worth what the standard library says it is worth.
//!
//! Against the whole library and resolved the way `sysml rustgen`
//! resolves -- what the model reaches, and no more -- since a unit's
//! worth is three or four definitions away from where it is written:
//! `kW` is `kilo` times `W`, which is `J/s`, which is `N*m` over a
//! second, which is `kg*m/s^2`.

use sysml_semantics::Workspace;

const MODEL: &str = "package Q {\n\
    \tprivate import ISQ::*;\n\
    \tprivate import SI::*;\n\
    \tprivate import USCustomaryUnits::*;\n\
    \tprivate import ScalarValues::Real;\n\
    \tattribute def Payload :> MassValue {\n\
    \t\tdoc /* What the truck carries. */\n\
    \t}\n\
    \tattribute <twice> twiceMetre : LengthUnit = m + m;\n\
    \tattribute <named> namedMetre : LengthUnit = \"m\";\n\
    \tattribute <loopA> loopA : LengthUnit = loopB;\n\
    \tattribute <loopB> loopB : LengthUnit = loopA;\n\
    \tattribute plainNumber : Real = 2;\n\
    \tpart def Truck {\n\
    \t\tattribute load : Payload = 3 [kg];\n\
    \t\tattribute fuel : MassValue = 2500 [g];\n\
    \t\tattribute speed : SpeedValue = 100 [km/h];\n\
    \t\tattribute power : PowerValue = 3 [kW];\n\
    \t\tattribute stride : LengthValue = 3 [ft];\n\
    \t\tattribute hum : FrequencyValue = 50 [Hz];\n\
    \t\tattribute field : MagneticFluxDensityValue = 2 [T];\n\
    \t\tattribute drag : ReynoldsNumberValue;\n\
    \t\tattribute hot : ThermodynamicTemperatureValue = 300 [K];\n\
    \t\tattribute warmer : TemperatureDifferenceValue = 20 ['°C'];\n\
    \t\tattribute odd : LengthValue = 1 [twice];\n\
    \t\tattribute text : LengthValue = 1 [named];\n\
    \t\tattribute circular : LengthValue = 1 [loopA];\n\
    \t\tattribute counted : LengthValue = 1 [plainNumber];\n\
    \t\tattribute count : Real = 2;\n\
    \t}\n\
    \tcalc def Power {\n\
    \t\tin whlpwr : PowerValue; in cd : Real; in v : SpeedValue;\n\
    \t\treturn tp : PowerValue = whlpwr - cd * v;\n\
    \t}\n\
    \tcalc def Faster { in s : Real; return : SpeedValue = s [km/h]; }\n\
    \tpart def Cart {\n\
    \t\tattribute mass :> ISQ::mass = 12 [kg];\n\
    \t\tattribute cargo :> mass;\n\
    \t\tattribute masses :> ISQ::mass[*];\n\
    \t\tattribute ahead :> behind;\n\
    \t\tattribute behind :> ahead;\n\
    \t}\n\
    \tcalc def Lighter { in m :> ISQ::mass; return :> ISQ::mass = m - 500 [g]; }\n\
    }\n";

fn workspace() -> (Workspace, usize) {
    let mut ws = Workspace::new();
    let file = ws.add_file("q.sysml", MODEL);
    for (name, text) in sysml_stdlib::FILES {
        ws.add_file(*name, text);
    }
    let stats = ws.resolve_reached(&[file]);
    assert_eq!(stats.unresolved, 0, "{:?}", ws.unresolved());
    (ws, file)
}

fn generated() -> sysml_rust::Generated {
    let (ws, file) = workspace();
    let roots = ws.file_roots(file).to_vec();
    sysml_rust::generate(ws.model(), &roots).expect("generates")
}

/// The line of `rust` that starts, after its indentation, with `start`.
#[track_caller]
fn line<'a>(rust: &'a str, start: &str) -> &'a str {
    rust.lines()
        .map(str::trim)
        .find(|it| it.starts_with(start))
        .unwrap_or_else(|| panic!("no line starts {start:?} in:\n{rust}"))
}

#[test]
fn a_quantity_is_the_number_of_si_units_it_holds() {
    let rust = generated().rust;
    // the field is the number, and the line above it says what it counts
    assert!(
        rust.contains("    /// `MassValue` in `kg`.\n    pub fuel: f64,"),
        "{rust}"
    );
    assert!(rust.contains("/// `SpeedValue` in `m/s`.\n    pub speed: f64,"));
    assert!(rust.contains("/// `PowerValue` in `kg*m^2/s^3`.\n    pub power: f64,"));
    assert!(rust.contains("/// `FrequencyValue` in `1/s`."));
    assert!(rust.contains("/// `MagneticFluxDensityValue` in `kg/(s^2*A)`."));
    // a characteristic number counts in ones, which no unit names
    assert!(rust.contains("/// `ReynoldsNumberValue` in SI coherent units."));
    // and a value written in another unit is converted to the SI's
    for (field, value) in [
        ("load", "3.0"),
        ("fuel", "2.5"),
        ("speed", "27.7777777777778"),
        ("power", "3000.0"),
        ("stride", "0.9144"),
        ("hum", "50.0"),
        ("field", "2.0"),
        ("hot", "300.0"),
    ] {
        assert_eq!(
            line(&rust, &format!("{field}: ")),
            format!("{field}: {value},")
        );
    }
    // a primitive stays what it was, with nothing said about units
    assert!(rust.contains("\n    pub count: f64,"), "{rust}");
    assert!(!rust.contains("`Real` in"));
}

/// A model's own quantity is a quantity: the `f64` under its own name,
/// rather than the library's machinery for a quantity value flattened
/// into a struct beside a default that was the number.
#[test]
fn a_quantity_of_the_models_own_is_a_number_too() {
    let rust = generated().rust;
    assert!(
        rust.contains(
            "/// SysML: `attribute def Payload`\n/// What the truck carries.\n\
             /// `Payload` in `kg`.\npub type Payload = f64;"
        ),
        "{rust}"
    );
    assert!(!rust.contains("pub struct Payload"));
    assert!(rust.contains("/// `Payload` in `kg`.\n    pub load: f64,"));
}

/// A unit this cannot put a number to leaves the value it is in
/// unwritten, and says so -- rather than a declared `20 ['°C']` reading
/// as a zero the model chose.
#[test]
fn a_unit_without_a_number_is_said_and_not_guessed() {
    let generated = generated();
    let rust = &generated.rust;
    for (field, written) in [
        // a temperature difference, where a temperature may be meant
        ("warmer", "20 ['°C']"),
        // `+` is no way to make a unit
        ("odd", "1 [twice]"),
        // nor is a string
        ("text", "1 [named]"),
        // a unit defined by itself is defined by nothing
        ("circular", "1 [loopA]"),
        // and a number is no unit at all
        ("counted", "1 [plainNumber]"),
    ] {
        let said = format!(
            "    // left open: `{field}` starts at its type's default: `= {written}` is beyond the simple subset\n"
        );
        assert!(rust.contains(&said), "{said}\n{rust}");
        assert_eq!(
            line(rust, &format!("{field}: ")),
            format!("{field}: Default::default(),")
        );
        assert!(
            generated
                .open
                .iter()
                .any(|open| open.why.starts_with(&format!("`{field}` starts at"))),
            "{:?}",
            generated.open
        );
    }
}

/// What a calculation over quantities computes is computed over their
/// numbers, which are all in one system of units already.
#[test]
fn a_calculation_over_quantities_is_arithmetic_on_numbers() {
    let rust = generated().rust;
    assert!(
        rust.contains(
            "pub fn power(whlpwr: f64, cd: f64, v: f64) -> f64 {\n    whlpwr - (cd * v)\n}"
        ),
        "{rust}"
    );
    assert!(rust.contains("pub fn faster(s: f64) -> f64 {\n    s * 0.2777777777777778\n}"));
}

/// And all of it is Rust.
#[test]
fn what_is_generated_compiles() {
    let rust = generated().rust;
    let dir = std::env::temp_dir().join(format!("sysml-rust-quantity-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let at = dir.join("q.rs");
    std::fs::write(&at, &rust).unwrap();
    let out = std::process::Command::new(std::env::var("RUSTC").unwrap_or("rustc".into()))
        .args([
            "--crate-type",
            "lib",
            "--edition",
            "2021",
            "--emit=metadata",
        ])
        .arg("-o")
        .arg(dir.join("q.rmeta"))
        .arg(&at)
        .output()
        .expect("rustc runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "generated Rust does not compile:\n{}\n---\n{rust}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The plan says the same, in no language: a quantity is a `Real`, and
/// the unit its number counts.
#[test]
fn the_plan_says_what_a_quantity_counts() {
    let (mut ws, file) = workspace();
    let roots = ws.file_roots(file).to_vec();
    let plan = sysml_rust::plan::of(&mut ws, &roots);
    let truck = plan
        .definitions
        .iter()
        .find(|it| it.of == "Q::Truck")
        .expect("the truck is planned");
    let feature = |name: &str| {
        truck
            .features
            .iter()
            .find(|it| it.name == name)
            .unwrap_or_else(|| panic!("{name} is planned"))
    };
    for (name, unit) in [("fuel", Some("kg")), ("speed", Some("m/s")), ("drag", None)] {
        assert_eq!(feature(name).primitive.as_deref(), Some("Real"), "{name}");
        assert_eq!(feature(name).unit.as_deref(), unit, "{name}");
    }
    assert_eq!(feature("count").primitive.as_deref(), Some("Real"));
    assert_eq!(feature("count").unit, None);
    let payload = plan
        .definitions
        .iter()
        .find(|it| it.of == "Q::Payload")
        .expect("the payload is planned");
    assert_eq!(payload.primitive.as_deref(), Some("Real"));
    // and the unit survives the trip through JSON
    let json = serde_json::to_string(&plan).unwrap();
    assert!(json.contains(r#""unit":"kg""#), "{json}");
    let read: sysml_rust::plan::Plan = serde_json::from_str(&json).unwrap();
    assert_eq!(read, plan);
}

/// A feature that names no type of its own holds what the feature it
/// subsets holds: `attribute mass :> ISQ::mass;` is a mass, which is how
/// the corpus writes most of its quantities.
#[test]
fn a_subset_of_a_quantity_is_a_quantity() {
    let rust = generated().rust;
    assert!(
        rust.contains("    /// `MassValue` in `kg`.\n    pub mass: f64,"),
        "{rust}"
    );
    assert_eq!(line(&rust, "mass: "), "mass: 12.0,");
    // however many subsets away
    assert!(rust.contains("    /// `MassValue` in `kg`.\n    pub cargo: f64,"));
    // the multiplicity is the subset's own: `ISQ::mass` is `[*]`, and a
    // feature that says nothing about how many is one of them
    assert!(rust.contains("    pub masses: Vec<f64>,"));
    assert!(
        rust.contains("pub fn lighter(m: f64) -> f64 {\n    m - 0.5\n}"),
        "{rust}"
    );
    // and two features that subset each other hold nothing anyone wrote
    for field in ["ahead", "behind"] {
        assert!(
            rust.contains(&format!(
                "// not generated: `{field}` -- no Rust type for its SysML type"
            )),
            "{rust}"
        );
    }
}

/// The plan reads the same type the generator does.
#[test]
fn the_plan_types_a_subset_by_what_it_subsets() {
    let (mut ws, file) = workspace();
    let roots = ws.file_roots(file).to_vec();
    let plan = sysml_rust::plan::of(&mut ws, &roots);
    let cart = plan
        .definitions
        .iter()
        .find(|it| it.of == "Q::Cart")
        .expect("the cart is planned");
    let mass = cart.features.iter().find(|it| it.name == "mass").unwrap();
    assert_eq!(mass.typed_by.as_deref(), Some("ISQBase::MassValue"));
    assert_eq!(mass.primitive.as_deref(), Some("Real"));
    assert_eq!(mass.unit.as_deref(), Some("kg"));
    assert!(mass.multiplicity.is_one());
}
