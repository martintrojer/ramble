use std::io::Write;
use std::process::ExitCode;

use clap::Parser;
use ramble::cli::{self, Args, Env, Plan};
use ramble::config::{self, Config};
use ramble::{app, doc, render};

fn fail(msg: impl std::fmt::Display, code: u8) -> ExitCode {
    eprintln!("{msg}");
    ExitCode::from(code)
}

fn main() -> ExitCode {
    let args = Args::parse();
    let mut env = Env::from_process();

    let config = if args.init_config {
        Config::default()
    } else {
        let path = args.config.clone().or_else(Config::default_path);
        match path.map(|p| Config::load(&p)).transpose() {
            Ok(c) => c.unwrap_or_default(),
            Err(e) => return fail(format!("ramble: {e:#}"), 1),
        }
    };

    match cli::plan(&args, &mut env, &config) {
        Plan::UsageError { msg, code } => fail(msg, code.clamp(1, 255) as u8),
        Plan::InitConfig(path) => {
            let Some(path) = path.or_else(Config::default_path) else {
                return fail(
                    "ramble: cannot find a config directory (set HOME or XDG_CONFIG_HOME)",
                    1,
                );
            };
            match config::write_default(&path) {
                Ok(()) => {
                    println!("{}", path.display());
                    ExitCode::SUCCESS
                }
                Err(e) => fail(format!("ramble: {e:#}"), 1),
            }
        }
        Plan::Print {
            bytes,
            label,
            width,
        } => {
            let Some(document) = doc::from_bytes(&bytes) else {
                return fail(format!("ramble: {label} looks binary"), 1);
            };
            let theme = render::Theme {
                math: config.render.math,
                ..render::Theme::catppuccin_mocha()
            };
            let page = render::render(&document, width, &theme);
            let mut out = std::io::stdout().lock();
            match out
                .write_all(render::to_ansi(&page).as_bytes())
                .and_then(|()| out.flush())
            {
                Ok(()) => ExitCode::SUCCESS,
                // A closed pipe (e.g. `| head`) is not an error worth reporting.
                Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => ExitCode::SUCCESS,
                Err(e) => fail(format!("ramble: {e}"), 1),
            }
        }
        Plan::Interactive(opts) => match app::run(opts) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(format!("ramble: {e:#}"), 1),
        },
    }
}
