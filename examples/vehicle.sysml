package VehicleModel {
    doc /* A small vehicle model: definitions, usages, typing,
         * redefinition and a specialization hierarchy. */

    import ScalarValues::*;

    abstract part def PowerSource;

    part def Engine :> PowerSource {
        port fuelIn : FuelPort;
        attribute power : Real;
    }

    part def Wheel {
        attribute diameter : Real = 0.6;
    }

    port def FuelPort;

    part def Vehicle {
        //* Mass is in kilograms. */
        attribute mass : Real = 1200.0;
        part eng : Engine;
        part wheels : Wheel[4];
    }

    part myCar : Vehicle {
        attribute :>> mass = 1450.0; // heavier variant
    }

    alias Car for Vehicle;

    comment about Vehicle /* The root element of this example. */
}
