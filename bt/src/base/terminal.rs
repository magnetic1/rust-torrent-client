use crossterm::style::Print;
use crossterm::{
    cursor,
    style::{self, Colorize},
    terminal, ExecutableCommand,
};
use std::io::{stdout, Stdout};
use std::thread;
use std::time::Duration;
use crate::base::spawn_and_log_error;
use once_cell::sync::Lazy;
use crossbeam::channel::{unbounded, Sender};


type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

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

enum PrintMessage {
    Log(String),
    State(State),
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

// static mut SENDER: Option<Sender<PrintMessage>> = None;
// static mut JOIN_HANDLE: Option<task::JoinHandle<()>> = None;
static QUEUE: Lazy<Sender<PrintMessage>> = Lazy::new(|| {
    let (mut sender, receiver) = unbounded();

    let _j = spawn_and_log_error(async move {
        let stdout = stdout();
        let mut printer = Printer::new(stdout);
        loop {
            let pm = receiver.recv()?;
            match pm {
                PrintMessage::Log(s) => {
                    printer.print_log(&s)?;
                }
                PrintMessage::State(state) => {
                    printer.fresh_state(state)?;
                }
            }
        }
    });
    sender
});

fn print(log: String) -> Result<()> {
    QUEUE.send(PrintMessage::Log(log))?;
    Ok(())
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

#[cfg(test)]
mod test {
    use crate::base::terminal::print;
    use std::time::Duration;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

    #[test]
    fn print_test() -> Result<()> {
        print("123".to_string())?;
        print(456.to_string())?;
        std::thread::sleep(Duration::from_micros(1000));
        Ok(())
    }
}