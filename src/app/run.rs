//! The terminal and the event loop.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Context;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event,
};
use ratatui::DefaultTerminal;

use super::{App, AppEvent, StartOptions};
use crate::ui;

/// How long one loop iteration waits for terminal input.
const POLL: Duration = Duration::from_millis(100);

/// Run the interactive TUI until the user quits.
pub fn run(opts: StartOptions) -> anyhow::Result<()> {
    let size = crossterm::terminal::size().context("reading terminal size")?;
    let mouse = opts.config.mouse.enabled;
    let mut app = App::new(opts, size)?;
    push_title();
    // Installs a panic hook that restores the terminal before unwinding.
    let mut terminal = ratatui::try_init().context("initialising terminal")?;
    term_cmds(&term_setup(mouse))?;
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = term_cmds(&term_teardown(mouse));
        pop_title();
        hook(info);
    }));
    let result = event_loop(&mut terminal, &mut app, mouse);
    let _ = term_cmds(&term_teardown(mouse));
    ratatui::restore();
    pop_title();
    result
}

/// A terminal mode change made around the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TermCmd {
    EnableMouse,
    DisableMouse,
    /// Bracketed paste: a paste arrives as one [`Event::Paste`], not as
    /// keys (a pasted newline would be Enter).
    EnablePaste,
    DisablePaste,
}

/// What entering the TUI (or coming back from a launched program) sends:
/// bracketed paste, and mouse capture when `mouse` (`[mouse] enabled`).
pub fn term_setup(mouse: bool) -> Vec<TermCmd> {
    let mut cmds = vec![TermCmd::EnablePaste];
    if mouse {
        cmds.push(TermCmd::EnableMouse);
    }
    cmds
}

/// What leaving the TUI (exit, panic, launching a program) sends.
pub fn term_teardown(mouse: bool) -> Vec<TermCmd> {
    let mut cmds = vec![TermCmd::DisablePaste];
    if mouse {
        cmds.push(TermCmd::DisableMouse);
    }
    cmds
}

fn term_cmds(cmds: &[TermCmd]) -> std::io::Result<()> {
    let mut out = std::io::stdout();
    for c in cmds {
        match c {
            TermCmd::EnableMouse => crossterm::execute!(out, EnableMouseCapture)?,
            TermCmd::DisableMouse => crossterm::execute!(out, DisableMouseCapture)?,
            TermCmd::EnablePaste => crossterm::execute!(out, EnableBracketedPaste)?,
            TermCmd::DisablePaste => crossterm::execute!(out, DisableBracketedPaste)?,
        }
    }
    Ok(())
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

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App, mouse: bool) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::channel::<AppEvent>();
    app.set_sender(tx);
    if let Err(e) = app.start_watcher() {
        app.set_status(format!("live reload off: {e:#}"));
    }
    let mut title = None;
    while !app.should_quit() {
        update_title(app, &mut title)?;
        if app.pending_effect().is_some() {
            suspend_and_run(terminal, mouse, || app.run_pending_effect())?;
            title = None; // the command may have set its own title
        }
        if app.take_clear_request() {
            terminal.clear()?;
        }
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(POLL)? {
            match event::read()? {
                Event::Key(key) => app.event(AppEvent::Key(key)),
                Event::Resize(cols, rows) => app.event(AppEvent::Resize(cols, rows)),
                Event::Mouse(m) => app.event(AppEvent::Mouse(m, Instant::now())),
                Event::Paste(text) => app.event(AppEvent::Paste(text)),
                _ => {}
            }
        }
        while let Ok(ev) = rx.try_recv() {
            app.event(ev);
        }
        app.pump_lsp(Duration::ZERO);
        app.pump_mdroots(Duration::ZERO);
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

/// Leave raw mode, bracketed paste, mouse capture (when `mouse`) and the
/// alternate screen, run `f` (which may use the terminal), then re-enter
/// and clear so the next draw repaints fully.
pub fn suspend_and_run<R>(
    terminal: &mut DefaultTerminal,
    mouse: bool,
    f: impl FnOnce() -> R,
) -> anyhow::Result<R> {
    term_cmds(&term_teardown(mouse))?;
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen)?;
    let r = f();
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen)?;
    crossterm::terminal::enable_raw_mode()?;
    term_cmds(&term_setup(mouse))?;
    terminal.clear()?;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{StartOptions, StartTarget};

    #[test]
    fn mouse_capture_follows_the_config_and_paste_is_always_bracketed() {
        assert_eq!(
            term_setup(true),
            [TermCmd::EnablePaste, TermCmd::EnableMouse]
        );
        assert_eq!(
            term_teardown(true),
            [TermCmd::DisablePaste, TermCmd::DisableMouse]
        );
        assert_eq!(term_setup(false), [TermCmd::EnablePaste]);
        assert_eq!(term_teardown(false), [TermCmd::DisablePaste]);
    }

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
                review_cache: None,
                mdroots: crate::app::MdrootsOptions::memory(),
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
