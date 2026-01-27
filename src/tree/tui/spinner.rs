//! Animated spinner for loading indicators
//!
//! Provides a rotating spinner (◐◓◑◒) for displaying loading state.

/// Animated spinner with rotating frames
#[derive(Debug, Clone)]
pub struct Spinner {
    frames: &'static [char],
    tick: usize,
}

impl Spinner {
    /// Create a new spinner with circle animation frames
    pub fn new() -> Self {
        Spinner {
            frames: &['◐', '◓', '◑', '◒'],
            tick: 0,
        }
    }

    /// Advance to the next frame
    pub fn tick(&mut self) {
        self.tick = (self.tick + 1) % self.frames.len();
    }

    /// Get the current frame character
    pub fn frame(&self) -> char {
        self.frames[self.tick]
    }
}

impl Default for Spinner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spinner_cycles() {
        let mut spinner = Spinner::new();

        assert_eq!(spinner.frame(), '◐');
        spinner.tick();
        assert_eq!(spinner.frame(), '◓');
        spinner.tick();
        assert_eq!(spinner.frame(), '◑');
        spinner.tick();
        assert_eq!(spinner.frame(), '◒');
        spinner.tick();
        // Should wrap back to first frame
        assert_eq!(spinner.frame(), '◐');
    }
}
