# Arduino Uno

An Arduino Uno as three layers of one model, and the blink sketch that
falls out of it.

```console
$ cargo run
pin 13 every 500 ms
  500 ms  Lit  pin 13 = high
 1000 ms  Dark  pin 13 = low
 ...
6 writes, 3 of them high
```

## The three layers

`model/hardware.sysml` is the board: the ATmega328P and its clock and
operating voltage, the fourteen digital pins, the LED marked `L` and the
resistor beside it, and the connection that puts the LED on pin 13.
Nothing is generated from it -- it is the part a reader consults to see
why the application writes to that pin rather than another.

`model/middleware.sysml` is the board as the sketch may touch it: set a
pin, wait, read the clock. Each definition carries a `@rust { ... }`
usage naming the item it stands for, so the generated code calls the
real trait in `src/hal.rs` instead of a parallel one invented for it.
This is the shape `sysml import-rust` writes from a crate's rustdoc
JSON; it is written out here so the example reads without running
rustdoc first.

`model/blink.sysml` is the sketch: a part with a port typed by the HAL,
the pin and the half-period as attributes, and a two-state machine that
says what blinking is. The half-period is not in the machine -- when a
half-period has passed is the loop's business, and saying it in both
places is how the two come to disagree.

## What is generated and what is not

```sh
sysml rustgen model/blink.sysml \
    --library model/hardware.sysml \
    --library model/middleware.sysml \
    --library ../../vendor/sysml-v2-release/sysml.library \
    -o src/generated.rs
```

`src/generated.rs` holds what the model already decided:

- `BlinkApp<Board: crate::hal::Gpio>` -- the port became a generic
  parameter bound to the real trait, the attributes became fields with
  the model's own defaults
- `light`, `wait`, `since` -- each `perform`ed action a method
  delegating through the port, with the binding's own receiver
- `BlinkingState`, `BlinkingEvent`, `BlinkingHooks`, `step` -- the state
  machine, with every open decision a hook that has a default
- `elapsed` -- the `calc def`, translated as written

`src/main.rs` holds what the model left open: what the two states mean
at the pin, and the loop that decides when a half-period has passed.
`src/hal.rs` holds the trait the model binds to and a board that exists
only in memory, so the sketch runs anywhere.

Change the model and regenerate: the parts that were decided move, and
the parts that were left open do not.
