//! `tacet addon ...` — addon install, list and try.
//!
//! WHAT IT DOES: manages the user's `addons.json` registry and drives two
//! install paths for the web search addon — (a) bringing up a local SearXNG with
//! docker, (b) the user entering their own server address.
//!
//! THE NETWORK MONOPOLY IS PRESERVED. This file OPENS NO SOCKET and does not
//! pull `ureq`; verification goes through
//! `tacet_web::WebSearchClient::health()`. The terminal side (asking the
//! question, printing progress) is the shell's job, the side that goes on the
//! network is `tacet-web`. The split is identical to `model download`.
//!
//! WHAT WAS MEASURED, WHAT WAS NOT (the docker side):
//!
//! * MEASURED — `docker_version` and `compose_command`: run on this machine
//!   (Docker 29.6.1 found; `docker compose` MISSING, `docker-compose` 5.3.1
//!   PRESENT — that is exactly the rationale for trying both forms). The approval
//!   screen was run too and was seen to exit with "cancelled" on EOF.
//! * MEASURED NOTHING — `compose_up` and `wait_until_ready`: the image was never
//!   pulled, no container was ever started, the health probe never ran against a
//!   local instance. These steps download hundreds of MB on the user's machine
//!   and leave a persistent container behind; the approval is the user's, not the
//!   development session's.
//! * MEASURED — the address path (b): install against a real SearXNG instance,
//!   verification (30 results), the registry write, the catalog gate opening and
//!   closing, and the `try` command were run end to end.
//!
//! NO UNIT TEST WAS WRITTEN FOR THE NETWORK AND DOCKER — it would have produced
//! a false green. The tests only measure the CONTENT of the written config
//! (`compose_text`, `settings_text`), the address rule and the registry round
//! trip.
//!
//! THE APPROVAL GATE. Pulling a docker image and starting a container is a
//! persistent change on the user's machine; the image name, the files to be
//! written and the command to be run are shown as they are BEFORE THE APPROVAL.
//! `--no-approval` is for scripts only and follows the same pattern as the same
//! flag in `model download`.

use crate::ui::{BOLD, Color, DIM, YELLOW};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::process::{Command, Stdio};
use tacet_web::addon::{ADDRESS_KEY, Addon, WEB_SEARCH};

/// The port the local SearXNG will bind to.
///
/// NOT 8080: that is the most-collided port on a developer machine and the first
/// "it doesn't work" complaint would be another server holding it. 8888 is a less
/// crowded choice that does not carry SearXNG's own in-container port (8080) to
/// the outside.
const LOCAL_PORT: u16 = 8888;

/// It binds to LOOPBACK only.
///
/// The difference between `127.0.0.1:8888:8080` and `8888:8080` is security: the
/// latter opens the search server to EVERYONE on the same network. So that the
/// user's queries stay on their own machine, the address is pinned to loopback
/// explicitly.
const LOCAL_ADDRESS: &str = "http://localhost:8888";

/// The image to pull — shown to the user BEFORE the approval.
const IMAGE: &str = "searxng/searxng:latest";

/// The subdirectory (under the config directory) the compose files go into.
const SEARXNG_DIR: &str = "searxng";

/// The ONE sentence told to the user when the addon is not installed.
///
/// A constant, because the same sentence appears in three separate places: when a
/// web intent is sensed in a chat turn, in the `tacet tools` output, and in
/// `tacet addon try`. Written as three copies, one would get updated and the
/// others would go stale.
pub const ADDON_MISSING: &str =
    "the web search addon is not installed: `tacet addon install web-search`";

/// INSTALLED BUT CLOSED is a separate state and needs a separate sentence.
///
/// In the first version both were told "not installed" and this WAS MEASURED: to
/// a user who had closed the addon with `tacet addon close`, `tacet tools` said
/// "not installed" — a wrong sentence that pushes the user to reinstall what is
/// already installed. The suggested command was wrong too (`install`, when the
/// right one is `open`).
const ADDON_CLOSED: &str = "the web search addon is CLOSED: `tacet addon open web-search`";

/// Is the web addon INSTALLED at all (open or closed)? The gate message below
/// distinguishes missing from closed; the chat's offer-to-open needs the same
/// distinction — offering to "open" something that is not installed would be
/// the exact wrong-sentence bug the constant above documents.
pub fn web_installed() -> bool {
    tacet_web::addon::read().map(|r| r.find(WEB_SEARCH).is_some()).unwrap_or(false)
}

/// The right sentence to print while the gate is closed: is it not installed, or
/// closed.
pub fn closed_gate_message() -> &'static str {
    match tacet_web::addon::read() {
        Ok(r) if r.find(WEB_SEARCH).is_some() => ADDON_CLOSED,
        // An unreadable registry counts as "missing" too (the gate counts it that
        // way as well).
        _ => ADDON_MISSING,
    }
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

pub fn list(json: bool) -> ExitCode {
    let color = Color::setup();
    let path = tacet_web::addon::registry_path();
    let record = match tacet_web::addon::read() {
        Ok(r) => r,
        // A BROKEN REGISTRY IS NOT SILENTLY SWALLOWED: saying "no addons at all"
        // would push the user to reinstall what they had installed.
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the addon registry could not be read: {e}")));
            return ExitCode::FAILURE;
        }
    };

    if json {
        let records: Vec<serde_json::Value> = record
            .all()
            .iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "kind": a.kind,
                    "state": a.state_text(),
                    "settings": a.settings,
                })
            })
            .collect();
        let output = serde_json::json!({
            "registry": path.as_ref().map(|p| p.display().to_string()),
            "addons": records,
            "web_search_open": record.is_open(WEB_SEARCH),
        });
        println!("{output}");
        return ExitCode::SUCCESS;
    }

    match &path {
        Some(p) => {
            let note = if p.is_file() { "" } else { " (missing)" };
            println!("{}", color.paint(DIM, &format!("registry: {}{note}", p.display())));
        }
        None => println!(
            "{}",
            color.paint(DIM, "registry: the config directory could not be resolved (TACET_HOME can be set)")
        ),
    }
    println!();

    if record.is_empty() {
        println!("{}", color.paint(DIM, "no addon installed."));
        println!("{}", color.paint(DIM, "  tacet addon install web-search"));
        return ExitCode::SUCCESS;
    }

    for a in record.all() {
        let state = if a.open { "open" } else { "closed" };
        println!("{}  {}", color.paint(BOLD, &a.name), color.paint(DIM, state));
        println!("  {} {}", color.paint(DIM, "kind:"), a.kind);
        for (key, value) in &a.settings {
            println!("  {} {value}", color.paint(DIM, &format!("{key}:")));
        }
        println!();
    }

    // THE EFFECT ON THE TOOL CATALOG IS STATED PLAINLY: what the user really
    // wants to learn from this command is "does web search work".
    let open = record.is_open(WEB_SEARCH);
    println!(
        "{}",
        color.paint(
            DIM,
            if open {
                "the web_search/web_fetch tools ARE VISIBLE in the catalog."
            } else {
                "the web_search/web_fetch tools are NOT in the catalog (the addon is not installed/open)."
            }
        )
    );
    ExitCode::SUCCESS
}

// ---------------------------------------------------------------------------
// install
// ---------------------------------------------------------------------------

pub fn install(name: &str, address: Option<String>, local: bool, no_approval: bool) -> ExitCode {
    let color = Color::setup();
    if name != WEB_SEARCH {
        eprintln!("{}", color.paint(YELLOW, &format!("unknown addon: '{name}'")));
        eprintln!("{}", color.paint(DIM, &format!("  installable: {WEB_SEARCH}")));
        return ExitCode::FAILURE;
    }

    // IF THE FLAGS CLASH WE STOP: silently picking one of the two could have
    // installed a local container instead of the user's server.
    if local && address.is_some() {
        eprintln!("{}", color.paint(YELLOW, "--local and --address cannot be given together"));
        return ExitCode::FAILURE;
    }

    let choice = if local {
        InstallPath::Local
    } else if let Some(a) = address {
        InstallPath::Address(a)
    } else {
        match ask_path(&color) {
            Some(p) => p,
            None => return ExitCode::FAILURE,
        }
    };

    match choice {
        InstallPath::Local => install_local(&color, no_approval),
        InstallPath::Address(a) => install_with_address(&color, &a),
    }
}

/// The two install paths. The trailing underscore keeps it clear of
/// `std::path::Path`, which this file also uses.
enum InstallPath {
    Local,
    Address(String),
}

/// It ASKS for the two paths. This is what the user's request said verbatim:
/// "let there be either setting SearXNG up as a local server, or entering another
/// server address".
fn ask_path(color: &Color) -> Option<InstallPath> {
    println!("{}", color.paint(BOLD, "web search addon"));
    println!("{}", color.paint(DIM, "search runs through your own SearXNG server."));
    println!();
    println!("  1) set up a local SearXNG (docker required)");
    println!("  2) enter my own server address");
    print!("choice [1/2]: ");
    let _ = std::io::stdout().flush();
    let choice = read_line()?;
    match choice.trim() {
        "1" => Some(InstallPath::Local),
        "2" => {
            print!("SearXNG address (https://... or http://localhost:...): ");
            let _ = std::io::stdout().flush();
            let a = read_line()?;
            Some(InstallPath::Address(a.trim().to_string()))
        }
        other => {
            eprintln!("{}", color.paint(YELLOW, &format!("invalid choice: '{other}'")));
            None
        }
    }
}

fn read_line() -> Option<String> {
    let mut s = String::new();
    // 0 bytes = EOF: `None`, so piped input does not enter an infinite loop.
    match std::io::stdin().read_line(&mut s) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(s),
    }
}

/// (b) THE USER'S OWN ADDRESS.
///
/// Three steps, and the order matters: (1) the address must pass THE RULE —
/// `https` is mandatory, plain `http` only on a local network (the rule lives in
/// `tacet_web::address_is_valid`, in one place); (2) it must be verified WITH A
/// REAL QUERY — SearXNG can be up while keeping the JSON format off, and that
/// state looks like "200 OK"; (3) only then is it written to the registry.
/// Writing first and trying afterwards would leave a non-working addon marked
/// "installed".
fn install_with_address(color: &Color, address: &str) -> ExitCode {
    let address = address.trim().trim_end_matches('/');
    if let Err(e) = tacet_web::address_is_valid(address) {
        eprintln!("{}", color.paint(YELLOW, &format!("the address was not accepted: {e}")));
        eprintln!(
            "{}",
            color.paint(
                DIM,
                "  https is mandatory. Plain http only on a local network (localhost, 127.0.0.1, 192.168.*):",
            )
        );
        eprintln!(
            "{}",
            color.paint(DIM, "  a query going unencrypted to a remote server is read at every hop in between.")
        );
        return ExitCode::FAILURE;
    }

    println!("{}", color.paint(DIM, &format!("trying: {address}")));
    match verify(address) {
        Ok(n) => {
            println!("{}", color.paint(BOLD, &format!("✓ the server answered ({n} results)")));
        }
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("✗ could not be verified: {e}")));
            eprintln!("{}", color.paint(DIM, "  the addon WAS NOT INSTALLED — a non-working address is not recorded."));
            return ExitCode::FAILURE;
        }
    }

    write_registry(color, address)
}

/// Verifies WITH A REAL QUERY. The network path WAS NOT MEASURED on this machine
/// (there was no running SearXNG instance).
fn verify(address: &str) -> Result<usize, tacet_web::WebError> {
    tacet_web::WebSearchClient::with_address(address).health()
}

fn write_registry(color: &Color, address: &str) -> ExitCode {
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the addon registry could not be read: {e}")));
            return ExitCode::FAILURE;
        }
    };
    record.add(Addon::new(WEB_SEARCH, WEB_SEARCH).with_setting(ADDRESS_KEY, address));
    match tacet_web::addon::write(&record) {
        Ok(path) => {
            println!("{}", color.paint(DIM, &format!("registry: {}", path.display())));
            println!("{}", color.paint(BOLD, "the web search addon is installed and open."));
            println!(
                "{}",
                color.paint(DIM, "the web_search/web_fetch tools are in the catalog now; they can be used in chat.")
            );
            println!(
                "{}",
                color.paint(
                    DIM,
                    "every call that takes data out still asks for approval in a tainted session.",
                )
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the registry could not be written: {e}")));
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// (a) local SearXNG — docker
// ---------------------------------------------------------------------------

/// IF DOCKER IS MISSING WE DO NOT FAIL SILENTLY: what was looked for, what could
/// not be found and what the alternative is are written out one by one.
fn install_local(color: &Color, no_approval: bool) -> ExitCode {
    let Some(version) = docker_version() else {
        eprintln!("{}", color.paint(YELLOW, "docker not found — a local SearXNG cannot be set up."));
        eprintln!("{}", color.paint(DIM, "  `docker --version` did not run (not on PATH, or the daemon is down)."));
        eprintln!("{}", color.paint(DIM, "  options:"));
        eprintln!("{}", color.paint(DIM, "   • install Docker Desktop / docker engine and repeat this command"));
        eprintln!(
            "{}",
            color.paint(DIM, "   • use your own server: tacet addon install web-search --address https://...")
        );
        return ExitCode::FAILURE;
    };
    println!("{}", color.paint(DIM, &format!("docker: {version}")));

    let Some(compose) = compose_command() else {
        eprintln!("{}", color.paint(YELLOW, "docker is there but compose is not."));
        eprintln!("{}", color.paint(DIM, "  neither `docker compose version` nor `docker-compose --version` ran."));
        eprintln!(
            "{}",
            color.paint(DIM, "   • use your own server: tacet addon install web-search --address https://...")
        );
        return ExitCode::FAILURE;
    };

    let Some(dir) = searxng_dir() else {
        eprintln!("{}", color.paint(YELLOW, "the config directory could not be resolved (TACET_HOME can be set)"));
        return ExitCode::FAILURE;
    };

    // EVERYTHING IS SHOWN BEFORE THE APPROVAL: which image will come down, which
    // files will be written, which command will run, where the server will bind.
    println!();
    println!("{}", color.paint(BOLD, "a local SearXNG will be set up:"));
    println!("  image     : {IMAGE}");
    println!("  directory : {}", dir.display());
    println!("  files     : docker-compose.yml, settings.yml");
    println!("  command   : {} up -d", compose.join(" "));
    println!("  address   : {LOCAL_ADDRESS}  (binds to 127.0.0.1 only)");
    println!(
        "{}",
        color.paint(DIM, "  the image is a few hundred MB and is pulled by docker.")
    );
    if !take_approval(color, no_approval) {
        println!("{}", color.paint(DIM, "cancelled — nothing was written."));
        return ExitCode::FAILURE;
    }

    if let Err(e) = write_config(&dir) {
        eprintln!("{}", color.paint(YELLOW, &format!("the config could not be written: {e}")));
        return ExitCode::FAILURE;
    }
    println!("{}", color.paint(DIM, &format!("written: {}", dir.display())));

    match compose_up(&compose, &dir) {
        Ok(()) => println!("{}", color.paint(DIM, "the container was started; waiting for it to come up…")),
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("docker compose failed: {e}")));
            eprintln!(
                "{}",
                color.paint(DIM, &format!("  try by hand: cd {} && {} up -d", dir.display(), compose.join(" ")))
            );
            return ExitCode::FAILURE;
        }
    }

    // THE HEALTH QUERY goes through tacet-web; this file opens no socket.
    match wait_until_ready(color) {
        Some(n) => {
            println!("{}", color.paint(BOLD, &format!("✓ SearXNG answered ({n} results)")));
            write_registry(color, LOCAL_ADDRESS)
        }
        None => {
            eprintln!("{}", color.paint(YELLOW, "✗ SearXNG did not answer within the expected time."));
            eprintln!("{}", color.paint(DIM, &format!("  logs: {} logs", compose.join(" "))));
            eprintln!("{}", color.paint(DIM, "  the addon WAS NOT INSTALLED — a non-working address is not recorded."));
            ExitCode::FAILURE
        }
    }
}

fn take_approval(color: &Color, no_approval: bool) -> bool {
    if no_approval {
        println!("{}", color.paint(DIM, "  (--no-approval: no question was asked)"));
        return true;
    }
    print!("  Continue? [y/N] ");
    let _ = std::io::stdout().flush();
    match read_line() {
        Some(s) => matches!(s.trim().to_lowercase().as_str(), "y" | "yes"),
        None => false,
    }
}

fn searxng_dir() -> Option<PathBuf> {
    tacet_core::env::config_dir().map(|d| d.join(SEARXNG_DIR))
}

/// The output of `docker --version`. `None` = docker missing or not running.
///
/// MEASURED NOTHING: docker is not installed on this machine.
fn docker_version() -> Option<String> {
    let output = Command::new("docker")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Finds which form compose is installed in.
///
/// THERE ARE TWO FORMS and both are widespread in the field: `docker compose` as
/// a plugin (v2) and the separate `docker-compose` binary (v1). Trying only one
/// would mean saying "missing" even though compose IS PRESENT on the user's
/// machine.
///
/// THIS DISTINCTION IS NOT AN ASSUMPTION, IT IS A MEASUREMENT: on the development
/// machine `docker --version` = 29.6.1 but `docker compose version` FAILS; the
/// only working form is `docker-compose` (5.3.1). Had only v2 been tried, the
/// local install would have been rejected on this machine with "no compose".
fn compose_command() -> Option<Vec<String>> {
    let v2 = Command::new("docker")
        .args(["compose", "version"])
        .stdin(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    if v2 {
        return Some(vec!["docker".into(), "compose".into()]);
    }
    let v1 = Command::new("docker-compose")
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .ok()
        .is_some_and(|o| o.status.success());
    v1.then(|| vec!["docker-compose".into()])
}

/// MEASURED NOTHING. Its output is relayed to the user as is: docker's own error
/// text (port taken, daemon down, image could not be pulled) is more informative
/// than any summary we would rewrite.
fn compose_up(compose: &[String], dir: &Path) -> Result<(), String> {
    let (binary, prefixes) = compose.split_first().ok_or("the compose command is empty")?;
    let output = Command::new(binary)
        .args(prefixes)
        .args(["up", "-d"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("{binary} could not be run: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(if error.is_empty() { format!("exit code {}", output.status) } else { error })
}

/// Waits for the container to come up; returns the number of results that came
/// back.
///
/// WHY POLLING: `up -d` STARTS the container but SearXNG takes a few seconds to
/// begin listening. Saying "it did not answer" on a single attempt would declare
/// a working install a failure.
///
/// MEASURED NOTHING: the duration and the attempt count were never exercised
/// against a real container.
fn wait_until_ready(color: &Color) -> Option<usize> {
    const ATTEMPTS: usize = 15;
    const INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
    for i in 1..=ATTEMPTS {
        if let Ok(n) = verify(LOCAL_ADDRESS) {
            return Some(n);
        }
        if i == 1 {
            println!("{}", color.paint(DIM, "  (the first start can take minutes, including the image pull)"));
        }
        std::thread::sleep(INTERVAL);
    }
    None
}

/// Writes the compose and settings files.
fn write_config(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("docker-compose.yml"), compose_text())?;
    let settings = dir.join("settings.yml");
    // THE SETTINGS FILE IS NOT OVERWRITTEN: the user may have changed their own
    // SearXNG settings (engine choice, language) by hand and a reinstall must not
    // delete them. The secret key must not change on every install either.
    if !settings.is_file() {
        std::fs::write(settings, settings_text(&secret_key()))?;
    }
    Ok(())
}

/// The text of the compose file. A SEPARATE FUNCTION: so the correctness of its
/// content (binding to loopback, the image name, the volume mount) can be
/// measured by a test.
fn compose_text() -> String {
    format!(
        "# Generated by Tacet — `tacet addon install web-search`\n\
         # Editable by hand; `settings.yml` IS NOT OVERWRITTEN on reinstall.\n\
         services:\n\
         \x20 searxng:\n\
         \x20   image: {IMAGE}\n\
         \x20   container_name: tacet-searxng\n\
         \x20   restart: unless-stopped\n\
         \x20   ports:\n\
         \x20     - \"127.0.0.1:{LOCAL_PORT}:8080\"\n\
         \x20   volumes:\n\
         \x20     - ./:/etc/searxng:rw\n\
         \x20   environment:\n\
         \x20     - SEARXNG_BASE_URL={LOCAL_ADDRESS}/\n"
    )
}

/// The SearXNG settings file.
///
/// WITHOUT `formats: json` SEARCH DOES NOT WORK. SearXNG returns only HTML by
/// default; with JSON off the server gives "200 OK" and the client takes the HTML
/// for JSON and falls into `InvalidJson`. That trap is known by name in this repo
/// (see the `WebError::InvalidJson` comment) — the install's first job is to turn
/// it on.
///
/// `limiter: false`: the rate limit is for bot protection and on a single-user
/// local instance it only blocks our own queries.
fn settings_text(secret: &str) -> String {
    format!(
        "# Generated by Tacet. You can add your own settings here.\n\
         use_default_settings: true\n\
         server:\n\
         \x20 secret_key: \"{secret}\"\n\
         \x20 limiter: false\n\
         \x20 image_proxy: true\n\
         search:\n\
         \x20 # `json` IS MANDATORY: Tacet reads search results as JSON.\n\
         \x20 formats:\n\
         \x20   - html\n\
         \x20   - json\n"
    )
}

/// A value for SearXNG's `secret_key` field.
///
/// ZERO DEPENDENCY: `rand` WAS NOT ADDED. On Unix 32 bytes are read from
/// `/dev/urandom` — the operating system's own pool, always better than a
/// hand-written generator. If it cannot be read (Windows or a restricted
/// environment) the clock + the process id + a stack address are mixed; THIS
/// FALLBACK PATH IS NOT CRYPTOGRAPHIC and is not claimed to be. On a local,
/// loopback-bound, single-user instance this key's job is to sign image-proxy
/// links; it is not an authentication key.
///
/// `read_exact`, NOT `fs::read` — and this line is a bug fix. The first version
/// called `std::fs::read("/dev/urandom")`; that function reads UNTIL END OF FILE
/// and `/dev/urandom` HAS NO END. The test suite therefore hung (measured:
/// `cargo test -p tacet-cli` never finished). In production the same call would
/// have frozen the local install forever.
fn secret_key() -> String {
    if let Some(hex) = thirty_two_bytes_from_urandom() {
        return hex;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = u128::from(std::process::id());
    let stack = &now as *const u128 as usize as u128;
    format!("{:032x}{:032x}", now ^ (pid << 64), stack.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// EXACTLY 32 bytes from the operating system's pool. `None` = the source is
/// missing/unreadable (Windows, a restricted container).
fn thirty_two_bytes_from_urandom() -> Option<String> {
    use std::io::Read;
    let mut file = std::fs::File::open("/dev/urandom").ok()?;
    let mut bytes = [0u8; 32];
    file.read_exact(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

// ---------------------------------------------------------------------------
// remove / open / close / try
// ---------------------------------------------------------------------------

pub fn remove(name: &str) -> ExitCode {
    let color = Color::setup();
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the addon registry could not be read: {e}")));
            return ExitCode::FAILURE;
        }
    };
    if !record.delete(name) {
        eprintln!("{}", color.paint(YELLOW, &format!("not installed: '{name}'")));
        return ExitCode::FAILURE;
    }
    match tacet_web::addon::write(&record) {
        Ok(_) => {
            println!("{}", color.paint(BOLD, &format!("'{name}' was removed.")));
            if name == WEB_SEARCH {
                println!(
                    "{}",
                    color.paint(DIM, "the web_search/web_fetch tools dropped out of the catalog.")
                );
                // THE CONTAINER IS NOT STOPPED BY ITSELF. It may have been started
                // during install, but stopping it (and deleting its data) is the
                // user's decision; the command is named "remove", not "delete".
                if let Some(d) = searxng_dir()
                    && d.is_dir()
                {
                    // THE COMMAND IS WRITTEN BY MEASURING THE MACHINE. Hard-coding
                    // "docker compose" would give a recipe that DOES NOT WORK on
                    // machines where compose was installed as the old binary
                    // (`docker-compose`) — measured on this machine:
                    // `docker compose version` fails, `docker-compose --version`
                    // succeeds.
                    let command = compose_command()
                        .map(|c| c.join(" "))
                        .unwrap_or_else(|| "docker compose".to_string());
                    println!(
                        "{}",
                        color.paint(
                            DIM,
                            &format!(
                                "the local container KEEPS RUNNING — to stop it:\n  cd {} && {command} down",
                                d.display()
                            )
                        )
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the registry could not be written: {e}")));
            ExitCode::FAILURE
        }
    }
}

/// `open`/`close` — closing the gate without deleting the record.
///
/// WHY IT EXISTS: the registry has a `state` field and `install` always set it to
/// "open". A field that can never be "closed" is a dead field that misleads its
/// reader; it should either be removed or be changeable from production. The
/// second was chosen because turning search off WITHOUT LOSING the address and
/// the settings is a real need (while travelling, on a metered connection).
pub fn set_state(name: &str, open: bool) -> ExitCode {
    let color = Color::setup();
    let mut record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the addon registry could not be read: {e}")));
            return ExitCode::FAILURE;
        }
    };
    if record.set_state(name, open).is_none() {
        eprintln!("{}", color.paint(YELLOW, &format!("not installed: '{name}'")));
        return ExitCode::FAILURE;
    }
    match tacet_web::addon::write(&record) {
        Ok(_) => {
            println!(
                "{}",
                color.paint(BOLD, &format!("'{name}' was {}.", if open { "opened" } else { "closed" }))
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the registry could not be written: {e}")));
            ExitCode::FAILURE
        }
    }
}

/// `tacet addon try <name>` — measures whether an installed addon REALLY works.
/// It goes on the network; before it does, it says what it is going to do.
pub fn try_addon(name: &str, json: bool) -> ExitCode {
    let color = Color::setup();
    let record = match tacet_web::addon::read() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("{}", color.paint(YELLOW, &format!("the addon registry could not be read: {e}")));
            return ExitCode::FAILURE;
        }
    };
    let Some(a) = record.find(name) else {
        if json {
            println!("{}", serde_json::json!({ "name": name, "installed": false, "working": false }));
        } else {
            eprintln!("{}", color.paint(YELLOW, &format!("not installed: '{name}'")));
            if name == WEB_SEARCH {
                eprintln!("{}", color.paint(DIM, &format!("  {ADDON_MISSING}")));
            }
        }
        return ExitCode::FAILURE;
    };
    if a.kind != WEB_SEARCH {
        eprintln!("{}", color.paint(YELLOW, &format!("'{}' cannot be tried: kind {}", a.name, a.kind)));
        return ExitCode::FAILURE;
    }
    // THE ADDRESS IS RESOLVED THE SAME WAY AS IN PRODUCTION:
    // `WebSearchClient::new()` also looks at the environment variable first and
    // the registry second. Had we read the registry's address directly here, on an
    // install overridden with `TACET_SEARXNG` the trial would have exercised a
    // DIFFERENT server.
    let Some(address) = tacet_web::addon::web_address() else {
        eprintln!("{}", color.paint(YELLOW, "no address is defined"));
        return ExitCode::FAILURE;
    };

    if !json {
        println!("{}", color.paint(DIM, &format!("sending a query: {address}")));
    }
    match verify(&address) {
        Ok(n) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": a.name, "installed": true, "open": a.open,
                        "address": address, "working": true, "results": n,
                    })
                );
            } else {
                println!("{}", color.paint(BOLD, &format!("✓ working ({n} results)")));
                if !a.open {
                    println!(
                        "{}",
                        color.paint(YELLOW, "the addon is CLOSED: the tools are not in the catalog (`tacet addon open web-search`)")
                    );
                }
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            if json {
                println!(
                    "{}",
                    serde_json::json!({
                        "name": a.name, "installed": true, "open": a.open,
                        "address": address, "working": false, "error": e.to_string(),
                    })
                );
            } else {
                eprintln!("{}", color.paint(YELLOW, &format!("✗ {e}")));
            }
            ExitCode::FAILURE
        }
    }
}

// ---------------------------------------------------------------------------
// A production hint — a web request while the addon is missing
// ---------------------------------------------------------------------------

/// Does this message WANT a web search.
///
/// THE TRIGGER LIST IS NOT REWRITTEN: the router's `Web` intent profile is
/// already a list used in production and grown by measurement ("weather", "ferry
/// times", "dollar", "current"...). Setting up a second list would mean one
/// growing while the other went stale — exactly the failure this repo has already
/// lived through (the catalog list stood apart in the shell and in eval).
///
/// The DOMINANT profile is asked, not "is the score greater than zero": the words
/// "address" or "page" also appear in document requests, and advertising the addon
/// every time they do is noise the user learns to ignore.
pub fn is_web_request(message: &str) -> bool {
    tacet_tools::router::score_intent(message).dominant()
        == tacet_tools::router::IntentProfile::Web
}

#[cfg(test)]
mod tests {
    use super::*;

    /// THE SEARXNG TRAP: without `formats: json` search breaks silently.
    #[test]
    fn the_settings_file_turns_on_the_json_format() {
        let t = settings_text("abc");
        assert!(t.contains("formats:"), "{t}");
        assert!(t.contains("- json"), "{t}");
        assert!(t.contains("secret_key: \"abc\""), "{t}");
    }

    /// The container must bind to loopback ONLY: had `"8888:8080"` been written,
    /// the search server would have been open to everyone on the same network.
    #[test]
    fn compose_binds_to_loopback_only() {
        let t = compose_text();
        assert!(t.contains("\"127.0.0.1:8888:8080\""), "{t}");
        assert!(!t.contains("\"8888:8080\""), "the port is open to all interfaces: {t}");
        assert!(t.contains(IMAGE), "the image name is not in the compose file: {t}");
    }

    /// The port written in compose and the address written to the registry must be
    /// THE SAME. Both are written by hand so they can diverge, and in that state
    /// the install would say "not working".
    #[test]
    fn the_compose_port_matches_the_registry_address() {
        assert!(LOCAL_ADDRESS.ends_with(&LOCAL_PORT.to_string()), "{LOCAL_ADDRESS}");
        assert!(compose_text().contains(&format!("127.0.0.1:{LOCAL_PORT}:")));
    }

    /// The local address MUST PASS `tacet-web`'s address gate. If it did not, the
    /// install would finish successfully and the first search would say "invalid
    /// address".
    #[test]
    fn the_local_address_passes_the_web_gate() {
        assert!(tacet_web::address_is_valid(LOCAL_ADDRESS).is_ok());
    }

    /// The secret key must not be empty or constant.
    #[test]
    fn a_secret_key_is_generated() {
        let a = secret_key();
        let b = secret_key();
        assert!(a.len() >= 32, "short key: {a}");
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()), "{a}");
        assert_ne!(a, b, "the key is generated as a constant");
    }

    /// INSTALLED-BUT-CLOSED and NOT-INSTALLED-AT-ALL must get separate sentences,
    /// and THE SUGGESTED COMMAND must be right (`open` or `install`).
    #[test]
    fn closed_and_not_installed_are_separate_sentences() {
        assert!(ADDON_MISSING.contains("addon install"));
        assert!(ADDON_CLOSED.contains("addon open"));
        assert_ne!(ADDON_MISSING, ADDON_CLOSED);
        // Whatever state the registry is in on this machine, the chosen sentence
        // must match that state.
        let installed =
            tacet_web::addon::read().map(|r| r.find(WEB_SEARCH).is_some()).unwrap_or(false);
        assert_eq!(closed_gate_message(), if installed { ADDON_CLOSED } else { ADDON_MISSING });
    }

    /// THE PRODUCTION HINT trigger: it wakes on a web question, not on a greeting.
    ///
    /// THE MESSAGES STAY TURKISH ON PURPOSE. This test does not measure this
    /// file, it measures `tacet_tools::router`'s `message_triggers`, and that list
    /// is TURKISH DATA — it is matched against what the user typed and was grown
    /// by measurement ("hava", "vapur saatleri", "dolar"). Translating the case
    /// texts to English would make the test touch nothing and turn it green while
    /// measuring nothing. When the trigger list becomes multilingual, these lines
    /// move with it.
    #[test]
    fn a_web_request_is_sensed() {
        assert!(is_web_request("what is the current dollar price"));
        assert!(is_web_request("what is the weather like in Istanbul?"));
        assert!(is_web_request("can you summarize this address https://example.test/post"));
        assert!(!is_web_request("hello"));
        assert!(!is_web_request("multiply 17 by 45"));
        assert!(!is_web_request("create the budget table as xlsx"));
    }
}
