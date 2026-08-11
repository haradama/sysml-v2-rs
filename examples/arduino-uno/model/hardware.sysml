package ArduinoUnoHardware {
    doc
    /*
     * What the board is made of, and which pin the on-board LED sits on.
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

    part def Led {
        attribute forwardVoltage : ElectricPotentialValue;
    }

    part def Resistor {
        attribute resistance : Real;
    }

    part def Atmega328p {
        doc /* The microcontroller the sketch runs on. */
        attribute clock : FrequencyValue = 16000000 [Hz];
        attribute operatingVoltage : ElectricPotentialValue = 5 [V];
        attribute flashBytes : Integer = 32768;
        attribute sramBytes : Integer = 2048;
        port digital : DigitalPin[14];
    }

    part def UsbInterface;

    part def ArduinoUno {
        part mcu : Atmega328p;
        part usb : UsbInterface;

        part statusLed : Led {
            doc /* Marked `L` on the silkscreen, wired to digital pin 13. */
            attribute :>> forwardVoltage = 2 [V];
        }
        part statusLedResistor : Resistor {
            attribute :>> resistance = 1000.0;
        }

        connect mcu.digital to statusLed;
    }

    attribute ledPinNumber : Integer = 13;
}
