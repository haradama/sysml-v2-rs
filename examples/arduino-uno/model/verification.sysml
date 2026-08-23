package BlinkVerification {
    doc
    /*
     * How each requirement is found out to hold.
     *
     * A verification case says what answers for a requirement, and its
     * `@rust` binding says where that answer is written -- so the test
     * `sysml rustgen` writes for a requirement calls it rather than
     * standing there ignored. A requirement with no case here still
     * gets a test, and that test says it has not been verified.
     *
     * Only the software side is here. A hardware requirement is about a
     * board, and there is no board in the binary to ask: `H.1`--`H.3`
     * are found out on a bench, and what is written down is that they
     * have not been.
     */

    private import ArduinoHal::rust;
    private import Blink::*;
    private import BlinkRequirements::*;

    verification def <'T.1'> VerifyVisibleIndication {
        doc /* Run the sketch and watch the pin: it must go high, and low again. */
        @rust {
            :>> path = "crate::verification::visible_indication";
        }
        subject app : BlinkApp;
        objective {
            verify VisibleIndication;
        }
    }

    verification def <'T.2'> VerifyOnBoardLedOnly {
        doc /* Run the sketch and watch which pins it writes: one, its own. */
        @rust {
            :>> path = "crate::verification::on_board_led_only";
        }
        subject app : BlinkApp;
        objective {
            verify OnBoardLedOnly;
        }
    }

    verification def <'T.3'> VerifyPerceptiblePeriod {
        doc /* Read the half-period the model declares and hold it to the bounds. */
        @rust {
            :>> path = "crate::verification::perceptible_period";
        }
        subject app : BlinkApp;
        objective {
            verify PerceptiblePeriod;
        }
    }

    verification def <'T.4'> VerifyEqualDutyCycle {
        doc /* Run the sketch and compare how long the LED is lit with how long it is dark. */
        @rust {
            :>> path = "crate::verification::equal_duty_cycle";
        }
        subject app : BlinkApp;
        objective {
            verify EqualDutyCycle;
        }
    }
}
