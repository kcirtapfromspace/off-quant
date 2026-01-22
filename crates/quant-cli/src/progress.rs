//! Progress indicators and spinners for CLI feedback
//!
//! Provides visual feedback during long-running operations.

use std::io::{stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;

// ANSI escape codes
const CLEAR_LINE: &str = "\x1b[2K\r";
const HIDE_CURSOR: &str = "\x1b[?25l";
const SHOW_CURSOR: &str = "\x1b[?25h";
const CYAN: &str = "\x1b[96m";
const GREEN: &str = "\x1b[92m";
const YELLOW: &str = "\x1b[93m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// Spinner animation frames
const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Alternative ASCII spinner for terminals that don't support Unicode
const ASCII_SPINNER: &[&str] = &["|", "/", "-", "\\"];

/// A terminal spinner for showing progress
pub struct Spinner {
    message: String,
    is_running: Arc<AtomicBool>,
    handle: Option<tokio::task::JoinHandle<()>>,
    use_unicode: bool,
}

impl Spinner {
    /// Create a new spinner with a message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            is_running: Arc::new(AtomicBool::new(false)),
            handle: None,
            use_unicode: supports_unicode(),
        }
    }

    /// Start the spinner animation
    pub fn start(&mut self) {
        if self.is_running.load(Ordering::SeqCst) {
            return;
        }

        self.is_running.store(true, Ordering::SeqCst);

        let is_running = self.is_running.clone();
        let message = self.message.clone();
        let use_unicode = self.use_unicode;

        self.handle = Some(tokio::spawn(async move {
            let frames = if use_unicode {
                SPINNER_FRAMES
            } else {
                ASCII_SPINNER
            };

            let mut idx = 0;
            let mut tick = interval(Duration::from_millis(80));

            // Hide cursor during spinner
            print!("{}", HIDE_CURSOR);
            let _ = stdout().flush();

            while is_running.load(Ordering::SeqCst) {
                print!("{}{}{} {}{}", CLEAR_LINE, CYAN, frames[idx], message, RESET);
                let _ = stdout().flush();
                idx = (idx + 1) % frames.len();
                tick.tick().await;
            }

            // Show cursor again
            print!("{}{}", CLEAR_LINE, SHOW_CURSOR);
            let _ = stdout().flush();
        }));
    }

    /// Stop the spinner with a success message
    pub async fn stop_with_success(&mut self, message: impl Into<String>) {
        self.stop().await;
        let checkmark = if self.use_unicode { "✓" } else { "+" };
        println!("{}{} {}{}", GREEN, checkmark, message.into(), RESET);
    }

    /// Stop the spinner with a warning message
    pub async fn stop_with_warning(&mut self, message: impl Into<String>) {
        self.stop().await;
        let warn = if self.use_unicode { "⚠" } else { "!" };
        println!("{}{} {}{}", YELLOW, warn, message.into(), RESET);
    }

    /// Stop the spinner with an error message
    pub async fn stop_with_error(&mut self, message: impl Into<String>) {
        self.stop().await;
        let x = if self.use_unicode { "✗" } else { "x" };
        println!("\x1b[91m{} {}\x1b[0m", x, message.into());
    }

    /// Stop the spinner silently
    pub async fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
    }

    /// Update the spinner message
    pub fn set_message(&mut self, message: impl Into<String>) {
        self.message = message.into();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        // Make sure cursor is visible
        print!("{}", SHOW_CURSOR);
        let _ = stdout().flush();
    }
}

/// Progress bar for operations with known progress
pub struct ProgressBar {
    total: usize,
    current: usize,
    message: String,
    width: usize,
    use_unicode: bool,
}

impl ProgressBar {
    /// Create a new progress bar
    pub fn new(total: usize, message: impl Into<String>) -> Self {
        Self {
            total,
            current: 0,
            message: message.into(),
            width: 30,
            use_unicode: supports_unicode(),
        }
    }

    /// Set progress bar width
    pub fn with_width(mut self, width: usize) -> Self {
        self.width = width;
        self
    }

    /// Update progress
    pub fn update(&mut self, current: usize) {
        self.current = current.min(self.total);
        self.render();
    }

    /// Increment progress by 1
    pub fn increment(&mut self) {
        self.update(self.current + 1);
    }

    /// Render the progress bar
    fn render(&self) {
        let percent = if self.total > 0 {
            (self.current as f64 / self.total as f64 * 100.0) as usize
        } else {
            0
        };

        let filled = if self.total > 0 {
            self.width * self.current / self.total
        } else {
            0
        };
        let empty = self.width - filled;

        let (fill_char, empty_char) = if self.use_unicode {
            ("█", "░")
        } else {
            ("#", "-")
        };

        let bar: String = fill_char.repeat(filled) + &empty_char.repeat(empty);

        print!(
            "{}{}{} [{}{}{}{}{}/{} ({:3}%)]{}",
            CLEAR_LINE,
            DIM,
            self.message,
            CYAN,
            bar,
            RESET,
            DIM,
            self.current,
            self.total,
            percent,
            RESET
        );
        let _ = stdout().flush();
    }

    /// Finish the progress bar
    pub fn finish(&self) {
        println!();
    }

    /// Finish with a message
    pub fn finish_with_message(&self, message: impl Into<String>) {
        let checkmark = if self.use_unicode { "✓" } else { "+" };
        print!("{}", CLEAR_LINE);
        println!("{}{} {}{}", GREEN, checkmark, message.into(), RESET);
    }
}

/// Status indicator for showing operation status
pub struct StatusLine {
    use_unicode: bool,
}

impl StatusLine {
    pub fn new() -> Self {
        Self {
            use_unicode: supports_unicode(),
        }
    }

    /// Show a status message
    pub fn status(&self, message: impl Into<String>) {
        let arrow = if self.use_unicode { "→" } else { ">" };
        println!("{}{} {}{}", DIM, arrow, message.into(), RESET);
    }

    /// Show an info message
    pub fn info(&self, message: impl Into<String>) {
        let info = if self.use_unicode { "ℹ" } else { "i" };
        println!("{}{} {}{}", CYAN, info, message.into(), RESET);
    }

    /// Show a success message
    pub fn success(&self, message: impl Into<String>) {
        let check = if self.use_unicode { "✓" } else { "+" };
        println!("{}{} {}{}", GREEN, check, message.into(), RESET);
    }

    /// Show a warning message
    pub fn warning(&self, message: impl Into<String>) {
        let warn = if self.use_unicode { "⚠" } else { "!" };
        println!("{}{} {}{}", YELLOW, warn, message.into(), RESET);
    }

    /// Show an error message
    pub fn error(&self, message: impl Into<String>) {
        let x = if self.use_unicode { "✗" } else { "x" };
        println!("\x1b[91m{} {}\x1b[0m", x, message.into());
    }

    /// Show a step in a multi-step process
    pub fn step(&self, current: usize, total: usize, message: impl Into<String>) {
        println!(
            "{}[{}/{}]{} {}",
            DIM,
            current,
            total,
            RESET,
            message.into()
        );
    }
}

impl Default for StatusLine {
    fn default() -> Self {
        Self::new()
    }
}

/// Check if the terminal likely supports Unicode
fn supports_unicode() -> bool {
    // Check for common Unicode-supporting terminals
    if let Ok(term) = std::env::var("TERM") {
        if term.contains("xterm") || term.contains("256color") || term.contains("kitty") {
            return true;
        }
    }

    // Check for LC_ALL, LC_CTYPE, or LANG containing UTF-8
    for var in &["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(val) = std::env::var(var) {
            if val.to_lowercase().contains("utf") {
                return true;
            }
        }
    }

    // Default to false for safety
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_supports_unicode() {
        // This just tests that the function runs without panicking
        let _ = supports_unicode();
    }

    #[test]
    fn test_progress_bar_creation() {
        let bar = ProgressBar::new(100, "Testing");
        assert_eq!(bar.total, 100);
        assert_eq!(bar.current, 0);
    }

    #[test]
    fn test_progress_bar_update() {
        let mut bar = ProgressBar::new(100, "Testing");
        bar.update(50);
        assert_eq!(bar.current, 50);

        // Test clamping
        bar.update(150);
        assert_eq!(bar.current, 100);
    }

    #[test]
    fn test_status_line_creation() {
        let _status = StatusLine::new();
    }

    #[tokio::test]
    async fn test_spinner_basic() {
        let mut spinner = Spinner::new("Testing");
        spinner.start();
        tokio::time::sleep(Duration::from_millis(100)).await;
        spinner.stop().await;
    }
}
