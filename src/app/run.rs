//! The terminal and the event loop.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{self, Event};
use ratatui::DefaultTerminal;

use super::{App, AppEvent, StartOptions};
use crate::ui;

/// How long one loop iteration waits for terminal input.
const POLL: Duration = Duration::from_millis(100);

/// Run the interactive TUI until the user quits.
pub fn run(opts: StartOptions) -> anyhow::Result<()> {
    let size = crossterm::terminal::size().context("reading terminal size")?;
    let mut app = App::new(opts, size)?;
    // Installs a panic hook that restores the terminal before unwinding.
    let mut terminal = ratatui::try_init().context("initialising terminal")?;
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    if let Err(e) = app.start_watcher() {
        app.set_status(format!("live reload off: {e:#}"));
    }
    while !app.should_quit() {
        if app.pending_effect().is_some() {
            suspend_and_run(terminal, || app.run_pending_effect())?;
        }
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) => app.event(AppEvent::Key(key)),
                Event::Resize(cols, rows) => app.event(AppEvent::Resize(cols, rows)),
                _ => {}
            }
        }
        while let Ok(ev) = rx.try_recv() {
            app.event(ev);
        }
        app.pump_lsp(Duration::ZERO);
        app.tick(Instant::now());
        write_terminal_output(app, &mut std::io::stdout())?;
    }
    Ok(())
}

/// Write the bytes the app queued for the terminal (OSC 52) to `out`.
fn write_terminal_output(app: &mut App, out: &mut impl std::io::Write) -> std::io::Result<()> {
    let bytes = app.take_terminal_output();
    if !bytes.is_empty() {
        out.write_all(&bytes)?;
        out.flush()?;
    }
    Ok(())
}

/// Leave raw mode and the alternate screen, run `f` (which may use the
/// terminal), then re-enter and clear so the next draw repaints fully.
pub fn suspend_and_run<R>(
    terminal: &mut DefaultTerminal,
    f: impl FnOnce() -> R,
) -> anyhow::Result<R> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    let r = f();
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
    terminal.clear()?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{StartOptions, StartTarget};

    #[test]
    fn yank_reaches_the_terminal_as_osc52() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        std::fs::write(&path, "# A\n").unwrap();
        let mut app = App::new(
            StartOptions {
                target: StartTarget::File(path.clone()),
                tree_root: dir.path().to_path_buf(),
                config: crate::config::Config {
                    lsp: crate::config::LspConfig { server: vec![] },
                    ..Default::default()
                },
            },
            (40, 10),
        )
        .unwrap();
        app.yank();
        let mut out = Vec::new();
        write_terminal_output(&mut app, &mut out).unwrap();
        let abs = std::path::absolute(&path).unwrap();
        assert_eq!(
            out,
            crate::app::osc52(&abs.display().to_string()).into_bytes()
        );
        out.clear();
        write_terminal_output(&mut app, &mut out).unwrap();
        assert!(out.is_empty(), "queue drained");
    }
}
