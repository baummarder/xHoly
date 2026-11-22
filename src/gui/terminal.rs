//! Terminal setup/teardown for the ratatui backend (crossterm).

use crossterm::{
    cursor::Show,
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io::{self, Write};

/// Enter the alternate screen with raw mode + hidden cursor.
pub fn enter_alternate_screen() -> io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    crossterm::execute!(stdout, crossterm::cursor::Hide)?;
    Ok(())
}

/// Restore the terminal to its previous state.
pub fn leave_alternate_screen() -> io::Result<()> {
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen, Show)?;
    disable_raw_mode()?;
    Ok(())
}

/// Flush stdout (helper for crossterm queue! users).
pub fn flush() -> io::Result<()> {
    io::stdout().flush()
}
