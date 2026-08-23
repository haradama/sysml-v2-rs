# Arduino Uno

An Arduino Uno as four layers of one model -- the board, what an
application may touch it with, the sketch, and what the sketch has to be
right about -- and the Rust that falls out of them.

```console
$ cargo run
pin 13 every 500 ms
  500 ms  Lit  pin 13 = high
 1000 ms  Dark  pin 13 = low
 ...
6 writes, 3 of them high
```

## The layers

`model/hardware.sysml` is the board: the ATmega328P and its clock and
operating voltage, the fourteen digital pins, the LED marked `L` and the
resistor beside it, and the connection that puts the LED on pin 13.
Nothing is generated from it -- it is the part a reader consults to see
why the application writes to that pin rather than another.

`model/middleware.sysml` is the board as the sketch may touch it: set a
pin, wait, read the clock. It also says how wide the numbers are: an
unbounded `Integer` becomes an `i64`, and sixty-four bits of arithmetic
on an eight-bit microcontroller costs both flash and time, so the model
names `u8` for a pin and `u16` for a duration rather than leaving the
width to be assumed. Measured on the real target, the same arithmetic is
464 bytes as `i64` and 226 as `u16`. Each definition carries a `@rust { ... }`
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

`model/requirements.sysml` is what the sketch has to be right about, and
who answers for each: an Uno with the sketch on it satisfies four
requirements, three of them with a constraint the model can state in
full. A `satisfy` is not a comment. It names a part, so renaming that
part away makes `sysml check` fail, and it is what puts the
`Satisfied by` line in the generated stubs -- the trace from a promise
to the thing that keeps it.

The corpus writes a satisfaction as `satisfy requirement s : Req by p;`,
and the `: Req` is the part that names the requirement -- `s` is the
satisfaction's own name. Leave the typing out and `satisfy requirement s
by p;` still parses and still checks, but it satisfies nothing in
particular and traces nowhere.

## What is generated and what is not

```sh
sysml rustgen model/blink.sysml model/requirements.sysml \
    --library model/hardware.sysml \
    --library model/middleware.sysml \
    --library ../../vendor/sysml-v2-release/sysml.library \
    -o src/generated.rs
```

The board and the HAL are `--library`: names in them resolve, but no
Rust is written for them. The board is documentation, and the HAL
already exists as `src/hal.rs` -- generating either would be writing a
second copy of something that is already there.

`src/generated.rs` holds what the model already decided:

- `BlinkApp<Board: crate::hal::Gpio>` -- the port became a generic
  parameter bound to the real trait, the attributes became fields with
  the model's own defaults
- `light`, `wait`, `since` -- each `perform`ed action a method
  delegating through the port, with the binding's own receiver
- `BlinkingState`, `BlinkingEvent`, `BlinkingHooks`, `step` -- the state
  machine, with every open decision a hook that has a default
- `elapsed` -- the `calc def`, translated as written
- `BlinkingUno` -- the Uno with the sketch on it, from the part the
  requirements are satisfied by
- `mod requirements` -- one ignored test per requirement, carrying the
  documentation and the parts that answer for it:

  ```console
  $ cargo test
  test generated::requirements::equal_duty_cycle ... ignored, verification not written yet
  test generated::requirements::on_board_led_only ... ignored, verification not written yet
  test generated::requirements::perceptible_period ... ignored, verification not written yet
  test generated::requirements::visible_indication ... ignored, verification not written yet
  ```

  A requirement that nobody has verified is a test that says so, rather
  than a line in a document nobody runs.

`src/main.rs` holds what the model left open: what the two states mean
at the pin, and the loop that decides when a half-period has passed.
`src/hal.rs` holds the trait the model binds to and a board that exists
only in memory, so the sketch runs anywhere.

Change the model and regenerate: the parts that were decided move, and
the parts that were left open do not.

## Seeing it

```sh
sysml diagram model/*.sysml \
    --library ../../vendor/sysml-v2-release/sysml.library \
    --internal ArduinoUno -o board.svg
```

`--internal BlinkingUno` draws the other half instead: the application
and the requirements it answers for, with an edge per `satisfy`.
