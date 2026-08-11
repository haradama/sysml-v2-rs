//! The generator's own rules, on small models: what becomes a comment,
//! what becomes an error, and how names survive.

const SCALARS: &str = "package ScalarValues {\n\
    \tabstract datatype Boolean;\n\
    \tabstract datatype String;\n\
    \tabstract datatype Real;\n\
    \tabstract datatype Integer;\n\
    \tabstract datatype Natural;\n\
    \tabstract datatype Positive;\n\
    \tabstract datatype Complex;\n}\n";

/// A miniature bound API package, the shape `sysml import-rust` writes.
const API: &str = "package Api {\n\
    \tprivate import ScalarValues::*;\n\
    \tmetadata def rust {\n\
    \t\tattribute path : String;\n\
    \t\tattribute crateName : String;\n\
    \t\tattribute takesSelf : String;\n\
    \t\tattribute isAsync : Boolean;\n\
    \t\tattribute isFallible : Boolean;\n\
    \t}\n\
    \tport def Store { @rust { :>> path = \"fake::Store\"; } }\n\
    \tport def Metrics { @rust { :>> path = \"fake::Metrics\"; } }\n\
    \taction def Ping { @rust { :>> path = \"fake::Store::ping\"; :>> takesSelf = \"&self\"; :>> isAsync = false; :>> isFallible = false; } }\n\
    \taction def Consume { @rust { :>> path = \"fake::Store::consume\"; :>> takesSelf = \"self\"; } }\n\
    \taction def Orphan { @rust { :>> path = \"elsewhere::Api::orphan\"; :>> takesSelf = \"&self\"; } }\n\
    \taction def Unbound;\n\
    \taction def Fuzzy { @rust { :>> path = \"fake::Store::fuzzy\"; :>> takesSelf = \"&self\"; } in blob : Plain; }\n\
    \taction def Purge { @rust { :>> path = \"fake::Store::purge\"; :>> takesSelf = \"&self\"; :>> isFallible = true; } out error : String; }\n\
    \taction def Shuffle { @rust { :>> path = \"fake::Store::shuffle\"; :>> takesSelf = \"&self\"; } inout buffer : String; }\n\
    \taction def Vanish { @rust { :>> path = \"fake::Store::vanish\"; :>> takesSelf = \"&self\"; :>> isFallible = true; } out result : String; }\n\
    \taction def Count { @rust { :>> path = \"fake::Store::count\"; :>> takesSelf = \"&self\"; :>> isFallible = false; } out result : Real; }\n\
    \taction def Watch { @rust { :>> path = \"fake::Store::watch\"; :>> takesSelf = \"&mut self\"; :>> isAsync = true; :>> isFallible = true; } in sku : String; out result : Payload; out error : String; }\n\
    \titem def Payload { @rust { :>> path = \"fake::Payload\"; } }\n\
    \tport def Plain;\n}\n";

fn generate(system: &str) -> Result<String, sysml_rust::RustgenError> {
    let mut ws = sysml_semantics::Workspace::new();
    ws.add_file("scalars.kerml", SCALARS);
    ws.add_file("api.sysml", API);
    let file = ws.add_file("system.sysml", system);
    let stats = ws.resolve_all();
    assert_eq!(stats.unresolved, 0, "the test model must resolve");
    let roots = ws.file_roots(file).to_vec();
    sysml_rust::generate(ws.model(), &roots)
}

#[test]
fn what_has_no_shape_becomes_a_comment_not_silence() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Node {\n\
         \t\tport plain : Plain;\n\
         \t\tport store : Store;\n\
         \t\tattribute label;\n\
         \t\tpart sub : Node;\n\
         \t\tperform action ping : Ping;\n\
         \t\tperform action gone : Consume;\n\
         \t\tperform action unbound : Unbound;\n\
         \t}\n}\n",
    )
    .unwrap();
    // the bound port becomes a generic; the API package's own defs are
    // not regenerated, and composing a generic struct has no plain shape
    assert!(rust.contains("pub struct Node<Store: fake::Store>"));
    assert!(
        rust.contains("// not generated: port `plain` -- its type is neither bound nor generated")
    );
    assert!(rust.contains("// not generated: `label` -- no Rust type for its SysML type"));
    assert!(rust.contains(
        "// not generated: `sub` -- a composition cycle through API ports has no finite generic signature"
    ));
    assert!(!rust.contains("pub sub:"));
    assert!(rust.contains("pub fn ping(&self)"));
    assert!(rust.contains("-- `fake::Store::consume` consumes its receiver"));
    assert!(rust.contains("-- `Unbound` carries no `@rust` binding"));
}

#[test]
fn signatures_follow_the_binding_not_guesswork() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Cellar {\n\
         \t\tport main_store : Store;\n\
         \t\tperform action purge : Purge;\n\
         \t\tperform action fuzz : Fuzzy;\n\
         \t\tperform action shuffle : Shuffle;\n\
         \t\tperform action vanish : Vanish;\n\
         \t}\n\
         \tpart def Ledger {\n\
         \t\tref;\n\
         \t\tattribute 'loop' : Boolean;\n\
         \t\tattribute ratio : Real;\n\
         \t\tattribute shift : Integer;\n\
         \t}\n}\n",
    )
    .unwrap();
    // an error-only fallible call returns Result<(), _>
    assert!(rust.contains("pub fn purge(&self) -> Result<(), String>"));
    // but one that says it can fail without saying with what would have
    // to be declared as returning something the real function does not
    assert!(rust.contains(
        "// not generated: perform `vanish` -- it can fail, but the model does not say with what"
    ));
    assert!(!rust.contains("fn vanish"));
    // an unmappable parameter and an inout parameter degrade to comments
    assert!(rust.contains("parameter `blob` has no Rust type"));
    assert!(
        rust.contains("pub fn shuffle(&self)"),
        "inout is dropped, not fatal"
    );
    // underscored port names camel into parameters
    assert!(rust.contains("MainStore: fake::Store"));
    // a portless part still gets its struct, keywords become raw idents
    assert!(rust.contains("pub struct Ledger {"));
    assert!(rust.contains("pub r#loop: bool,"));
    assert!(rust.contains("pub ratio: f64,"));
    assert!(rust.contains("pub shift: i64,"));
}

#[test]
fn data_definitions_flatten_compose_and_recurse() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tenum def Color {\n\t\tenum red;\n\t\tenum green;\n\t}\n\
         \tabstract part def Asset {\n\t\tattribute owner : String;\n\t}\n\
         \tpart def Machine :> Asset {\n\
         \t\tdoc /* One machine on the floor. */\n\
         \t\tattribute serial : String = \"SN-0\";\n\
         \t\tattribute mass : Real = 120.5;\n\
         \t\tattribute color : Color;\n\
         \t\tattribute shift : Integer = -2;\n\
         \t\tattribute active : Boolean = true;\n\
         \t}\n\
         \tpart def Rack {\n\
         \t\tattribute slots : Integer[4];\n\
         \t}\n\
         \tpart def Chain {\n\
         \t\tpart next : Chain[0..1];\n\
         \t\tpart parts : Machine[*];\n\
         \t\tpart wheel : Wheel;\n\
         \t}\n\
         \tpart def Refined :> Machine {\n\
         \t\tattribute :>> mass = 99.0;\n\
         \t}\n\
         \tport def Gauge {\n\t\tattribute reading : Real;\n\t}\n\
         \tpart def Panel {\n\t\tport gauge : Gauge;\n\t}\n\
         \tvariation part def Wheel {\n\
         \t\tvariant part steel : Machine;\n\
         \t\tvariant part alloy;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // enums, with the first value as the default
    assert!(rust.contains("pub enum Color {"));
    assert!(rust.contains("    #[default]\n    Red,"));
    // the abstract base flattens into the subtype and gets no struct
    assert!(rust.contains("// `Asset` is abstract"));
    assert!(!rust.contains("pub struct Asset"));
    assert!(rust.contains("pub struct Machine {"));
    assert!(rust.contains("pub owner: String,"));
    // multiplicities: array, option-of-box on the cycle, vec
    assert!(rust.contains("pub slots: [i64; 4],"));
    assert!(rust.contains("pub next: Option<Box<Chain>>,"));
    assert!(rust.contains("pub parts: Vec<Machine>,"));
    // declared values become Default, redefinition overrides the value
    assert!(rust.contains("impl Default for Machine"));
    assert!(rust.contains("mass: 120.5,"));
    assert!(rust.contains("serial: \"SN-0\".to_string(),"));
    assert!(rust.contains("shift: -2,"));
    assert!(rust.contains("active: true,"));
    assert!(rust.contains("pub struct Refined {"));
    assert!(rust.contains("mass: 99.0,"));
    // a variation composes like any value type
    assert!(rust.contains("pub wheel: Wheel,"));
    // an unbound port composes the port def's own struct
    assert!(rust.contains("pub struct Gauge {"));
    assert!(rust.contains("pub gauge: Gauge,"));
    // a variation is an enum, typed variants carrying their struct
    assert!(rust.contains("pub enum Wheel {"));
    assert!(rust.contains("Steel(Machine),"));
    assert!(rust.contains("    Alloy,"));
    // an array is as startable as what it repeats, built without asking
    // the element to be `Copy`; empty containers start empty
    assert!(rust.contains("impl Default for Rack"));
    assert!(rust.contains("slots: std::array::from_fn(|_| Default::default()),"));
    assert!(rust.contains("impl Default for Chain"));
}

#[test]
fn a_port_is_a_field_that_default_has_to_start_too() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tport def Signal {\n\t\tattribute level : Real;\n\t}\n\
         \tpart def Layer {\n\
         \t\tdoc /* One layer.\n\
         \t\t     *\n\
         \t\t     * Its dimensions are counts, so they cannot be zero.\n\
         \t\t     */\n\
         \t\tattribute size : Positive = 8;\n\
         \t\tattribute scaling : Real = 1.0;\n\
         \t\tport emit : Signal;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // `Positive` is a `Natural` the model has ruled zero out of
    assert!(rust.contains("pub size: u64,"));
    assert!(rust.contains("size: 8,"));
    // the port is a field of the struct, so `Default` has to fill it in
    // -- a `Self { .. }` short of a field does not compile
    assert!(rust.contains("pub emit: Signal,"));
    assert!(rust.contains("impl Default for Layer"));
    assert!(rust.contains("emit: Default::default(),"));
    // documentation keeps the lines it was written on, and loses the
    // `*` that lines them up in the model text
    assert!(rust.contains(
        "/// One layer.\n\
         ///\n\
         /// Its dimensions are counts, so they cannot be zero.\n"
    ));
    assert!(!rust.contains("/// One layer. *"));
}

#[test]
fn names_arrive_in_rusts_case_and_clear_of_the_words_it_reserves() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Layer {\n\
         \t\tattribute inputScaling : Real = 1.0;\n\
         \t\tattribute 'loop' : Integer = 2;\n\
         \t\tattribute 'self' : Real;\n\
         \t\tcalc scaled : Real = inputScaling * 2.0;\n\
         \t}\n\
         \tcalc def Widen {\n\
         \t\tin reservoirSize : Integer;\n\
         \t\tin inputDimension : Integer;\n\
         \t\treturn : Integer = 1 + inputDimension + reservoirSize;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // the model's own case is the one Rust keeps for types alone
    assert!(rust.contains("pub struct Layer {"));
    assert!(rust.contains("pub input_scaling: f64,"));
    assert!(rust.contains("input_scaling: 1.0,"));
    // a reserved word takes its raw form, and the four with no raw form
    // a suffix instead
    assert!(rust.contains("pub r#loop: i64,"));
    assert!(rust.contains("pub self_: f64,"));
    // what is declared and what reads it have to move together, or the
    // renaming would generate expressions over names that do not exist
    assert!(rust.contains("pub fn widen(reservoir_size: i64, input_dimension: i64) -> i64 {"));
    assert!(rust.contains("(1 + input_dimension) + reservoir_size"));
    assert!(rust.contains("self.input_scaling * 2.0"));
}

#[test]
fn what_the_model_leaves_abstract_is_asked_for_rather_than_panicked_over() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tabstract calc def Activate {\n\
         \t\tdoc /* Componentwise, whatever it turns out to be. */\n\
         \t\tin x : Real;\n\
         \t\treturn : Real;\n\
         \t}\n\
         \tcalc def Blend {\n\
         \t\tin a : Real;\n\
         \t\tin b : Real;\n\
         \t\treturn : Real;\n\
         \t}\n\
         \tcalc def Twice {\n\
         \t\tin a : Real;\n\
         \t\treturn : Real = a * 2.0;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // abstract: a trait with one method and no default body, so the
    // compiler asks for it instead of a `todo!` reached at runtime
    assert!(rust.contains("pub trait Activate {\n    fn activate(&self, x: f64) -> f64;\n}"));
    assert!(!rust.contains("pub fn activate"));
    assert!(rust.contains("/// Componentwise, whatever it turns out to be."));
    // not abstract, only unwritten: the model claimed a formula it did
    // not give, and that is the generator's `todo!`, not the reader's
    // duty to implement
    assert!(rust.contains("pub fn blend(a: f64, b: f64) -> f64 {"));
    assert!(!rust.contains("pub trait Blend"));
    assert!(rust.contains("todo!()"));
    // and a formula it can read still translates
    assert!(rust.contains("a * 2.0"));
}

#[test]
fn a_calc_usage_performs_its_definition_over_what_the_part_bound() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Vector {\n\t\tattribute component : Real[*];\n\t}\n\
         \tabstract calc def Blend {\n\
         \t\tin previous : Vector;\n\
         \t\tin excitation : Vector;\n\
         \t\tin leak : Real;\n\
         \t\treturn : Vector;\n\
         \t}\n\
         \tcalc def Widen {\n\
         \t\tin size : Integer;\n\
         \t\tin extra : Integer;\n\
         \t\treturn : Integer = size + extra;\n\
         \t}\n\
         \tpart def Pool {\n\
         \t\tattribute stored : Vector;\n\
         \t\tattribute leakRate : Real;\n\
         \t\tattribute capacity : Integer;\n\
         \t\tcalc step : Blend {\n\
         \t\t\tin previous = stored;\n\
         \t\t\tin leak = leakRate;\n\
         \t\t}\n\
         \t\tcalc widen : Widen {\n\
         \t\t\tin size = capacity;\n\
         \t\t}\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // a bound parameter is read off the part -- cloned, since the method
    // only borrows it -- and an unbound one stays the caller's to give;
    // the abstract definition's implementation comes along as the trait
    assert!(rust.contains("pub fn step(&self, with: &impl Blend, excitation: Vector) -> Vector {"));
    assert!(rust.contains("with.blend(self.stored.clone(), excitation, self.leak_rate)"));
    // a definition that had a formula is simply called: no trait, and a
    // scalar read needs no clone
    assert!(rust.contains("pub fn widen(&self, extra: i64) -> i64 {"));
    assert!(rust.contains("widen(self.capacity, extra)"));
    // the parameters a usage binds declare no type of their own, and
    // that is no longer a reason to skip the method
    assert!(!rust.contains("not generated: calc"));
}

#[test]
fn a_performed_calculation_takes_literals_and_keeps_its_provider_apart() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tabstract calc def Blend {\n\
         \t\tdoc /* Documentation is a child, and no parameter of anything. */\n\
         \t\tin : Real;\n\
         \t\tin with : Real;\n\
         \t\tin leak : Real;\n\
         \t\treturn : Real;\n\
         \t}\n\
         \tpart def Pool {\n\
         \t\tattribute rate : Real;\n\
         \t\tcalc step : Blend {\n\
         \t\t\tin leak = 0.25;\n\
         \t\t}\n\
         \t\tcalc scaled : Real = rate * 2.0;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // a parameter of the definition named `with` would collide with the
    // implementation the caller brings, so that one steps aside instead
    assert!(rust.contains("pub fn step(&self, with_: &impl Blend, with: f64) -> f64 {"));
    // a literal binding is passed as written, and the nameless parameter
    // is skipped on both sides, so the call still fits the trait
    assert!(rust.contains("fn blend(&self, with: f64, leak: f64) -> f64;"));
    assert!(rust.contains("with_.blend(with, 0.25)"));
    // a usage with a formula of its own is not a performance of anything
    assert!(rust.contains("pub fn scaled(&self) -> f64 {"));
}

#[test]
fn a_calculation_that_cannot_be_performed_says_which_part_of_it_stopped() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute elsewhere : Real = 1.0;\n\
         \tport def Signal;\n\
         \tpart def Probe {\n\t\tport wire : Signal;\n\t}\n\
         \tabstract calc def Untyped {\n\t\tin odd : Complex;\n\t\treturn : Real;\n\t}\n\
         \tabstract calc def NoResult {\n\t\tin a : Real;\n\t\treturn : Complex;\n\t}\n\
         \tabstract calc def Rich {\n\t\tin leak : Real;\n\t\treturn : Real;\n\t}\n\
         \tabstract calc def Reads {\n\t\tin probe : Probe;\n\t\treturn : Real;\n\t}\n\
         \tpart def Pool {\n\
         \t\tpart sensor : Probe;\n\
         \t\tcalc one : Untyped;\n\
         \t\tcalc two : NoResult;\n\
         \t\tcalc three : Rich {\n\t\t\tin leak = elsewhere;\n\t\t}\n\
         \t\tcalc four : Reads {\n\t\t\tin probe = sensor;\n\t\t}\n\
         \t\tcalc five : Probe;\n\
         \t\tcalc six = 1.0;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // every refusal names the parameter that caused it, and none of them
    // is silence
    assert!(rust.contains("calc `one` -- parameter `odd` of `Untyped` has no Rust type"));
    assert!(rust.contains("calc `two` -- the result of `NoResult` has no Rust type"));
    assert!(rust.contains("calc `three` -- `leak` is bound to `elsewhere`, beyond the simple"));
    assert!(rust.contains("calc `four` -- `probe` is read off the part, and `Probe` cannot be"));
    assert!(!rust.contains("pub fn one"));
    // typed by something that is no calculation at all, the usage is not
    // a performance of anything and falls back to being one itself
    assert!(rust.contains("pub fn five(&self) -> Probe {"));
    // and typed by nothing at all, there is nothing to perform and no
    // result to return either
    assert!(rust.contains("calc `six` -- its result has no Rust type"));
}

#[test]
fn behaviour_constraints_and_states_are_asked_for_rather_than_dropped() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Vector {\n\t\tattribute component : Real[*];\n\t}\n\
         \taction def Harvest {\n\
         \t\tdoc /* Drive it and keep what it passes through. */\n\
         \t\tin stream : Vector[1..*];\n\
         \t\tin seed : Natural;\n\
         \t\tout kept : Vector[*];\n\
         \t\tout count : Natural;\n\
         \t}\n\
         \taction def Reset;\n\
         \tconstraint def Stable {\n\t\tin radius : Real;\n\t\tradius < 1.0\n\t}\n\
         \tstate def Phase {\n\
         \t\tstate idle;\n\
         \t\tstate running;\n\
         \t\ttransition go first idle then running;\n\
         \t}\n\
         \tpart def Pool {\n\
         \t\tattribute radius : Real = 0.5;\n\
         \t\tattribute seed : Natural = 7;\n\
         \t\tperform action gather : Harvest {\n\t\t\tin seed = seed;\n\t\t}\n\
         \t\tperform action clear : Reset;\n\
         \t\tassert constraint steady : Stable {\n\t\t\tin radius = radius;\n\t\t}\n\
         \t\tstate phase : Phase;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // a behaviour is a trait: the model names it without saying how it
    // runs, and several `out` parameters make a tuple of the result
    assert!(rust
        .contains("fn harvest(&mut self, stream: Vec<Vector>, seed: u64) -> (Vec<Vector>, u64);"));
    // nothing in and nothing out needs no return clause at all
    assert!(rust.contains("fn reset(&mut self);"));
    // a constraint is boolean by definition, declared or not
    assert!(rust.contains("pub fn stable(radius: f64) -> bool {"));
    assert!(rust.contains("radius < 1.0"));
    // performing one reads the bound parameters off the part, exactly as
    // a performed calculation does
    assert!(
        rust.contains("pub fn gather(&self, with: &mut impl Harvest, stream: Vec<Vector>) -> (Vec<Vector>, u64) {")
    );
    assert!(rust.contains("with.harvest(stream, self.seed)"));
    assert!(rust.contains("pub fn clear(&self, with: &mut impl Reset) {"));
    // an assertion is the means of asking, since Rust cannot be held to
    // the model's claim that it holds
    assert!(rust.contains("pub fn steady(&self) -> bool {"));
    assert!(rust.contains("stable(self.radius)"));
    // and the state a part is in is a field, starting where the machine
    // starts
    assert!(rust.contains("pub phase: PhaseState,"));
    assert!(rust.contains("phase: PhaseState::initial(),"));
    assert!(!rust.contains("not generated"));
}

#[test]
fn a_behaviour_or_a_claim_it_cannot_write_is_named_along_with_its_reason() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Thing;\n\
         \taction def OneOut {\n\t\tin : Real;\n\t\tin a : Real;\n\t\tout only : Real;\n\t}\n\
         \taction def BadIn {\n\t\tin odd : Complex;\n\t\tout r : Real;\n\t}\n\
         \taction def BadOut {\n\t\tin a : Real;\n\t\tout odd : Complex;\n\t}\n\
         \tpart def Pool {\n\
         \t\tattribute rate : Real;\n\
         \t\tperform action one : OneOut {\n\t\t\tin a = rate;\n\t\t}\n\
         \t\tperform action bad : BadIn;\n\
         \t\tperform action worse : BadOut;\n\
         \t\tassert constraint loose : Thing;\n\
         \t\tassert constraint mystery;\n\
         \t\tstate stray : Thing;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // one result is that result, and a nameless parameter is skipped on
    // both sides of the call
    assert!(rust.contains("fn one_out(&mut self, a: f64) -> f64;"));
    assert!(rust.contains("pub fn one(&self, with: &mut impl OneOut) -> f64 {"));
    // a definition it cannot write is a comment naming what stopped it,
    // and so is every use of that definition
    assert!(rust.contains("action def `BadIn` -- parameter `odd` has no Rust type"));
    assert!(rust.contains("action def `BadOut` -- a result of it has no Rust type"));
    assert!(rust.contains("perform `bad` -- parameter `odd` of `BadIn` has no Rust type"));
    assert!(rust.contains("perform `worse` -- a result of it has no Rust type"));
    // a claim about something that is no constraint, and a state whose
    // definition is no machine, are refused by name rather than in
    // silence
    assert!(rust.contains("assert `loose` -- `Thing` is no generated constraint"));
    assert!(rust.contains("assert `mystery` -- its constraint did not resolve"));
    assert!(rust.contains("state `stray` -- its definition is no generated state machine"));
}

#[test]
fn a_calculation_calls_another_only_where_it_can_name_the_function() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Thing;\n\
         \tcalc def Twice {\n\t\tin a : Real;\n\t\treturn : Real = a * 2.0;\n\t}\n\
         \tcalc def Zero {\n\t\treturn : Real = 0.0;\n\t}\n\
         \tabstract calc def Opaque {\n\t\tin a : Real;\n\t\treturn : Real;\n\t}\n\
         \tconstraint def Small {\n\t\tin a : Real;\n\t\tTwice(a) < 10.0 and Zero() >= 0.0\n\t}\n\
         \tconstraint def Blocked {\n\t\tin a : Real;\n\t\tOpaque(a) < 1.0\n\t}\n\
         \tconstraint def Absurd {\n\t\tin a : Real;\n\t\tThing(a) < 1.0\n\t}\n\
         }\n",
    )
    .unwrap();
    // a definition that generated a function can be called, with
    // arguments or without
    assert!(rust.contains("(twice(a) < 10.0) && (zero() >= 0.0)"));
    // an abstract one generated a trait, and a trait method is not
    // callable out of nowhere -- so the formula stays the model's own
    assert!(rust.contains("todo!(\"{}\", \"Opaque(a) < 1.0\")"));
    // and neither is something that is no calculation at all
    assert!(rust.contains("todo!(\"{}\", \"Thing(a) < 1.0\")"));
}

#[test]
fn the_long_tail_of_shapes_and_signatures() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tdoc /* the whole package */\n\
         \tenum def Mood {\n\t\tdoc /* how it feels */\n\t\tenum calm;\n\t}\n\
         \tvariation part def Pick {\n\
         \t\tdoc /* one of these */\n\
         \t\tvariant part first_choice;\n\
         \t}\n\
         \tpart def A2 :> B2;\n\
         \tpart def B2 :> A2;\n\
         \tstate def Panel;\n\
         \tpart def Holder {\n\
         \t\tpart loose;\n\
         \t\taction helper;\n\
         \t\tattribute external_item : Payload;\n\
         \t\tattribute machinelike : Panel;\n\
         \t\tattribute counted : Integer[count];\n\
         \t\tattribute window : Integer[2..5];\n\
         \t\tattribute count : Integer;\n\
         \t\tattribute vague : Real =;\n\
         \t\tattribute derived_default : Real = 1.0 + 2.0;\n\
         \t\tport gauge : Meter {\n\t\t\tdoc /* the meter */\n\t\t}\n\
         \t}\n\
         \tport def Meter {\n\t\tattribute level : Real;\n\t}\n\
         \tpart def Caller {\n\
         \t\tport store : Store;\n\
         \t\tperform action local : LocalPing;\n\
         \t}\n\
         \taction def LocalPing {\n\
         \t\t@rust { :>> path = \"fake::Store::local\"; :>> takesSelf = \"&self\"; }\n\
         \t\tin holder : Holder;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // docs travel onto enums, variations and ports
    assert!(rust.contains("/// how it feels"));
    assert!(rust.contains("/// one of these"));
    assert!(rust.contains("/// the meter"));
    // a mutual specialization cannot flatten and says so
    assert!(rust.contains("// not generated: `A2` -- its specializations form a circle"));
    assert!(rust.contains("// not generated: `B2` -- its specializations form a circle"));
    // externals as fields; state machines are not value types
    assert!(rust.contains("pub external_item: fake::Payload,"));
    assert!(rust.contains("// not generated: `machinelike` -- no Rust type for its SysML type"));
    // untyped members and stray actions become notes
    assert!(rust.contains("// not generated: `loose` -- no Rust type for its SysML type"));
    assert!(rust.contains("`helper` -- ActionUsage not generated"));
    // a named or ranged bound degrades to a Vec
    assert!(rust.contains("pub counted: Vec<i64>,"));
    assert!(rust.contains("pub window: Vec<i64>,"));
    // a valueless or computed default falls back to the type's own;
    // the External field keeps Holder from any Default impl at all
    assert!(rust.contains("pub vague: f64,"));
    assert!(rust.contains("pub derived_default: f64,"));
    assert!(!rust.contains("impl Default for Holder"));
    // a parameter typed by a generated struct uses that struct
    assert!(rust.contains("pub fn local(&self, holder: Holder)"));
}

#[test]
fn generic_parts_compose_with_their_parameters_carried_along() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tpart def Probe {\n\
         \t\tport store : Store;\n\
         \t\tperform action ping : Ping;\n\
         \t}\n\
         \tpart def Bay {\n\
         \t\tattribute label : String;\n\
         \t\tpart probe : Probe;\n\
         \t}\n\
         \tpart def Hall {\n\
         \t\tpart left_bay : Bay;\n\
         \t\tpart right_bay : Bay;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // the parameter travels up two levels, renamed after each field
    assert!(rust.contains("pub struct Bay<ProbeStore: fake::Store>"));
    assert!(rust.contains("pub probe: Probe<ProbeStore>,"));
    assert!(rust.contains(
        "pub struct Hall<LeftBayProbeStore: fake::Store, RightBayProbeStore: fake::Store>"
    ));
    assert!(rust.contains("pub left_bay: Bay<LeftBayProbeStore>,"));
    assert!(rust.contains("pub right_bay: Bay<RightBayProbeStore>,"));
    // generic structs claim neither Default nor derives
    assert!(!rust.contains("impl Default for Bay"));
    assert!(!rust.contains("#[derive(Debug, Clone, PartialEq)]\npub struct Bay"));
}

#[test]
fn requirement_stubs_dedupe_and_survive_odd_satisfactions() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tpart def Probe {\n\
         \t\tport store : Store;\n\
         \t\tperform action ping : Ping;\n\
         \t}\n\
         \tpart def Rig {\n\
         \t\tpart x_y : Probe;\n\
         \t\tpart xY : Probe;\n\
         \t}\n\
         \trequirement def Latency {\n\t\tdoc /* under 100ms */\n\t}\n\
         \trequirement def Coverage;\n\
         \trequirement : Latency;\n\
         \tsatisfy Latency by Probe;\n\
         \tsatisfy Coverage;\n\
         \tpackage Inner {\n\
         \t\trequirement def Latency;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // colliding parameter names get a bump instead of a clash
    assert!(rust.contains("pub struct Rig<XYStore: fake::Store, XYStore1: fake::Store>"));
    assert!(rust.contains("pub x_y: Probe<XYStore>,"));
    // one stub per requirement name, docs and satisfiers where they exist
    assert_eq!(rust.matches("fn latency()").count(), 1);
    assert!(rust.contains("/// under 100ms"));
    assert!(rust.contains("/// Satisfied by `Probe`."));
    assert!(rust.contains("fn coverage() {}"));
}

#[test]
fn state_machines_cover_their_table() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \titem def Job { attribute weight : Natural; }\n\
         \tstate def Flow {\n\
         \t\tstate rest;\n\
         \t\tstate busy;\n\
         \t\ttransition start first rest accept job : Job if job.weight > 0 do send job to rest then busy;\n\
         \t\ttransition stop first busy if true then rest;\n\
         \t\ttransition label first rest accept tag : String then busy;\n\
         \t\ttransition remote first busy accept p : Payload then rest;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // a trigger typed by a generated item carries it in the event
    assert!(rust.contains("Start(Job),"));
    // a scalar-typed trigger has no event payload shape: bare event
    assert!(rust.contains("    Label,"));
    // a bound item rides along by its Rust path
    assert!(rust.contains("Remote(fake::Payload),"));
    // guard without payload, guard with payload, effect hook and call
    assert!(rust.contains("fn guard_stop(&self) -> bool"));
    assert!(rust.contains("if hooks.guard_start(job)"));
    assert!(rust.contains("/// SysML effect of `start`."));
    assert!(rust.contains("hooks.effect_start(job);"));
    assert!(rust.contains("hooks.on_exit_rest();"));
}

#[test]
fn a_perform_whose_action_never_resolved_is_a_comment() {
    // no unresolved==0 assertion here: the point is what generation does
    // when resolution failed upstream
    let mut ws = sysml_semantics::Workspace::new();
    let file = ws.add_file(
        "system.sysml",
        "package S {\n\tpart def Node {\n\t\tperform action ghost : Nowhere;\n\t}\n}\n",
    );
    let stats = ws.resolve_all();
    assert!(stats.unresolved > 0);
    let roots = ws.file_roots(file).to_vec();
    let rust = sysml_rust::generate(ws.model(), &roots).unwrap();
    assert!(rust.contains("perform `ghost` -- its action did not resolve"));
}

#[test]
fn foreign_models_with_odd_bindings_do_not_confuse_the_reader() {
    // hand-built shapes an importer would never write: relationships
    // without targets, values that are not literals, unnamed attributes
    use sysml_model::{ElementKind, Model, Value};
    let mut model = Model::new();
    let part = model.create(ElementKind::PartDefinition);
    model.set(part, "declaredName", Value::String("Odd".to_string()));
    let perform = model.create(ElementKind::PerformActionUsage);
    model.set(perform, "declaredName", Value::String("go".to_string()));
    model.add_owned(part, perform);
    // a typing that never says what it types
    let typing = model.create(ElementKind::FeatureTyping);
    model.add_owned(perform, typing);

    // a metadata usage whose settings are broken in every way
    let meta = model.create(ElementKind::MetadataUsage);
    model.add_owned(part, meta);
    let hollow = model.create(ElementKind::ReferenceUsage);
    model.add_owned(meta, hollow);
    let untargeted = model.create(ElementKind::Redefinition);
    model.add_owned(hollow, untargeted);
    let unvalued = model.create(ElementKind::FeatureValue);
    model.add_owned(hollow, unvalued);

    let numbered = model.create(ElementKind::ReferenceUsage);
    model.add_owned(meta, numbered);
    let redefinition = model.create(ElementKind::Redefinition);
    model.add_owned(numbered, redefinition);
    let attribute = model.create(ElementKind::AttributeUsage);
    model.set(
        attribute,
        "declaredName",
        Value::String("retries".to_string()),
    );
    model.add_owned(part, attribute);
    model.set(redefinition, "redefinedFeature", Value::Ref(attribute));
    let value = model.create(ElementKind::FeatureValue);
    model.add_owned(numbered, value);
    let literal = model.create(ElementKind::LiteralInteger);
    model.set(literal, "value", Value::Int(3));
    model.add_owned(value, literal);
    model.set(value, "value", Value::Ref(literal));

    let rust = sysml_rust::generate(&model, &[part]).unwrap();
    // the part has a binding now (retries = 3), so it is treated as an
    // imported API and not generated at all
    assert!(!rust.contains("struct Odd"));

    // without the metadata, the dangling typing is a comment
    let mut plain = Model::new();
    let part = plain.create(ElementKind::PartDefinition);
    plain.set(part, "declaredName", Value::String("Bare".to_string()));
    let perform = plain.create(ElementKind::PerformActionUsage);
    plain.set(perform, "declaredName", Value::String("go".to_string()));
    plain.add_owned(part, perform);
    let typing = plain.create(ElementKind::FeatureTyping);
    plain.add_owned(perform, typing);
    // an unnamed member contributes nothing, quietly -- even when it
    // carries a redefinition that never says what it redefines
    let anonymous = plain.create(ElementKind::AttributeUsage);
    plain.add_owned(part, anonymous);
    let hollow_redefinition = plain.create(ElementKind::Redefinition);
    plain.add_owned(anonymous, hollow_redefinition);
    let rust = sysml_rust::generate(&plain, &[part]).unwrap();
    assert!(rust.contains("perform `go` -- its action did not resolve"));
    assert!(rust.contains("pub struct Bare"));

    // a hand-built machine with a broken transition: one end, and that
    // end a feature that chains to nothing -- the transition is skipped
    let mut machine = Model::new();
    let flow = machine.create(ElementKind::StateDefinition);
    machine.set(flow, "declaredName", Value::String("Flow".to_string()));
    let rest = machine.create(ElementKind::StateUsage);
    machine.set(rest, "declaredName", Value::String("rest".to_string()));
    machine.add_owned(flow, rest);
    let broken = machine.create(ElementKind::TransitionUsage);
    machine.set(broken, "declaredName", Value::String("broken".to_string()));
    machine.add_owned(flow, broken);
    let unchained = machine.create(ElementKind::Feature);
    machine.add_owned(broken, unchained);
    let rust = sysml_rust::generate(&machine, &[flow]).unwrap();
    assert!(rust.contains("pub enum FlowState {"));
    assert!(!rust.contains("Broken"), "the transition must be skipped");
}

#[test]
fn half_broken_binding_settings_are_passed_over() {
    use sysml_model::{ElementKind, Model, Value};
    let mut model = Model::new();
    let part = model.create(ElementKind::PartDefinition);
    model.set(part, "declaredName", Value::String("Shaky".to_string()));
    let meta = model.create(ElementKind::MetadataUsage);
    model.add_owned(part, meta);

    // a setting redefining an UNNAMED attribute: value present, no key
    let unnamed_target = model.create(ElementKind::AttributeUsage);
    model.add_owned(part, unnamed_target);
    let nameless = model.create(ElementKind::ReferenceUsage);
    model.add_owned(meta, nameless);
    let redefinition = model.create(ElementKind::Redefinition);
    model.add_owned(nameless, redefinition);
    model.set(redefinition, "redefinedFeature", Value::Ref(unnamed_target));
    let value = model.create(ElementKind::FeatureValue);
    model.add_owned(nameless, value);
    let literal = model.create(ElementKind::LiteralInteger);
    model.set(literal, "value", Value::Int(1));
    model.add_owned(value, literal);
    model.set(value, "value", Value::Ref(literal));

    // a setting whose literal never got a value
    let named_target = model.create(ElementKind::AttributeUsage);
    model.set(
        named_target,
        "declaredName",
        Value::String("path".to_string()),
    );
    model.add_owned(part, named_target);
    let hollow = model.create(ElementKind::ReferenceUsage);
    model.add_owned(meta, hollow);
    let redefinition = model.create(ElementKind::Redefinition);
    model.add_owned(hollow, redefinition);
    model.set(redefinition, "redefinedFeature", Value::Ref(named_target));
    let value = model.create(ElementKind::FeatureValue);
    model.add_owned(hollow, value);
    let empty = model.create(ElementKind::LiteralString);
    model.add_owned(value, empty);
    model.set(value, "value", Value::Ref(empty));

    // no usable pair survived, so the part carries no binding and is a
    // plain generated struct
    let rust = sysml_rust::generate(&model, &[part]).unwrap();
    assert!(rust.contains("pub struct Shaky"));
}

#[test]
fn an_action_no_port_provides_is_an_error() {
    let error = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tpart def Node {\n\
         \t\tport store : Store;\n\
         \t\tperform action stray : Orphan;\n\
         \t}\n}\n",
    )
    .unwrap_err();
    let text = error.to_string();
    assert!(text.contains("`Node` performs `stray`"), "{text}");
    assert!(text.contains("elsewhere::Api::orphan"), "{text}");
    assert!(!format!("{error:?}").is_empty());
}

#[test]
fn ports_and_parts_that_collide_still_name_their_parameters() {
    // a port whose camel-cased name equals the part's falls back to an
    // indexed parameter, and two ports coexist
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tpart def Depot {\n\
         \t\tport depot : Store;\n\
         \t\tport metrics : Metrics;\n\
         \t\tperform action ping : Ping;\n\
         \t}\n}\n",
    )
    .unwrap();
    assert!(rust.contains("pub struct Depot<P0: fake::Store, Metrics: fake::Metrics>"));
    assert!(rust.contains("pub depot: P0,"));
    assert!(rust.contains("self.depot.ping()"));
}

#[test]
fn calculations_translate_where_the_simple_subset_allows() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tcalc def Area {\n\
         \t\tin w : Real;\n\
         \t\tin h : Real;\n\
         \t\tw * h\n\
         \t}\n\
         \tcalc def Braking { in v : Real; return d : Real = v * v / 2.0; }\n\
         \tcalc def Weird { in x : Real; return r : Real = x[1]; }\n\
         \tcalc def Blank { in a : Real; return r : Real; }\n\
         \tcalc def Squared { in x : Real; return r : Real = x ** 2; }\n\
         \tcalc def Anon { in : Real; return r : Real = 1.0; }\n\
         \titem def Job { attribute weight : Integer; }\n\
         \tpart def Sensor {\n\
         \t\tattribute gain : Real = 2.0;\n\
         \t\tattribute offset : Real;\n\
         \t\tcalc reading : Real { in raw : Real; raw * gain + offset }\n\
         \t\tcalc silent : Real;\n\
         \t\tcalc def Inner { in a : Real; a + 1.0 }\n\
         \t}\n\
         \tstate def Chime {\n\
         \t\tstate quiet;\n\
         \t\tstate loud;\n\
         \t\ttransition ring first quiet accept p : Payload if p then loud;\n\
         \t\ttransition spark first quiet accept q : Job if q.weight[1] > 1 then loud;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // a trailing result expression, its bool-or-unified type inferred
    assert!(rust.contains("pub fn area(w: f64, h: f64) -> f64 {\n    w * h\n}"));
    // a declared return parameter with its own value clause
    assert!(rust.contains("pub fn braking(v: f64) -> f64 {\n    (v * v) / 2.0\n}"));
    // beyond the subset: the formula stays in the model's words
    assert!(rust.contains("/// The formula is beyond the simple subset"));
    assert!(rust.contains(
        "#[allow(unused_variables)]\npub fn weird(x: f64) -> f64 {\n    todo!(\"{}\", \"x[1]\")\n}"
    ));
    // typed but formula-less: an honest empty todo
    assert!(rust.contains("pub fn blank(a: f64) -> f64 {\n    todo!()\n}"));
    // `**` is Rust's `powf`, and a whole-number exponent is spelled as
    // the real number it stands for
    assert!(rust.contains("pub fn squared(x: f64) -> f64 {\n    x.powf(2.0)\n}"));
    // a literal return value, an anonymous `in` skipped from the arguments
    assert!(rust.contains("pub fn anon() -> f64 {\n    1.0\n}"));
    // a calc usage reads the struct through `self`, its own params plainly
    assert!(rust.contains(
        "    pub fn reading(&self, raw: f64) -> f64 {\n        (raw * self.gain) + self.offset\n    }"
    ));
    assert!(rust.contains("    pub fn silent(&self) -> f64 {\n        todo!()\n    }"));
    // a nested calc def generates at the top level, not as a note
    assert!(rust.contains("pub fn inner(a: f64) -> f64 {\n    a + 1.0\n}"));
    assert!(!rust.contains("CalculationDefinition not generated"));
    // guards that do not translate to a bool keep their open default
    assert!(rust.contains("fn guard_ring(&self, p: &fake::Payload) -> bool {\n        true\n    }"));
    assert!(rust.contains("fn guard_spark(&self, q: &Job) -> bool {\n        true\n    }"));
}

#[test]
fn calculations_note_what_they_cannot_type() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tcalc def Mixed { in a : Real; in b : Integer; a * b }\n\
         \tcalc def Loose { in a : Real; }\n\
         \tcalc def Untyped { in a : Unbound; a }\n\
         \tcalc def Opaque { return r : Unbound; }\n\
         \tpart def Meter {\n\
         \t\tattribute reach : Real;\n\
         \t\tcalc opaque : Unbound;\n\
         \t\tcalc unfit : Real { in k : Unbound; k }\n\
         \t\tcalc fixed : Real = 9.8;\n\
         \t\tcalc odd : Real = reach[1];\n\
         \t\tcalc alone : Real { in : Integer; 4.0 }\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // mixed-type arithmetic and a missing formula both leave the type open
    assert!(rust.contains(
        "// not generated: calc def `Mixed` -- its result type is neither declared nor inferable"
    ));
    assert!(rust.contains(
        "// not generated: calc def `Loose` -- its result type is neither declared nor inferable"
    ));
    assert!(rust.contains("// not generated: calc def `Untyped` -- parameter `a` has no Rust type"));
    assert!(rust.contains("// not generated: calc def `Opaque` -- its return has no Rust type"));
    assert!(rust.contains("// not generated: calc `opaque` -- its result has no Rust type"));
    assert!(rust.contains("// not generated: calc `unfit` -- parameter `k` has no Rust type"));
    // literal clauses and untranslated formulas on calc usages
    assert!(rust.contains("    pub fn fixed(&self) -> f64 {\n        9.8\n    }"));
    assert!(rust.contains("    /// The formula is beyond the simple subset"));
    assert!(rust.contains("        todo!(\"{}\", \"reach[1]\")"));
    assert!(rust.contains("    pub fn alone(&self) -> f64 {\n        4.0\n    }"));
}

#[test]
fn what_the_model_is_made_of_is_said_even_where_it_cannot_be_written() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tport def Fuel {\n\t\tout item rate : Real;\n\t}\n\
         \tport def FuelIn {\n\t\tin item rate : Real;\n\t}\n\
         \tinterface def FuelLine {\n\
         \t\tend supplier : Fuel;\n\
         \t\tend consumer : FuelIn;\n\
         \t}\n\
         \taction def Wash {\n\t\tout washed : Real;\n\t}\n\
         \taction def Fit {\n\t\tin start : Real;\n\t\tout weights : Real;\n\t}\n\
         \taction def Session {\n\
         \t\taction wash : Wash;\n\
         \t\taction fit : Fit;\n\
         \t\taction;\n\
         \t\tfirst wash then fit;\n\
         \t\tflow from wash.washed to fit.start;\n\
         \t}\n\
         \tview def Overview;\n\
         \tviewpoint def Stakeholder;\n\
         \tpart def Stage {\n\
         \t\tattribute 'cross section' : Real;\n\
         \t\tattribute '2nd reading' : Real;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // an interface is its ends, which is a struct like any other
    assert!(rust.contains("pub struct FuelLine {"));
    assert!(rust.contains("pub supplier: Fuel,"));
    assert!(rust.contains("pub consumer: FuelIn,"));
    // a behaviour still cannot be written, but what the model says it is
    // made of goes to whoever has to write it
    assert!(rust.contains("/// Made of `wash : Wash`, `fit : Fit`."));
    assert!(rust.contains("/// then: `wash` -> `fit`"));
    assert!(rust.contains("/// flow: `washed` -> `start`"));
    // and a definition with no Rust shape at all is named, not passed
    // over in silence
    assert!(rust.contains("// not generated: `Overview` -- ViewDefinition has no Rust shape"));
    assert!(
        rust.contains("// not generated: `Stakeholder` -- ViewpointDefinition has no Rust shape")
    );
    // an escaped name holds whatever the modeller wrote; Rust spells
    // identifiers out of a smaller alphabet, and cannot start one with a
    // digit
    assert!(rust.contains("pub cross_section: f64,"));
    assert!(rust.contains("pub _2nd_reading: f64,"));
}

#[test]
fn a_dataflow_the_model_wired_completely_becomes_the_body() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Wash {\n\t\tin stream : Real;\n\t\tin steps : Integer;\n\t\tout washed : Real;\n\t}\n\
         \taction def Reap {\n\t\tin stream : Real;\n\t\tin start : Real;\n\
         \t\tout rows : Real;\n\t\tout teacher : Real;\n\t}\n\
         \taction def Fit {\n\t\tin rows : Real;\n\t\tin teacher : Real;\n\t\tout weights : Real;\n\t}\n\
         \taction def Session {\n\
         \t\tin stream : Real;\n\t\tin washout : Integer;\n\t\tout weights : Real;\n\
         \t\taction wash : Wash {\n\t\t\tin stream = Session::stream;\n\t\t\tin steps = Session::washout;\n\t\t}\n\
         \t\taction reap : Reap {\n\t\t\tin stream = Session::stream;\n\t\t}\n\
         \t\taction fit : Fit;\n\
         \t\tfirst wash then reap;\n\
         \t\tflow from wash.washed to reap.start;\n\
         \t\tfirst reap then fit;\n\
         \t\tflow from reap.rows to fit.rows;\n\
         \t\tflow from reap.teacher to fit.teacher;\n\
         \t\tflow from fit.weights to weights;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // the parts it is made of are what it demands of its implementor,
    // and the wiring is the body -- the order from the successions, the
    // arguments from the flows and the bindings, the result from the
    // flow into its own `out`
    assert!(
        rust.contains(concat!(
            "pub trait Session: Wash + Reap + Fit {\n",
            "    fn session(&mut self, stream: f64, washout: i64) -> f64 {\n",
            "        let wash = self.wash(stream, washout);\n",
            "        let reap = self.reap(stream, wash);\n",
            "        let fit = self.fit(reap.0, reap.1);\n",
            "        fit\n",
            "    }\n",
            "}\n",
        )),
        "{rust}"
    );
}

#[test]
fn a_dataflow_with_a_gap_in_it_is_not_written_half_way() {
    // the same shape, with nothing feeding `fit.teacher` and nothing
    // producing the result: an implementation that guessed at either
    // would be worse than one that asks
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Reap {\n\t\tin stream : Real;\n\t\tout rows : Real;\n\t}\n\
         \taction def Fit {\n\t\tin rows : Real;\n\t\tin teacher : Real;\n\t\tout weights : Real;\n\t}\n\
         \taction def Session {\n\
         \t\tin stream : Real;\n\t\tout weights : Real;\n\
         \t\taction reap : Reap {\n\t\t\tin stream = Session::stream;\n\t\t}\n\
         \t\taction fit : Fit;\n\
         \t\tfirst reap then fit;\n\
         \t\tflow from reap.rows to fit.rows;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    assert!(rust.contains("pub trait Session {\n    fn session(&mut self, stream: f64) -> f64;\n}"));
    // and what the model did say is still handed to whoever writes it,
    // along with which gap stopped the rest -- a modeller one flow short
    // should not have to guess which one
    assert!(rust.contains("/// Made of `reap : Reap`, `fit : Fit`."));
    assert!(rust.contains("/// then: `reap` -> `fit`"));
    assert!(rust.contains("/// Not performed here: nothing feeds `fit.teacher`."));
    // a leaf behaviour is not a dataflow with a gap in it, and is not
    // told off for having no parts
    assert!(
        rust.contains(
            "/// SysML: `action def Reap`\n\
             /// The model names this behaviour without saying how it runs; \
             the implementation is yours.\n\
             pub trait Reap {"
        ),
        "{rust}"
    );
}

#[test]
fn a_dataflow_is_refused_by_name_for_each_way_it_can_be_short() {
    // an array of a struct, a result made of several outs, a flow whose
    // end names the `in` of the same name, a literal bound to an input,
    // and one parameter fed to two subactions: the shapes an emulator
    // written this way actually has
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Word {\n\t\tattribute bits : Natural;\n\t}\n\
         \tpart def Regs {\n\t\tattribute x : Word[8] ordered;\n\t}\n\
         \taction def Fetch {\n\t\tin pc : Word;\n\t\tin bias : Natural;\n\t\tout word : Word;\n\t}\n\
         \taction def Run {\n\
         \t\tin regs : Regs;\n\t\tin pc : Word;\n\
         \t\tout regs : Regs;\n\t\tout pc : Word;\n\t}\n\
         \taction def Cycle {\n\
         \t\tin regs : Regs;\n\t\tin pc : Word;\n\
         \t\tout nextRegs : Regs;\n\t\tout nextPc : Word;\n\
         \t\taction fetch : Fetch {\n\t\t\tin pc = Cycle::pc;\n\t\t\tin bias = 4;\n\t\t}\n\
         \t\taction run : Run {\n\t\t\tin regs = Cycle::regs;\n\t\t\tin pc = Cycle::pc;\n\t\t}\n\
         \t\tfirst fetch then run;\n\
         \t\tflow from run.regs to nextRegs;\n\
         \t\tflow from run.pc to nextPc;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // an array of a struct starts without asking the struct to be `Copy`
    assert!(rust.contains("x: std::array::from_fn(|_| Default::default()),"));
    // `pc` goes to two subactions, so the first takes a copy; a literal
    // bound to an input is passed as written; and `run.regs` names the
    // `in` of that name, which the `out` is what was meant by
    assert!(
        rust.contains(concat!(
            "        let fetch = self.fetch(pc.clone(), 4);\n",
            "        let run = self.run(regs, pc);\n",
            "        (run.0, run.1)\n",
        )),
        "{rust}"
    );
}

#[test]
fn a_behaviour_performing_what_was_never_generated_says_so() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Outer {\n\
         \t\tin a : Real;\n\t\tout b : Real;\n\
         \t\taction bound : Ping;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    assert!(
        rust.contains("/// Not performed here: `bound` performs no generated action."),
        "{rust}"
    );
}

#[test]
fn a_flow_that_does_not_join_two_subactions_is_named_as_the_gap() {
    // a flow out of the definition's own parameter rather than out of a
    // subaction: there is no earlier call to read it from
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Pair {\n\t\tattribute rows : Real;\n\t}\n\
         \taction def Reap {\n\t\tin stream : Pair;\n\t\tout rows : Real;\n\t}\n\
         \taction def Fit {\n\t\tin rows : Real;\n\t\tout weights : Real;\n\t}\n\
         \taction def Session {\n\
         \t\tin stream : Pair;\n\t\tout weights : Real;\n\
         \t\taction reap : Reap {\n\t\t\tin stream = Session::stream;\n\t\t}\n\
         \t\taction fit : Fit;\n\
         \t\tfirst reap then fit;\n\
         \t\tflow from stream to fit.rows;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    assert!(
        rust.contains("/// Not performed here: a flow that does not come out of a subaction."),
        "{rust}"
    );
}

#[test]
fn a_behaviour_that_produces_nothing_performs_its_parts_for_their_effect() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Log {\n\t\tin line : String;\n\t}\n\
         \taction def Twice {\n\
         \t\tin line : String;\n\
         \t\taction first_ : Log {\n\t\t\tin line = Twice::line;\n\t\t}\n\
         \t\taction second : Log {\n\t\t\tin line = Twice::line;\n\t\t}\n\
         \t\tfirst first_ then second;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // nothing to return, so nothing is returned -- and the line fed to
    // both is cloned for the first of them
    assert!(
        rust.contains(concat!(
            "    fn twice(&mut self, line: String) {\n",
            "        let first_ = self.log(line.clone());\n",
            "        let second = self.log(line);\n",
            "    }\n",
        )),
        "{rust}"
    );
}

#[test]
fn one_output_read_by_two_subactions_is_read_twice() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tattribute def Rows {\n\t\tattribute n : Natural;\n\t}\n\
         \taction def Reap {\n\t\tin seed : Natural;\n\t\tout rows : Rows;\n\t}\n\
         \taction def Score {\n\t\tin rows : Rows;\n\t\tout score : Natural;\n\t}\n\
         \taction def Keep {\n\t\tin rows : Rows;\n\t\tout kept : Natural;\n\t}\n\
         \taction def Both {\n\
         \t\tin seed : Natural;\n\t\tout total : Natural;\n\t\tout kept : Natural;\n\
         \t\taction reap : Reap {\n\t\t\tin seed = Both::seed;\n\t\t}\n\
         \t\taction scoring : Score;\n\
         \t\taction keeping : Keep;\n\
         \t\tfirst reap then scoring;\n\
         \t\tfirst scoring then keeping;\n\
         \t\tflow from reap.rows to scoring.rows;\n\
         \t\tflow from reap.rows to keeping.rows;\n\
         \t\tflow from scoring.score to total;\n\
         \t\tflow from keeping.kept to kept;\n\
         \t}\n\
         \tpart def Held {\n\t\tport store : Store;\n\t\tattribute seats : Natural[4] ordered;\n\t}\n\
         }\n",
    )
    .unwrap();
    // one producer, two consumers: the first of them takes a copy. Only
    // one flow arrives at each, so the second may take it
    assert!(
        rust.contains("let scoring = self.score(reap.clone());"),
        "{rust}"
    );
    assert!(rust.contains("let keeping = self.keep(reap);"), "{rust}");
    // an array of something bound to an API type cannot be started, so
    // the struct holding it gets no `Default`
    assert!(!rust.contains("impl Default for Held"), "{rust}");
}

#[test]
fn a_formula_over_two_numeric_types_says_so_where_the_error_will_be() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \tcalc def Mean {\n\t\tin sum : Real;\n\t\tin count : Natural;\n\
         \t\treturn : Real = sum / count;\n\t}\n\
         \tcalc def Half {\n\t\tin sum : Real;\n\t\tin count : Real;\n\
         \t\treturn : Real = sum / count;\n\t}\n\
         }\n",
    )
    .unwrap();
    // the literals were kept as written rather than coerced, so this one
    // will not compile -- and the reason belongs where the reader of the
    // error will be looking
    assert!(
        rust.contains("/// Mixed numeric parameters (`sum` is f64, `count` is u64)."),
        "{rust}"
    );
    // and one that agrees with itself is not told off
    assert!(
        rust.contains("pub fn half(sum: f64, count: f64) -> f64 {"),
        "{rust}"
    );
    assert_eq!(
        rust.matches("Mixed numeric parameters").count(),
        1,
        "{rust}"
    );
}

/// SysML names an escaped name anything at all -- `'Ideal Gas Parcel'`
/// is one name -- and Rust spells type names out of a much smaller
/// alphabet. Two names that differ only in what Rust cannot spell arrive
/// at the same type, and only one of them can have it.
#[test]
fn names_rust_cannot_spell_are_joined_up_and_never_collide() {
    let rust = generate(
        "package S {\n\
         \tpart def 'Ideal Gas Parcel';\n\
         \tpart def '2nd Stage';\n\
         \tpart def '+/-';\n\
         \tpart def IdealGasParcel;\n\
         \tpackage Inner {\n\t\tpart def 'Ideal-Gas-Parcel';\n\t}\n\
         }\n",
    )
    .unwrap();
    // the words join up rather than the name ending at the first space,
    // a leading digit gets out of Rust's way, and a name with nothing
    // Rust can use still has to be called something
    assert!(rust.contains("pub struct IdealGasParcel"), "{rust}");
    assert!(rust.contains("pub struct _2ndStage"), "{rust}");
    assert!(rust.contains("pub struct _ "), "{rust}");
    // three names arrive at `IdealGasParcel`; the first keeps it and the
    // others are named by where they came from, not written twice over
    assert_eq!(
        rust.matches("pub struct IdealGasParcel").count(),
        1,
        "{rust}"
    );
    assert!(
        rust.contains(
            "// not generated: `IdealGasParcel` -- `IdealGasParcel` is already the Rust name of `S::Ideal Gas Parcel`"
        ),
        "{rust}"
    );
    assert!(
        rust.contains("// not generated: `Ideal-Gas-Parcel` -- `IdealGasParcel` is already the Rust name of `S::Ideal Gas Parcel`"),
        "{rust}"
    );
}

/// A behaviour that performs itself would have to be its own supertrait,
/// which Rust reads as a circle and refuses.
#[test]
fn a_behaviour_made_of_itself_is_refused_by_name() {
    let rust = generate(
        "package S {\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Prep {\n\t\tin whole : Real;\n\t\tout ready : Real;\n\t}\n\
         \taction def Grind {\n\
         \t\tin bean : Real;\n\t\tout dust : Real;\n\
         \t\taction prep : Prep {\n\t\t\tin whole = Grind::bean;\n\t\t}\n\
         \t\taction again : Grind;\n\
         \t\tfirst prep then again;\n\
         \t\tflow from prep.ready to again.bean;\n\
         \t\tflow from again.dust to dust;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    assert!(
        rust.contains("/// Not performed here: it is made of itself."),
        "{rust}"
    );
    assert!(!rust.contains("pub trait Grind: Grind"), "{rust}");
}

/// Two shapes a model reaches that a signature cannot: a succession with
/// one end, and a repeated field of a type that lives in someone else's
/// crate.
#[test]
fn a_one_ended_succession_and_an_array_of_a_bound_type() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \taction def Prep {\n\t\tin whole : Real;\n\t\tout ready : Real;\n\t}\n\
         \taction def Once {\n\
         \t\tin bean : Real;\n\t\tout dust : Real;\n\
         \t\taction prep : Prep {\n\t\t\tin whole = Once::bean;\n\t\t}\n\
         \t\tfirst prep;\n\
         \t\tflow from prep.ready to dust;\n\
         \t}\n\
         \tpart def Crate {\n\t\titem batch : Payload[3];\n\t}\n\
         }\n",
    )
    .unwrap();
    // `first prep;` names one end, and an order cannot be read off it
    assert!(
        rust.contains("/// Not performed here: a succession whose ends did not resolve."),
        "{rust}"
    );
    // three of someone else's type is an array, and nothing here knows
    // how to start one
    assert!(rust.contains("pub batch: [fake::Payload; 3],"), "{rust}");
    assert!(!rust.contains("impl Default for Crate"), "{rust}");
}

/// The shapes an API binding can ask for that a model written by hand
/// rarely does: a receiver that mutates, a call to await, a `Result` of
/// both halves, and a parameter typed by the API's own item. Their
/// documentation comes along, and a calculation whose result is a
/// comparison is a `bool`.
#[test]
fn a_binding_says_how_the_call_is_made_and_the_signature_follows() {
    let rust = generate(
        "package S {\n\
         \tprivate import Api::*;\n\
         \tprivate import ScalarValues::*;\n\
         \tdoc /* what the package is for */\n\
         \tpart def Cellar {\n\
         \t\tdoc /* where the store is kept */\n\
         \t\tport main_store : Store;\n\
         \t\tperform action watch : Watch;\n\
         \t\tperform action count : Count;\n\
         \t\tcalc def Cold {\n\
         \t\t\tdoc /* whether it is cold enough */\n\
         \t\t\tin t : Real;\n\t\t\tt < 4.0\n\t\t}\n\
         \t}\n\
         \tstate def Cooling {\n\
         \t\tdoc /* how the cellar behaves */\n\
         \t\tstate warm;\n\t\tstate cold;\n\
         \t\ttransition chill first warm then cold;\n\
         \t}\n\
         }\n",
    )
    .unwrap();
    // the receiver, the await and both halves of the Result are the
    // binding's, not a guess
    assert!(
        rust.contains(
            "pub async fn watch(&mut self, sku: String) \
             -> Result<fake::Payload, String> {"
        ),
        "{rust}"
    );
    assert!(rust.contains(".watch(sku).await"), "{rust}");
    // one that cannot fail returns its result plainly
    assert!(rust.contains("pub fn count(&self) -> f64 {"), "{rust}");
    // a comparison is a `bool`
    assert!(rust.contains("pub fn cold(t: f64) -> bool {"), "{rust}");
    // and what the model documents, the Rust documents
    assert!(rust.contains("/// where the store is kept"), "{rust}");
    assert!(rust.contains("/// whether it is cold enough"), "{rust}");
    assert!(rust.contains("/// how the cellar behaves"), "{rust}");
}
