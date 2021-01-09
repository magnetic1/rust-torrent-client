use std::io::{Write, stdout, Stdout};
use crossterm::{ExecutableCommand, QueueableCommand, terminal, cursor, style::{self, Colorize}, Result, Command};
use std::thread::Thread;
use std::{thread, fmt, result};
use std::time::Duration;
use crossterm::style::{Styler, Print};
use std::fmt::Display;
use crate::State::Magenta;


fn main() -> Result<()> {
    let mut stdout = stdout();


    // in this loop we are more efficient by not flushing the buffer.
    // stdout.execute(Print("----------start---------\n"))?;
    //
    // let state = style::PrintStyledContent("1█".magenta());
    // stdout.execute(state)?;
    // let (_, y) = cursor::position()?;
    // for i in 0..5 {
    //     thread::sleep(Duration::from_millis(500));
    //
    //     stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    //     stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
    //     stdout.execute(Print(format!("{}\n", i)));
    //     stdout.execute(state)?;
    // }
    //
    // stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
    // stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
    // stdout.execute(style::PrintStyledContent("2█".magenta()))?;

    // let mut y = cursor::position()?.1;
    // y = print_log(&mut stdout, y, "------start------", Print(""))?;
    // let mut state = style::PrintStyledContent("1█".magenta());
    // fresh_state(&mut stdout, y, state)?;
    // for i in 0..5 {
    //     thread::sleep(Duration::from_millis(500));
    //     y = print_log(&mut stdout, y, &*format!("{}", i), state)?;
    // }
    // state = style::PrintStyledContent("2█".magenta());
    // fresh_state(&mut stdout, y, state)?;

    let mut printer = Printer::new(stdout);

    printer.print_log("------start------");

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
    log_lines: u16,
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
            log_lines: cursor::position().unwrap().1,
            state: State::Print("".to_string()),
        }
    }

    fn print_log(&mut self, log: &str) -> Result<()> {
        let mut now = cursor::position()?.1;

        while now >= self.log_lines {
            self.stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
            now = now - 1;
        }
        self.stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
        self.stdout.execute(Print(format!("{}\n", log)))?;

        self.log_lines = cursor::position()?.1;
        self.state.print_state(&mut self.stdout)?;

        Ok(())
    }

    fn fresh_state(&mut self, new_state: State) -> Result<()> {
        self.state = new_state;

        let mut now = cursor::position()?.1;
        while now >= self.log_lines {
            self.stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
            now = now - 1;
        }

        self.stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
        self.state.print_state(&mut self.stdout)?;

        Ok(())
    }
}

fn print_log(stdout: &mut Stdout, y: u16, log: &str, state: impl Command) -> Result<u16> {
    let mut now = cursor::position()?.1;

    while now >= y {
        stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
        now = now - 1;
    }
    stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
    stdout.execute(Print(format!("{}\n", log)))?;
    let new_y = cursor::position()?.1;
    stdout.execute(state)?;

    Ok(new_y)
}

fn fresh_state(stdout: &mut Stdout, y: u16, new_state: impl Command) -> Result<()> {
    let mut now = cursor::position()?.1;

    while now >= y {
        stdout.execute(terminal::Clear(terminal::ClearType::CurrentLine))?;
        now = now - 1;
    }
    stdout.execute(cursor::MoveTo(0, cursor::position()?.1))?;
    stdout.execute(new_state)?;

    Ok(())
}