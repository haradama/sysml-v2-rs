package Blink {
    doc
    /*
     * The sketch: hold the LED on for a while, then off for the same
     * while, forever. The period is an attribute of the application, not
     * a number buried in the code, so changing it is a change to the
     * model.
     */

    private import ScalarValues::*;
    private import ArduinoHal::*;
    private import ArduinoCompatibleHardware::ledPinNumber;

    part def BlinkApp {
        doc /* What runs on the board. */

        port board : Gpio;

        attribute pin : PinNumber = ledPinNumber;
        attribute halfPeriodMillis : Millis = 500;

        perform action light : SetPin;
        perform action wait : Wait;
        perform action since : Uptime;

        calc def Elapsed {
            doc /* How long the current half-period has run. */
            in now : Millis;
            in startedAt : Millis;
            now - startedAt
        }

        state def Blinking {
            doc
            /*
             * Two states, and the LED follows which one is current. The
             * half-period is not in here: when a half-period has passed
             * is the loop's business, and saying so twice is how the two
             * come to disagree.
             */
            entry;
            then dark;

            state dark;
            state lit;

            transition dark_to_lit first dark accept tick : Tick then lit;
            transition lit_to_dark first lit accept tick : Tick then dark;
        }
    }

    item def Tick {
        doc /* A half-period has passed; how long it took is the payload. */
        attribute elapsed : Millis;
    }
}
