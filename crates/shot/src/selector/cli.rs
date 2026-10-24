use std::io::{self, Write};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};

pub(crate) fn select(names: &[String]) -> io::Result<Option<usize>> {
    if names.is_empty() {
        return Ok(None);
    }
    let mut stdout = io::stdout();
    println!("Select a monitor (↑/↓ or h/j/k/l, Enter to capture, Esc to cancel):");
    terminal::enable_raw_mode()?;
    let result = (|| {
        execute!(stdout, cursor::Hide)?;
        execute!(stdout, cursor::SavePosition)?;
        let mut selected = 0;

        loop {
            for (monitor, name) in names.iter().enumerate() {
                execute!(
                    stdout,
                    cursor::MoveToColumn(0),
                    terminal::Clear(ClearType::CurrentLine)
                )?;

                if monitor == selected {
                    writeln!(stdout, "> {name}")?;
                } else {
                    writeln!(stdout, "  {name}")?;
                }
            }

            stdout.flush()?;

            if let Event::Key(key) = event::read()?
                && key.kind.is_press()
            {
                match key.code {
                    KeyCode::Up | KeyCode::Left | KeyCode::Char('k') | KeyCode::Char('h') => {
                        selected = previous(selected, names.len());
                    }
                    KeyCode::Down | KeyCode::Right | KeyCode::Char('j') | KeyCode::Char('l') => {
                        selected = next(selected, names.len());
                    }
                    KeyCode::Enter => break Ok(Some(selected)),
                    KeyCode::Esc | KeyCode::Char('q') => break Ok(None),
                    _ => continue,
                }

                execute!(stdout, cursor::MoveUp(names.len() as u16))?;
            }
        }
    })();
    let cleanup =
        terminal::disable_raw_mode().and(execute!(stdout, cursor::Show, cursor::MoveToColumn(0)));
    match cleanup {
        Ok(()) => result,
        Err(error) => Err(error),
    }
}

fn previous(selected: usize, count: usize) -> usize {
    selected.checked_sub(1).unwrap_or(count - 1)
}

fn next(selected: usize, count: usize) -> usize {
    (selected + 1) % count
}
