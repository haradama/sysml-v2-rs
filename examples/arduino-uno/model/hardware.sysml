package ArduinoCompatibleHardware {
    doc
    /*
     * An Arduino-compatible board: not one product, but what a board has
     * to be for the sketch to run on it. Boards differ -- the USB bridge
     * on a clone is not the one on a board from Arduino -- and what may
     * differ is exactly what the requirements do not pin down.
     *
     * Nothing here is generated into Rust: it is the part of the model a
     * reader consults to see why the application writes to pin 13.
     */

    private import ScalarValues::*;
    private import ISQ::*;
    private import SI::*;

    port def DigitalPin {
        doc /* One of the fourteen digital I/O pins along the headers. */
        attribute number : Integer;
    }

    port def SerialPort {
        doc /* The pair of pins a sketch is uploaded over. */
    }

    part def Led {
        attribute forwardVoltage : ElectricPotentialValue;
    }

    part def Resistor {
        attribute resistance : Real;
    }

    part def Microcontroller {
        doc
        /*
         * The part the sketch runs on. The numbers are left open here:
         * what they have to be is a requirement, not a definition, and
         * writing them twice is how the two come to disagree.
         */
        attribute clock : FrequencyValue;
        attribute operatingVoltage : ElectricPotentialValue;
        attribute flashBytes : Integer;
        attribute sramBytes : Integer;
        port digital : DigitalPin[14];
        port serial : SerialPort;
    }

    part def Atmega328p :> Microcontroller {
        attribute :>> clock = 16000000 [Hz];
        attribute :>> operatingVoltage = 5 [V];
        attribute :>> flashBytes = 32768;
        attribute :>> sramBytes = 2048;
    }

    part def UsbBridge;
    part def Atmega16u2 :> UsbBridge;
    part def Ch340 :> UsbBridge;

    part def ArduinoCompatibleBoard {
        doc /* The board the sketch is written against. */

        attribute statusLedPin : Integer = ledPinNumber;

        part mcu : Microcontroller;
        part usb : UsbBridge;

        part statusLed : Led {
            doc /* Marked `L` on the silkscreen, wired to a digital pin. */
            attribute :>> forwardVoltage = 2 [V];
        }
        part statusLedResistor : Resistor {
            attribute :>> resistance = 1000.0;
        }

        // The LED is not on the pin directly: the resistor is in series
        // with it, and that is what keeps the current inside what the
        // pin and the LED can each take.
        connect mcu.digital to statusLedResistor;
        connect statusLedResistor to statusLed;
        // The bridge is on the serial pins, which is how a sketch gets
        // onto the board in the first place.
        connect usb to mcu.serial;
    }

    part def ArduinoUno :> ArduinoCompatibleBoard {
        doc /* The board from Arduino: an ATmega328P, and a second AVR for USB. */
        part redefines mcu : Atmega328p;
        part redefines usb : Atmega16u2;
    }

    part def Ch340Clone :> ArduinoCompatibleBoard {
        doc
        /*
         * A clone: the same microcontroller behind a cheaper USB bridge.
         * Nothing the sketch needs is stated in terms of the bridge, so
         * the difference is allowed rather than overlooked.
         */
        part redefines mcu : Atmega328p;
        part redefines usb : Ch340;
    }

    attribute ledPinNumber : Integer = 13;
}
