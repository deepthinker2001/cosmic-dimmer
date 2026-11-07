//! Approval-gated live check of the alpha backend on the running compositor.
//!
//! Without `--live-approved` it connects to nothing. With it, it starts the
//! real [`OverlayBackend`], dims every output to `--level` (retained percent;
//! default 100, i.e. no surface at all), prints every change it sees (outputs
//! appearing and leaving, levels applied, surfaces, the connection) and exits
//! after `--seconds` of awake time (a watchdog thread, independent of Wayland).
//! Exiting closes the connection, so the compositor removes every surface.
//!
//! Never captures screen contents or input, never binds the lock screen, and
//! prints no EDID identities (they carry serial numbers): only output names
//! and whether the EDID was matched.
//!
//! ```sh
//! cargo run -p dimmer-overlay --example overlay_live --locked -- \
//!     --live-approved --level 70 --seconds 90
//! ```

use std::time::{Duration, Instant};

use dimmer_core::{Backend, Level, with_timeout};
use dimmer_overlay::{OverlayBackend, Snapshot, Status};

const MAX_SECONDS: u64 = 600;

#[derive(Debug, PartialEq)]
struct Options {
    live: bool,
    level: u8,
    seconds: u64,
}

fn parse(args: impl IntoIterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        live: false,
        level: 100,
        seconds: 60,
    };
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--live-approved" => options.live = true,
            "--level" => {
                options.level = value("--level")?
                    .parse()
                    .map_err(|_| "--level must be a whole percent")?;
            }
            "--seconds" => {
                options.seconds = value("--seconds")?
                    .parse()
                    .map_err(|_| "--seconds must be a whole number")?;
            }
            _ => return Err(format!("unknown option: {arg}")),
        }
    }
    if !(dimmer_overlay::ALPHA_FLOOR..=100).contains(&options.level) {
        return Err(format!(
            "--level must be {}..=100 (retained brightness)",
            dimmer_overlay::ALPHA_FLOOR
        ));
    }
    if !(1..=MAX_SECONDS).contains(&options.seconds) {
        return Err(format!("--seconds must be 1..={MAX_SECONDS}"));
    }
    Ok(options)
}

fn describe(snapshot: &Snapshot) -> String {
    let status = match &snapshot.status {
        Status::Connecting => "connecting".to_owned(),
        Status::Connected => "connected".to_owned(),
        Status::Unavailable(reason) => format!("unavailable ({reason})"),
    };
    let outputs: Vec<String> = snapshot
        .outputs
        .iter()
        .map(|view| {
            let matched = if view.id.0.contains(':') {
                "no EDID match"
            } else {
                "EDID matched"
            };
            format!(
                "{} [{matched}, level {}%{}]",
                view.name,
                view.level.percent(),
                if view.writable { "" } else { ", closed" }
            )
        })
        .collect();
    format!(
        "{status}; surfaces {}; outputs: {}",
        snapshot.surfaces,
        if outputs.is_empty() {
            "none".to_owned()
        } else {
            outputs.join(", ")
        }
    )
}

fn main() {
    let options = match parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    };
    if !options.live {
        println!(
            "No connection made. A live run needs the user's approval first, then \
             --live-approved.\nDefaults: --level 100 (no surface), --seconds 60 (max {MAX_SECONDS})."
        );
        return;
    }
    // Independent of Wayland and of the runtime: process exit closes the
    // socket and the compositor removes every surface. thread::sleep counts
    // CLOCK_MONOTONIC, which stops during suspend: after a sleep/wake the
    // remaining awake time still runs, so the result can be observed.
    let seconds = options.seconds;
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(seconds));
        println!(
            "watchdog: {seconds} s of awake time elapsed; exiting (surfaces go with the connection)"
        );
        std::process::exit(0);
    });
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .try_init();

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("runtime: {error}");
            std::process::exit(1);
        }
    };
    runtime.block_on(run(&options));
}

async fn run(options: &Options) {
    let start = Instant::now();
    let stamp = || format!("[{:>6.1} s]", start.elapsed().as_secs_f32());
    let Ok(level) = Level::new(options.level) else {
        return;
    };
    let backend = OverlayBackend::spawn();
    let limit = Duration::from_secs(5);
    match with_timeout(limit, backend.outputs()).await {
        Ok(outputs) => {
            for output in outputs {
                let result = with_timeout(limit, backend.set_level(&output.id, level)).await;
                println!(
                    "{} set {}% on {}: {result:?}",
                    stamp(),
                    options.level,
                    output.name
                );
            }
        }
        Err(error) => println!("{} outputs: {error}", stamp()),
    }
    let mut watch = backend.watch();
    let mut last = String::new();
    loop {
        let snapshot = watch.borrow_and_update().clone();
        let line = describe(&snapshot);
        if line != last {
            println!("{} {line}", stamp());
            last = line;
        }
        // An output whose identity is new to this run (a different monitor,
        // or one first seen now) starts undimmed: dim it too. A returning
        // monitor keeps its level without help; that is what is being tested.
        if snapshot.status == Status::Connected {
            for view in &snapshot.outputs {
                if view.level == Level::FULL && level != Level::FULL {
                    let result = with_timeout(limit, backend.set_level(&view.id, level)).await;
                    println!(
                        "{} set {}% on new output {}: {result:?}",
                        stamp(),
                        options.level,
                        view.name
                    );
                }
            }
        }
        if watch.changed().await.is_err() {
            println!("{} overlay thread stopped", stamp());
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Result<Options, String> {
        parse(list.iter().map(|arg| (*arg).to_owned()))
    }

    #[test]
    fn defaults_connect_to_nothing_and_draw_nothing() {
        assert_eq!(
            args(&[]),
            Ok(Options {
                live: false,
                level: 100,
                seconds: 60
            })
        );
    }

    #[test]
    fn level_and_duration_are_bounded() {
        for bad in ["0", "9", "101", "-1", "x"] {
            assert!(args(&["--level", bad]).is_err());
        }
        for bad in ["0", "601", "x"] {
            assert!(args(&["--seconds", bad]).is_err());
        }
        assert!(args(&["--level"]).is_err());
        assert!(args(&["--wat"]).is_err());
        assert_eq!(
            args(&["--live-approved", "--level", "10"]).map(|o| o.level),
            Ok(10)
        );
    }
}
