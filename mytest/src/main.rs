use crossterm::style::Print;
use crossterm::{
    cursor,
    style::{self, Colorize},
    terminal, ExecutableCommand, Result,
};
use std::io::{stdout, Stdout};
use std::thread;
use std::time::Duration;

fn main() -> Result<()> {
    let stdout = stdout();
    let mut printer = Printer::new(stdout);

    printer.print_log("------start------")?;

    thread::sleep(Duration::from_millis(1500));
    printer.fresh_state(State::Magenta("1█".to_string()))?;
    for i in 0..20 {
        thread::sleep(Duration::from_millis(500));
        printer.print_log(&*format!("{}", i))?;
    }
    printer.fresh_state(State::Magenta("2█".to_string()))?;

    Ok(())
}

struct Printer {
    stdout: Stdout,
    state_lines: u16,
    state: State,
}

enum State {
    Print(String),
    Magenta(String),
}

impl State {
    fn print_state(&self, stdout: &mut Stdout) -> Result<()> {
        match self {
            State::Print(t) => {
                stdout.execute(Print(format!("{}", t)))?;
            }
            State::Magenta(t) => {
                stdout.execute(style::PrintStyledContent(t.clone().magenta()))?;
            }
        }

        Ok(())
    }
}

impl Printer {
    fn new(stdout: Stdout) -> Printer {
        Printer {
            stdout,
            state_lines: 1,
            state: State::Print("".to_string()),
        }
    }

    fn print_log(&mut self, log: &str) -> Result<()> {
        self.clear_state()?;

        self.stdout.execute(Print(format!("{}\n", log)))?;
        self.state.print_state(&mut self.stdout)?;

        Ok(())
    }

    fn fresh_state(&mut self, new_state: State) -> Result<()> {
        self.clear_state()?;
        self.state = new_state;
        self.state.print_state(&mut self.stdout)?;

        Ok(())
    }

    fn clear_state(&mut self) -> Result<()> {
        let log_cursor = cursor::position()?.1 - self.state_lines + 1;
        self.stdout.execute(cursor::MoveTo(0, log_cursor))?;
        self.stdout
            .execute(terminal::Clear(terminal::ClearType::FromCursorDown))?;
        Ok(())
    }
}
