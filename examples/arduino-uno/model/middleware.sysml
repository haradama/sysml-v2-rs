package ArduinoHal {
    doc
    /*
     * The board as an application may touch it: set a pin, wait, read the
     * clock. Every definition carries a `@rust { ... }` usage naming the
     * item it stands for, so what `sysml rustgen` writes calls the real
     * trait in `src/hal.rs` rather than inventing a parallel one.
     *
     * This is what `sysml import-rust` writes from a crate's rustdoc
     * JSON; it is hand-written here so the example is readable without
     * running rustdoc first.
     */

    private import ScalarValues::*;

    metadata def rust {
        attribute path : String;
        attribute crateName : String;
        attribute takesSelf : String;
        attribute isAsync : Boolean;
        attribute isFallible : Boolean;
    }

    port def Gpio {
        doc /* What the application is given to reach the board with. */
        @rust { :>> path = "crate::hal::Gpio"; :>> crateName = "arduino_uno"; }
    }

    action def SetPin {
        doc /* Drive one digital pin high or low. */
        @rust {
            :>> path = "crate::hal::Gpio::set_pin";
            :>> crateName = "arduino_uno";
            :>> takesSelf = "&mut self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        in pin : Integer;
        in high : Boolean;
    }

    action def Wait {
        doc /* Block for that many milliseconds. */
        @rust {
            :>> path = "crate::hal::Gpio::wait_millis";
            :>> crateName = "arduino_uno";
            :>> takesSelf = "&mut self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        in millis : Integer;
    }

    action def Uptime {
        doc /* Milliseconds since the board came up. */
        @rust {
            :>> path = "crate::hal::Gpio::uptime_millis";
            :>> crateName = "arduino_uno";
            :>> takesSelf = "&self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        out result : Integer;
    }
}
