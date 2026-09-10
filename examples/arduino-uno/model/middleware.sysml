package ArduinoHal {
    doc
    /*
     * The board as an application may touch it: set a pin, wait, read the
     * clock. Every definition carries a `@code { ... }` usage naming the
     * item it stands for, so what `sysml rustgen` writes calls the real
     * trait in `src/hal.rs` rather than inventing a parallel one.
     *
     * This is what `sysml import-rust` writes from a crate's rustdoc
     * JSON; it is hand-written here so the example is readable without
     * running rustdoc first.
     */

    private import ScalarValues::*;

    metadata def code {
        attribute writtenIn : String;
        attribute path : String;
        attribute module : String;
        attribute takesSelf : String;
        attribute isAsync : Boolean;
        attribute isFallible : Boolean;
        attribute capabilities : String;
    }

    attribute def PinNumber :> Integer {
        doc
        /*
         * Which of the fourteen pins. An unbounded `Integer` would be an
         * `i64`, and sixty-four bits of arithmetic on an eight-bit
         * microcontroller costs both flash and time, so the model says
         * the width it means rather than leaving it to be assumed.
         */
        @code {
            :>> writtenIn = "rust";
            :>> path = "u8";
            :>> capabilities = "Debug, Clone, PartialEq, Default";
        }
    }

    attribute def Millis :> Integer {
        doc /* A duration in milliseconds; a minute of them fits in u16. */
        @code {
            :>> writtenIn = "rust";
            :>> path = "u16";
            :>> capabilities = "Debug, Clone, PartialEq, Default";
        }
    }

    port def Gpio {
        doc /* What the application is given to reach the board with. */
        @code {
            :>> writtenIn = "rust";
            :>> path = "crate::hal::Gpio";
            :>> module = "arduino_uno";
        }
    }

    action def SetPin {
        doc /* Drive one digital pin high or low. */
        @code {
            :>> writtenIn = "rust";
            :>> path = "crate::hal::Gpio::set_pin";
            :>> module = "arduino_uno";
            :>> takesSelf = "&mut self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        in pin : PinNumber;
        in high : Boolean;
    }

    action def Wait {
        doc /* Block for that many milliseconds. */
        @code {
            :>> writtenIn = "rust";
            :>> path = "crate::hal::Gpio::wait_millis";
            :>> module = "arduino_uno";
            :>> takesSelf = "&mut self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        in millis : Millis;
    }

    action def Uptime {
        doc /* Milliseconds since the board came up. */
        @code {
            :>> writtenIn = "rust";
            :>> path = "crate::hal::Gpio::uptime_millis";
            :>> module = "arduino_uno";
            :>> takesSelf = "&self";
            :>> isAsync = false;
            :>> isFallible = false;
        }
        out result : Millis;
    }
}
