#![allow(dead_code)]

use core::time::Duration;
use std::time::Instant;

use rand::seq::SliceRandom;

#[cfg(feature = "unicode")]
mod constants {
    pub static SUCCESS_MARKER: &'static str = "✔";
    pub static FAIL_MARKER: &'static str = "💩";
    pub static SPINNERS_CHOICES: &[&[&'static str]] = &[
        &["┤", "┘", "┴", "└", "├", "┌", "┬", "┐"],
        &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
        &["◰", "◳", "◲", "◱"],
        &["⢎⡰", "⢎⡡", "⢎⡑", "⢎⠱", "⠎⡱", "⢊⡱", "⢌⡱", "⢆⡱"],
        &[".", "o", "O", "°", "O", "o", "."],
        &["◡", "⊙", "◠"],
    ];
}

#[cfg(not(feature = "unicode"))]
mod constants {
    pub static SUCCESS_MARKER: &'static str = "+";
    pub static FAIL_MARKER: &'static str = "!";
    pub static SPINNERS_CHOICES: &[&[&'static str]] = &[
        &["x", "+"],
        &["|", "/", "-", "\\"],
        &[".", "o", "O", "@", "O", "o", "."],
    ];
}

pub use constants::*;

pub fn get_spinner() -> &'static [&'static str] {
    let mut rng = rand::thread_rng();
    SPINNERS_CHOICES.choose(&mut rng).unwrap()
}

/// Just a little container for a "spinning" sequence of strings that displays
/// progress.
pub struct Spinner<'a> {
    sequence: &'a [&'a str],
    idx: usize,
}

impl<'a> Spinner<'a> {
    pub fn new(sequence: &'a [&'a str]) -> Self {
        Self { sequence, idx: 0 }
    }

    pub fn get_and_advance(&mut self) -> &'a str {
        let c = self.get_current();
        self.step();
        c
    }

    pub fn get_next(&mut self) -> &'a str {
        self.step();
        self.get_current()
    }

    pub fn get_current(&mut self) -> &'a str {
        self.sequence[self.idx]
    }

    pub fn step(&mut self) {
        if self.idx == self.sequence.len() - 1 {
            self.idx = 0;
        } else {
            self.idx += 1;
        }
    }
}

/// TickSpinner is a spinner that will only change symbol on a given tick
/// schedule.
pub struct TickSpinner<'a> {
    wrapped: Spinner<'a>,
    rate: Duration,
    last_tick: Instant,
}

impl Default for TickSpinner<'static> {
    fn default() -> Self {
        Self::new_rand(Duration::from_millis(250))
    }
}

impl<'a> TickSpinner<'a> {
    pub fn new(sequence: &'a [&'a str], rate: Duration) -> Self {
        Self {
            wrapped: Spinner::new(sequence),
            rate,
            last_tick: Instant::now(),
        }
    }

    /// Get the current spinner string and advance only when a tick has
    /// occured.
    pub fn get(&mut self) -> &'a str {
        self.maybe_step();
        let c = self.wrapped.get_current();
        c
    }

    fn maybe_step(&mut self) {
        let now = Instant::now();

        let spread = now
            .checked_sub(self.last_tick.elapsed())
            .map(|f| f.elapsed())
            .unwrap_or_else(|| Duration::from_millis(0));

        if spread < self.rate {
            return;
        }
        self.last_tick = now;
        self.wrapped.step();
    }
}

impl TickSpinner<'static> {
    /// Get a new `TickSpinner` with a randomly selected sequence of strings.
    pub fn new_rand(rate: Duration) -> Self {
        Self::new(get_spinner(), rate)
    }
}

impl Spinner<'static> {
    pub fn new_rand() -> Self {
        let rand = get_spinner();
        Self::new(rand)
    }
}

/// A `Spinner` is also an infinite iterator.
impl<'a> Iterator for Spinner<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<Self::Item> {
        Some(self.get_and_advance())
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[test]
    fn test_tick_spinner() {
        let spinner_items = SPINNERS_CHOICES[0];
        let mut ticker = TickSpinner::new(spinner_items, Duration::from_millis(10));

        // Shouldn't advance
        let first = ticker.get();
        let second = ticker.get();
        assert_eq!(first, second);

        std::thread::sleep(Duration::from_millis(11));

        // Should advance
        let second = ticker.get();
        assert_ne!(first, second);
    }

    #[test]
    fn test_spinner() {
        let spinner_items = SPINNERS_CHOICES[0];
        let mut spinner = Spinner::new(spinner_items);
        let first = spinner.get_and_advance();
        for _ in 0..spinner_items.len() - 1 {
            let next = spinner.get_and_advance();
            assert_ne!(first, next);
        }
        let next = spinner.get_and_advance();
        assert_eq!(first, next);
    }
}
