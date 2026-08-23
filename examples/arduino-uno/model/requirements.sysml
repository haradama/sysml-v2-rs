package BlinkRequirements {
    doc
    /*
     * What the sketch is for, and what has to hold for it to be right.
     *
     * A requirement is a hardware requirement or a software requirement
     * by virtue of its `subject`, and by nothing else: the syntax is the
     * same either way. `H.n` here takes a board or a part of one, `S.n`
     * takes the application. `N.1` is the need both sides answer to.
     *
     * A requirement is also not a comment: `satisfy` names the part that
     * answers for it, so `sysml check` fails when that part is renamed
     * away, and `sysml rustgen` writes one ignored test per requirement
     * -- a list of what has been promised and not yet verified.
     */

    private import ScalarValues::*;
    private import ISQ::*;
    private import SI::*;
    private import ArduinoCompatibleHardware::*;
    private import ArduinoHal::Millis;
    private import Blink::*;

    requirement def <'N.1'> VisibleIndication {
        doc /* The board shall show, without instruments, that it is running. */
        subject board : ArduinoCompatibleBoard;
        require constraint {
            board.statusLed.forwardVoltage > 0 [V]
        }
    }

    // ---- hardware: the subject is a board, or a part of one ----

    requirement def <'H.1'> LedOnTheSketchesPin {
        doc
        /*
         * The status LED shall sit on the pin the sketch writes to. This
         * is the one requirement that is about neither side alone: the
         * board decides where the LED is, the sketch decides where it
         * writes, and the two are only the same by agreement.
         */
        subject board : ArduinoCompatibleBoard;
        require constraint {
            board.statusLedPin == ledPinNumber
        }
    }

    requirement def <'H.2'> FiveVoltLogic {
        doc
        /*
         * The microcontroller shall drive 5 V logic. A 3.3 V board takes
         * the same sketch and lights the same LED more dimly, which is
         * why this is written down rather than assumed from the fact
         * that it works.
         */
        subject mcu : Microcontroller;
        require constraint {
            mcu.operatingVoltage == 5 [V]
        }
    }

    requirement def <'H.3'> RoomForTheSketch {
        doc /* The flash shall hold the sketch, with the bootloader still in it. */
        subject mcu : Microcontroller;
        attribute sketchBytes : Integer;
        attribute bootloaderBytes : Integer = 512;
        require constraint {
            mcu.flashBytes >= sketchBytes + bootloaderBytes
        }
    }

    // ---- software: the subject is the application ----

    requirement def <'S.1'> OnBoardLedOnly {
        doc
        /*
         * The indication shall use the LED already on the board, so that
         * a bare board with nothing wired to it still shows life.
         */
        subject app : BlinkApp;
        require constraint {
            app.pin == ledPinNumber
        }
    }

    requirement def <'S.2'> PerceptiblePeriod {
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

    requirement def <'S.3'> EqualDutyCycle {
        doc
        /*
         * Lit and dark shall last the same, so the blink reads as even.
         * This one carries no constraint on purpose: it is a property of
         * the state machine rather than of any number, and the model has
         * no two numbers to compare. What answers for it is a run of the
         * machine rather than an inspection of it, which is what
         * `T.4` in `verification.sysml` does.
         */
        subject app : BlinkApp;
    }

    // ---- the two sides, on one board ----

    part def BlinkingBoard :> ArduinoCompatibleBoard {
        doc /* An Arduino-compatible board with the sketch on it. */
        part app : BlinkApp;

        // Naming the satisfaction and typing it with the requirement is
        // what carries the requirement through to the generated tests;
        // `satisfy requirement <name> by ...` alone only declares one.
        satisfy requirement <'V.0'> indication : VisibleIndication by statusLed;
        satisfy requirement <'V.1'> ledPin : LedOnTheSketchesPin by statusLed;
        satisfy requirement <'V.2'> logicLevel : FiveVoltLogic by mcu;
        satisfy requirement <'V.3'> room : RoomForTheSketch by mcu;
        satisfy requirement <'V.4'> onBoardLed : OnBoardLedOnly by app;
        satisfy requirement <'V.5'> period : PerceptiblePeriod by app;
        satisfy requirement <'V.6'> duty : EqualDutyCycle by app;
    }

    // One specification rather than two lists. `satisfy` above says who
    // answers for a requirement; this says what each is *about* -- and
    // it is what lets a requirement written once for any microcontroller
    // be required here of this board's own.
    requirement <'R.1'> blinkingBoardSpecification {
        doc /* What a compatible board plus this sketch has to be. */
        subject board : BlinkingBoard;

        requirement ledPin : LedOnTheSketchesPin {
            subject = board;
        }
        requirement logicLevel : FiveVoltLogic {
            subject = board.mcu;
        }
        requirement room : RoomForTheSketch {
            subject = board.mcu;
            attribute :>> sketchBytes = 1024;
        }

        requirement onBoardLed : OnBoardLedOnly {
            subject = board.app;
        }
        requirement period : PerceptiblePeriod {
            subject = board.app;
        }
        requirement duty : EqualDutyCycle {
            subject = board.app;
        }
    }
}
