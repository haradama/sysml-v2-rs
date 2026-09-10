//! How each construct is written, with an example that has been checked.
//!
//! SysML v2 was adopted in 2025, and there is very little of it in what
//! any language model was trained on. What an agent asked to transcribe
//! a specification gets wrong is not the systems engineering -- it is
//! the notation: whether a requirement's condition goes in `require
//! constraint { }` or after `assert`, whether a transition is written
//! `first ... then ...` or `transition ... to ...`, what a port
//! declares.
//!
//! It cannot be told from the standard library, which is bundled and
//! which an agent can read: the library declares the *types* a model
//! reaches for and almost none of the constructs a model is written
//! with. There is no `requirement def` in it, no `state def`, no
//! `connect`.
//!
//! So the examples are here. Each one is real SysML that parses,
//! resolves against the standard library and holds the constraints the
//! specification states -- `every_example_is_checked` is what makes that
//! true rather than intended.

/// One construct, and how it is written.
pub struct Notation {
    /// What to ask for.
    pub of: &'static str,
    /// When a modeller reaches for this, in a sentence. What an agent
    /// needs is not the grammar but which construct answers the sentence
    /// in front of it.
    pub when: &'static str,
    /// A model that uses it, whole enough to check.
    pub sysml: &'static str,
    /// What is usually written with it.
    pub see_also: &'static [&'static str],
}

/// Every construct this answers for.
pub const NOTATION: &[Notation] = &[
    Notation {
        of: "part",
        when: "Something the system is made of, and the kinds of thing there are. \
               `part def` declares a kind; `part` declares one that a whole is assembled from.",
        sysml: "package Vehicles {\n\
                \tpart def Engine;\n\
                \tpart def Wheel;\n\n\
                \tpart def Vehicle {\n\
                \t\tpart engine : Engine;\n\
                \t\tpart wheels : Wheel[4];\n\
                \t}\n\
                }\n",
        see_also: &["attribute", "port", "specialization"],
    },
    Notation {
        of: "attribute",
        when: "A value something has: a number, a quantity with units, a string. \
               The type comes from the standard library -- search it rather than inventing one.",
        sysml: "package Fuel {\n\
                \tprivate import ScalarValues::*;\n\
                \tprivate import ISQ::*;\n\
                \tprivate import SI::*;\n\n\
                \tpart def Tank {\n\
                \t\tattribute capacity : VolumeValue = 60 [litre];\n\
                \t\tattribute label : String = \"main\";\n\
                \t\tattribute baffles : Integer = 3;\n\
                \t}\n\
                }\n",
        see_also: &["part", "enumeration"],
    },
    Notation {
        of: "specialization",
        when: "One kind of thing is a kind of another. `:>` is `specializes`; \
               a usage narrows what it is typed by with `:>>` (`redefines`).",
        sysml: "package Vehicles {\n\
                \tpart def PowerSource;\n\
                \tpart def Engine :> PowerSource {\n\
                \t\tattribute power;\n\
                \t}\n\
                \tpart def TurboEngine :> Engine {\n\
                \t\tattribute :>> power;\n\
                \t}\n\
                }\n",
        see_also: &["part", "variation"],
    },
    Notation {
        of: "port",
        when: "Where a part is reached from outside it -- what it offers and what it needs. \
               `connect` wires two together. A port cannot be called `in` or `out`: both are reserved words.",
        sysml: "package Wiring {\n\
                \tport def PowerPort {\n\
                \t\tattribute volts;\n\
                \t}\n\
                \tpart def Battery {\n\
                \t\tport supply : PowerPort;\n\
                \t}\n\
                \tpart def Lamp {\n\
                \t\tport draw : PowerPort;\n\
                \t}\n\
                \tpart def Torch {\n\
                \t\tpart cell : Battery;\n\
                \t\tpart bulb : Lamp;\n\
                \t\tconnect cell.supply to bulb.draw;\n\
                \t}\n\
                }\n",
        see_also: &["part", "interface", "flow"],
    },
    Notation {
        of: "connection",
        when: "Two parts joined: a pipe, a cable, a shaft. `connect a to b` where only the \
               joining matters, and `connection <name> : <def> connect a to b` where the \
               joining is a thing in its own right, with ends of its own.",
        sysml: "package Plumbing {\n\
                \tpart def Pump;\n\
                \tpart def Tank;\n\
                \tconnection def Pipe {\n\
                \t\tend source : Pump;\n\
                \t\tend target : Tank;\n\
                \t}\n\
                \tpart def Waterworks {\n\
                \t\tpart pump : Pump;\n\
                \t\tpart tank : Tank;\n\
                \t\tconnect pump to tank;\n\
                \t\tconnection feed : Pipe connect pump to tank;\n\
                \t}\n\
                }\n",
        see_also: &["part", "interface", "flow"],
    },
    Notation {
        of: "interface",
        when: "A connection with a kind of its own, so that what runs between two ports \
               is declared once and used wherever that pairing occurs.",
        sysml: "package Wiring {\n\
                \tport def PowerPort;\n\
                \tinterface def PowerLink {\n\
                \t\tend supply : PowerPort;\n\
                \t\tend draw : PowerPort;\n\
                \t}\n\
                \tpart def Battery { port supply : PowerPort; }\n\
                \tpart def Lamp { port draw : PowerPort; }\n\
                \tpart def Torch {\n\
                \t\tpart cell : Battery;\n\
                \t\tpart bulb : Lamp;\n\
                \t\tinterface : PowerLink connect cell.supply to bulb.draw;\n\
                \t}\n\
                }\n",
        see_also: &["port", "connection", "flow"],
    },
    Notation {
        of: "requirement",
        when: "Something that has to hold. `subject` says what it is about, \
               `require constraint { }` says it in a formula, and prose says it in words.",
        sysml: "package Safety {\n\
                \tprivate import ISQ::*;\n\
                \tprivate import SI::*;\n\n\
                \tpart def Boiler {\n\
                \t\tattribute pressure : PressureValue;\n\
                \t}\n\n\
                \trequirement def <'R.1'> SafePressure {\n\
                \t\tdoc /* The boiler shall stay below its rated pressure. */\n\
                \t\tsubject boiler : Boiler;\n\
                \t\tattribute rated : PressureValue = 500000 [pascal];\n\
                \t\trequire constraint {\n\
                \t\t\tboiler.pressure <= rated\n\
                \t\t}\n\
                \t}\n\
                }\n",
        see_also: &["satisfy", "constraint", "verification"],
    },
    Notation {
        of: "satisfy",
        when: "Which part answers for a requirement. Name the satisfaction and type it \
               with the requirement, or nothing downstream can tell which requirement was met.",
        sysml: "package Safety {\n\
                \tpart def Boiler;\n\
                \trequirement def SafePressure {\n\
                \t\tsubject boiler : Boiler;\n\
                \t}\n\
                \tpart def Plant {\n\
                \t\tpart boiler : Boiler;\n\
                \t\tsatisfy requirement <'V.1'> pressure : SafePressure by boiler;\n\
                \t}\n\
                }\n",
        see_also: &["requirement", "verification"],
    },
    Notation {
        of: "constraint",
        when: "A formula on its own, to be asserted or required in more than one place.",
        sysml: "package Checks {\n\
                \tprivate import ScalarValues::*;\n\n\
                \tconstraint def Positive {\n\
                \t\tin value : Real;\n\
                \t\tvalue > 0.0\n\
                \t}\n\
                \tpart def Tank {\n\
                \t\tattribute litres : Real;\n\
                \t\tassert constraint : Positive { in value = litres; }\n\
                \t}\n\
                }\n",
        see_also: &["requirement", "calculation"],
    },
    Notation {
        of: "state",
        when: "What something does over time: the states it is in and what moves it between \
               them. A transition is `first <from> then <to>`, with `accept` for the event \
               and `if` for the guard.",
        sysml: "package Blinking {\n\
                \tattribute def Tick;\n\
                \tpart def Lamp {\n\
                \t\tstate def LampStates {\n\
                \t\t\tentry; then off;\n\
                \t\t\tstate off;\n\
                \t\t\tstate on;\n\
                \t\t\ttransition lighting\n\
                \t\t\t\tfirst off\n\
                \t\t\t\taccept Tick\n\
                \t\t\t\tthen on;\n\
                \t\t\ttransition darkening\n\
                \t\t\t\tfirst on\n\
                \t\t\t\taccept Tick\n\
                \t\t\t\tthen off;\n\
                \t\t}\n\
                \t}\n\
                }\n",
        see_also: &["action", "occurrence"],
    },
    Notation {
        of: "occurrence",
        when: "Something that happens rather than something that is: a flight, a shift, a \
               failure. `occurrence def` names the kind of happening, and `first ... then \
               ...` orders two of them in time.",
        sysml: "package Missions {\n\
                \toccurrence def Departure;\n\
                \toccurrence def Arrival;\n\
                \toccurrence def Flight {\n\
                \t\tevent occurrence leaves : Departure;\n\
                \t\tevent occurrence lands : Arrival;\n\
                \t\tfirst leaves then lands;\n\
                \t}\n\
                }\n",
        see_also: &["state", "action"],
    },
    Notation {
        of: "action",
        when: "Something the system does, and the order it happens in. \
               `first ... then ...` sequences steps; `flow` carries a value between them.",
        sysml: "package Sensing {\n\
                \tprivate import ScalarValues::*;\n\n\
                \titem def Reading;\n\
                \taction def Sample { out sample : Reading; }\n\
                \taction def Record { in seen : Reading; }\n\n\
                \taction def Monitor {\n\
                \t\taction take : Sample;\n\
                \t\taction keep : Record;\n\
                \t\tfirst take then keep;\n\
                \t\tflow take.sample to keep.seen;\n\
                \t}\n\
                }\n",
        see_also: &["state", "calculation", "item"],
    },
    Notation {
        of: "calculation",
        when: "A value worked out from others. `calc def` names the formula; \
               the last expression in the body is what it returns.",
        sysml: "package Physics {\n\
                \tprivate import ScalarValues::*;\n\n\
                \tcalc def Kinetic {\n\
                \t\tin mass : Real;\n\
                \t\tin speed : Real;\n\
                \t\treturn : Real;\n\
                \t\t0.5 * mass * speed * speed\n\
                \t}\n\
                }\n",
        see_also: &["constraint", "attribute"],
    },
    Notation {
        of: "flow",
        when: "Something passing from one place to another -- a value out of one action \
               into the next, or an item between two ports. `flow` is untimed; \
               `succession flow` says the sending happens before the receiving.",
        sysml: "package Signals {\n\
                \titem def Sample;\n\
                \taction def Read { out taken : Sample; }\n\
                \taction def Store { in given : Sample; }\n\
                \taction def Logging {\n\
                \t\taction read : Read;\n\
                \t\taction store : Store;\n\
                \t\tflow read.taken to store.given;\n\
                \t}\n\
                }\n",
        see_also: &["action", "item", "port"],
    },
    Notation {
        of: "item",
        when: "Something that flows or is exchanged rather than something the system is \
               made of -- a message, a fluid, a part being carried.",
        sysml: "package Plumbing {\n\
                \titem def Water;\n\
                \tpart def Pump { out port outlet; }\n\
                \tpart def Tank { in port inlet; }\n\
                \tpart def Loop {\n\
                \t\tpart pump : Pump;\n\
                \t\tpart tank : Tank;\n\
                \t\tflow of Water from pump.outlet to tank.inlet;\n\
                \t}\n\
                }\n",
        see_also: &["action", "port"],
    },
    Notation {
        of: "enumeration",
        when: "A value that is one of a fixed set, named rather than numbered. \
               The members are reached through the enumeration: `Gear::park`.",
        sysml: "package Modes {\n\
                \tenum def Gear {\n\
                \t\tenum park;\n\
                \t\tenum drive;\n\
                \t\tenum reverse;\n\
                \t}\n\
                \tpart def Transmission {\n\
                \t\tattribute selected : Gear = Gear::park;\n\
                \t}\n\
                }\n",
        see_also: &["attribute", "variation"],
    },
    Notation {
        of: "variation",
        when: "A point where a product line varies: the alternatives are `variant`s, \
               and one of them is chosen.",
        sysml: "package Product {\n\
                \tpart def Engine;\n\
                \tpart def Petrol :> Engine;\n\
                \tpart def Diesel :> Engine;\n\n\
                \tvariation part def EngineChoice :> Engine {\n\
                \t\tvariant part petrol : Petrol;\n\
                \t\tvariant part diesel : Diesel;\n\
                \t}\n\
                }\n",
        see_also: &["specialization", "enumeration"],
    },
    Notation {
        of: "allocation",
        when: "One thing is realised by another -- a function by a component, \
               a logical part by a physical one.",
        sysml: "package Mapping {\n\
                \taction def Braking;\n\
                \tpart def BrakeUnit;\n\
                \tpart def Car {\n\
                \t\taction brake : Braking;\n\
                \t\tpart unit : BrakeUnit;\n\
                \t\tallocate brake to unit;\n\
                \t}\n\
                }\n",
        see_also: &["action", "part"],
    },
    Notation {
        of: "verification",
        when: "How a requirement is shown to be met: a case with the requirement as its \
               objective, which is what turns a requirement into something testable.",
        sysml: "package Testing {\n\
                \tpart def Boiler;\n\
                \trequirement def SafePressure { subject boiler : Boiler; }\n\n\
                \tverification def <'T.1'> PressureTest {\n\
                \t\tsubject boiler : Boiler;\n\
                \t\tobjective {\n\
                \t\t\tverify SafePressure;\n\
                \t\t}\n\
                \t}\n\
                }\n",
        see_also: &["requirement", "satisfy"],
    },
    Notation {
        of: "documentation",
        when: "Prose attached to what it is about. `doc` documents the element it is \
               written in; `comment about` documents one written elsewhere.",
        sysml: "package Notes {\n\
                \tdoc /* What this package is for. */\n\n\
                \tpart def Pump {\n\
                \t\tdoc\n\
                \t\t/*\n\
                \t\t * Several lines read as one paragraph, and the `*` margin\n\
                \t\t * is decoration rather than part of what is said.\n\
                \t\t */\n\
                \t}\n\
                \tcomment about Pump /* Said from outside the thing it is about. */\n\
                }\n",
        see_also: &["metadata"],
    },
    Notation {
        of: "metadata",
        when: "A label a model puts on its own elements, and a keyword for writing it. \
               `#Name` in front of a declaration applies it.",
        sysml: "package Tagging {\n\
                \tprivate import ScalarValues::*;\n\n\
                \tmetadata def Safety {\n\
                \t\tattribute level : String;\n\
                \t}\n\
                \t#Safety part def Boiler {\n\
                \t\t@Safety { level = \"critical\"; }\n\
                \t}\n\
                }\n",
        see_also: &["documentation"],
    },
    Notation {
        of: "package",
        when: "How a model is divided up, and how one part of it reaches another. \
               `private import` brings names in without passing them on.",
        sysml: "package Library {\n\
                \tpart def Wheel;\n\
                }\n\
                package Assembly {\n\
                \tprivate import Library::*;\n\
                \tpart def Cart {\n\
                \t\tpart wheels : Wheel[2];\n\
                \t}\n\
                }\n",
        see_also: &["part", "specialization"],
    },
];

/// What `of` names, or nothing where nothing does.
pub fn notation(of: &str) -> Option<&'static Notation> {
    NOTATION.iter().find(|it| it.of.eq_ignore_ascii_case(of))
}
