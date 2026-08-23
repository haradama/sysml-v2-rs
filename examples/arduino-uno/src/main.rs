//! The sketch, run against a board that exists only in memory.
//!
//! Nothing here decides what blinking *is* -- `src/generated.rs` holds
//! that, written from `model/blink.sysml`. `src/sketch.rs` holds the
//! half the model leaves open, and this runs it and says what happened.

mod generated;
mod hal;
mod sketch;
#[cfg(test)]
mod verification;

use generated::BlinkApp;
use hal::Simulated;

fn main() {
    let mut app: BlinkApp<Simulated> = BlinkApp::default();
    println!("pin {} every {} ms", app.pin, app.half_period_millis);

    for step in sketch::run(&mut app, 6) {
        println!(
            "{:>5} ms  {:?}  pin {} = {}",
            step.at_millis,
            step.state,
            app.pin,
            if step.lit { "high" } else { "low" }
        );
    }

    let high = app.board.written.iter().filter(|(_, on)| *on).count();
    println!("{} writes, {high} of them high", app.board.written.len());
}
