//! The half the model leaves open: the loop that decides when a
//! half-period has passed, and what the two states mean at the pin.
//!
//! `main` runs it to be watched and `verification` runs it to be
//! checked, and neither has a loop of its own -- a verification that
//! re-implemented the sketch would verify the re-implementation.

use crate::generated::{self, BlinkApp, BlinkingEvent, BlinkingHooks, BlinkingState, Tick};
use crate::hal::Gpio;

/// What the model leaves to the implementor: the two states, at the pin.
#[derive(Default)]
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

/// What the board looked like at the end of one half-period.
pub struct Step {
    pub at_millis: u16,
    pub state: BlinkingState,
    pub lit: bool,
}

/// Run the sketch for that many half-periods.
pub fn run<B: Gpio>(app: &mut BlinkApp<B>, half_periods: usize) -> Vec<Step> {
    let mut sketch = Sketch::default();
    let mut state = BlinkingState::initial();
    let mut changed_at = app.since();
    let mut steps = Vec::new();

    for _ in 0..half_periods {
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
        steps.push(Step {
            at_millis: now,
            state,
            lit: sketch.lit,
        });
    }
    steps
}
