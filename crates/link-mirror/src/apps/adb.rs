//! The phone over wireless debugging: finding it (`adb devices`, then the `_adb-tls-connect` services `adb mdns`
//! lists), pairing it once with the code its Wireless debugging screen shows, listing and launching its apps
//! through scrcpy, which puts each app on its own virtual display in its own window, and starting Link's clipboard
//! helper.

use std::process::{Command, Stdio};

use anyhow::{Context, bail};

const LINK: &str = "org.andromeda.link";
const HELPER: &str = "org.andromeda.link.clipboard.LogHelper";
const HELPER_LOG: &str = "/data/local/tmp/andromeda-link-helper.log";

pub struct App {
    pub name: String,
    pub package: String,
}

pub enum Found {
    Serial(String),
    /// Not paired: the pairing service when the phone's "Pair with code" dialog is open, as `ip:port`.
    Unpaired {
        pairing: Option<String>,
    },
}

fn adb(args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("adb").args(args).stdin(Stdio::null()).output().context("running adb")?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned() + &String::from_utf8_lossy(&output.stderr);
    if output.status.success() { Ok(text) } else { bail!("adb {}: {}", args.join(" "), text.trim()) }
}

/// Serials `adb devices` reports ready.
fn ready() -> anyhow::Result<Vec<String>> {
    Ok(adb(&["devices"])?
        .lines()
        .skip(1)
        .filter_map(|line| line.split_once('\t'))
        .filter(|(_, state)| state.trim() == "device")
        .map(|(serial, _)| serial.to_owned())
        .collect())
}

/// The name the phone shows for itself, as the Link app reports it.
fn device_name(serial: &str) -> String {
    adb(&["-s", serial, "shell", "settings", "get", "global", "device_name"])
        .map(|name| name.trim().to_owned())
        .unwrap_or_default()
}

/// `ip:port` of each mDNS service of `kind` `adb mdns services` lists.
fn services(kind: &str) -> Vec<String> {
    adb(&["mdns", "services"])
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(kind))
        .filter_map(|line| line.split_whitespace().last().map(str::to_owned))
        .collect()
}

/// The phone named `name`, connecting to phones that advertise wireless debugging first; with one phone, whatever it
/// is called. `ANDROID_SERIAL`, as adb reads it, picks a ready device by serial.
pub fn find(name: &str) -> anyhow::Result<Found> {
    if let Ok(serial) = std::env::var("ANDROID_SERIAL")
        && ready()?.contains(&serial)
    {
        return Ok(Found::Serial(serial));
    }
    let pick = |serials: Vec<String>| -> Option<String> {
        if serials.len() == 1 {
            return serials.into_iter().next();
        }
        serials.into_iter().find(|serial| device_name(serial) == name)
    };
    if let Some(serial) = pick(ready()?) {
        return Ok(Found::Serial(serial));
    }
    for target in services("_adb-tls-connect._tcp") {
        if let Err(error) = adb(&["connect", &target]) {
            log::info!("{error}");
        }
    }
    if let Some(serial) = pick(ready()?) {
        return Ok(Found::Serial(serial));
    }
    Ok(Found::Unpaired { pairing: services("_adb-tls-pairing._tcp").into_iter().next() })
}

/// Pairs with the phone's "Pair device with pairing code" dialog.
pub fn pair(target: &str, code: &str) -> anyhow::Result<()> {
    let answer = adb(&["pair", target, code])?;
    if answer.contains("Successfully paired") { Ok(()) } else { bail!("{}", answer.trim()) }
}

/// The phone's apps with a launcher entry, by name.
pub fn apps(serial: &str) -> anyhow::Result<Vec<App>> {
    let output = Command::new("scrcpy")
        .args(["-s", serial, "--list-apps"])
        .stdin(Stdio::null())
        .output()
        .context("running scrcpy; install scrcpy")?;
    let text = String::from_utf8_lossy(&output.stdout);
    let mut apps: Vec<App> = text.lines().filter_map(parse_app).collect();
    if apps.is_empty() && !output.status.success() {
        bail!("scrcpy --list-apps: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    apps.sort_by_key(|app| app.name.to_lowercase());
    Ok(apps)
}

/// One line of `scrcpy --list-apps`: ` * Name   package` for user apps, ` - Name   package` for system ones.
fn parse_app(line: &str) -> Option<App> {
    let rest = line.trim_start().strip_prefix("* ").or_else(|| line.trim_start().strip_prefix("- "))?;
    let (name, package) = rest.trim_end().rsplit_once(char::is_whitespace)?;
    package.contains('.').then(|| App { name: name.trim().to_owned(), package: package.to_owned() })
}

/// Opens `package` on a new virtual display in its own window; the window closing ends the display. Sound stays on
/// the phone: scrcpy would forward all of it, once per window.
pub fn launch(serial: &str, app: &App) -> anyhow::Result<()> {
    Command::new("scrcpy")
        .args([
            "-s",
            serial,
            "--new-display",
            &format!("--start-app={}", app.package),
            &format!("--window-title={}", app.name),
            "--no-vd-system-decorations",
            "--no-audio",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .context("running scrcpy; install scrcpy")?;
    Ok(())
}

/// Starts Link's log helper as the shell user, the way Shizuku starts its server, and lets Link draw over other apps,
/// which its clipboard read needs. The helper outlives this connection and runs until the phone restarts; a second
/// start finds it running and exits.
pub fn start_helper(serial: &str) -> anyhow::Result<()> {
    let paths = adb(&["-s", serial, "shell", "pm", "path", LINK])?;
    let apk = paths
        .lines()
        .filter_map(|line| line.trim().strip_prefix("package:"))
        .find(|path| path.ends_with("/base.apk"))
        .context("Link is not installed on the phone")?;
    adb(&["-s", serial, "shell", "appops", "set", LINK, "SYSTEM_ALERT_WINDOW", "allow"])?;
    adb(&[
        "-s",
        serial,
        "shell",
        &format!("CLASSPATH='{apk}' setsid app_process /system/bin {HELPER} {LINK} </dev/null >{HELPER_LOG} 2>&1 &"),
    ])?;
    let mut log = String::new();
    for _ in 0..50 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        log = adb(&["-s", serial, "shell", "cat", HELPER_LOG]).unwrap_or_default();
        if log.contains("helper for") || log.contains("already running") {
            return Ok(());
        }
    }
    bail!("the clipboard helper did not start: {}", log.trim())
}
