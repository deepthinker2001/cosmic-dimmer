//! `dimmer-samsung`: wake and power the Samsung TV over the network.
//! See `docs/network-findings.md` for what each command does to the TV.

use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::process::ExitCode;
use std::thread::sleep;
use std::time::{Duration, Instant};

use dimmer_samsung::config::{self, Config};
use dimmer_samsung::info::{self, DeviceInfo};
use dimmer_samsung::pin::{self, Policy};
use dimmer_samsung::remote::{self, KeyAction, POWER_OFF_HOLD};
use dimmer_samsung::{Error, IO_TIMEOUT, Result, validate_host, wol};

const USAGE: &str = "\
dimmer-samsung: wake and power a Samsung Tizen TV over the local network

USAGE:
  dimmer-samsung setup <host>     Read the TV's device info (passive, no prompt)
                                  and save its address and Wake-on-LAN MAC
  dimmer-samsung info             Show the TV's power state and model (passive)
  dimmer-samsung pair [--repin]   Ask the TV for a token: the TV shows an
                                  Allow/Deny prompt that someone must accept
  dimmer-samsung wake [--timeout SECS]
                                  Wake-on-LAN, then wait for the TV to be on;
                                  does nothing if it's already on
  dimmer-samsung check            Check that the saved token and pinned
                                  certificate are accepted; sends no key
  dimmer-samsung power-key        Press the remote's power key once (a toggle:
                                  on the tested Frame, picture <-> screen-off
                                  standby; on others, TV <-> Art Mode)
  dimmer-samsung off              Hold the power key (full power-off); does
                                  nothing unless the TV reports it's on
  dimmer-samsung forget           Delete the saved token and certificate pin

The config (with the token) is $XDG_CONFIG_HOME/samsung-dimmer/samsung-tv.json,
mode 600. The token is never printed.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.as_slice() {
        ["setup", host] => setup(host),
        ["info"] => with_config(|path, c| show_info(path, &c)),
        ["pair"] => with_config(|path, c| pair(path, c, false)),
        ["pair", "--repin"] => with_config(|path, c| pair(path, c, true)),
        ["wake"] => with_config(|path, c| wake(path, c, Duration::from_secs(30))),
        ["wake", "--timeout", secs] => match secs.parse::<u64>() {
            Ok(s) if (1..=300).contains(&s) => {
                with_config(|path, c| wake(path, c, Duration::from_secs(s)))
            }
            _ => return usage("--timeout takes 1 to 300 seconds"),
        },
        ["check"] => with_config(check),
        ["power-key"] => with_config(power_key),
        ["off"] => with_config(off),
        ["forget"] => with_config(forget),
        ["help" | "--help" | "-h"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => return usage("unknown command"),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("dimmer-samsung: {e}");
            ExitCode::FAILURE
        }
    }
}

fn usage(why: &str) -> ExitCode {
    eprintln!("dimmer-samsung: {why}\n\n{USAGE}");
    ExitCode::from(2)
}

fn with_config(f: impl FnOnce(&Path, Config) -> Result<()>) -> Result<()> {
    let path = config::default_path()?;
    let config = Config::load(&path)?.ok_or_else(|| {
        Error::Config("not set up yet: run `dimmer-samsung setup <tv address>` first".into())
    })?;
    f(&path, config)
}

fn describe(info: &DeviceInfo) {
    let or = |v: &Option<String>| v.clone().unwrap_or_else(|| "unknown".into());
    println!("Model:         {}", or(&info.model_name));
    println!("OS:            {}", or(&info.os));
    println!("Network:       {}", or(&info.network_type));
    println!("Power state:   {}", or(&info.power_state));
    println!(
        "Token pairing: {}",
        if info.token_auth_support {
            "supported"
        } else {
            "not reported"
        }
    );
}

fn setup(host: &str) -> Result<()> {
    validate_host(host)?;
    let path = config::default_path()?;
    let info = info::fetch(host, IO_TIMEOUT)?;
    let mut config = Config::load(&path)?.unwrap_or_default();
    if config.host != host {
        // A different TV (or address): its token and certificate don't apply.
        config = Config {
            host: host.to_owned(),
            client_name: config::DEFAULT_CLIENT_NAME.into(),
            ..Config::default()
        };
    }
    if info.mac.is_some() {
        config.mac = info.mac;
    }
    config.save(&path)?;
    describe(&info);
    println!(
        "Wake-on-LAN:   {}",
        if config.mac.is_some() {
            "MAC saved"
        } else {
            "the TV didn't report a MAC; wake won't work"
        }
    );
    println!("Saved to {}", path.display());
    Ok(())
}

fn show_info(path: &Path, config: &Config) -> Result<()> {
    match info::fetch(&config.host, IO_TIMEOUT) {
        Ok(info) => describe(&info),
        Err(Error::Timeout | Error::Io(_)) => {
            println!("The TV isn't answering on port 8001 (off, or asleep without network).");
        }
        Err(e) => return Err(e),
    }
    println!(
        "Paired:        {}",
        if config.token.is_some() { "yes" } else { "no" }
    );
    println!("Config:        {}", path.display());
    Ok(())
}

fn policy_of(config: &Config) -> Result<Policy> {
    config
        .cert_sha256
        .as_deref()
        .and_then(pin::from_hex)
        .map(Policy::Pinned)
        .ok_or_else(|| Error::Config("not paired yet: run `dimmer-samsung pair`".into()))
}

fn pair(path: &Path, mut config: Config, repin: bool) -> Result<()> {
    let policy = match (repin, policy_of(&config)) {
        (false, Ok(pinned)) => pinned,
        _ => Policy::TrustOnFirstUse,
    };
    let url = remote::channel_url(&config.host, &config.client_name, None)?;
    println!(
        "Look at the TV: it will ask whether to allow \"{}\". Choose Allow.",
        config.client_name
    );
    let (mut channel, fingerprint) =
        remote::connect_tls(&config.host, &url, policy, Duration::from_secs(10))?;
    let token = channel.wait_connect(Instant::now() + Duration::from_secs(60));
    channel.close();
    let token = token?;
    config.cert_sha256 = Some(pin::to_hex(&fingerprint));
    let issued = token.is_some();
    if issued {
        config.token = token;
    }
    config.save(path)?;
    if issued {
        println!("Paired. Token saved to {} (mode 600).", path.display());
    } else {
        println!("The TV allowed the connection but issued no token; certificate pinned.");
    }
    Ok(())
}

/// Opens an authorized channel (pinned certificate, saved token), saving a
/// token the TV re-issues.
fn open(
    path: &Path,
    config: &mut Config,
) -> Result<remote::Remote<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>> {
    let policy = policy_of(config)?;
    if config.token.is_none() {
        return Err(Error::Config(
            "no token saved: run `dimmer-samsung pair`".into(),
        ));
    }
    let url = remote::channel_url(&config.host, &config.client_name, config.token.as_ref())?;
    let (mut channel, _) = remote::connect_tls(&config.host, &url, policy, IO_TIMEOUT)?;
    // A revoked token makes the TV prompt again; give it a little time, but
    // don't wait out a whole pairing here.
    let token = channel.wait_connect(Instant::now() + Duration::from_secs(10))?;
    if let Some(token) = token
        && Some(&token) != config.token.as_ref()
    {
        config.token = Some(token);
        config.save(path)?;
    }
    Ok(channel)
}

fn check(path: &Path, mut config: Config) -> Result<()> {
    let channel = open(path, &mut config)?;
    channel.close();
    println!("The TV accepted the saved token over the pinned certificate.");
    Ok(())
}

fn power_key(path: &Path, mut config: Config) -> Result<()> {
    let mut channel = open(path, &mut config)?;
    let sent = channel.send_key(KeyAction::Click, "KEY_POWER");
    channel.close();
    sent?;
    println!("Sent the power key.");
    Ok(())
}

fn off(path: &Path, mut config: Config) -> Result<()> {
    if !settled(&config.host).is_some_and(|i| i.is_on()) {
        println!("The TV doesn't consistently report that it's on; nothing sent.");
        return Ok(());
    }
    let mut channel = open(path, &mut config)?;
    let result = channel
        .send_key(KeyAction::Press, "KEY_POWER")
        .and_then(|()| {
            sleep(POWER_OFF_HOLD);
            channel.send_key(KeyAction::Release, "KEY_POWER")
        });
    channel.close();
    result?;
    println!("Held the power key for {}s.", POWER_OFF_HOLD.as_secs());
    Ok(())
}

fn poll(host: &str) -> Option<DeviceInfo> {
    info::fetch(host, Duration::from_secs(1)).ok()
}

/// `PowerState` comes from an unauthenticated plain-HTTP endpoint and changes
/// during transitions, and `KEY_POWER` is a toggle. So before a toggle is
/// decided on it, two reads half a second apart must agree.
fn settled(host: &str) -> Option<DeviceInfo> {
    let first = poll(host)?;
    sleep(Duration::from_millis(500));
    let second = poll(host)?;
    (first.power_state.is_some() && first.power_state == second.power_state).then_some(second)
}

fn wake(path: &Path, mut config: Config, timeout: Duration) -> Result<()> {
    if settled(&config.host).is_some_and(|i| i.is_on()) {
        println!("The TV is already on.");
        return Ok(());
    }
    let mac = config.mac.ok_or_else(|| {
        Error::Config("no MAC saved: run `dimmer-samsung setup` while the TV is on".into())
    })?;
    let ip = config.host.parse::<IpAddr>().ok().and_then(|ip| match ip {
        IpAddr::V4(v4) => Some(v4),
        IpAddr::V6(_) => None,
    });
    let targets = wol::targets(ip.filter(|ip: &Ipv4Addr| !ip.is_broadcast()));
    wol::send(mac, &targets)?;
    println!("Sent Wake-on-LAN.");
    let deadline = Instant::now() + timeout;
    let mut pressed = false;
    let mut standby_reads = 0;
    while Instant::now() < deadline {
        let info = poll(&config.host);
        standby_reads = match &info {
            Some(i) if i.power_state.as_deref() == Some("standby") => standby_reads + 1,
            _ => 0,
        };
        match info {
            Some(info) if info.is_on() => {
                println!("The TV is on.");
                return Ok(());
            }
            // Reachable but in standby (network up, screen off, or Art Mode)
            // on two reads in a row: one power-key press turns the picture
            // on, and the known state means it can't turn the TV off.
            Some(_) if standby_reads >= 2 && !pressed && config.token.is_some() => {
                pressed = true;
                let mut channel = open(path, &mut config)?;
                let sent = channel.send_key(KeyAction::Click, "KEY_POWER");
                channel.close();
                sent?;
                println!("The TV answered in standby; sent the power key.");
            }
            Some(_) => {}
            None => wol::send(mac, &targets).map(|_| ())?,
        }
        sleep(Duration::from_secs(1));
    }
    Err(Error::Timeout)
}

fn forget(path: &Path, mut config: Config) -> Result<()> {
    config.token = None;
    config.cert_sha256 = None;
    config.save(path)?;
    println!(
        "Forgot the token and certificate pin. To revoke it on the TV too, remove \"{}\" in the TV's device connection settings.",
        config.client_name
    );
    Ok(())
}
