//! The board as the sketch may touch it.
//!
//! `model/middleware.sysml` binds to this trait by path, so what
//! `sysml rustgen` writes calls these methods rather than inventing its
//! own. On real hardware an implementation would poke the ATmega's
//! registers; here [`Simulated`] keeps the pins in memory and lets a
//! clock be stepped by hand, so the example runs anywhere.

/// What the application is given to reach the board with.
pub trait Gpio {
    /// Drive one digital pin high or low.
    fn set_pin(&mut self, pin: u8, high: bool);
    /// Block for that many milliseconds.
    fn wait_millis(&mut self, millis: u16);
    /// Milliseconds since the board came up.
    fn uptime_millis(&self) -> u16;
}

/// A board that exists only in memory, so the sketch can be watched.
#[derive(Default)]
pub struct Simulated {
    /// High or low, for each of the fourteen digital pins.
    pub pins: [bool; 14],
    /// What `wait_millis` advances instead of sleeping.
    pub clock: u16,
    /// Every `set_pin`, in order, for a test to read back.
    pub written: Vec<(u8, bool)>,
}

impl Gpio for Simulated {
    fn set_pin(&mut self, pin: u8, high: bool) {
        if let Some(slot) = self.pins.get_mut(usize::from(pin)) {
            *slot = high;
        }
        self.written.push((pin, high));
    }

    fn wait_millis(&mut self, millis: u16) {
        self.clock = self.clock.wrapping_add(millis);
    }

    fn uptime_millis(&self) -> u16 {
        self.clock
    }
}
