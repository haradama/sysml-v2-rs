//! How each construct is written, with an example that has been checked.
//!
//! SysML v2 is recent and there is very little of it in what any
//! language model was trained on. What an agent transcribing a
//! specification gets wrong is not the systems engineering -- it is the
//! notation: whether a requirement's condition goes in `require constraint
//! { }` or after `assert`, what a port declares.
//!
//! The standard library cannot tell it either: the library declares the
//! *types* a model reaches for and almost none of the constructs a model
//! is written with. There is no `requirement def` in it, no `state def`,
//! no `connect`.
//!
//! So the examples are here, and `every_example_is_checked` holds each to
//! parsing, resolving and obeying the specification's constraints.

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
    Notation {
        of: "use case",
        when: "What the system is for, from outside it: a goal an actor has and the subject \
               that meets it. `subject` is the system, `actor` who takes part, `objective` the \
               goal; `include use case` makes one use case a step of another.",
        sysml: "package Transport {\n\
                \tpart def Vehicle;\n\
                \tpart def Person;\n\
                \n\
                \tuse case def ProvideTransportation {\n\
                \t\tsubject vehicle : Vehicle;\n\
                \t\tactor driver : Person;\n\
                \t\tactor passengers : Person[0..4];\n\
                \t\tobjective {\n\
                \t\t\tdoc /* Carry the driver and passengers where they want to go. */\n\
                \t\t}\n\
                \t}\n\
                \tuse case def EnterVehicle {\n\
                \t\tsubject vehicle : Vehicle;\n\
                \t\tactor driver : Person;\n\
                \t}\n\
                \n\
                \tuse case transport : ProvideTransportation {\n\
                \t\tsubject vehicle;\n\
                \t\tfirst start;\n\
                \t\tthen include use case enter : EnterVehicle {\n\
                \t\t\tsubject vehicle;\n\
                \t\t\tactor driver = transport::driver;\n\
                \t\t}\n\
                \t\tthen done;\n\
                \t}\n\
                }\n",
        see_also: &["requirement", "action", "concern"],
    },
    Notation {
        of: "analysis",
        when: "A question asked of a design and answered by calculation: the subject under \
               study, an `objective` saying what is wanted of the answer, and `return` for \
               the answer itself. A trade study is one of these.",
        sysml: "package Sizing {\n\
                \tprivate import ISQ::*;\n\
                \tprivate import SI::*;\n\
                \n\
                \tpart def Vehicle {\n\
                \t\tattribute mass : MassValue;\n\
                \t}\n\
                \tanalysis def MassAnalysis {\n\
                \t\tsubject vehicle : Vehicle;\n\
                \t\tobjective {\n\
                \t\t\tdoc /* Is the vehicle light enough? */\n\
                \t\t\trequire constraint { totalMass <= 2000 [kg] }\n\
                \t\t}\n\
                \t\treturn totalMass : MassValue = vehicle.mass;\n\
                \t}\n\
                \n\
                \tpart car : Vehicle {\n\
                \t\tattribute :>> mass = 1500 [kg];\n\
                \t}\n\
                \tanalysis carMass : MassAnalysis {\n\
                \t\tsubject vehicle = car;\n\
                \t}\n\
                }\n",
        see_also: &["calculation", "verification", "quantity"],
    },
    Notation {
        of: "view",
        when: "What a reader is shown of the model, and why. A `viewpoint` frames the concerns a \
               view answers; a `view def` satisfies one and filters what it shows; a `view` \
               exposes the part of the model it is of and says how it is rendered.",
        sysml: "package Documentation {\n\
                \tprivate import Views::*;\n\
                \n\
                \tpart def Engineer;\n\
                \tconcern breakdown {\n\
                \t\tdoc /* How the system is broken down into parts. */\n\
                \t\tsubject;\n\
                \t\tstakeholder engineer : Engineer;\n\
                \t}\n\
                \tviewpoint structure {\n\
                \t\tframe breakdown;\n\
                \t}\n\
                \n\
                \tpart vehicle {\n\
                \t\tpart engine;\n\
                \t\tpart wheels[4];\n\
                \t}\n\
                \n\
                \tview def StructureView {\n\
                \t\tsatisfy structure;\n\
                \t\tfilter @SysML::PartUsage;\n\
                \t}\n\
                \tview vehicleStructure : StructureView {\n\
                \t\texpose vehicle::**;\n\
                \t\trender asTreeDiagram;\n\
                \t}\n\
                }\n",
        see_also: &["concern", "filter", "metadata"],
    },
    Notation {
        of: "concern",
        when: "What a stakeholder cares about, said before anyone decides what meets it. A \
               `concern` names the `stakeholder`s who hold it; a requirement or a viewpoint \
               that answers it says so with `frame`.",
        sysml: "package Stakeholders {\n\
                \tpart def Vehicle;\n\
                \tpart def Driver;\n\
                \tpart def Regulator;\n\
                \n\
                \tconcern def Safety {\n\
                \t\tdoc /* That nobody is hurt by the vehicle. */\n\
                \t\tsubject vehicle : Vehicle;\n\
                \t\tstakeholder driver : Driver;\n\
                \t}\n\
                \tconcern crashworthiness : Safety {\n\
                \t\tdoc /* That the occupants survive a collision. */\n\
                \t\tsubject vehicle;\n\
                \t\tstakeholder regulator : Regulator;\n\
                \t}\n\
                \n\
                \trequirement def CrashProtection {\n\
                \t\tdoc /* The cabin shall stay intact in a frontal collision. */\n\
                \t\tsubject vehicle : Vehicle;\n\
                \t\tframe crashworthiness;\n\
                \t}\n\
                }\n",
        see_also: &["requirement", "view"],
    },
    Notation {
        of: "message",
        when: "Something sent from one participant to another in an interaction -- what a \
               sequence diagram draws. Each participant declares the `event occurrence`s it \
               takes part in; `message ... of ... from ... to ...` joins two of them, and \
               `first ... then ...` orders messages.",
        sysml: "package Cruise {\n\
                \titem def SetSpeed;\n\
                \titem def FuelCommand;\n\
                \n\
                \tpart def Driver {\n\
                \t\tevent occurrence setSpeedSent;\n\
                \t}\n\
                \tpart def Controller {\n\
                \t\tevent occurrence setSpeedReceived;\n\
                \t\tthen event occurrence fuelCommandSent;\n\
                \t}\n\
                \tpart def Engine {\n\
                \t\tevent occurrence fuelCommandReceived;\n\
                \t}\n\
                \n\
                \toccurrence def CruiseInteraction {\n\
                \t\tref part driver : Driver;\n\
                \t\tref part controller : Controller;\n\
                \t\tref part engine : Engine;\n\
                \t\tmessage setSpeed of SetSpeed\n\
                \t\t\tfrom driver.setSpeedSent to controller.setSpeedReceived;\n\
                \t\tmessage fuelCommand of FuelCommand\n\
                \t\t\tfrom controller.fuelCommandSent to engine.fuelCommandReceived;\n\
                \t\tfirst setSpeed then fuelCommand;\n\
                \t}\n\
                }\n",
        see_also: &["occurrence", "flow", "item"],
    },
    Notation {
        of: "individual",
        when: "One particular thing rather than a kind of thing -- this vehicle, not vehicles -- \
               and what it is like at a moment (`snapshot`) or over a stretch of its life \
               (`timeslice`).",
        sysml: "package Fleet {\n\
                \tprivate import ScalarValues::*;\n\
                \n\
                \titem def Person;\n\
                \tpart def Vehicle {\n\
                \t\tattribute mass : Real;\n\
                \t\tref item driver : Person;\n\
                \t}\n\
                \n\
                \tindividual item def Alice :> Person;\n\
                \tindividual item def Bob :> Person;\n\
                \n\
                \tindividual part def Vehicle1 :> Vehicle {\n\
                \t\tsnapshot part loaded {\n\
                \t\t\t:>> mass = 2000.0;\n\
                \t\t}\n\
                \t\tsnapshot part unloaded {\n\
                \t\t\t:>> mass = 1500.0;\n\
                \t\t}\n\
                \t\tfirst loaded then unloaded;\n\
                \t}\n\
                \n\
                \tindividual part trip : Vehicle1 {\n\
                \t\ttimeslice withAlice {\n\
                \t\t\tref individual item :>> driver : Alice;\n\
                \t\t}\n\
                \t\tthen timeslice withBob {\n\
                \t\t\tref individual item :>> driver : Bob;\n\
                \t\t}\n\
                \t}\n\
                }\n",
        see_also: &["occurrence", "part"],
    },
    Notation {
        of: "quantity",
        when: "A physical value with a unit: a mass, a speed, a length. The quantity types come \
               from `ISQ` and the units from `SI` (or `USCustomaryUnits`); the unit is \
               written in brackets after the number.",
        sysml: "package Performance {\n\
                \tprivate import ISQ::*;\n\
                \tprivate import SI::*;\n\
                \tprivate import USCustomaryUnits::*;\n\
                \n\
                \tpart def Vehicle {\n\
                \t\tattribute mass : MassValue = 1200 [kg];\n\
                \t\tattribute payload : MassValue = 900 [lb];\n\
                \t\tattribute range : LengthValue = 500 [km];\n\
                \t\tattribute topSpeed : SpeedValue = 50 [m / s];\n\
                \t\tattribute power : PowerValue = 90000 [W];\n\
                \t}\n\
                }\n",
        see_also: &["attribute", "constraint", "calculation"],
    },
    Notation {
        of: "dependency",
        when: "One element relies on another without anything more being said of how -- a \
               subsystem on a software design, a document on what it describes. `dependency \
               from ... to ...`, named where it is referred to.",
        sysml: "package Architecture {\n\
                \tpart system {\n\
                \t\tpart computer;\n\
                \t\tpart storage;\n\
                \t}\n\
                \tpackage Software {\n\
                \t\titem def MessageSchema;\n\
                \t\titem def DataSchema;\n\
                \t}\n\
                \n\
                \tdependency from system::computer to Software;\n\
                \tdependency schemata\n\
                \t\tfrom system::storage\n\
                \t\tto Software::MessageSchema, Software::DataSchema;\n\
                }\n",
        see_also: &["allocation", "package"],
    },
    Notation {
        of: "filter",
        when: "Bringing in only some of what a package holds, chosen by the metadata on it. \
               `import X::**[@M];` imports what carries `@M`; `filter` in a package does the \
               same for every import in it.",
        sysml: "package Features {\n\
                \tprivate import ScalarValues::Boolean;\n\
                \n\
                \tmetadata def Safety {\n\
                \t\tattribute mandatory : Boolean;\n\
                \t}\n\
                \n\
                \tpart vehicle {\n\
                \t\tpart brakes { @Safety { mandatory = true; } }\n\
                \t\tpart airbag { @Safety { mandatory = false; } }\n\
                \t\tpart radio;\n\
                \t}\n\
                \n\
                \tpackage SafetyFeatures {\n\
                \t\tpublic import vehicle::**[@Safety];\n\
                \t}\n\
                \tpackage MandatoryFeatures {\n\
                \t\tpublic import vehicle::**;\n\
                \t\tfilter @Safety and (as Safety).mandatory;\n\
                \t}\n\
                }\n",
        see_also: &["metadata", "package", "view"],
    },
    Notation {
        of: "alias",
        when: "A second name for something declared elsewhere, so it can be reached by a name \
               that suits where it is used. `alias <new> for <existing>;` -- and the alias \
               is visible wherever a member would be.",
        sysml: "package Naming {\n\
                \tprivate import ISQ::*;\n\
                \n\
                \tpart def Automobile;\n\
                \talias Car for Automobile;\n\
                \talias Mass for ISQ::MassValue;\n\
                \n\
                \tpart def Garage {\n\
                \t\tpart parked : Car[2];\n\
                \t\tattribute load : Mass;\n\
                \t}\n\
                }\n",
        see_also: &["package", "specialization"],
    },
];

/// The other words a caller asks for a construct by, and the one that
/// shows it.
///
/// A construct shown with another is asked for by its own name as often
/// as by the one it is filed under -- a viewpoint by `viewpoint`, a
/// snapshot by `snapshot` -- and answering those with "nothing here is
/// written as that" says the language has not got them.
pub const ASKED_AS: &[(&str, &str)] = &[
    ("usecase", "use case"),
    ("actor", "use case"),
    ("include", "use case"),
    ("analysis case", "analysis"),
    ("trade study", "analysis"),
    ("viewpoint", "view"),
    ("expose", "view"),
    ("render", "view"),
    ("rendering", "view"),
    ("stakeholder", "concern"),
    ("frame", "concern"),
    ("sequence", "message"),
    ("interaction", "message"),
    ("event", "message"),
    ("snapshot", "individual"),
    ("timeslice", "individual"),
    ("unit", "quantity"),
    ("units", "quantity"),
    ("import filter", "filter"),
];

/// What `of` names, or nothing where nothing does. `use_case` and
/// `use-case` are `use case`, as a caller that cannot put a space in a
/// name writes it.
pub fn notation(of: &str) -> Option<&'static Notation> {
    let asked = of.trim().replace(['_', '-'], " ");
    let asked = ASKED_AS
        .iter()
        .find(|(word, _)| word.eq_ignore_ascii_case(&asked))
        .map_or(asked.as_str(), |(_, filed)| filed);
    NOTATION.iter().find(|it| it.of.eq_ignore_ascii_case(asked))
}
