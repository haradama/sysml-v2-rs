package BlinkRequirements {
    doc
    /*
     * What the sketch is for, and what has to hold for it to be right.
     * A requirement here is not a comment: `satisfy` names the part that
     * answers for it, so `sysml check` fails when that part is renamed
     * away, and `sysml rustgen` writes one ignored test per requirement
     * -- a list of what has been promised and not yet verified.
     *
     * `R.n` numbers the requirement and `S.n` the satisfaction of it, so
     * a review can say which is which without reading either name.
     */

    private import ScalarValues::*;
    private import ArduinoUnoHardware::*;
    private import ArduinoHal::Millis;
    private import Blink::*;

    requirement def <'R.1'> VisibleIndication {
        doc /* The board shall show, without instruments, that it is running. */
        subject board : ArduinoUno;
        require constraint {
            board.statusLed.forwardVoltage > 0 [V]
        }
    }

    requirement def <'R.2'> OnBoardLedOnly {
        doc
        /*
         * The indication shall use the LED already on the board, so that
         * a bare Uno with nothing wired to it still shows life.
         */
        subject app : BlinkApp;
        require constraint {
            app.pin == ledPinNumber
        }
    }

    requirement def <'R.3'> PerceptiblePeriod {
        doc
        /*
         * A half-period between a tenth of a second and two seconds:
         * faster reads as a flicker, slower as a fault.
         */
        subject app : BlinkApp;
        attribute lowerBound : Millis = 100;
        attribute upperBound : Millis = 2000;
        require constraint {
            app.halfPeriodMillis >= lowerBound and app.halfPeriodMillis <= upperBound
        }
    }

    requirement def <'R.4'> EqualDutyCycle {
        doc
        /*
         * Lit and dark shall last the same, so the blink reads as even.
         * This one carries no constraint on purpose: it is a property of
         * the state machine rather than of any number, and the model has
         * no number to compare. It is on the list all the same -- the
         * generated stub is what says it has not been verified.
         */
        subject app : BlinkApp;
    }

    part def BlinkingUno :> ArduinoUno {
        doc /* An Uno with the sketch on it, and what that promises. */
        part app : BlinkApp;

        // Naming the satisfaction and typing it with the requirement is
        // what carries the requirement through to the generated tests;
        // `satisfy requirement <name> by ...` alone only declares one.
        satisfy requirement <'S.1'> visibleIndication : VisibleIndication by statusLed;
        satisfy requirement <'S.2'> onBoardLedOnly : OnBoardLedOnly by app;
        satisfy requirement <'S.3'> perceptiblePeriod : PerceptiblePeriod by app;
        satisfy requirement <'S.4'> equalDutyCycle : EqualDutyCycle by app;
    }
}
