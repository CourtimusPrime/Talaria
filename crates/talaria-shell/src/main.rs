//! Talaria — a lightweight, lightning-fast web browser for humans and agents.

mod app;
mod control;
mod gui;
mod keyutils;
mod tabs;
mod vault;

use std::error::Error;

use url::Url;
use winit::event_loop::EventLoop;

use app::{App, AppEvent};

const DEFAULT_URL: &str = "https://servo.org";

fn main() -> Result<(), Box<dyn Error>> {
    // No env_logger here: servo's `setup_logging()` installs the global
    // logger (and panics if one is already set); it honors RUST_LOG and
    // covers our own log:: macros too.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .expect("install crypto provider");

    let input = std::env::args().nth(1).unwrap_or_else(|| DEFAULT_URL.to_owned());
    let url = Url::parse(&input)
        .or_else(|_| Url::parse(&format!("https://{input}")))
        .unwrap_or_else(|_| app::resolve_location(&input));

    // Single instance: a running Talaria owns the control socket (and the
    // engine profile). Hand it the URL and exit rather than stealing the
    // socket path — the first instance would otherwise become unreachable
    // to agents the moment we quit.
    match control::forward_to_running_instance(url.as_str()) {
        Ok(true) => {
            eprintln!("Talaria is already running — opened {url} there.");
            return Ok(());
        },
        Ok(false) => {},
        Err(error) => {
            eprintln!("Talaria appears to be running but did not accept the URL ({error}); starting anyway.");
        },
    }

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    control::spawn(event_loop.create_proxy());
    let mut app = App::new(&event_loop, url);
    event_loop.run_app(&mut app)?;
    Ok(())
}
