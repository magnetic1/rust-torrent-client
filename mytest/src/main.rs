use std::io::{Write, stdout};
use crossterm::{
    ExecutableCommand, QueueableCommand,
    terminal, cursor, style::{self, Colorize}, Result,
};
use std::thread::Thread;
use std::thread;
use std::time::Duration;
use crossterm::style::{Styler, Print};


fn main() -> Result<()> {
    let mut stdout = stdout();

    stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;

    // in this loop we are more efficient by not flushing the buffer.
    stdout.execute(Print("123"))?;
    thread::sleep(Duration::from_millis(1000));
    stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
    stdout.execute(style::PrintStyledContent("1█\n".magenta()))?;

    Ok(())
}
