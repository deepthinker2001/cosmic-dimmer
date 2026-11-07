use std::process::ExitCode;

fn main() -> ExitCode {
    // Only these are taken as options: cosmic-panel starts applets with none,
    // and anything else is left to the applet as before.
    match std::env::args().nth(1).as_deref() {
        Some("--reset") => return cosmic_dimmer::reset_from_terminal(),
        Some("--help" | "-h") => {
            println!("{}", cosmic_dimmer::usage());
            return ExitCode::SUCCESS;
        }
        Some("--version" | "-V") => {
            println!("cosmic-dimmer {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        _ => {}
    }
    // Logging only for the applet: the options above talk to a person.
    tracing_subscriber::fmt::init();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        app_id = cosmic_dimmer::APP_ID,
        "starting dimmer applet",
    );
    match cosmic_dimmer::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("cosmic-dimmer: {error}");
            ExitCode::FAILURE
        }
    }
}
