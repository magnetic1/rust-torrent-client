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
use async_channel::{unbounded, Sender};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct Printer {
    stdout: Stdout,
    state_lines: u16,
    state: State,
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

pub enum State {
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
            let pm = receiver.recv().await?;
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

pub async fn print_log(log: String) -> Result<()> {
    QUEUE.send(PrintMessage::Log(log)).await?;
    Ok(())
}

pub(crate) async fn fresh_state(new_state: State) -> Result<()> {
    QUEUE.send(PrintMessage::State(new_state)).await?;
    Ok(())
}


#[cfg(test)]
mod test {
    use crate::base::terminal::{print_log, fresh_state, State};
    use std::time::Duration;
    use async_std::task;
    use async_std::task::JoinHandle;

    type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

    #[test]
    fn print_test() -> Result<()> {
        let mut j = task::spawn(async {
            fresh_state(State::Magenta("2█".to_string())).await
        });
        task::block_on(async move {
            print_log("123".to_string()).await?;
            print_log(456.to_string()).await?;
            task::sleep(Duration::from_micros(1000)).await;
            j.await
        })
    }
}