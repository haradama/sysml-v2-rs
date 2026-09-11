# Arduino-compatible board

An Arduino-compatible board as five layers of one model -- the board,
what an application may touch it with, the sketch, what both have to be
right about, and how that is found out -- and the Rust that falls out of
them.

```console
$ cargo run
pin 13 every 500 ms
  500 ms  Lit  pin 13 = high
 1000 ms  Dark  pin 13 = low
 ...
6 writes, 3 of them high
```

## The layers

`model/hardware.sysml` is the board -- not one product, but what a board
has to be for the sketch to run on it: a microcontroller with a clock, an
operating voltage and a flash size, fourteen digital pins, and the LED
marked `L` with its resistor. `ArduinoUno` and `Ch340Clone` are two
boards that answer to it, differing in the USB bridge and in nothing the
sketch can tell apart. What may differ is exactly what the requirements
do not pin down. Nothing is generated from this file -- it is the part a
reader consults to see why the application writes to pin 13.

`model/middleware.sysml` is the board as the sketch may touch it: set a
pin, wait, read the clock. It also says how wide the numbers are: an
unbounded `Integer` becomes an `i64`, and sixty-four bits of arithmetic
on an eight-bit microcontroller costs both flash and time, so the model
names `u8` for a pin and `u16` for a duration rather than leaving the
width to be assumed. Measured on the real target, the same arithmetic is
464 bytes as `i64` and 226 as `u16`. Each definition carries a `@code { ... }`
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

`model/requirements.sysml` is what has to be right, on both sides.

### Hardware requirements and software requirements

They are written the same way. What makes one a hardware requirement and
another a software requirement is its **subject**:

```sysml
requirement def <'H.2'> FiveVoltLogic {
    subject mcu : Microcontroller;                     // hardware
    require constraint { mcu.operatingVoltage == 5 [V] }
}

requirement def <'S.1'> OnBoardLedOnly {
    subject app : BlinkApp;                            // software
    require constraint { app.pin == ledPinNumber }
}
```

`H.1` is the interesting one, because it is about neither side alone: the
board decides where the LED is wired, the sketch decides where it writes,
and `require constraint { board.statusLedPin == ledPinNumber }` is the
agreement between them. That is where a hardware/software interface
requirement lives -- in one statement, not in a paragraph on each side.

A requirement written for any microcontroller becomes a requirement of
*this* board's by being required of it, which is what a requirement group
is for:

```sysml
requirement <'R.1'> blinkingBoardSpecification {
    subject board : BlinkingBoard;

    requirement logicLevel : FiveVoltLogic { subject = board.mcu; }
    requirement onBoardLed : OnBoardLedOnly { subject = board.app; }
}
```

One specification, both sides, each bound to the part it is about. The
generated stubs list the definitions and this group; a member like
`logicLevel` restates `FiveVoltLogic` for a particular subject rather
than being a second thing to verify, so it gets no stub of its own.

### How it is found out

`model/verification.sysml` says what answers for each requirement, and
binds it to the Rust that runs it:

```sysml
verification def <'T.3'> VerifyPerceptiblePeriod {
    @code { :>> writtenIn = "rust"; :>> path = "crate::verification::perceptible_period"; }
    subject app : BlinkApp;
    objective { verify PerceptiblePeriod; }
}
```

`sysml rustgen` then writes a test that calls it, handing over the
numbers the requirement states about itself:

```rust
/// SysML: `requirement def PerceptiblePeriod`
/// Satisfied by `app`.
/// Verified by `VerifyPerceptiblePeriod`.
#[test]
fn perceptible_period() {
    crate::verification::perceptible_period(100, 2000);
}
```

Neither `100` nor `2000` is written in Rust, and neither is the
half-period they bound -- `BlinkApp::default()` carries the model's own
value. Change either in the model and the test changes with it; change
one so the other no longer fits and the test fails. `src/verification.rs`
holds the four verifications, and nothing else in the crate does.

Three requirements have no verification here, and their tests say so:
`H.1`--`H.3` are about a board, and there is no board in the binary to
ask.

### Who answers for it

`satisfy` is separate, and says something the group does not: which part
answers for the requirement. It names that part, so renaming it away
makes `sysml check` fail, and it is what puts the `Satisfied by` line in
the generated stubs -- the trace from a promise to the thing that keeps
it.

The corpus writes a satisfaction as `satisfy requirement s : Req by p;`,
and the `: Req` is the part that names the requirement -- `s` is the
satisfaction's own name. Leave the typing out and `satisfy requirement s
by p;` still parses and still checks, but it satisfies nothing in
particular and traces nowhere.

## What is generated and what is not

```sh
sysml rustgen model/blink.sysml model/requirements.sysml model/verification.sysml \
    --library model/hardware.sysml \
    --library model/middleware.sysml \
    --library ../../crates/sysml-stdlib/library \
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
- `BlinkingBoard` -- a compatible board with the sketch on it, from the
  part the requirements are satisfied by
- `mod requirements` -- one test per requirement, carrying the
  documentation, the parts that answer for it, and the verification that
  finds out whether it holds:

  ```console
  $ cargo test
  test generated::requirements::blinking_board_specification ... ignored, verification not written yet
  test generated::requirements::five_volt_logic ... ignored, verification not written yet
  test generated::requirements::led_on_the_sketches_pin ... ignored, verification not written yet
  test generated::requirements::room_for_the_sketch ... ignored, verification not written yet
  test generated::requirements::equal_duty_cycle ... ok
  test generated::requirements::on_board_led_only ... ok
  test generated::requirements::perceptible_period ... ok
  test generated::requirements::visible_indication ... ok
  ```

  A requirement that nobody has verified is a test that says so, rather
  than a line in a document nobody runs.

`src/sketch.rs` holds what the model left open: what the two states mean
at the pin, and the loop that decides when a half-period has passed.
`src/main.rs` runs it and says what happened; `src/verification.rs` runs
the same code and checks it, so that a verification cannot pass by
verifying a second copy of the sketch. `src/hal.rs` holds the trait the
model binds to and a board that exists only in memory, so it all runs
anywhere.

Change the model and regenerate: the parts that were decided move, and
the parts that were left open do not.

## Seeing it

```sh
sysml diagram model/*.sysml \
    --library ../../crates/sysml-stdlib/library \
    --internal ArduinoCompatibleBoard -o board.svg
```

`--internal BlinkingBoard` draws the other half instead: the application
and the requirements it answers for, with an edge per `satisfy`.
