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
    push_title();
    // Installs a panic hook that restores the terminal before unwinding.
    let mut terminal = ratatui::try_init().context("initialising terminal")?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        pop_title();
        hook(info);
    }));
    let result = event_loop(&mut terminal, &mut app);
    ratatui::restore();
    pop_title();
    result
}

/// XTWINOPS: save the terminal title on the terminal's title stack.
const PUSH_TITLE: &str = "\x1b[22;0t";
/// XTWINOPS: restore the title saved by [`PUSH_TITLE`].
const POP_TITLE: &str = "\x1b[23;0t";

fn push_title() {
    let _ = write_flush(&mut std::io::stdout(), PUSH_TITLE.as_bytes());
}

fn pop_title() {
    let _ = write_flush(&mut std::io::stdout(), POP_TITLE.as_bytes());
}

fn write_flush(out: &mut impl std::io::Write, bytes: &[u8]) -> std::io::Result<()> {
    out.write_all(bytes)?;
    out.flush()
}

/// The title to send, if `next` differs from the one last sent.
fn title_change<'a>(last: Option<&str>, next: &'a str) -> Option<&'a str> {
    (last != Some(next)).then_some(next)
}

/// Set the terminal title to the app's when it changed since `last`.
fn update_title(app: &App, last: &mut Option<String>) -> std::io::Result<()> {
    let next = app.title_text();
    if let Some(t) = title_change(last.as_deref(), &next) {
        crossterm::execute!(std::io::stdout(), crossterm::terminal::SetTitle(t))?;
        *last = Some(next);
    }
    Ok(())
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    if let Err(e) = app.start_watcher() {
        app.set_status(format!("live reload off: {e:#}"));
    }
    app.start_review();
    let mut title = None;
    while !app.should_quit() {
        update_title(app, &mut title)?;
        if app.pending_effect().is_some() {
            suspend_and_run(terminal, || app.run_pending_effect())?;
            title = None; // the command may have set its own title
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
    fn title_is_sent_only_when_it_changes() {
        assert_eq!(title_change(None, "ramble"), Some("ramble"));
        assert_eq!(title_change(Some("ramble"), "ramble"), None);
        assert_eq!(
            title_change(Some("ramble"), "ramble — a.md"),
            Some("ramble — a.md")
        );
    }

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
