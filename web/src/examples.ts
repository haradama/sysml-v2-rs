// What the page opens with, and what the menu offers.
//
// Each of these resolves against the standard library inside the module
// and violates none of the specification's constraints -- `sysml check`
// says so -- so a reader who has never written SysML v2 starts from
// something that works rather than from something to fix.

/// One model in the menu: what it is called, which drawing shows it off,
/// and, for an internal view, what that view is of.
export interface Example {
  name: string;
  view: "definitions" | "internal" | "browser";
  element?: string;
  source: string;
}

export const EXAMPLES: Example[] = [
  {
    name: "Vehicle — definitions",
    view: "definitions",
    source: `package VehicleDefinitions {
    doc
    /*
     * Type a model on the left; it is drawn on the right as you type.
     * Nothing is uploaded and nothing is compiled anywhere else: the
     * parser, name resolution, the specification's constraints and the
     * renderer are a WebAssembly module running in this tab.
     */

    private import ScalarValues::*;
    private import ISQ::*;
    private import SI::*;

    part def Vehicle {
        attribute mass : MassValue;
        part engine : Engine;
        part wheel : Wheel[4];
        part transmission : Transmission;
    }

    part def PowerSource {
        attribute power : PowerValue;
    }

    part def Engine :> PowerSource {
        attribute displacement : Real;
    }

    part def ElectricMotor :> PowerSource;

    part def Wheel {
        attribute diameter : LengthValue;
    }

    part def Transmission;

    part def HybridVehicle :> Vehicle {
        part motor : ElectricMotor;
    }
}
`,
  },
  {
    name: "Camera — internal structure",
    view: "internal",
    element: "Camera",
    source: `package CameraStructure {
    doc
    /*
     * An internal view draws one definition from the inside: the parts
     * it is assembled from, their ports, and what is connected to what.
     * The name in the box beside the view menu is what it is a view of.
     */

    private import ScalarValues::*;

    port def FramePort {
        attribute width : Integer;
        attribute height : Integer;
    }

    part def Sensor {
        port captured : FramePort;
    }

    part def Processor {
        port incoming : ~FramePort;
        port encoded : FramePort;
    }

    part def Storage {
        port arriving : ~FramePort;
    }

    part def Camera {
        part sensor : Sensor;
        part processor : Processor;
        part storage : Storage;

        connect sensor.captured to processor.incoming;
        connect processor.encoded to storage.arriving;
    }
}
`,
  },
  {
    name: "Traffic light — actions and states",
    view: "definitions",
    source: `package TrafficLight {
    private import ScalarValues::*;
    private import ISQ::*;
    private import SI::*;

    part def Lamp {
        attribute lit : Boolean;
    }

    part def SignalHead {
        part red : Lamp;
        part amber : Lamp;
        part green : Lamp;
    }

    action def Show {
        in lamp : Lamp;
        in duration : TimeValue;
    }

    action def Cycle {
        in head : SignalHead;

        first start;
        then action showGreen : Show;
        then action showAmber : Show;
        then action showRed : Show;
        then done;
    }

    state def Signalling {
        entry; then stopped;

        state stopped;
        state going;

        transition stopped then going;
        transition going then stopped;
    }
}
`,
  },
  {
    name: "Mass budget — requirements",
    view: "browser",
    source: `package MassBudget {
    doc
    /*
     * A requirement is not a comment: \`satisfy\` names the part that
     * answers for it, and the constraint is evaluated against the model.
     * The tree view below shows what this package actually contains.
     */

    private import ISQ::*;
    private import SI::*;

    part def Vehicle {
        attribute mass : MassValue;
    }

    requirement def <'R.1'> MassLimit {
        doc /* The vehicle shall not exceed its mass budget. */
        subject vehicle : Vehicle;
        attribute budget : MassValue;
        require constraint {
            vehicle.mass <= budget
        }
    }

    part sedan : Vehicle {
        attribute :>> mass = 1500 [kg];
    }

    satisfy MassLimit by sedan;
}
`,
  },
];
