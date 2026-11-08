package Thermostat {
    doc
    /*
     * A room thermostat: what it measures, what it decides, and what it
     * must never do. Small on purpose -- one of each shape a plan has,
     * so that what a generator makes of each can be read in one file.
     */

    private import ScalarValues::*;

    // how a definition says it already exists somewhere
    metadata def code {
        attribute writtenIn : String;
        attribute path : String;
        attribute module : String;
    }

    // a value: a primitive under another name
    attribute def Celsius :> Real;

    // a constant of the package
    attribute hysteresis : Celsius = 0.5;

    enum def Mode {
        off;
        heating;
        cooling;
    }

    port def Thermometer {
        doc /* The sensor, which already exists in Python: imported, not written again. */
        @code { :>> writtenIn = "python"; :>> path = "sensors.Thermometer"; :>> module = "sensors"; }
    }

    port def Display {
        doc /* Where the reading is shown. */
        attribute shown : Celsius;
    }

    abstract part def Space {
        doc /* Anything with a temperature. */
        attribute current : Celsius;
    }

    part def Room :> Space {
        doc /* One room, with what it is asked to be and what it is. */
        attribute target : Celsius = 21.0;
        attribute mode : Mode = Mode::off;
        attribute log : String[0..*] ordered;
        attribute nickname : String[0..1];
        port thermometer : Thermometer;
        port display : Display;
        perform action regulate : Regulate;
    }

    part def Building {
        part rooms : Room[1..*];
        part lobby : Room;
    }

    part def Radiator;
    part def UnderfloorCoil;
    variation part def Heater {
        doc /* One of the two, never both. */
        variant part radiator : Radiator;
        variant part underfloor : UnderfloorCoil;
    }

    calc def Error {
        doc /* How far the room is from what it was asked to be. */
        in target : Celsius;
        in current : Celsius;
        return : Real;
        target - current
    }

    action def Measure { out reading : Celsius; }
    action def Decide { in reading : Celsius; out chosen : Mode; }
    action def Regulate {
        doc /* One pass of the loop. */
        action measure : Measure;
        action choose : Decide;
        first measure then decide;
        flow measure.reading to choose.reading;
    }

    state def Regulation {
        doc /* What the thermostat is doing, and when it changes its mind. */
        entry; then idle;
        state idle;
        state heating;
        state cooling;
        transition warm
            first idle
            accept error : Celsius
            if error > hysteresis
            then heating;
        transition cool
            first idle
            accept error : Celsius
            if error < -hysteresis
            then cooling;
        transition settleFromHeating first heating then idle;
        transition settleFromCooling first cooling then idle;
    }

    requirement def <'R.1'> Comfort {
        doc /* A room is never far from what it was asked to be. */
        subject room : Room;
        assume constraint { room.mode != Mode::off }
        require constraint { room.current >= room.target - 1.0 }
    }
}
