//! The verifications the model names.
//!
//! `model/verification.sysml` binds each of these by path, so the test
//! `sysml rustgen` writes for a requirement calls the one the model
//! says answers for it, and the numbers the requirement states about
//! itself arrive as arguments. What has to hold is the requirement's
//! business; all that is here is how to find out.
//!
//! Nothing verifies `H.1`--`H.3`. They are about a board, and there is
//! no board in this binary to ask -- which is why their tests are the
//! ones still marked ignored.

use crate::generated::BlinkApp;
use crate::hal::Simulated;
use crate::sketch;

/// Long enough to see the LED both ways round, twice.
const HALF_PERIODS: usize = 4;

/// The app as the model configures it: the pin and the half-period are
/// the model's own values, carried into `Default` by `sysml rustgen`.
fn app() -> BlinkApp<Simulated> {
    BlinkApp::default()
}

/// `N.1` -- the board shall show, without instruments, that it is running.
pub fn visible_indication() {
    let mut app = app();
    sketch::run(&mut app, HALF_PERIODS);

    let pin = app.pin;
    let levels: Vec<bool> = app
        .board
        .written
        .iter()
        .filter(|(at, _)| *at == pin)
        .map(|(_, high)| *high)
        .collect();
    assert!(levels.contains(&true), "pin {pin} is never driven high");
    assert!(levels.contains(&false), "pin {pin} is never driven low");
}

/// `S.1` -- the indication shall use the LED already on the board.
///
/// What Rust can see is that the sketch touches one pin and it is the
/// app's own. That this is the pin the LED sits on is `H.1`, and it is
/// about the board rather than about the binary.
pub fn on_board_led_only() {
    let mut app = app();
    sketch::run(&mut app, HALF_PERIODS);

    let pin = app.pin;
    let elsewhere: Vec<u8> = app
        .board
        .written
        .iter()
        .map(|(at, _)| *at)
        .filter(|at| *at != pin)
        .collect();
    assert!(
        elsewhere.is_empty(),
        "the sketch also wrote to {elsewhere:?}, not only to pin {pin}"
    );
    assert!(
        usize::from(pin) < app.board.pins.len(),
        "pin {pin} is not one of the headers"
    );
}

/// `S.2` -- a half-period between a tenth of a second and two seconds.
///
/// The bounds are the requirement's own, handed over by the test the
/// generator writes; the half-period is the model's, carried into
/// `Default`. Neither number is written here.
pub fn perceptible_period(lower_bound: u16, upper_bound: u16) {
    let half_period = app().half_period_millis;
    assert!(
        half_period >= lower_bound && half_period <= upper_bound,
        "a half-period of {half_period} ms is outside {lower_bound}..={upper_bound}"
    );
}

/// `S.3` -- lit and dark shall last the same.
pub fn equal_duty_cycle() {
    let mut app = app();
    let steps = sketch::run(&mut app, HALF_PERIODS);

    let mut lit = 0u32;
    let mut dark = 0u32;
    let mut since = 0u16;
    for step in &steps {
        let held = u32::from(step.at_millis - since);
        since = step.at_millis;
        // the step reports the state it has just entered, so the time
        // that has passed belongs to the state before it
        if step.lit {
            dark += held;
        } else {
            lit += held;
        }
    }
    // the first half-period runs dark before anything is lit, and the
    // last one is still running when the sketch stops, so one of each
    // is left out rather than counted crooked
    assert_eq!(lit, dark, "lit for {lit} ms against dark for {dark} ms");
}
