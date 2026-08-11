//! The sketch, run against a board that exists only in memory.
//!
//! Nothing here decides what blinking *is* -- `src/generated.rs` holds
//! that, written from `model/blink.sysml`. What is left for a person is
//! the half the model leaves open: the loop that decides when a
//! half-period has passed, and what the two states mean at the pin.

mod generated;
mod hal;

use generated::{BlinkApp, BlinkingEvent, BlinkingHooks, BlinkingState, Tick};
use hal::Simulated;

/// What the model leaves to the implementor: the two states, at the pin.
struct Sketch {
    lit: bool,
}

impl BlinkingHooks for Sketch {
    fn on_entry_lit(&mut self) {
        self.lit = true;
    }
    fn on_entry_dark(&mut self) {
        self.lit = false;
    }
}

fn main() {
    let mut app = BlinkApp {
        board: Simulated::default(),
        pin: 13,
        half_period_millis: 500,
    };
    let mut sketch = Sketch { lit: false };
    let mut state = BlinkingState::initial();
    let mut changed_at = app.since();

    println!("pin {} every {} ms", app.pin, app.half_period_millis);
    for _ in 0..6 {
        // the loop's business: wait out a half-period, then say so
        app.wait(app.half_period_millis);
        let now = app.since();
        let tick = Tick {
            elapsed: generated::elapsed(now, changed_at),
        };
        changed_at = now;

        state = state.step(
            &match state {
                BlinkingState::Dark => BlinkingEvent::DarkToLit(tick),
                BlinkingState::Lit => BlinkingEvent::LitToDark(tick),
            },
            &mut sketch,
        );

        // and the model's: which state the machine is in decides the pin
        app.light(app.pin, sketch.lit);
        println!(
            "{:>5} ms  {:?}  pin {} = {}",
            now,
            state,
            app.pin,
            if sketch.lit { "high" } else { "low" }
        );
    }

    let high = app.board.written.iter().filter(|(_, on)| *on).count();
    println!("{} writes, {high} of them high", app.board.written.len());
}
