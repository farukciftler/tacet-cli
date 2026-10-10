//! `run_code` — the tool that runs model output. THIS FILE IS A SECURITY
//! BOUNDARY; every mechanism here exists for an attack or an accident.
//!
//! The Swift counterpart is `CodeEngine` + `RunCodeTool`. There the engine was
//! JavaScriptCore: an interpreter inside the operating system with NO file or
//! network bridge. There is NO counterpart in Rust and, under the
//! zero-dependency identity, pulling in a JS engine is out of the question — so
//! here the model's code runs IN A SEPARATE PROCESS, with an interpreter
//! already installed on the system. That changes the threat model: in JSC the
//! network was absent "because it was not bridged", here it must be absent
//! "because it is explicitly cut". Where we cannot cut it, the tool DOES NOT
//! PUT ITSELF IN THE CATALOG (see `discover`).
//! FOUR GATES:
//!
//! 1. **NETWORK** — `sandbox-exec` with `(deny network*)` on macOS, `bwrap`
//!    with `--unshare-net` on Linux. The cut is REALLY MEASURED DURING
//!    DISCOVERY (`verify_shield` ACTUALLY TRIES a connection under the shield):
//!    writing a profile file and saying "it probably works" would be exactly
//!    what silently calling something safe looks like. If the shield cannot be
//!    set up, `discover` returns `None` and the tool IS NOT VISIBLE in the catalog.
//!
//! PLATFORM RECORD: there is NO counterpart of these tools (`run_code` and
//! `write_code`) on Windows and the tool is DELIBERATELY off there. The reason
//! is the absence of a shield: on Windows there is no mechanism installable
//! with zero dependencies that cuts BOTH the network AND the file system
//! (AppContainer/Job Object want `windows-sys`). Being off is the right
//! behaviour — treating a gate we cannot close as "probably closed" and running
//! model code is exactly what this file stands against. The user learns THE
//! REASON from `diagnose()`.
//! 2. **FILES** — the same profile locks writing into the sandbox, and READING
//!    is an allowlist: the per-call sandbox plus the system paths an
//!    interpreter needs to start (see `profile`). The sandbox is a FRESH,
//!    EMPTY directory made for the one call and deleted after it
//!    (`sandbox_path::fresh_private_dir`), never a folder the user already has.
//! 3. **TIME** — a mandatory timeout (10 s by default). If it is exceeded the
//!    process is KILLED (`kill` + `wait`), no zombie is left behind.
//! 4. **OUTPUT** — a byte cap. So an infinite loop does not eat all the memory,
//!    the pipes KEEP BEING READ in separate threads but anything over the cap is
//!    DISCARDED: stopping the reads locks the child on the pipe, having no cap eats the memory.
//! THE ATTEMPT COUNTER (code-spec §5.4): at most 2 real runs per turn; the 3rd
//! call is rejected without EVER seeing the engine. In Swift the counter was on
//! a `weak` reference and, when it was not bound, "fail-closed" counted it as 3.
//! Here the counter is INSIDE THE TOOL and is always bound by default — there is
//! no binding to forget.
//! SUCCESS WITH NO OUTPUT IS NOT A SUCCESS (measured in Swift): a model that saw
//! "ok" MADE THE RESULT UP. A script that prints nothing counts as an error.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tacet_kernel::context::CancelToken;
use tacet_kernel::{
    ArgSchema, Field, Tool, ToolContext, ToolError, ToolFuture, ToolOutcome, ToolResult, ToolState,
    TraceUpdate, boxed,
};

/// The default timeout (seconds). The base the task gives.
pub const DEFAULT_TIMEOUT: u64 = 10;
/// The longest duration the model may ask for. The model CANNOT CHOOSE the cap, only shorten it.
pub const MAX_TIMEOUT: u64 = 30;

/// The byte cap of the captured output. Anything over it is discarded and the discarding IS STATED.
const OUTPUT_CAP: usize = 10_000;
/// The cap of the output going to the model — the full output goes to the chip (code-spec §5.2).
pub(crate) const MODEL_OUTPUT_CAP: usize = 500;
/// The cap of the error text: it must not eat the 4096 budget.
pub(crate) const MODEL_ERROR_CAP: usize = 400;
/// The cap of the script source. Anything above this is not a "short script".
pub(crate) const CODE_CAP: usize = 20_000;
/// The number of real runs per turn (code-spec §5.4).
pub(crate) const MAX_ATTEMPTS: usize = 2;

/// The polling interval for the process finishing. Std has no `wait_timeout`;
/// the polling interval is kept short so the timeout measurement stays
/// sensitive, but long enough not to burn the CPU for nothing.
const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// How long we wait for the pipe readers AFTER the process has finished or been
/// killed, before abandoning them (see the bounded join in `run_program`).
///
/// Generous on purpose: on the normal path the readers only have a pipe buffer
/// left to drain and finish in microseconds, so this bound never fires for
/// legitimate work. It exists so a stuck reader costs 2 seconds instead of the
/// whole turn.
const JOIN_GRACE: Duration = Duration::from_secs(2);

// `killpg`/`setsid` WITHOUT A libc CRATE — the zero-dependency identity holds;
// these two symbols live in the C library every unix already links.
#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
    fn killpg(group: i32, signal: i32) -> i32;
}

/// `SIGKILL` is 9 on every unix (POSIX fixes the first fifteen numbers).
#[cfg(unix)]
const SIGKILL: i32 = 9;
/// `SIGTERM` is 15, from the same fixed range.
#[cfg(unix)]
const SIGTERM: i32 = 15;

/// After a stop (audit M4), the time between SIGTERM and SIGKILL — the same
/// half second `shell` gives, so a Ctrl-C ends either tool's process inside the
/// ~1 s the audit asks for.
const TERM_GRACE: Duration = Duration::from_millis(500);

/// The chip sentence for a run that was stopped. The model never sees it — a
/// stopped turn is answered by the executor's own fixed text.
const STOPPED: &str = "the script was stopped";

// ---------------------------------------------------------------------------
// The network shield
// ---------------------------------------------------------------------------

/// The mechanism that cuts the child process's network and file access.
///
/// "NO SHIELD" IS NOT A VARIANT — that rule has not changed. If the shield
/// cannot be set up, `RunCodeTool` simply does not exist; had "shieldless mode"
/// been an enum arm, a future patch could silently make it the default. The
/// variants are ONLY real shields and each of them must close THE SAME two
/// gates: the network AND writing outside the sandbox. A mechanism that cuts
/// only the network (e.g. a bare `unshare -n`) IS NOT ADDED here — the sentence
/// "NO access to your files outside the working folder" in `description()`
/// would then be a lie.
#[derive(Debug, Clone)]
pub enum NetworkShield {
    /// macOS `sandbox-exec`. The profile is given inline (`-p`); we leave no
    /// profile file on disk that could be tampered with.
    SandboxExec { tool: PathBuf },
    /// Linux `bwrap` (bubblewrap). `--unshare-net` cuts the network and
    /// `--ro-bind / /` cuts writing outside the sandbox; the user's home
    /// directory is COVERED with an empty `--tmpfs` (the counterpart of the
    /// `deny file-read* (subpath home)` rule in the macOS profile).
    ///
    /// NOT MEASURED ON THIS MACHINE: there is no Linux here, no `rustup`, no
    /// cross compilation. But that is NOT the same as "we hope it works": on
    /// the production path the shield is measured ON EVERY STARTUP with
    /// `verify_shield` (see that function). If bwrap is installed wrong on this
    /// machine, or `--unshare-net` is refused by the kernel, the measurement
    /// fails, `discover` returns `None` and the tool IS NOT VISIBLE in the
    /// catalog — so the worst case is today's Linux behaviour, not a new silent hole.
    ///
    /// MEASURED IN CI SINCE 2026-09-04, which is a weaker claim than it sounds
    /// and is stated precisely: the ubuntu leg of `.github/workflows/ci.yml`
    /// installs `bubblewrap`, preflights it, and runs the whole suite with
    /// `TACET_SANDBOX_MUST_RUN=1`, under which a shield that cannot be
    /// discovered PANICS instead of printing a skip (see `SANDBOX_MUST_RUN`).
    /// Before that, every Linux run of the execution tests passed by returning
    /// immediately. What is still not covered: this is one kernel and one
    /// AppArmor policy on a throwaway runner, and the author has no Linux
    /// machine on which to reproduce a failure it reports.
    Bwrap { tool: PathBuf },
}

impl NetworkShield {
    /// Looks for a usable shield on the system. IT DOES NOT VALIDATE — the
    /// caller measures that it really cuts, with `verify_shield`.
    ///
    /// The paths are FIXED, `PATH` is not searched: `PATH` comes from the
    /// caller's environment and can be poisoned; picking the shield itself out
    /// of PATH would make the shield meaningless (see `discover_interpreters`).
    pub fn find() -> Option<NetworkShield> {
        let sandbox = Path::new("/usr/bin/sandbox-exec");
        if sandbox.is_file() {
            return Some(NetworkShield::SandboxExec {
                tool: sandbox.to_path_buf(),
            });
        }
        BWRAP_PATHS
            .iter()
            .map(Path::new)
            .find(|y| y.is_file())
            .map(|y| NetworkShield::Bwrap {
                tool: y.to_path_buf(),
            })
    }

    pub fn name(&self) -> &'static str {
        match self {
            NetworkShield::SandboxExec { .. } => "sandbox-exec",
            NetworkShield::Bwrap { .. } => "bwrap",
        }
    }

    /// Wraps the command with the shield.
    ///
    /// `sandbox` must be ABSOLUTE and RESOLVED (symlink free): on macOS `/tmp`
    /// is a link to `/private/tmp` and a profile that says `(subpath "/tmp/...")`
    /// permits nothing — the rule silently falls to "everything forbidden" and
    /// the script crashes incomprehensibly.
    ///
    /// `args` is free: a direct run passes `[script]`, while a syntax check
    /// (`write_code`) carries a flag such as `["--check", script]`. The shield
    /// profile is THE SAME on both paths — even the verification step cannot get out of the sandbox.
    fn wrap(&self, sandbox: &Path, home: &Path, program: &Path, args: &[OsString]) -> Command {
        match self {
            NetworkShield::SandboxExec { tool } => {
                let mut k = Command::new(tool);
                // THE PATHS GO IN AS PARAMETERS, NEVER AS PROFILE TEXT (see
                // `profile`). `-D KEY=value` is a separate argv element, so no
                // character in the value can ever reach the SBPL parser.
                let mut sandbox_param = OsString::from(format!("{PARAM_SANDBOX}="));
                sandbox_param.push(sandbox);
                let mut home_param = OsString::from(format!("{PARAM_HOME}="));
                home_param.push(home);
                k.arg("-D")
                    .arg(sandbox_param)
                    .arg("-D")
                    .arg(home_param)
                    .arg("-p")
                    .arg(profile())
                    .arg(program)
                    .args(args);
                k
            }
            NetworkShield::Bwrap { tool } => {
                let mut k = Command::new(tool);
                // `HOME` is passed as a PARAMETER and is not read inside the
                // function: an environment variable is process-wide and
                // changing it in a test would break other tests running in
                // parallel. A pure function = a measurable function. It
                // arrives CANONICAL (see `canonical_home`): `--tmpfs` over a
                // link name covers the link, not the directory behind it.
                // `HOME=/` still reaches `bwrap_args`, whose guard refuses to
                // cover the root — nothing is covered rather than everything.
                // A home that does not exist is not covered: `--tmpfs` on a
                // missing path makes bwrap `mkdir` inside the read-only root,
                // which fails and takes the whole shield down.
                let home = Some(home).filter(|h| h.is_dir());
                let shadows = shadows_for(sandbox, home, &ShadowHost::from_env());
                k.args(bwrap_args(sandbox, home, &shadows));
                // `--` IS REQUIRED: everything after it is the child's command
                // line, not bwrap's. Without it a script flag such as `--check`
                // would be taken for bwrap's own argument.
                k.arg("--").arg(program).args(args);
                k
            }
        }
    }
}

/// The FIXED paths where bwrap is looked for (they differ per distribution).
const BWRAP_PATHS: &[&str] = &["/usr/bin/bwrap", "/bin/bwrap", "/usr/local/bin/bwrap"];

/// The bwrap arguments — the Linux counterpart of `profile()`.
///
/// Rule maps to rule:
///   * `(deny network*)`            -> `--unshare-net`
///   * `(allow file-read*)`         -> `--ro-bind / /` (the interpreter must be
///     able to read its own library; narrowed down, python3 does not start at
///     all — the lesson learned on the macOS side applies as it is)
///   * `(deny file-read* home)`     -> `--tmpfs {home}` (the home directory looks EMPTY)
///   * `(allow file-write* sandbox)` -> `--bind {sandbox} {sandbox}`, and since
///     `/` is read only nothing can be written anywhere else
///
/// THE ORDER MATTERS: bwrap applies the operations IN THE ORDER GIVEN.
/// `--tmpfs {home}` must come first and `--bind {sandbox} {sandbox}` after — in
/// production the sandbox is often UNDER the home directory and in the reverse
/// order the tmpfs would COVER it (the script would see its own working folder empty).
///
/// `--dev`/`--proc` are required for the interpreters (node wants
/// `/dev/urandom`, python probes `/proc`); `--proc` also wants `--unshare-pid`.
///
/// NOT MEASURED ON THIS MACHINE: there is no Linux here. If it is wrong,
/// `verify_shield` fails and the tool leaves the catalog; it does not produce a
/// silent hole. What DOES run these exact arguments is the ubuntu leg of CI
/// (see `SANDBOX_MUST_RUN`) — the argument-order test below has always checked
/// the LIST, and only that job checks that the kernel accepts it.
///
/// THE SOCKETS (audit H7). `--unshare-net` separates IP and the ABSTRACT unix
/// namespace; it does nothing to a unix socket that is a FILE. `--ro-bind / /`
/// made every such file visible, and `connect` on a socket is not a write, so a
/// read-only mount does not stop it: the session bus (`systemd-run --user`
/// starts a process outside the sandbox, with the network and past
/// `--die-with-parent`), `docker.sock` (root on the host), an ssh-agent, an X11
/// display (keystrokes into the user's terminal), a tmux server (`tmux -S …
/// send-keys`). `shadows` covers them, in two shapes:
///   * `Shadow::Dir`    -> `--tmpfs {dir}`: the runtime directories that exist
///     to hold sockets (`/run`, `/var/run`, `$XDG_RUNTIME_DIR`, `/tmp/.X11-unix`);
///   * `Shadow::Socket` -> `--ro-bind /dev/null {socket}`: a single socket
///     living in a directory that also holds ordinary files (`/tmp`, the
///     working folder, `/var/lib/incus`), where a tmpfs over the whole
///     directory would hide more than the socket. A bind of a device file over
///     a socket file is legal (both are non-directories) and `connect` on the
///     result fails.
///
/// WHAT IS COVERED: the runtime directories, the daemon sockets in
/// `DAEMON_SOCKETS`, every socket `/proc/net/unix` lists as bound in this
/// network namespace (`bound_sockets_from` — any directory, any depth), and a
/// two-level scan of `/tmp`, `$TMPDIR` and the working folder (see
/// `socket_scan_roots`). WHAT IS STILL OPEN, STATED AS A LIMIT: a socket bound
/// in ANOTHER network namespace (a container's, visible on the host's
/// filesystem) is not in this namespace's list, and is shadowed only if one of
/// the other rules names it. None of this has been run on Linux by the person
/// who wrote it (there is no Linux here); the ubuntu CI leg is where
/// `verify_shield` measures it.
///
/// They go AFTER the home tmpfs and BEFORE the sandbox bind, for the same
/// reason the home tmpfs does.
fn bwrap_args(sandbox: &Path, home: Option<&Path>, shadows: &[Shadow]) -> Vec<OsString> {
    let mut a: Vec<OsString> = vec![
        "--unshare-net".into(),
        "--unshare-pid".into(),
        "--unshare-ipc".into(),
        "--unshare-uts".into(),
        // So the child does not stay behind if the shell is killed (the second
        // layer of the `kill`+`wait` discipline on the timeout path).
        "--die-with-parent".into(),
        "--ro-bind".into(),
        "/".into(),
        "/".into(),
        "--dev".into(),
        "/dev".into(),
        "--proc".into(),
        "/proc".into(),
    ];
    // The home directory is covered. `/` and a relative value are EXCLUDED: `--tmpfs /`
    // empties the whole root directory and the interpreter does not start at all —
    // that is a crash, not a protection.
    if let Some(home) = home
        && home.is_absolute()
        && home != Path::new("/")
    {
        a.push("--tmpfs".into());
        a.push(home.into());
    }
    for shadow in shadows {
        match shadow {
            // The same `/` guard as the home directory, for the same reason.
            Shadow::Dir(dir) if dir.is_absolute() && dir != Path::new("/") => {
                a.push("--tmpfs".into());
                a.push(dir.into());
            }
            Shadow::Socket(socket) if socket.is_absolute() => {
                a.push("--ro-bind".into());
                a.push("/dev/null".into());
                a.push(socket.into());
            }
            _ => {}
        }
    }
    a.push("--bind".into());
    a.push(sandbox.into());
    a.push(sandbox.into());
    a.push("--chdir".into());
    a.push(sandbox.into());
    a
}

/// One thing the Linux shield covers so the script cannot `connect` to it (see
/// `bwrap_args`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Shadow {
    Dir(PathBuf),
    Socket(PathBuf),
}

/// The directories that exist to hold sockets, unresolved. `/var/run` is a link
/// to `/run` on every current distribution; `plan_shadows` resolves and
/// deduplicates, so it costs one `canonicalize`.
fn socket_dirs(xdg_runtime: Option<&std::ffi::OsStr>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = ["/run", "/var/run", "/tmp/.X11-unix"]
        .iter()
        .map(PathBuf::from)
        .collect();
    if let Some(xdg) = xdg_runtime.filter(|x| !x.is_empty()) {
        dirs.push(PathBuf::from(xdg));
    }
    dirs
}

/// Daemon sockets that live OUTSIDE the runtime directories, so neither a
/// `Shadow::Dir` nor the `/tmp` scan reaches them (audit H7, review of 10 Oct
/// 2026). Each is root on the host to whoever may `connect`: the LXD and Incus
/// APIs can exec in a container that HAS the network, or start a privileged
/// one that mounts the host's `/`. Ubuntu Server ships LXD as a snap and puts
/// the first user in the `lxd` group, and bwrap here keeps the user's groups
/// (there is no `--unshare-user`), so the group permission does not stop the
/// script. `--ro-bind / /` is what exposed them, and `connect` is not a write.
///
/// Paths from LXD's and Incus's own documentation, not measured: there is no
/// Linux machine where this was written. The `-user` / `.user` sockets are the
/// restricted per-user endpoints; restricted is still a container API.
/// They are shadowed one by one, never by a tmpfs over the directory:
/// `/var/lib/incus` also holds the containers' storage, and a missing entry is
/// dropped by `plan_shadows`, so listing a daemon this host lacks costs a stat.
///
/// Multipass (sudo-group access on Ubuntu: VM launch and host mounts) and
/// MicroK8s's containerd were found missing by the second review of the same
/// day, from their documentation too. The list is the floor, not the coverage:
/// `bound_sockets` reads every socket the kernel has bound, wherever it is.
const DAEMON_SOCKETS: &[&str] = &[
    "/var/snap/lxd/common/lxd/unix.socket",
    "/var/snap/lxd/common/lxd-user/unix.socket",
    "/var/lib/lxd/unix.socket",
    "/var/lib/incus/unix.socket",
    "/var/lib/incus/unix.socket.user",
    "/var/snap/multipass/common/multipass_socket",
    "/var/snap/microk8s/common/run/containerd.sock",
];

/// The filesystem paths of the unix sockets bound in this network namespace,
/// read from the text of `/proc/net/unix`.
///
/// WHY THE KERNEL'S LIST AND NOT A LONGER HAND-KEPT ONE (review of 10 Oct
/// 2026): `DAEMON_SOCKETS` and the two-level scan cover the places someone
/// thought of, and every review found another daemon outside them. The kernel
/// lists every bound socket — its `Path` column — whatever directory it is in
/// and however deep. Abstract sockets (`@name`) are skipped: they live in the
/// network namespace, which `--unshare-net` already replaces. Only absolute
/// paths are kept; a relative one names nothing the child can resolve.
///
/// Format (`net/unix/af_unix.c`): a header line, then `Num RefCount Protocol
/// Flags Type St Inode Path`, the path last and optional, and it may contain
/// spaces — so it is the rest of the line after the seventh field.
fn bound_sockets_from(proc_net_unix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for line in proc_net_unix.lines().skip(1) {
        let mut rest = line.trim_start();
        for _ in 0..7 {
            let Some(end) = rest.find(char::is_whitespace) else {
                rest = "";
                break;
            };
            rest = rest[end..].trim_start();
        }
        if rest.starts_with('/') {
            let path = PathBuf::from(rest.trim_end());
            if !out.contains(&path) {
                out.push(path);
            }
        }
    }
    out
}

/// What the shield plan reads from the host, gathered in one place so the plan
/// itself (`shadows_for`) is a pure function a test can feed fixtures: the
/// environment is process-wide and a test that changed it would race the
/// others.
#[derive(Debug, Clone, Default)]
struct ShadowHost {
    xdg_runtime: Option<OsString>,
    tmpdir: Option<OsString>,
    ssh_auth_sock: Option<OsString>,
    daemon_sockets: Vec<PathBuf>,
    /// Every socket the kernel lists as bound (`bound_sockets_from`), on
    /// Linux; empty where there is no `/proc/net/unix`.
    bound_sockets: Vec<PathBuf>,
}

impl ShadowHost {
    fn from_env() -> ShadowHost {
        ShadowHost {
            xdg_runtime: std::env::var_os("XDG_RUNTIME_DIR"),
            tmpdir: std::env::var_os("TMPDIR"),
            ssh_auth_sock: std::env::var_os("SSH_AUTH_SOCK"),
            daemon_sockets: DAEMON_SOCKETS.iter().map(PathBuf::from).collect(),
            // Only what IS a socket now: a path the kernel still lists after
            // it was unlinked and replaced by a directory would make bwrap's
            // file-over-directory bind fail, and with it the whole shield.
            bound_sockets: std::fs::read_to_string("/proc/net/unix")
                .map(|t| bound_sockets_from(&t))
                .unwrap_or_default()
                .into_iter()
                .filter(|p| is_socket_now(p))
                .collect(),
        }
    }
}

/// Whether `path` is a unix socket right now, without following a link.
fn is_socket_now(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        path.symlink_metadata()
            .is_ok_and(|m| m.file_type().is_socket())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// Everything the Linux shield covers for one run in `sandbox` (see
/// `bwrap_args`): the runtime directories, the known daemon sockets, the ssh
/// agent, and the loose sockets found by scanning the temp directories and the
/// folder the sandbox was made in.
fn shadows_for(sandbox: &Path, home: Option<&Path>, host: &ShadowHost) -> Vec<Shadow> {
    let roots = socket_scan_roots(host.tmpdir.as_deref(), sandbox.parent(), home);
    let mut sockets = host.daemon_sockets.clone();
    sockets.extend(host.bound_sockets.iter().cloned());
    sockets.extend(scan_sockets(&roots, home));
    plan_shadows(
        socket_dirs(host.xdg_runtime.as_deref()),
        sockets,
        host.ssh_auth_sock.as_ref().map(PathBuf::from),
        home,
    )
}

/// The directories scanned for loose sockets: `/tmp`, `$TMPDIR` when it is
/// somewhere else, and `workdir` — the folder the sandbox is made in — when it
/// is NOT under the home directory.
///
/// THE WORKING FOLDER WAS MISSING (audit H7, review of 10 Oct 2026). A sandbox
/// made in `/srv/app` sits next to whatever that folder holds, and
/// `--ro-bind / /` shows all of it, a `gunicorn.sock` or a `.s.PGSQL.5432`
/// included. Under the home directory there is nothing to scan: the home tmpfs
/// already hides the folder, and the sandbox bind brings back only the sandbox.
fn socket_scan_roots(
    tmpdir: Option<&std::ffi::OsStr>,
    workdir: Option<&Path>,
    home: Option<&Path>,
) -> Vec<PathBuf> {
    let mut roots = vec![PathBuf::from("/tmp")];
    if let Some(tmp) = tmpdir.filter(|t| !t.is_empty()) {
        roots.push(PathBuf::from(tmp));
    }
    if let Some(work) = workdir.filter(|w| w.is_absolute() && *w != Path::new("/")) {
        let real = work.canonicalize().unwrap_or_else(|_| work.to_path_buf());
        if !home.is_some_and(|h| real.starts_with(h)) {
            roots.push(real);
        }
    }
    roots
}

/// How deep `scan_sockets` looks, and how many directory entries it reads in
/// total before it stops. Two levels is where the sockets that matter live —
/// `/tmp/tmux-1000/default`, `/tmp/ssh-XXXX/agent.N`, `/tmp/.s.PGSQL.5432`,
/// `/tmp/dotnet-diagnostic-…-socket` — and the entry budget keeps a /tmp with a
/// million files from turning every call into a directory walk.
///
/// THE BUDGET IS PER ROOT. It was one budget shared by every root, and the
/// working folder was pushed last onto a stack and so walked FIRST: a folder
/// outside $HOME with more than ~4,096 entries in two levels (any project with
/// a `node_modules`) spent the whole budget before /tmp was opened, and every
/// tmux, ssh-agent and `.s.PGSQL` socket there dropped out of the plan (review
/// of 10 Oct 2026, measured on macOS with 5,000 files: the /tmp socket was
/// shadowed beside a small folder and not beside the big one).
///
/// THE LIMIT IS A GAP AND IS STATED AS ONE: a socket deeper than two levels, or
/// past the first 4,096 entries of its root, is not shadowed — unless the
/// kernel lists it as bound (`bound_sockets`), which reaches every depth. Covering all of `/tmp` with a
/// tmpfs instead would close it, but would also turn the child's view of a
/// write outside the sandbox from "refused" into "accepted and discarded" —
/// a different guarantee from the one `the_sandbox_cannot_be_escaped` holds
/// bwrap to, and one that cannot be measured on the machine this was written on.
const SOCKET_SCAN_DEPTH: usize = 2;
const SOCKET_SCAN_BUDGET: usize = 4_096;

/// The unix sockets reachable under `roots`, at most `SOCKET_SCAN_DEPTH`
/// levels down.
///
/// A directory this process cannot read is skipped, and that is not a gap: a
/// socket inside a directory we cannot search is a socket our own `connect`
/// cannot reach either, so the child — the same uid — cannot reach it.
/// Nothing is followed: `symlink_metadata` throughout, so a link in `/tmp`
/// cannot send the walk somewhere else. `home` is not entered; its tmpfs covers
/// everything inside it already.
fn scan_sockets(roots: &[PathBuf], home: Option<&Path>) -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let mut found = Vec::new();
        for root in roots.iter().filter_map(|r| r.canonicalize().ok()) {
            let mut budget = SOCKET_SCAN_BUDGET;
            let mut queue: Vec<(PathBuf, usize)> = vec![(root, 0)];
            'root: while let Some((dir, depth)) = queue.pop() {
                if home.is_some_and(|h| dir.starts_with(h)) {
                    continue;
                }
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    if budget == 0 {
                        break 'root;
                    }
                    budget -= 1;
                    let Ok(kind) = entry.file_type() else {
                        continue;
                    };
                    if kind.is_socket() {
                        found.push(entry.path());
                    } else if kind.is_dir() && depth + 1 < SOCKET_SCAN_DEPTH {
                        queue.push((entry.path(), depth + 1));
                    }
                }
            }
        }
        found.sort();
        found.dedup();
        found
    }
    #[cfg(not(unix))]
    {
        let _ = (roots, home);
        Vec::new()
    }
}

/// Turns the candidates into the list `bwrap_args` covers: resolved, existing,
/// deduplicated, and nothing that is already covered by something else.
///
/// A socket under a covered directory (or under the home directory) is dropped:
/// after the tmpfs it does not exist, and asking bwrap to bind over it would
/// make bwrap CREATE it first — a file nobody needs. A missing directory is
/// dropped too, and that one matters: `--tmpfs` on a path that does not exist
/// makes bwrap `mkdir` it inside a READ-ONLY root, which fails and takes the
/// whole shield down with it.
fn plan_shadows(
    dirs: Vec<PathBuf>,
    sockets: Vec<PathBuf>,
    ssh_agent: Option<PathBuf>,
    home: Option<&Path>,
) -> Vec<Shadow> {
    let mut covered: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let Ok(real) = dir.canonicalize() else {
            continue;
        };
        if !real.is_dir() || real == Path::new("/") {
            continue;
        }
        if home.is_some_and(|h| real.starts_with(h)) {
            continue;
        }
        if covered.iter().any(|c| real.starts_with(c)) {
            continue;
        }
        // A directory that CONTAINS one already listed replaces it.
        covered.retain(|c| !c.starts_with(&real));
        covered.push(real);
    }
    let mut shadows: Vec<Shadow> = covered.iter().cloned().map(Shadow::Dir).collect();

    let mut loose: Vec<PathBuf> = Vec::new();
    for socket in sockets
        .into_iter()
        .chain(ssh_agent)
        .filter(|s| s.is_absolute())
    {
        // The PARENT is resolved, not the socket: a socket path that is itself a
        // link would otherwise be replaced by its target and the link — the name
        // the script would use — left reachable.
        let (Some(parent), Some(name)) = (socket.parent(), socket.file_name()) else {
            continue;
        };
        let Ok(real_parent) = parent.canonicalize() else {
            continue;
        };
        let real = real_parent.join(name);
        if real.symlink_metadata().is_err() {
            continue;
        }
        if home.is_some_and(|h| real.starts_with(h)) || covered.iter().any(|c| real.starts_with(c))
        {
            continue;
        }
        if !loose.contains(&real) {
            loose.push(real);
        }
    }
    shadows.extend(loose.into_iter().map(Shadow::Socket));
    shadows
}

/// The sandbox-exec profile.
///
/// READING IS AN ALLOWLIST (audit L20). It used to be `(allow file-read*)` minus
/// the home directory, on the belief that "narrowed down, python3 does not
/// start at all". That left everything OUTSIDE the home directory readable —
/// MEASURED by the audit: the user's temp directory (46,797 entries), the
/// temporary copies `db_write` makes, `/Users/Shared` and `/private/tmp` — while
/// `description()` said "NO access to ANY file". The belief was measured too,
/// on this machine (macOS 26.5.2, 9 Oct 2026): homebrew node 26.5.0 and homebrew
/// python 3.12 both start under the list below, and python imports json,
/// sqlite3, ssl, zoneinfo, socket and subprocess, compiles with py_compile and
/// reports the local timezone; node reports the timezone through Intl. What
/// does NOT start is `/usr/bin/python3`, the Xcode shim — and it did not start
/// under the old profile either (it needs fork, see below).
///
/// Every entry is software, not user data: the OS (`/System`, `/usr`, `/bin`,
/// `/sbin`, `/private/etc`, `/dev`), the timezone and dyld databases, and the
/// three places this file looks for interpreters (`/usr/local` is under
/// `/usr`, Homebrew's software directories, and python.org's
/// `/Library/Frameworks`). `/Library/Developer/CommandLineTools` is the Xcode
/// shim's own lookup.
///
/// HOMEBREW IS LISTED BY ITS SOFTWARE DIRECTORIES, NOT AS `/opt/homebrew`. The
/// whole prefix was on this list once, and it holds user data too: `var/` is
/// where `brew services` keep their state and `etc/` their configuration.
/// MEASURED by the audit's review (9 Oct 2026): from inside the sandbox a script
/// listed and read `/opt/homebrew/var/postgresql@17` — 894 files, 22.9 MB of a
/// live database, `pg_hba.conf` included — and a properties file under `etc/`
/// that can hold a token. The Intel prefix, `/usr/local`, sits under `/usr` and
/// cannot be narrowed the same way, so its `var/` and `etc/` are DENIED after
/// the allow instead (SBPL: the later rule wins).
///
/// ONE FILE UNDER `etc/` IS LET BACK IN, AND IT IS NOT OPTIONAL: Homebrew node
/// reads OpenSSL's `openssl.cnf` at start, and when the open fails with EPERM
/// (rather than ENOENT) it dies — MEASURED, macOS 26.5.2, node 26.5.0: "OpenSSL
/// configuration error: BIO_new_file: Operation not permitted" under the
/// narrowed list, and every `run_code` call would have failed. With only that
/// file re-allowed, node and homebrew python 3.12 both start, python imports
/// json, sqlite3, ssl and zoneinfo and `py_compile` passes, `node --check`
/// passes, and `/opt/homebrew/var`, `/opt/homebrew/etc` and `openssl@3/cert.pem`
/// all give EPERM. The regex names the config file and nothing beside it.
/// `(literal "/")` lets a `readdir` of the root succeed, which some runtimes do
/// on start; it lists names, not contents. The home-directory deny is KEPT
/// even though the allowlist no longer covers `/Users`: it is the rule that
/// still holds if a future entry ever does.
///
/// `(allow file-read-metadata)` WAS ADDED LATER and closes a failure: before
/// running a script, node calls `realpathSync`, which `lstat`s EVERY parent
/// directory ON THE SCRIPT'S PATH — if the sandbox is under the home directory
/// (and in production it is, the working directory is the sandbox) the home
/// directory's turn comes, `deny file-read*` cuts it and node crashes with
/// `EPERM: lstat '/Users/...'` before reading even the first line. That is what
/// was seen in the field: because js is the preferred interpreter, `run_code`
/// returned "the script returned an error" on EVERY call and the model wrote
/// the list from memory instead. The metadata permission DOES NOT OPEN CONTENT
/// — that a file exists and its size are visible, reading its content is still
/// forbidden. The alternative would have been "take node off the list"; that
/// would have crippled the tool instead of closing a gate.
///
/// THE PROFILE IS A CONSTANT AND CARRIES NO PATH. It used to `format!` the
/// sandbox path and `HOME` straight into the text, and SBPL string escapes were
/// never applied — a directory named `evil")) (allow default) ;` (macOS allows
/// those characters in a name, and a zip or repo can carry one) closed the
/// quote, added its own rule and commented out THE REST OF THE PROFILE, network
/// deny included. MEASURED: under the spliced form that directory gave
/// `OUTSIDE_WRITE_OK` + `NET_OPEN` and the file outside really appeared; under
/// the form below the same directory gives `OUTSIDE_WRITE_BLOCKED` +
/// `NET_BLOCKED`. DO NOT try to fix this by escaping — SBPL string escaping is
/// version dependent and fails silently. `(param "...")` is the prepared
/// statement of this world: the value is handed to `sandbox-exec` as a separate
/// `-D KEY=value` argv element and never passes through the parser as text.
///
/// `(allow mach-lookup)` IS NOT LEFT UNFILTERED. Unfiltered it opens XPC to
/// every system Mach service, the pasteboard among them, and the model could
/// then run `/usr/bin/pbpaste` and print the user's clipboard — typically the
/// password just copied out of a password manager — straight into its own
/// window, while `description()` promises "NO access to this device". MEASURED:
/// node and python3 both start under the whitelist below and `pbpaste` dies.
///
/// `(allow process-fork)` WAS REMOVED, and that is the root fix for the timeout
/// gate. With fork allowed the script could `spawn('/bin/sleep', {detached:true})`;
/// node makes such a child its own session leader, so it survived the kill,
/// KEPT THE STDOUT PIPE OPEN and the turn hung forever. MEASURED: node and
/// python3 (homebrew) both start without this permission, `node --check` and
/// `python3 -m py_compile` both pass, and the detached spawn fails with
/// `SPAWN_BLOCKED`. It also puts macOS on a par with the Linux path, where
/// `--unshare-pid` already tears down the whole namespace with the child.
/// KNOWN COST, ACCEPTED: `/usr/bin/python3` on macOS is an Xcode shim that
/// spawns the real interpreter and therefore needs fork. On a machine whose
/// ONLY interpreter is that shim, `verify_shield` will now fail and the tool
/// leaves the catalog — fail-closed with a reason in `diagnose()`, which is this
/// file's rule, not a silent hole. Spawning processes was never part of what
/// this tool promises.
///
/// `(allow signal)` IS TARGETED (audit M1). Unfiltered, the script could signal
/// ANY process of the user: MEASURED by the audit, a `sleep 120` started
/// outside died with 143 from a SIGTERM sent inside, and `process.kill(-1)`
/// would take down tacet and every editor. `(target self)` is all an
/// interpreter needs — node and python both ran the probes above under it — and
/// `kill(<tacet's pid>, 0)` from inside now returns EPERM (measured, both
/// interpreters), which `verify_shield` checks on every start.
fn profile() -> &'static str {
    "(version 1)\
     (deny default)\
     (allow process-exec*)\
     (allow file-read* \
       (literal \"/\") \
       (subpath \"/System\") \
       (subpath \"/usr\") \
       (subpath \"/bin\") \
       (subpath \"/sbin\") \
       (subpath \"/private/etc\") \
       (subpath \"/private/var/db/timezone\") \
       (subpath \"/private/var/db/dyld\") \
       (subpath \"/dev\") \
       (subpath \"/opt/homebrew/Cellar\") \
       (subpath \"/opt/homebrew/opt\") \
       (subpath \"/opt/homebrew/bin\") \
       (subpath \"/opt/homebrew/lib\") \
       (subpath \"/opt/homebrew/Frameworks\") \
       (subpath \"/opt/homebrew/share\") \
       (subpath \"/Library/Frameworks\") \
       (subpath \"/Library/Developer/CommandLineTools\"))\
     (deny file-read* \
       (subpath \"/usr/local/var\") \
       (subpath \"/usr/local/etc\"))\
     (allow file-read* \
       (regex #\"^/(opt/homebrew|usr/local)/etc/openssl@[0-9.]+/openssl\\.cnf$\"))\
     (deny file-read* (subpath (param \"HOMEDIR\")))\
     (allow file-read-metadata)\
     (allow file-read* (subpath (param \"SBOX\")))\
     (allow file-write* (subpath (param \"SBOX\")))\
     (allow file-write-data (literal \"/dev/null\"))\
     (allow sysctl-read)\
     (allow mach-lookup \
       (global-name \"com.apple.system.opendirectoryd.libinfo\") \
       (global-name \"com.apple.system.notification_center\") \
       (global-name \"com.apple.CoreServices.coreservicesd\") \
       (global-name \"com.apple.SystemConfiguration.configd\"))\
     (allow ipc-posix-shm)\
     (allow signal (target self))\
     (deny network*)"
}

/// The parameter names the profile reads. They are spelled out in the profile
/// text above too; changing one without the other makes `sandbox-exec` refuse
/// the profile, which is loud — the failure mode we want.
const PARAM_SANDBOX: &str = "SBOX";
const PARAM_HOME: &str = "HOMEDIR";

/// Where the home-directory rule points when `HOME` is unusable: the directory
/// every home lives under. An EMPTY value would make the deny rule match nothing
/// and quietly open the user's whole home directory.
#[cfg(target_os = "macos")]
const FALLBACK_HOME: &str = "/Users";
#[cfg(not(target_os = "macos"))]
const FALLBACK_HOME: &str = "/home";

/// The home directory AS THE KERNEL SEES IT (audit M2).
///
/// `sandbox-exec` matches `(subpath ...)` against the RESOLVED path of every
/// access, so a deny written with a link in it matches nothing. `$HOME` went in
/// raw: on a machine whose home is a link (a moved home, `/Users/me ->
/// /Volumes/Data/me`) the deny rule was dead — MEASURED by the audit: with the
/// canonical HOMEDIR the read gave EPERM, with the link alias a planted
/// `TOP-SECRET-DUMMY` file was read. bwrap has the mirror problem: `--tmpfs`
/// over a link covers the link, not the directory behind it.
///
/// A home that does not exist cannot be resolved; its raw value is kept, since
/// there is nothing behind it to protect and an unresolved deny is still
/// stricter than the fallback. A relative value is not a home at all.
fn canonical_home(raw: Option<OsString>) -> PathBuf {
    let raw = match raw {
        Some(h) if !h.is_empty() && Path::new(&h).is_absolute() => PathBuf::from(h),
        _ => return PathBuf::from(FALLBACK_HOME),
    };
    raw.canonicalize().unwrap_or(raw)
}

/// FAIL-CLOSED, second layer. `(param ...)` already makes splicing impossible;
/// this exists so that a future patch "simplifying" the profile back into a
/// `format!` cannot silently reopen the hole.
///
/// ONLY THE CHARACTERS THAT CAN BREAK OUT OF AN SBPL STRING ARE REFUSED — the
/// quote, the backslash, control characters, and a path that is not UTF-8 at
/// all. `(`, `)` and `;` are DELIBERATELY ALLOWED: on their own they are inert
/// inside a quoted string, and macOS users really do have folders called
/// `Project (old)`. Refusing those would turn a hardening layer into a bug
/// report.
fn path_is_shield_safe(path: &Path) -> bool {
    let Some(text) = path.to_str() else {
        return false; // not UTF-8: we cannot reason about it, so we refuse
    };
    !text
        .chars()
        .any(|c| c.is_control() || matches!(c, '"' | '\\'))
}

// ---------------------------------------------------------------------------
// Interpreter discovery
// ---------------------------------------------------------------------------

/// An interpreter found on the machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interpreter {
    /// The `Choice` value in the schema and the name visible to the model.
    pub key: &'static str,
    pub path: PathBuf,
    pub extension: &'static str,
}

/// The interpreters looked for, IN ORDER OF PREFERENCE.
///
/// JS comes first because the skill file (`code.md`) and the Swift side are
/// built on JS; the same prompt should produce the same language in both ports.
/// If python is present it is a real second option — a gain Swift did not have,
/// and the `language` field exists exactly for that (see `schema`).
const CANDIDATES: &[(&str, &str, &[&str])] = &[
    (
        "js",
        "js",
        &[
            "/opt/homebrew/bin/node",
            "/usr/local/bin/node",
            "/usr/bin/node",
        ],
    ),
    (
        "python",
        "py",
        &[
            "/opt/homebrew/bin/python3",
            "/usr/local/bin/python3",
            "/usr/bin/python3",
        ],
    ),
];

fn discover_interpreters() -> Vec<Interpreter> {
    let mut found = Vec::new();
    for (key, extension, paths) in CANDIDATES {
        // PATH IS NOT SEARCHED, fixed paths are used. Rationale: `PATH` comes
        // from the calling process's environment and can be poisoned; finding
        // something called "node" on PATH and running it is not behaviour that
        // suits a tool which runs model output.
        if let Some(y) = paths.iter().map(Path::new).find(|y| y.is_file()) {
            found.push(Interpreter {
                key,
                path: y.to_path_buf(),
                extension,
            });
        }
    }
    found
}

// ---------------------------------------------------------------------------
// The execution outcome
// ---------------------------------------------------------------------------

/// The raw outcome of one execution (the same distinction as code-spec §5.2/§5.3).
#[derive(Debug, Clone, PartialEq)]
pub enum CodeOutcome {
    /// The script finished without an error. If `output` is EMPTY the caller MUST NOT PRESENT this as a success.
    Succeeded {
        output: String,
        ms: u128,
        truncated: bool,
    },
    /// The script returned an error (a non-zero exit code) — stderr + partial output.
    Error(String),
    /// The time ran out; the process was killed.
    Timeout,
}

/// Runs the script under the shield. Free and pure so it can be called from
/// outside the tool (test, eval): it holds no state.
pub fn run(
    shield: &NetworkShield,
    interpreter: &Interpreter,
    sandbox: &Path,
    code: &str,
    timeout: Duration,
) -> ToolResult<CodeOutcome> {
    run_until(
        shield,
        interpreter,
        sandbox,
        code,
        timeout,
        &CancelToken::new(),
    )
}

/// `run`, listening to `cancel`: a fired token ends the script's process group
/// (SIGTERM, then SIGKILL) and the call returns an error saying it was stopped.
pub fn run_until(
    shield: &NetworkShield,
    interpreter: &Interpreter,
    sandbox: &Path,
    code: &str,
    timeout: Duration,
    cancel: &CancelToken,
) -> ToolResult<CodeOutcome> {
    if code.trim().is_empty() {
        return Err(ToolError::InvalidArgument("empty script".into()));
    }
    if code.len() > CODE_CAP {
        return Err(ToolError::InvalidArgument("the script is too long".into()));
    }

    // THE SANDBOX IS A FRESH DIRECTORY FOR THIS ONE CALL (audit M5), created
    // under `sandbox` and deleted after it. It used to be `<sandbox>/code`: an
    // ORDINARY name, so a user's real `code/` directory became readable and
    // writable by the model's script, and a `code` that was a link sent the
    // script to the link's target (`create_dir_all` follows it, and
    // `O_NOFOLLOW` only guards the last component). Now the name is new on
    // every call, `mkdir` refuses one that exists, and no component of the
    // path may be a link (`sandbox_path::fresh_private_dir`).
    //
    // The script is written INSIDE it: the shield profile only lets that place
    // be read, so keeping the script anywhere else would make it unreadable.
    if !path_is_shield_safe(sandbox) {
        return Err(ToolError::SandboxViolation(sandbox.to_path_buf()));
    }
    let runtime = crate::sandbox_path::fresh_private_dir(sandbox, &nonce())?;
    let script = runtime.join(format!("script-{}.{}", nonce(), interpreter.extension));
    let outcome = match create_script(&script, code.as_bytes()) {
        Ok(()) => run_program_until(
            shield,
            &runtime,
            &interpreter.path,
            &[script.clone().into()],
            timeout,
            cancel,
        ),
        Err(e) => Err(ToolError::Io(e)),
    };
    // THE WHOLE DIRECTORY GOES, ON EVERY PATH: if model output piles up on
    // disk the working folder gets dirty and `find_file` lists it as if it
    // were user documents. `remove_dir_all` does not follow a link the script
    // planted inside — it unlinks it (std has done this race-free on unix
    // since 1.58.1).
    std::fs::remove_dir_all(&runtime).ok();
    outcome
}

/// A per-call name fragment that an ATTACKER CANNOT PREDICT.
///
/// The old names were `script-{pid}-{Instant::now().elapsed().as_nanos() ^ len}`
/// and `verify-{pid}-{code.len()}` — fully determined by values the model
/// controls or can read. (`Instant::now().elapsed()` measures the time between
/// two adjacent statements: MEASURED on this machine it only ever returned 0,
/// 41 or 42 nanoseconds, i.e. three candidates.) Under the shield the script can
/// read its own path (`__filename` / `sys.argv[0]`), learn the pid, and plant a
/// symlink at the NEXT call's name — a link created inside the sandbox, which
/// the profile permits. The parent process then wrote through it with the user's
/// full privileges.
///
/// THE REAL BARRIER IS `create_script`, NOT THIS. Wall-clock nanoseconds plus a
/// process-wide counter are only defence in depth, so the attacker cannot even
/// aim.
pub(crate) fn nonce() -> String {
    static SEQUENCE: AtomicUsize = AtomicUsize::new(0);
    let step = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{now:x}-{step:x}", std::process::id())
}

/// Writes the script WITHOUT EVER FOLLOWING an existing path.
///
/// THE ATTACK THIS CLOSES: a script running under the shield may create a
/// symlink inside the sandbox (the profile allows writing there, and a link IS a
/// write). Pointed at `~/.zshrc`, `~/Library/LaunchAgents/*.plist` or
/// `~/.ssh/authorized_keys`, the PARENT process — which is `tacet` itself, with
/// no shield and the user's full privileges — used to `std::fs::write` straight
/// through it. That is arbitrary file write outside the sandbox and, with any of
/// those three targets, persistent code execution as the user.
///
/// `create_new` is `O_CREAT|O_EXCL`, which POSIX requires to fail with `EEXIST`
/// when the path names an existing symlink — DANGLING OR NOT — so the link is
/// refused instead of followed. `O_NOFOLLOW` is a second belt for the case where
/// the final component is a link.
///
/// WINDOWS HAS ONLY THE FIRST BELT, AND IT HOLDS — measured rather than assumed.
/// `custom_flags` is `#[cfg(unix)]`, so `O_NOFOLLOW` is never applied there (it
/// used to be a `dead_code` constant on Windows that no CI job failed on,
/// because `clippy -D warnings` runs only on ubuntu). The paragraph above rested
/// the whole guarantee on what POSIX requires, and Windows is not POSIX — so the
/// claim covered the one platform it did not apply to.
///
/// MEASURED on Windows Server 2019, 4 Sep 2026: a real `SymbolicLink` at the
/// target path, opened with exactly these options, gives
/// `The file exists. (os error 80)` / `ErrorKind::AlreadyExists`, and the file
/// the link pointed at is untouched afterwards. `CREATE_NEW` refuses a name that
/// exists, and a symlink is such a name. The second belt is genuinely redundant
/// there rather than merely absent.
///
/// `a_dangling_symlink_does_not_deliver_the_file_outside` in `write_code.rs` is
/// `#[cfg(unix)]` and does not compile on Windows, so nothing in the suite states
/// this. It is written here instead of left to the next person to re-derive.
pub(crate) fn create_script(script: &Path, code: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    // THE BIT IS PER ARCHITECTURE, from the table `create_document` keeps. This
    // file had its own `0x20000` for every Linux, which on aarch64 — a published
    // target — is `O_LARGEFILE`, so the second belt was silently absent there
    // (audit L15 review; the first belt, `create_new`, still held).
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(crate::create_document::o_nofollow_for(
            std::env::consts::OS,
            std::env::consts::ARCH,
        ));
    }
    let mut file = options.open(script)?;
    file.write_all(code)
}

/// The command `run_program` spawns, built from the RAW `$HOME` value.
///
/// WHY THE RAW VALUE COMES IN AS A PARAMETER (review of 10 Oct 2026): the M2
/// test used to check `canonical_home` on its own, so `run_program` going back
/// to splicing raw `$HOME` into `HOMEDIR=` would have stayed green on every CI
/// machine, none of which has a symlinked home. The canonicalisation now
/// happens HERE, on the path `run_program` takes, and the test drives this
/// function with a symlinked fixture — `HOME` itself is process-wide and
/// cannot be changed from a test without breaking the tests beside it.
fn shielded_command(
    shield: &NetworkShield,
    runtime: &Path,
    program: &Path,
    args: &[OsString],
    raw_home: Option<OsString>,
) -> ToolResult<Command> {
    // FAIL-CLOSED BEFORE THE SHIELD IS BUILT (see `path_is_shield_safe`): if a
    // path cannot be safely represented we do not run at all. Refusing is the
    // natural extension of this file's rule that a shield we cannot set up means
    // the tool does not exist.
    let home = canonical_home(raw_home);
    if !path_is_shield_safe(runtime) || !path_is_shield_safe(&home) {
        return Err(ToolError::SandboxViolation(runtime.to_path_buf()));
    }

    let mut command = shield.wrap(runtime, &home, program, args);
    command
        .current_dir(runtime)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // THE ENVIRONMENT IS WIPED. The calling process's environment holds API
    // keys, session tokens and the home directory path; all of them are within
    // reach of the model's output. `PATH` stays minimal only so the interpreter
    // can find its own subprocesses, and `HOME` is deliberately turned into the
    // sandbox so the interpreter does not go looking for the user's config files.
    command.env_clear();
    command.env("PATH", "/usr/bin:/bin");
    command.env("HOME", runtime);
    command.env("LANG", "en_US.UTF-8");
    // Turn off python's user site-packages and its cache writing: both are paths
    // reaching outside the sandbox.
    command.env("PYTHONDONTWRITEBYTECODE", "1");
    command.env("PYTHONNOUSERSITE", "1");
    command.env("NODE_OPTIONS", "");

    // ITS OWN SESSION / PROCESS GROUP. On the timeout path we must be able to
    // signal THE WHOLE TREE, not just the process we spawned: a child left
    // behind keeps the stdout/stderr pipe ends open (see the join in
    // `run_program`) and goes on burning the user's CPU after the tool has
    // already reported "timed out".
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and touches nothing but the
        // calling process's session id, which is exactly what `pre_exec`
        // permits. A failure means we are already a group leader — harmless.
        unsafe {
            command.pre_exec(|| {
                setsid();
                Ok(())
            });
        }
    }
    Ok(command)
}

/// Runs any program with the given arguments under the shield.
///
/// The generalisation of `run` and the shared path of `write_code`'s
/// verification steps: a syntax check (`node --check f`, `python3 -m
/// py_compile f`) wants a different command line from running the script
/// DIRECTLY but still wants THE SAME shield, the same environment cleaning and
/// the same timeout discipline. Had two separate runners been written, security
/// fixes would have stayed in one of them.
pub fn run_program(
    shield: &NetworkShield,
    runtime: &Path,
    program: &Path,
    args: &[OsString],
    timeout: Duration,
) -> ToolResult<CodeOutcome> {
    run_program_until(shield, runtime, program, args, timeout, &CancelToken::new())
}

/// `run_program`, listening to `cancel` (audit M4). A SEPARATE ENTRY rather than
/// a new parameter on `run_program`, so its other callers (`write_code`'s
/// syntax checks) keep compiling unchanged; a check that takes ten seconds at
/// most has less need of a Ctrl-C than a script that may loop.
pub fn run_program_until(
    shield: &NetworkShield,
    runtime: &Path,
    program: &Path,
    args: &[OsString],
    timeout: Duration,
    cancel: &CancelToken,
) -> ToolResult<CodeOutcome> {
    let mut command = shielded_command(shield, runtime, program, args, std::env::var_os("HOME"))?;

    let start = Instant::now();
    let mut child = command.spawn()?;
    #[cfg(unix)]
    let group = child.id() as i32;

    // The two pipes are read IN TWO SEPARATE THREADS. Reading them in turn on
    // one thread is the classic deadlock: while waiting for stdout to the end
    // the child fills the stderr pipe, blocks, and the two wait for each other forever.
    //
    // THE READERS WRITE INTO A SHARED BUFFER instead of returning their result,
    // so the main path can give up on them (see the bounded join) and still keep
    // everything they managed to read.
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let out_sink = std::sync::Arc::new(Mutex::new(PipeSink::default()));
    let err_sink = std::sync::Arc::new(Mutex::new(PipeSink::default()));
    let out_arm = {
        let sink = out_sink.clone();
        std::thread::spawn(move || read_pipe(stdout, &sink))
    };
    let err_arm = {
        let sink = err_sink.clone();
        std::thread::spawn(move || read_pipe(stderr, &sink))
    };

    let mut timed_out = false;
    let mut stopped = false;
    let exit = loop {
        match child.try_wait() {
            Ok(Some(state)) => break Some(state),
            Ok(None) => {}
            // If the poll errored we do not know the process; killing it and
            // leaving is better than leaving an uncertain process behind.
            Err(_) => {
                timed_out = true;
                break None;
            }
        }
        if start.elapsed() >= timeout {
            timed_out = true;
            break None;
        }
        // THE STOP IS READ ON THE CLOCK'S TICK (audit M4). Before, a Ctrl-C
        // could not reach a running script at all: it ran to its own timeout
        // while the screen said "cancelling".
        if cancel.is_cancelled() {
            stopped = true;
            break None;
        }
        std::thread::sleep(POLL_INTERVAL);
    };

    if stopped {
        // SIGTERM to the group, a bounded wait, SIGKILL, reap — `shell`'s
        // order, for the same reasons (see `TERM_GRACE`).
        #[cfg(unix)]
        unsafe {
            killpg(group, SIGTERM);
        }
        let asked = Instant::now();
        while asked.elapsed() < TERM_GRACE {
            if matches!(child.try_wait(), Ok(Some(_))) {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }
        #[cfg(unix)]
        unsafe {
            killpg(group, SIGKILL);
        }
        child.kill().ok();
        child.wait().ok();
    } else if timed_out {
        // KILL THE WHOLE GROUP, then reap. `child.kill()` alone signals only the
        // process we spawned; anything it left behind survives. `kill` without
        // `wait` leaves a zombie and in a long session the process table fills up.
        #[cfg(unix)]
        unsafe {
            killpg(group, SIGKILL);
        }
        child.kill().ok();
        child.wait().ok();
    } else {
        // AND ON THE NORMAL PATH TOO. The sweep used to run on the timeout path
        // ONLY, so a script that EXITED CLEANLY after starting a background
        // child left that child running — outside every timeout, holding the
        // sandbox directory and the pipe ends, burning CPU — while the tool
        // reported success and the turn moved on.
        //
        // Today no such child can exist: the macOS profile refuses the spawn
        // (`spawn EPERM`, measured) and Linux `bwrap` tears the pid namespace
        // down with `--die-with-parent`. This is the third layer, and it is here
        // because the first two are somebody else's configuration file. It costs
        // one syscall that returns ESRCH when the group is already empty, which
        // is every ordinary run.
        //
        // ORDER MATTERS: after the child has exited, so its own output is
        // already in the pipe, and BEFORE the bounded join, so a survivor
        // holding the pipe ends cannot make the readers wait out their deadline.
        #[cfg(unix)]
        unsafe {
            killpg(group, SIGKILL);
        }
    }
    let ms = start.elapsed().as_millis();

    // THE JOIN IS BOUNDED, AND THAT IS NOT BELT AND BRACES.
    //
    // The comment that used to stand here claimed the readers "get EOF because
    // the child is gone, so join does not hang". THAT IS WRONG, and it was
    // measured wrong: a pipe end closes when the LAST process holding it dies,
    // not when the process we spawned dies. Any surviving descendant keeps the
    // fd open, `read` never returns 0, and an unbounded `join` blocks the tool
    // call — and because this runs synchronously inside the async body, gate 4
    // (cancellation) cannot rescue the user either; the turn is simply lost.
    // Removing `(allow process-fork)` from the profile takes the macOS way of
    // producing such a descendant away, but the bound stays: we must never again
    // depend on "nothing can be holding this pipe".
    //
    // Whatever the reader managed to store IS KEPT — it lives in the shared
    // buffer, not in the thread's return value — and an abandoned thread is left
    // to finish on its own.
    join_before(out_arm, JOIN_GRACE);
    join_before(err_arm, JOIN_GRACE);
    let (output, truncated_out) = drain(&out_sink);
    let (error_text, truncated_err) = drain(&err_sink);

    if stopped {
        // AN ERROR, NOT A NEW `CodeOutcome` VARIANT: `write_code` matches that
        // enum exhaustively, and a stopped run is, from its side, a run that
        // produced nothing to check.
        return Err(ToolError::Other(STOPPED.into()));
    }
    if timed_out {
        return Ok(CodeOutcome::Timeout);
    }
    let Some(state) = exit else {
        return Ok(CodeOutcome::Timeout);
    };

    if state.success() {
        return Ok(CodeOutcome::Succeeded {
            output: output.trim_end().to_string(),
            ms,
            truncated: truncated_out,
        });
    }

    // On the error path stderr comes FIRST but the PARTIAL OUTPUT is carried
    // too: measured in Swift — "SyntaxError" on its own is not enough for the
    // model to fix the code, the output before the error shows how far it got.
    let mut text = error_text.trim().to_string();
    if text.is_empty() {
        text = format!("script exited with status {state}");
    }
    if truncated_err {
        text.push_str("\n(error output truncated)");
    }
    if !output.trim().is_empty() {
        text.push_str(&format!(
            "\n--- output before the error ---\n{}",
            output.trim()
        ));
    }
    // THE INTERNAL FILE NAME DOES NOT LEAK. Interpreter errors quote the
    // script's own path ('File "/tmp/.../verify-26379-...py", line 1') and the
    // model treats whatever name it sees as THE name of the user's file —
    // measured: asked to "save this as a file" one turn later, it saved the
    // user's script under verify-26379-…-0.py. Every argument that lives under
    // the runtime is scrubbed from the error text: the full path goes, the
    // bare temp name becomes a neutral `script.<ext>`.
    for arg in args {
        let p = Path::new(arg);
        if !p.starts_with(runtime) {
            continue;
        }
        let neutral = match p.extension().and_then(|e| e.to_str()) {
            Some(ext) => format!("script.{ext}"),
            None => "script".to_string(),
        };
        if let Some(full) = p.to_str() {
            text = text.replace(full, &neutral);
        }
        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
            text = text.replace(name, &neutral);
        }
    }
    Ok(CodeOutcome::Error(text))
}

/// What a reader thread has collected SO FAR.
///
/// It is shared rather than returned because the main path may have to give up
/// on the thread (see the bounded join in `run_program`) and must still keep
/// what was read up to that point.
#[derive(Default)]
struct PipeSink {
    accumulated: Vec<u8>,
    truncated: bool,
}

/// Reads the pipe TO THE END but DISCARDS anything over the cap.
///
/// Stopping the reads locks the child once the pipe fills (and keeps it waiting
/// until the timeout); having no cap at all lets infinite output eat all the
/// memory. The middle of the two: keep reading, stop storing.
fn read_pipe<R: Read>(pipe: Option<R>, sink: &Mutex<PipeSink>) {
    let Some(mut pipe) = pipe else { return };
    let mut buffer = [0u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let mut sink = sink.lock().expect("pipe sink lock");
                let slot = OUTPUT_CAP.saturating_sub(sink.accumulated.len());
                if slot == 0 {
                    sink.truncated = true;
                    continue;
                }
                let taken = n.min(slot);
                sink.accumulated.extend_from_slice(&buffer[..taken]);
                if taken < n {
                    sink.truncated = true;
                }
            }
        }
    }
}

/// Takes what a reader has collected. A poisoned lock (the reader panicked) is
/// treated as "nothing was read": a crashing helper thread must not take the
/// whole turn down with it.
fn drain(sink: &Mutex<PipeSink>) -> (String, bool) {
    let Ok(sink) = sink.lock() else {
        return (String::new(), false);
    };
    (
        String::from_utf8_lossy(&sink.accumulated).into_owned(),
        sink.truncated,
    )
}

/// Joins the thread, but GIVES UP after `grace` and leaves it orphaned.
///
/// Std has no timed join, so the finish flag is polled — the same trick as the
/// process wait loop above. Abandoning a thread is deliberate: the alternative
/// is a tool call that never returns (see the rationale at the call site).
fn join_before(handle: std::thread::JoinHandle<()>, grace: Duration) {
    let deadline = Instant::now() + grace;
    while !handle.is_finished() {
        if Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    let _ = handle.join();
}

// ---------------------------------------------------------------------------
// The attempt counter
// ---------------------------------------------------------------------------

/// The per-turn attempt counter (code-spec §5.4).
///
/// In Swift this was on a `weak` reference and, so the cap was not ignored when
/// it was not bound, 3 was returned with "fail-closed". Here the tool KEEPS the
/// counter ITSELF: an owned field cannot be forgotten, so the fail-closed arm is
/// not needed at all. The shell does the reset with `new_turn()`.
#[derive(Debug, Default)]
pub struct CodeState {
    attempt: AtomicUsize,
}

impl CodeState {
    pub fn new() -> Self {
        Self::default()
    }

    /// A new turn — the counter is reset.
    pub fn new_turn(&self) {
        self.attempt.store(0, Ordering::SeqCst);
    }

    /// Records the attempt and returns WHICH one it is (starting at 1).
    ///
    /// `pub(crate)`: the counter is SHARED with `write_code` — the sum of the
    /// model-code runs in the turn is one budget, not a separate budget per tool.
    pub(crate) fn next_attempt(&self) -> usize {
        self.attempt.fetch_add(1, Ordering::SeqCst) + 1
    }
}

// ---------------------------------------------------------------------------
// The tool
// ---------------------------------------------------------------------------

/// `run_code` — runs a short script under the shield.
pub struct RunCodeTool {
    shield: NetworkShield,
    interpreters: Vec<Interpreter>,
    state: std::sync::Arc<CodeState>,
    /// A cache so the schema is not rebuilt on every call; `schema()` is called
    /// on the hot path (prompt build + grammar).
    schema_cache: Mutex<Option<ArgSchema>>,
}

impl RunCodeTool {
    /// Builds the tool ONLY if we can really run safely.
    ///
    /// Returning `None` is not a malfunction but the RIGHT behaviour: a tool
    /// that calls a non-existent interpreter, or cannot cut the network, is a
    /// trap for the model — it sees it in the catalog, calls it, gets an error
    /// every time and loses the turn. If the shell sees `None` it does not add
    /// the tool; `diagnose()` says why.
    pub fn discover() -> Option<RunCodeTool> {
        let interpreters = discover_interpreters();
        if interpreters.is_empty() {
            return None;
        }
        // THE SHIELD IS MEASURED, NOT ASSUMED.
        let shield = NetworkShield::find()?;
        if !verify_shield(&shield, &interpreters[0]) {
            return None;
        }
        Some(RunCodeTool {
            shield,
            interpreters,
            state: std::sync::Arc::new(CodeState::new()),
            schema_cache: Mutex::new(None),
        })
    }

    /// Why discovery failed — the shell prints this to the user/developer. A
    /// silent absence would not make an accidentally removed tool noticeable.
    pub fn diagnose() -> String {
        let interpreters = discover_interpreters();
        if interpreters.is_empty() {
            return "run_code is off: neither node nor python3 was found on the machine"
                .to_string();
        }
        let names: Vec<&str> = interpreters.iter().map(|y| y.key).collect();
        let Some(shield) = NetworkShield::find() else {
            // REASON + REMEDY. The previous text stated only the reason, and
            // for a Linux user that was a dead end: there WAS a way to bring
            // the tool back (installing bubblewrap) and the text did not say
            // so. A documented gap instead of a silent one.
            return format!(
                "run_code is off: an interpreter exists ({}) but no shield was found that can \
                 cut the process's network; model code is not run without cutting the network. \
                 Paths searched: macOS /usr/bin/sandbox-exec, Linux {}. On Linux, installing the \
                 `bubblewrap` package brings the tool back. There is NO counterpart on Windows: \
                 the tool is deliberately off there.",
                names.join(", "),
                BWRAP_PATHS.join(" | ")
            );
        };
        if let Err(reason) = measure_shield(&shield, &interpreters[0]) {
            // THE MOST LIKELY CAUSE IS NAMED FIRST, and on the most common Linux
            // it is not any of the three this message used to list.
            //
            // MEASURED on a stock Ubuntu 24.04 (Kamatera, 4 Sep 2026): as an
            // ordinary user `bwrap --unshare-net` dies with
            // "loopback: Failed RTM_NEWADDR: Operation not permitted", because
            // the distro ships `kernel.apparmor_restrict_unprivileged_userns=1`.
            // As root on the SAME machine it succeeds — root is exempt — so the
            // failure is invisible to anyone testing with sudo. The same error
            // had already stopped the GitHub ubuntu runner, which we had written
            // off as a runner quirk; it is the default.
            //
            // WITHOUT THIS LINE the user was told the measurement failed and left
            // to guess why, on the platform where the guess is hardest and the
            // remedy is one command.
            let hint = if cfg!(target_os = "linux") {
                "\n  Most likely: this distribution restricts unprivileged user namespaces \
                 (Ubuntu 24.04 ships `kernel.apparmor_restrict_unprivileged_userns=1`), so \
                 `bwrap --unshare-net` cannot bring up loopback for a non-root user. Check with \
                 `sysctl kernel.apparmor_restrict_unprivileged_userns`; allow it with \
                 `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0`. Running tacet \
                 as root also works and is the worse answer."
            } else {
                ""
            };
            return format!(
                "run_code is off: {} was found but the measurement did not pass: {reason} \
                 (see verify_shield){hint}",
                shield.name()
            );
        }
        format!(
            "run_code is on: interpreter {}, shield {} (network cut, writes confined to the sandbox)",
            names.join(", "),
            shield.name()
        )
    }

    /// The lever handed to the shell that resets the counter — bound to
    /// `ToolExecutor`'s turn hook.
    pub fn interpreters(&self) -> &[Interpreter] {
        &self.interpreters
    }

    pub fn turn_state(&self) -> std::sync::Arc<CodeState> {
        self.state.clone()
    }

    /// The measured shield — handed out so `write_code` can share the same one.
    /// Discovery (and the expensive `verify_shield` measurement) is therefore
    /// done ONCE; had the two tools measured separately, startup would have
    /// started two processes.
    pub fn shield(&self) -> &NetworkShield {
        &self.shield
    }

    /// The interpreter to run: first the model's EXPLICIT choice, failing that
    /// THE SCRIPT ITSELF, and if that does not speak either, the first in preference order.
    ///
    /// THE MIDDLE STEP WAS ADDED LATER and closes a measured failure:
    /// `language` is an optional field and the small model almost never fills
    /// it; when it did not, the script ran with the first in the list (js). But
    /// the model was writing Python. The result was "the script returned an
    /// error" every time -> a second attempt -> "no output", and the model went
    /// back to writing the list from memory — that is, the tool existed, worked, and WAS OF NO USE AT ALL.
    ///
    /// The alternative was making `language` REQUIRED; that adds a decode slot
    /// to every call, mostly needlessly, and does not remove the chance of the
    /// model filling it wrong. The source itself already answers this question:
    /// a script that writes `console.log` is JS, one that writes `print(...)` is Python.
    ///
    /// The markers are chosen ONE WAY ONLY — syntax valid in both languages
    /// (`for`, `if`, a `//` comment) is NEVER looked at; only things that are a
    /// SYNTAX ERROR in the other one count. On a tie the preference order wins,
    /// so this step only speaks in CLEAR cases.
    fn pick_interpreter(&self, requested: Option<&str>) -> &Interpreter {
        if let Some(y) = requested.and_then(|a| self.interpreters.iter().find(|y| y.key == a)) {
            return y;
        }
        &self.interpreters[0]
    }

    /// The interpreter to run. In order: the SOURCE if it speaks plainly,
    /// failing that the model's `language` choice, failing that the first in preference order.
    ///
    /// THAT THE SOURCE OVERRIDES THE `language` FIELD CAME FROM MEASUREMENT. In
    /// the first writing the guess only kicked in when `language` was empty
    /// ("never override an explicit choice"), but what was seen in the field was
    /// this: the model wrote JS and sent `language:"python"`. So the declaration
    /// is a GUESS by the model while the source is EVIDENCE; when the two
    /// conflict, following the evidence is the right thing. Doing what the
    /// declaration says looks like "respecting the model's intent" but in
    /// practice is a guaranteed syntax error and wastes the turn.
    /// The source has to "speak plainly": syntax valid in both languages does
    /// not count, and if the count ties the guess stays silent and the declaration holds.
    fn resolve_interpreter(&self, requested: Option<&str>, code: &str) -> &Interpreter {
        if let Some(key) = language_from_source(code)
            && let Some(y) = self.interpreters.iter().find(|y| y.key == key)
        {
            return y;
        }
        self.pick_interpreter(requested)
    }
}

/// Guesses the script's language from the source. `None` if undecided.
///
/// Only markers that WOULD NOT WORK in the other language are looked at; shared
/// syntax is deliberately ignored (see `resolve_interpreter`).
pub fn language_from_source(code: &str) -> Option<&'static str> {
    const PY: [&str; 6] = ["print(", "def ", "import ", "elif ", "lambda ", "range("];
    const JS: [&str; 6] = ["console.log", "=>", "const ", "let ", "function ", "var "];
    let py = PY.iter().filter(|i| code.contains(**i)).count();
    let js = JS.iter().filter(|i| code.contains(**i)).count();
    match py.cmp(&js) {
        std::cmp::Ordering::Greater => Some("python"),
        std::cmp::Ordering::Less => Some("js"),
        std::cmp::Ordering::Equal => None,
    }
}

/// Measures that the shield IS REALLY SET UP AND CUTS THE NETWORK.
///
/// Just checking "does the file exist" is not enough: if the profile syntax
/// does not hold, if the OS version refuses the rule, or if the interpreter
/// cannot start at all under the shield, the result would be a silent error on
/// every call. It is measured once, at discovery time, with a real process.
///
/// THE MEASUREMENT COVERS THE NETWORK TOO — it USED NOT TO. The previous
/// version only ran `print(1)`, i.e. it measured "a script runs under the
/// shield" while writing the sentence "the cut is REALLY MEASURED DURING
/// DISCOVERY" at the top of the file. The difference was harmless on macOS (the
/// network was measured by the `the_network_is_really_cut` test) but would
/// become dangerous the moment a second shield (bwrap) was added: the tests run
/// on the developer's machine, while nothing measures the shield on the user's
/// machine. The measurement now ASKS THE QUESTION DIRECTLY: the script under
/// the shield tries to open a connection and prints that it could not.
/// THE LIMIT OF THE MEASUREMENT IS WRITTEN OUT PLAINLY: this measurement proves
/// "a connection WAS NOT OPENED", not "the shield cut it". On a machine whose
/// network is already off the two cannot be told apart (bwrap `--unshare-net`
/// and an offline machine both give the same `ENETUNREACH`). The wrong
/// direction is the safe one: the error is not saying "there is a shield" when
/// there is none, it is "counting the shield as verified while the network is
/// off"; and in that case there is no real network anyway. The reverse — taking
/// the shield as valid while the network is UP — IS NOT POSSIBLE, because if the
/// connection succeeds the measurement fails.
fn verify_shield(shield: &NetworkShield, interpreter: &Interpreter) -> bool {
    measure_shield(shield, interpreter).is_ok()
}

/// `verify_shield` with the reason kept, so `diagnose()` can name the gate that
/// did not close instead of leaving the user to guess.
fn measure_shield(shield: &NetworkShield, interpreter: &Interpreter) -> Result<(), String> {
    let sandbox = temp_measurement_dir()
        .map_err(|e| format!("the measurement directory could not be made: {e}"))?;
    let script = sandbox.join(format!("measurement.{}", interpreter.extension));
    let body = measurement_script(interpreter.key, &ProbeTargets::from_host(&sandbox));
    let outcome = match std::fs::write(&script, body) {
        Err(e) => Err(format!("the measurement script could not be written: {e}")),
        Ok(()) => match run_program(
            shield,
            &sandbox,
            &interpreter.path,
            &[script.clone().into_os_string()],
            Duration::from_secs(15),
        ) {
            // Every gate is read from the SAME output: measuring them in
            // separate runs would multiply the startup time.
            Ok(CodeOutcome::Succeeded { output, .. }) => shield_verdict(&output)
                .map_err(|gate| format!("{gate} could NOT BE SEEN as closed")),
            Ok(other) => Err(format!(
                "the measurement script did not run cleanly under the shield: {other:?}"
            )),
            Err(e) => Err(format!("the shield could not be started: {e}")),
        },
    };
    let _ = std::fs::remove_dir_all(&sandbox);
    outcome
}

/// Reads the measurement's output. `Err` names the FIRST gate that is not
/// closed.
///
/// Each gate has two markers and BOTH are checked: the "blocked" one must be
/// there (so a probe that never ran does not count as a pass) and the "open"
/// one must not.
///
/// THE CLIPBOARD WAS THE SECOND GATE HERE, and the reason it was added is the
/// reason for every gate after it: only measuring the network is what let
/// `(allow mach-lookup)` sit there unfiltered for so long — the sentence "NO
/// access to this device" was checked against exactly one of the things it
/// claims. The same held for unix sockets (audit H7: bwrap left them all
/// reachable), the home directory (audit M2: a symlinked `$HOME` made the deny
/// rule dead) and signals (audit M1: `(allow signal)` reached every process of
/// the user). If someone loosens the shield again the tool now DISAPPEARS from
/// the catalog instead of quietly handing that gate to the model.
fn shield_verdict(output: &str) -> Result<(), &'static str> {
    let closed = |blocked: &str, open: &str| output.contains(blocked) && !output.contains(open);
    if !closed("NET_BLOCKED", "NET_OPEN") {
        return Err("the network");
    }
    if !closed("UNIX_BLOCKED", "UNIX_OPEN") {
        return Err("a unix socket outside the sandbox");
    }
    if !closed("HOME_BLOCKED", "HOME_OPEN") {
        return Err("the home directory");
    }
    if cfg!(target_os = "macos") {
        if !closed("PB_BLOCKED", "PB_OPEN") {
            return Err("the clipboard");
        }
        if !closed("SIG_BLOCKED", "SIG_OPEN") {
            return Err("signals to processes outside the sandbox");
        }
    }
    Ok(())
}

/// What the measurement aims at, read from the HOST before the script runs —
/// the child cannot name the user's home or the host's sockets any other way.
#[derive(Debug, Clone, Default)]
struct ProbeTargets {
    /// `$HOME` as written AND as resolved. Both, because M2 was exactly the case
    /// where they differ and only one of them was denied.
    home: Vec<PathBuf>,
    /// A name that REALLY IS in the home directory. Listing the directory is
    /// not enough on its own: bwrap's `--tmpfs` makes it list successfully and
    /// EMPTY, so "the listing worked" would fail a shield that works. What is
    /// asked is whether THIS name can be seen.
    home_sentinel: Option<String>,
    /// Unix sockets that exist on the host (see `probe_sockets`).
    sockets: Vec<PathBuf>,
    /// A process outside the sandbox the child must not be able to signal.
    /// `kill(pid, 0)` delivers nothing; it only asks whether a signal WOULD be
    /// allowed. macOS only: under bwrap `--unshare-pid` an outside pid is not
    /// nameable, and the same number may be a process inside the namespace.
    outside_pid: u32,
}

impl ProbeTargets {
    fn from_host(sandbox: &Path) -> ProbeTargets {
        let raw = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute() && h != Path::new("/"));
        let mut home = Vec::new();
        if let Some(raw) = raw {
            let real = raw.canonicalize().unwrap_or_else(|_| raw.clone());
            home.push(raw);
            if !home.contains(&real) {
                home.push(real);
            }
        }
        let home_sentinel = home.last().and_then(|real| home_sentinel(real, sandbox));
        let sockets = probe_sockets(sandbox, home.last().map(PathBuf::as_path));
        ProbeTargets {
            home,
            home_sentinel,
            sockets,
            outside_pid: std::process::id(),
        }
    }
}

/// A name in `home` that the measurement can look for, skipping the one entry
/// the shield itself puts there: if the measurement directory lives under the
/// home directory (`TMPDIR` inside it), bwrap recreates its first component
/// inside the tmpfs, and finding THAT would fail a working shield. Sorted, so
/// the choice is the same on every start.
fn home_sentinel(home: &Path, sandbox: &Path) -> Option<String> {
    let mut names: Vec<String> = std::fs::read_dir(home)
        .ok()?
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_string))
        .filter(|name| !sandbox.starts_with(home.join(name)))
        .collect();
    names.sort();
    names.into_iter().next()
}

/// The most sockets one measurement tries. Each refused `connect` is
/// immediate, but a socket that ACCEPTS costs nothing either — this only
/// bounds the script's length.
const PROBE_SOCKET_CAP: usize = 12;

/// Unix sockets that exist on this host and that a script must not reach.
///
/// The well-known ones first — the session and system bus, docker, LXD and
/// Incus (`DAEMON_SOCKETS`), the ssh agent, an X11 display, macOS's
/// mDNSResponder — then whatever loose sockets `/tmp` holds. Only paths that ARE sockets right now are kept (`metadata` +
/// `is_socket`: a stat, no connection is made from this process). A host with
/// none of them measures nothing here, and that is stated rather than hidden:
/// there is then nothing for the gate to protect.
///
/// THE SAME ROOTS AS THE PLAN. It scanned `/tmp` and `$TMPDIR` with no working
/// folder while every call's plan scanned the working folder too, so a hole in
/// the plan's handling of that third root (the shared budget it starved, review
/// of 10 Oct 2026) could not show here. `sandbox` is the measurement's own
/// folder, and its parent is scanned as a call's would be.
fn probe_sockets(sandbox: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        let host = ShadowHost::from_env();
        let mut candidates = probe_socket_candidates(&host);
        candidates.extend(scan_sockets(
            &socket_scan_roots(host.tmpdir.as_deref(), sandbox.parent(), home),
            home,
        ));
        let mut found: Vec<PathBuf> = Vec::new();
        for candidate in candidates {
            let is_socket = std::fs::metadata(&candidate)
                .map(|m| m.file_type().is_socket())
                .unwrap_or(false);
            if is_socket && candidate.to_str().is_some() && !found.contains(&candidate) {
                found.push(candidate);
            }
            if found.len() >= PROBE_SOCKET_CAP {
                break;
            }
        }
        found
    }
    #[cfg(not(unix))]
    {
        let _ = (sandbox, home);
        Vec::new()
    }
}

/// The well-known sockets `probe_sockets` stats, before the `/tmp` scan.
#[cfg(unix)]
///
/// The daemon sockets are the SAME list the shield covers, read from the same
/// constant: a socket the shield shadows but the measurement never tries is a
/// shadow nobody has seen work, and one the measurement tries but the shield
/// does not cover fails `verify_shield` — which is the point.
fn probe_socket_candidates(host: &ShadowHost) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = [
        "/run/dbus/system_bus_socket",
        "/var/run/dbus/system_bus_socket",
        "/run/docker.sock",
        "/var/run/docker.sock",
        "/var/run/mDNSResponder",
        "/tmp/.X11-unix/X0",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    candidates.extend(host.daemon_sockets.iter().cloned());
    candidates.extend(host.bound_sockets.iter().cloned());
    if let Some(xdg) = host.xdg_runtime.as_ref().filter(|x| !x.is_empty()) {
        candidates.push(Path::new(xdg).join("bus"));
    }
    if let Some(agent) = host.ssh_auth_sock.as_ref().filter(|x| !x.is_empty()) {
        candidates.push(PathBuf::from(agent));
    }
    candidates
}

/// A string as a literal that is valid in BOTH probe languages. JSON's string
/// syntax is a subset of both JavaScript's and Python's, so the host's paths go
/// in through `serde_json` and never through hand-written escaping — the same
/// lesson as the SBPL profile, where hand-splicing a path was the hole.
fn literal(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"\"".to_string())
}

/// A list of paths as a literal list valid in both languages (strings only:
/// JSON's `null` is not Python). A path that is not UTF-8 is left out.
fn literal_paths(paths: &[PathBuf]) -> String {
    let texts: Vec<&str> = paths.iter().filter_map(|p| p.to_str()).collect();
    serde_json::to_string(&texts).unwrap_or_else(|_| "[]".to_string())
}

/// The probes that touch nothing outside this machine: the home directory,
/// signals (macOS) and unix sockets.
///
/// SEPARATE FROM THE NETWORK PROBE so a test can run them WITHOUT the shield
/// and watch each one report "open" — a probe that can only ever say "blocked"
/// measures nothing. The network probe is never run unshielded: it would open a
/// real connection.
///
/// `then_js` is what the js version runs after its last (asynchronous) socket
/// probe: the network probe in production, an exit in that test. The python
/// version is synchronous and the caller appends what follows.
fn local_probes(key: &str, targets: &ProbeTargets, then_js: &str) -> String {
    let home = literal_paths(&targets.home);
    let sentinel = literal(targets.home_sentinel.as_deref().unwrap_or(""));
    let sockets = literal_paths(&targets.sockets);
    let pid = targets.outside_pid;
    let macos = cfg!(target_os = "macos");
    match key {
        "python" => {
            let signal = if macos {
                format!(
                    "try:\n\
                     \x20   os.kill({pid}, 0)\n\
                     \x20   print('SIG_OPEN')\n\
                     except PermissionError:\n\
                     \x20   print('SIG_BLOCKED')\n\
                     except Exception:\n\
                     \x20   print('SIG_UNCERTAIN')\n"
                )
            } else {
                String::new()
            };
            format!(
                "import os, socket\n\
                 _ho = False\n\
                 if {sentinel}:\n\
                 \x20   for _h in {home}:\n\
                 \x20       try:\n\
                 \x20           if {sentinel} in os.listdir(_h):\n\
                 \x20               _ho = True\n\
                 \x20       except Exception:\n\
                 \x20           pass\n\
                 print('HOME_OPEN' if _ho else 'HOME_BLOCKED')\n\
                 {signal}\
                 _uo = False\n\
                 for _p in {sockets}:\n\
                 \x20   _s = None\n\
                 \x20   try:\n\
                 \x20       _s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)\n\
                 \x20       _s.settimeout(1)\n\
                 \x20       _s.connect(_p)\n\
                 \x20       _uo = True\n\
                 \x20   except socket.timeout:\n\
                 \x20       _uo = True\n\
                 \x20   except Exception:\n\
                 \x20       pass\n\
                 \x20   finally:\n\
                 \x20       if _s:\n\
                 \x20           _s.close()\n\
                 print('UNIX_OPEN' if _uo else 'UNIX_BLOCKED')\n"
            )
        }
        _ => {
            let signal = if macos {
                format!(
                    "try{{process.kill({pid},0);console.log('SIG_OPEN')}}\
                     catch(e){{console.log(e.code==='EPERM'?'SIG_BLOCKED':'SIG_UNCERTAIN')}}\n"
                )
            } else {
                String::new()
            };
            // A connect left HANGING counts as open: the path was reachable.
            // Fail-closed, the same rule as `NET_UNCERTAIN`.
            format!(
                "{{let o=false;const S={sentinel};\
                 if(S){{for(const h of {home}){{try{{if(require('fs').readdirSync(h)\
                 .includes(S))o=true}}catch(e){{}}}}}}\
                 console.log(o?'HOME_OPEN':'HOME_BLOCKED')}}\n\
                 {signal}\
                 function __after(){{{then_js}}}\n\
                 const __U={sockets};\n\
                 (function __u(i,o){{if(o||i>=__U.length){{\
                 console.log(o?'UNIX_OPEN':'UNIX_BLOCKED');__after();return}}\
                 let d=false;const s=require('net').connect(__U[i]);\
                 const f=(x)=>{{if(d)return;d=true;s.destroy();__u(i+1,x)}};\
                 s.on('connect',()=>f(true));s.on('error',()=>f(false));\
                 s.setTimeout(1000,()=>f(true))}})(0,false);\n"
            )
        }
    }
}

/// The script of the discovery measurement: it tries a connection under the shield and prints the result.
///
/// THREE OUTCOMES, THE SAME IN BOTH LANGUAGES: `NET_OPEN` (a connection was
/// made), `NET_BLOCKED` (the request was refused IMMEDIATELY) and
/// `NET_UNCERTAIN` (the time ran out). The third is kept separate because a
/// connection LEFT HANGING is NOT a cut one; a firewall may be silently
/// dropping the packet, or the shield may never have been set up.
/// `verify_shield` only counts `NET_BLOCKED` as valid — on uncertainty the tool
/// leaves the catalog (fail-closed). Merging the two would be the easiest way
/// to say "there is a shield" and is exactly what this file forbids.
/// The timeout is SHORT (2 s) and INSIDE the script: if the shield really cuts,
/// the error comes at once and startup is not delayed; if it does not, 2
/// seconds are added to startup — but in that case the tool will leave the
/// catalog anyway, so the price is paid once.
///
/// The local probes (`local_probes`) run between the clipboard and the network:
/// the js network probe exits the process, so it has to be last.
fn measurement_script(key: &str, targets: &ProbeTargets) -> String {
    match key {
        "python" => format!(
            "{}{}{}",
            clipboard_probe(key),
            local_probes(key, targets, ""),
            network_probe(key)
        ),
        _ => format!(
            "{}{}",
            clipboard_probe(key),
            local_probes(key, targets, network_probe(key))
        ),
    }
}

/// The clipboard half of the measurement — macOS only, because `pbpaste` and the
/// pasteboard Mach service are macOS notions. It runs BEFORE the network probe:
/// the js network probe ends in `process.exit(0)` inside a callback, so anything
/// placed after it might never run.
#[cfg(target_os = "macos")]
fn clipboard_probe(key: &str) -> &'static str {
    match key {
        "python" => {
            "import subprocess\n\
             try:\n\
             \x20   subprocess.run(['/usr/bin/pbpaste'], check=True, capture_output=True)\n\
             \x20   print('PB_OPEN')\n\
             except Exception:\n\
             \x20   print('PB_BLOCKED')\n"
        }
        _ => {
            "try{require('child_process')\
             .execFileSync('/usr/bin/pbpaste',{stdio:['ignore','pipe','ignore']});\
             console.log('PB_OPEN')}catch(e){console.log('PB_BLOCKED')}\n"
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn clipboard_probe(_key: &str) -> &'static str {
    ""
}

fn network_probe(key: &str) -> &'static str {
    match key {
        "python" => {
            "import socket\n\
             try:\n\
             \x20   socket.create_connection(('1.1.1.1', 80), timeout=2).close()\n\
             \x20   print('NET_OPEN')\n\
             except socket.timeout:\n\
             \x20   print('NET_UNCERTAIN')\n\
             except Exception:\n\
             \x20   print('NET_BLOCKED')\n"
        }
        _ => {
            "const s=require('net').connect(80,'1.1.1.1');\
             s.setTimeout(2000,()=>{console.log('NET_UNCERTAIN');process.exit(0)});\
             s.on('connect',()=>{console.log('NET_OPEN');process.exit(0)});\
             s.on('error',()=>{console.log('NET_BLOCKED');process.exit(0)});"
        }
    }
}

/// A temporary, RESOLVED (symlink free) directory for the measurement.
///
/// `canonicalize` IS REQUIRED: on macOS `std::env::temp_dir()` gives something
/// under `/var/folders/...`, and `/var` is a link to `/private/var`. Written
/// unresolved into the sandbox profile, the rule matches nothing.
///
/// THE NAME COMES FROM `nonce()` AND THE DIRECTORY MUST BE NEW. It used to be
/// `{pid}-{wall-clock nanoseconds}` with `create_dir_all`, and the macOS wall
/// clock moves in whole microseconds (MEASURED: `time.time_ns() % 1000` is 0 on
/// every read, macOS 26.5.2): two discoveries started in the same
/// microsecond — the test suite starts dozens at once — got the SAME directory,
/// and the first to finish deleted the other's script before it ran. Seen
/// while writing the audit fixes of 9 Oct 2026: one discovery in a full
/// `cargo test` run failed while `diagnose()`, measuring again a moment later,
/// reported the shield on. Reproduced by `parallel_measurements_never_share_a_directory`
/// on the old code: 16 threads released together, 3, 2 and 4 of them shared a
/// directory in three runs out of three. `nonce()` carries a process-wide counter, and
/// `create_dir` refuses a name that already exists instead of sharing it.
fn temp_measurement_dir() -> std::io::Result<PathBuf> {
    let root = std::env::temp_dir().join(format!("tacet-code-measurement-{}", nonce()));
    std::fs::create_dir(&root)?;
    root.canonicalize()
}

/// The part of the description both shields share.
///
/// The sentences close two failures measured in Swift: the model was calling
/// the tool to inspect a server/container, and it took scripts that printed
/// nothing for a success.
macro_rules! run_code_usage {
    () => {
        "To run logic from a saved file, put the CODE ITSELF into this call instead of trying \
         to execute the file. Call this for any calculation or transformation too complex for \
         the calculate tool (loops, dates, text processing, simulations), and whenever the user \
         asks you to PRODUCE A LIST OR A SEQUENCE - prime numbers, Fibonacci terms, sorting, \
         filtering, counting - instead of writing the items out from memory. Never call it to \
         inspect servers, containers, ports, processes, disks or the operating system — it \
         cannot see them. The LAST LINE of the script must be print(...) in python or \
         console.log(...) in js: a bare expression is NOT output and a script that prints \
         nothing returns an error. If the tool returns an error, fix the code and call it ONCE \
         more."
    };
}

/// What the model is told under `sandbox-exec`.
///
/// EACH CLAIM IS A MEASURED FACT (see `verify_shield` and the macOS-only tests
/// below), and the wording is no wider than the measurement. It used to say "NO
/// access to ANY file on this device" while the profile let the script read
/// everything outside the home directory — temp folders, /Users/Shared,
/// db_write's temporary copies (audit L20). Reading is now an allowlist, and
/// what the allowlist still opens is NAMED rather than denied. That is "system
/// files", not "system libraries": the review of 10 Oct 2026 read /etc/hosts
/// from inside, listed /usr/local and /opt/homebrew/share, and stat'ed files in
/// $HOME (`file-read-metadata` is unfiltered, so names and sizes show, never
/// contents); every probe at user data — $HOME and its case and `..` aliases,
/// /Users/Shared, $TMPDIR, /private/tmp, /Volumes, Homebrew's var and etc —
/// was refused. "A new, empty folder" is true since M5: the folder is created
/// for the call and deleted after it.
///
/// It is also SHORT on purpose: `catalog.rs` caps every description at ~420
/// tokens because the block is re-sent on every turn of the tool loop.
///
/// AND EVERY WORD ROUTES. The router breaks ties on shared four-letter stems
/// (`router::overlap`, two needed), and the usage text already carries "call".
/// MEASURED with `every_benchmark_step_can_pass`: "nothing else BUT system
/// libraries" here, and "read-ONLY" in the bwrap text, each made a second stem
/// with "…375 grams of flour but I only want a third…" and pushed `calculate`
/// out of the nine. Hence "except" and "can still read".
const DESCRIPTION_SANDBOX_EXEC: &str = concat!(
    "Runs a short script in an isolated sandbox with NO network and NO access to your files: \
     it starts in a new, empty folder and can read nothing except system files - not \
     even a file just saved with write_code, so open('file') always fails. ",
    run_code_usage!()
);

/// What the model is told under `bwrap`.
///
/// NARROWER THAN THE macOS TEXT, ON PURPOSE. bwrap binds the whole root READ
/// ONLY and covers the home directory and the socket directories (see
/// `bwrap_args`); everything else outside the home directory — /tmp, /srv, a
/// working folder that is not under home — is still readable. Saying "your
/// files" there would be the exact overclaim L20 was about, so the text says
/// what is covered and what is not. The same goes for sockets: only the known
/// ones are shadowed (see `bwrap_args`, "WHAT IS STILL OPEN"), so the text
/// says "known" rather than promising that no local service can be reached.
/// (Not "read-only": see the routing note above.)
const DESCRIPTION_BWRAP: &str = concat!(
    "Runs a short script in an isolated sandbox with NO network and NO access to your home \
     folder: it starts in a new, empty folder, can write nowhere else, and can still read other \
     system files; only known sockets are hidden. Never open a saved file, not even one \
     just saved with write_code. ",
    run_code_usage!()
);

impl Tool for RunCodeTool {
    fn name(&self) -> &str {
        "run_code"
    }

    fn description(&self) -> &str {
        match self.shield {
            NetworkShield::SandboxExec { .. } => DESCRIPTION_SANDBOX_EXEC,
            NetworkShield::Bwrap { .. } => DESCRIPTION_BWRAP,
        }
    }

    fn schema(&self) -> ArgSchema {
        if let Some(s) = self.schema_cache.lock().expect("schema lock").clone() {
            return s;
        }
        let mut fields = vec![
            Field::new(
                "code",
                ArgSchema::text()
                    .description("The script. Keep it minimal and PRINT the final result."),
            )
            .required(),
        ];

        // THE `language` FIELD IS ADDED ONLY IF THERE ARE TWO INTERPRETERS. In
        // Swift this field stood even though it had a single valid value, and it
        // ate a decode slot the small model had to fill on EVERY call; that is
        // why it was removed. Here it is legitimate if there really is a CHOICE;
        // otherwise it would be the same waste.
        if self.interpreters.len() > 1 {
            let choices: Vec<&str> = self.interpreters.iter().map(|y| y.key).collect();
            let default = self.interpreters[0].key;
            fields.push(Field::new(
                "language",
                ArgSchema::choice(choices)
                    .description(format!("Script language (default '{default}').")),
            ));
        }

        fields.push(Field::new(
            "timeout_s",
            ArgSchema::integer()
                .range(Some(1.0), Some(MAX_TIMEOUT as f64))
                .description(format!(
                    "Seconds to allow before the script is killed (default {DEFAULT_TIMEOUT})."
                )),
        ));

        let schema =
            ArgSchema::object(fields).description("Run a short script and return its output");
        *self.schema_cache.lock().expect("schema lock") = Some(schema.clone());
        schema
    }

    /// AN EXTERNAL TOOL. It starts a process on the device and runs model
    /// output; `ToolExecutor`'s approval gate has to see this.
    fn taints_session(&self) -> bool {
        true
    }

    fn run<'a>(&'a self, args: serde_json::Value, ctx: &'a mut ToolContext) -> ToolFuture<'a> {
        boxed(async move {
            if let Err(h) = self.schema().validate(&args) {
                return ToolOutcome::failed(&h);
            }

            // The counter is incremented BEFORE THE DENIAL DECISION: the 3rd call never sees the engine.
            let attempt = self.state.next_attempt();
            if attempt > MAX_ATTEMPTS {
                return ToolOutcome::new(
                    "Code execution limit",
                    ToolState::Failed("two attempts exhausted".into()),
                    "error_final: give the user a short honest answer, do NOT retry",
                );
            }

            let trace = ctx.start_chip("code", "Running code…");
            let outcome = self.execute(&args, ctx, attempt);

            ctx.update_chip(
                trace,
                TraceUpdate::state(outcome.state.clone())
                    .text(outcome.chip_text.clone())
                    .raw_input(
                        args.get("code")
                            .and_then(|k| k.as_str())
                            .unwrap_or_default(),
                    )
                    .raw_output(outcome.raw_output.clone().unwrap_or_default()),
            );
            // Tainting only for a script that REALLY ran: a denied or
            // never-started call did not touch the outside world.
            if !matches!(outcome.state, ToolState::Failed(_)) {
                ctx.taint();
            }
            outcome
        })
    }
}

impl RunCodeTool {
    fn execute(&self, args: &serde_json::Value, ctx: &ToolContext, attempt: usize) -> ToolOutcome {
        let last_attempt = attempt >= MAX_ATTEMPTS;

        let Some(code) = args
            .get("code")
            .and_then(|v| v.as_str())
            .filter(|k| !k.trim().is_empty())
        else {
            return ToolOutcome::failed(&ToolError::MissingField("code".into()));
        };
        let interpreter =
            self.resolve_interpreter(args.get("language").and_then(|v| v.as_str()), code);
        let timeout = Duration::from_secs(
            args.get("timeout_s")
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_TIMEOUT),
        );

        // The shield profile writes the sandbox as a subpath; an unresolved path
        // silently falls to "nothing is permitted" (see `NetworkShield::wrap`).
        let sandbox = match ctx
            .resolve_path(".")
            .and_then(|k| k.canonicalize().map_err(ToolError::Io))
        {
            Ok(k) => k,
            Err(h) => return ToolOutcome::failed(&h),
        };

        let raw = match run_until(
            &self.shield,
            interpreter,
            &sandbox,
            code,
            timeout,
            ctx.cancel_token(),
        ) {
            Ok(s) => s,
            Err(h) => return ToolOutcome::failed(&h),
        };

        match raw {
            // SUCCESS WITH NO OUTPUT IS NOT A SUCCESS (measured in Swift): when
            // the model saw "ok" it MADE THE RESULT UP.
            CodeOutcome::Succeeded { output, .. } if output.trim().is_empty() => ToolOutcome::new(
                if last_attempt {
                    "The code could not be run"
                } else {
                    "Error · retrying"
                },
                ToolState::Failed("no output".into()),
                error_text(
                    "error: the script ran but printed nothing. A bare last expression is NOT \
                     output - wrap the value you need in print(...) for python or \
                     console.log(...) for js",
                    last_attempt,
                ),
            )
            .raw_output("no output"),

            CodeOutcome::Succeeded {
                output,
                ms,
                truncated,
            } => {
                let short = truncate(&output, MODEL_OUTPUT_CAP);
                let note = if truncated || short.len() < output.len() {
                    "\n(the output was truncated)"
                } else {
                    ""
                };
                // If the output is large, the bypass channel: the full body to
                // the store, the first 500 characters + source_ref to the model.
                // The head is enough for the model to summarize the result; the
                // next tool reaches the whole thing through the reference.
                let suffix = if output.len() > MODEL_OUTPUT_CAP {
                    let r = ctx.store(
                        "code",
                        &format!("{} characters of code output", output.len()),
                        output.clone(),
                    );
                    tacet_kernel::source_ref_suffix(r.as_str())
                } else {
                    String::new()
                };
                ToolOutcome::read_ok(
                    format!("Code executed · {ms} ms"),
                    format!("ok ({ms} ms)\n{short}{note}{suffix}"),
                )
                .raw_output(output)
            }

            CodeOutcome::Error(message) => {
                // THE LANGUAGE DIAGNOSIS COMES FIRST (measured in Swift): if the
                // script is Python but the engine is JS, the raw parser message
                // drags the model into the WRONG repair — it adds parentheses,
                // does not change the language, and loses the one remaining attempt.
                let cause = if interpreter.key == "js" && looks_like_python(code) {
                    format!(
                        "error: this script is Python, but this sandbox runs JavaScript. \
                         Rewrite the SAME logic in JavaScript: for (let i = 0; i < n; i++) \
                         {{...}}, console.log(x). Do not use range(), def, or indent blocks.\n{message}"
                    )
                } else {
                    format!("error: {message}")
                };
                ToolOutcome::new(
                    if last_attempt {
                        "The code could not be run"
                    } else {
                        "Error · retrying"
                    },
                    ToolState::Failed("the script returned an error".into()),
                    error_text(&cause, last_attempt),
                )
                .raw_output(message)
            }

            CodeOutcome::Timeout => ToolOutcome::new(
                "The code timed out",
                ToolState::Failed("timed out".into()),
                error_text(
                    "error: the script did not finish in time — it probably has an endless \
                     loop; bound every loop",
                    last_attempt,
                ),
            )
            .raw_output("timeout"),
        }
    }
}

/// Builds the error return in one place: on the last attempt `error_final` is
/// appended (code-spec §5.4) — but THE REASON is given too, because the honest
/// short answer the model will give the user must contain the reason.
pub(crate) fn error_text(cause: &str, last_attempt: bool) -> String {
    let short = truncate(cause, MODEL_ERROR_CAP);
    if last_attempt {
        format!("{short}\nerror_final: give the user a short honest answer, do NOT retry")
    } else {
        short
    }
}

/// Truncates at a character boundary (not a byte one: cutting in the middle of
/// a multi-byte character would produce invalid UTF-8).
pub(crate) fn truncate(text: &str, cap: usize) -> String {
    if text.chars().count() <= cap {
        return text.to_string();
    }
    text.chars().take(cap).collect()
}

/// Does the script look like Python?
///
/// A measurement finding inherited from Swift: the small model writes Python in
/// most code cases. Pure and static — testable without a model.
pub fn looks_like_python(code: &str) -> bool {
    // Each of these is either a syntax error or undefined in JS; there is no
    // risk of mistaking a valid JS script for Python. `print(` is DELIBERATELY
    // absent: in many shells it can be defined on the JS side too.
    const PYTHON_SPECIFIC: &[&str] = &[
        "range(", "def ", "elif ", "end=", " True", " False", " None",
    ];
    if PYTHON_SPECIFIC.iter().any(|i| code.contains(i)) {
        return true;
    }
    // A block header ending in a colon. In JS a `for`/`if`/`while` header opens
    // with a parenthesis and DOES NOT END with a colon. If there is a `{`, leave
    // it alone: `for (const k in o) { ... }` is valid JS and object literals
    // carry colons too.
    if code.contains('{') {
        return false;
    }
    const BLOCK_START: &[&str] = &["for ", "if ", "while ", "else", "try", "except"];
    code.lines().any(|s| {
        let s = s.trim();
        s.ends_with(':') && !s.contains('{') && BLOCK_START.iter().any(|b| s.starts_with(b))
    })
}

// ---------------------------------------------------------------------------
// The gate that decides whether "no sandbox here" is a skip or a failure
// ---------------------------------------------------------------------------

/// The environment variable that turns every sandbox skip in this crate into a
/// FAILURE, and the name is shared by `run_code`'s and `write_code`'s test
/// modules so there is exactly one spelling of it.
///
/// WHY IT EXISTS. `tool_or_skip()` in both files returns `None` and prints a
/// line when the shield cannot be discovered, and every execution test then
/// passes by doing nothing. On this machine that is right — a developer without
/// bubblewrap should not be blocked. On a CI runner where the job has ALREADY
/// installed bubblewrap and ALREADY run `bwrap ... /bin/echo SANDBOX_UP` as a
/// preflight, a skip is not a skip: it is the suite reporting green for
/// measurements it never made, which is the exact failure this repository is
/// built to prevent.
#[cfg(test)]
pub(crate) const SANDBOX_MUST_RUN: &str = "TACET_SANDBOX_MUST_RUN";

/// Is a skip forbidden on this run?
///
/// Set to `1` by the Linux leg of the `test` job in `.github/workflows/ci.yml`
/// — and by nothing else. Empty and `0` count as unset because the workflow
/// sets the variable for all three matrix platforms and hands macOS and Windows
/// the empty string; on those there is no bwrap and a skip is the truth.
///
/// `ci_sets_the_variable_that_turns_a_skip_into_a_failure` asserts that the
/// workflow still sets it: without that test this whole mechanism could be
/// removed from CI by deleting three lines of YAML and nothing would go red.
#[cfg(test)]
pub(crate) fn sandbox_must_run() -> bool {
    match std::env::var(SANDBOX_MUST_RUN) {
        Ok(value) => !value.is_empty() && value != "0",
        Err(_) => false,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Arc;
    use tacet_kernel::{InMemoryDataStore, SilentReporter, SourceRef};

    fn hold<F: std::future::Future>(future: F) -> F::Output {
        use std::pin::pin;
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        static VTABLE: RawWakerVTable = RawWakerVTable::new(
            |_| RawWaker::new(std::ptr::null(), &VTABLE),
            |_| {},
            |_| {},
            |_| {},
        );
        let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
        let mut cx = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
                return output;
            }
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "tacet-code-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&path).expect("temp dir");
        path.canonicalize().expect("resolved path")
    }

    fn context(root: &Path) -> ToolContext {
        ToolContext::new(
            Arc::new(InMemoryDataStore::new()),
            root.to_path_buf(),
            Arc::new(SilentReporter),
        )
    }

    /// If there is no shield/interpreter (a developer machine without
    /// bubblewrap) the execution tests are SKIPPED — but not SILENTLY: if
    /// `discover` returns `None` the tool is not in the catalog anyway and that
    /// is the right behaviour; the test passes by printing it.
    ///
    /// EXCEPT WHERE A SKIP WOULD BE A LIE. Under `TACET_SANDBOX_MUST_RUN` the
    /// same `None` is a PANIC carrying `diagnose()`'s text (see
    /// `SANDBOX_MUST_RUN`). Until that variable existed, "CI, Linux" in the old
    /// version of this comment was not a caveat but a description: the Linux leg
    /// of CI had no bwrap, so EVERY test below this line returned immediately
    /// and the suite reported green having measured nothing on the one platform
    /// whose shield had never been run.
    fn tool_or_skip() -> Option<RunCodeTool> {
        match RunCodeTool::discover() {
            Some(a) => Some(a),
            None => {
                let why = RunCodeTool::diagnose();
                assert!(
                    !sandbox_must_run(),
                    "{SANDBOX_MUST_RUN} is set, so this test may not skip — \
                     the sandbox was expected to work here: {why}"
                );
                eprintln!("run_code was not discovered, the execution tests were skipped: {why}");
                None
            }
        }
    }

    // --- Model-free, pure tests (they run on every machine) ---

    /// 1) The Python diagnosis: it turns a model sending Python to the JS engine the RIGHT way.
    #[test]
    fn the_python_diagnosis_gives_no_false_positive() {
        assert!(looks_like_python("def f(x):\n  return x"));
        assert!(looks_like_python("if x == 1:\n  y = True"));
        assert!(
            looks_like_python("for i in range(20):\n  print(i)"),
            "a block ending in a colon"
        );
        // Valid JS must not be taken for Python.
        assert!(!looks_like_python(
            "for (let i = 0; i < 10; i++) { console.log(i) }"
        ));
        assert!(!looks_like_python(
            "const o = {a: 1}; console.log(JSON.stringify(o))"
        ));
        assert!(!looks_like_python("for (const k in o) { console.log(k) }"));
    }

    /// 2) The error text carries `error_final` on the last attempt and DOES NOT
    ///    on the earlier one; THE REASON is preserved in both cases.
    #[test]
    fn the_error_text_appends_error_final_on_the_last_attempt() {
        let first = error_text("error: boom", false);
        assert_eq!(first, "error: boom");
        assert!(!first.contains("error_final"));

        let last = error_text("error: boom", true);
        assert!(last.contains("error_final"));
        assert!(last.contains("do NOT retry"));

        // The cap: a long reason is truncated but error_final IS NOT DROPPED.
        let long = error_text(&"x".repeat(5_000), true);
        assert!(long.contains("error_final"));
        assert!(long.chars().count() < MODEL_ERROR_CAP + 120);
    }

    /// 3) The counter: two real attempts, on the third the engine is NEVER seen.
    #[test]
    fn the_attempt_counter_rejects_the_third_call() {
        let d = CodeState::new();
        assert_eq!(d.next_attempt(), 1);
        assert_eq!(d.next_attempt(), 2);
        assert_eq!(
            d.next_attempt(),
            3,
            "the third call must exceed MAX_ATTEMPTS"
        );
        d.new_turn();
        assert_eq!(d.next_attempt(), 1, "it must reset at the start of a turn");
    }

    /// 4) The shield profile binds the sandbox and the home directory to the RIGHT rules.
    #[test]
    fn the_shield_profile_closes_the_network_and_the_home_directory() {
        let p = profile();
        assert!(p.contains("(deny network*)"), "the network was left open");
        assert!(
            p.contains("(deny default)"),
            "the default was left permissive"
        );
        assert!(p.contains("(allow file-write* (subpath (param \"SBOX\")))"));
        // Writing only into the sandbox: there must be no write permission for another subpath.
        assert_eq!(p.matches("allow file-write*").count(), 1);
        assert!(p.contains("(deny file-read* (subpath (param \"HOMEDIR\")))"));

        // NO PATH MAY BE SPLICED INTO THE PROFILE TEXT. This is the claim that
        // keeps the SBPL-injection fix from being undone by a well-meaning
        // "simplification" back to `format!` (see `profile`).
        //
        // IT USED TO READ "no `(subpath \"` at all", which the read allowlist
        // (audit L20) makes impossible: that allowlist IS a list of literal
        // system paths. The claim is kept, made exact instead: every literal
        // subpath must be one of the FIXED system roots below, so the sandbox
        // or the home directory spliced in as text — the attack — is still a
        // literal that is not on the list and still turns this red. The two
        // lists are compared both ways, so neither can change alone.
        // `(literal "/dev/null")` and `(literal "/")` are fixed too.
        const SYSTEM_READ_ROOTS: &[&str] = &[
            "/System",
            "/usr",
            "/bin",
            "/sbin",
            "/private/etc",
            "/private/var/db/timezone",
            "/private/var/db/dyld",
            "/dev",
            "/opt/homebrew/Cellar",
            "/opt/homebrew/opt",
            "/opt/homebrew/bin",
            "/opt/homebrew/lib",
            "/opt/homebrew/Frameworks",
            "/opt/homebrew/share",
            "/Library/Frameworks",
            "/Library/Developer/CommandLineTools",
        ];
        // The data directories of the Intel Homebrew prefix, which `/usr`
        // covers and which are therefore denied by name (see `profile`).
        const DENIED_DATA_ROOTS: &[&str] = &["/usr/local/var", "/usr/local/etc"];
        let literal_subpaths: Vec<&str> = p
            .split("(subpath \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        let fixed: Vec<&str> = SYSTEM_READ_ROOTS
            .iter()
            .chain(DENIED_DATA_ROOTS)
            .copied()
            .collect();
        assert_eq!(
            literal_subpaths, fixed,
            "a literal path that is not a fixed system root is in a subpath rule: {p}"
        );
        // NO ALLOWED ROOT MAY CONTAIN A DIRECTORY THAT HOLDS USER DATA unless
        // that directory is denied after it. This is the claim the old
        // `(subpath "/opt/homebrew")` broke — the review read a live postgres
        // database out of `/opt/homebrew/var` — and it is checked here, on the
        // TEXT, so it goes red on every machine and not only on one that
        // happens to have Homebrew services installed.
        let deny_data = p
            .find("(deny file-read* (subpath \"/usr/local/var\")")
            .expect("the Intel Homebrew data deny");
        let allow_system = p.find("(allow file-read* ").expect("the system allow");
        assert!(
            allow_system < deny_data,
            "the data deny comes before the allow it narrows, so the allow wins: {p}"
        );
        for data in [
            "/opt/homebrew/var",
            "/opt/homebrew/etc",
            "/usr/local/var",
            "/usr/local/etc",
        ] {
            let opened_by: Vec<&&str> = SYSTEM_READ_ROOTS
                .iter()
                .filter(|root| Path::new(data).starts_with(root))
                .collect();
            assert!(
                opened_by.is_empty() || DENIED_DATA_ROOTS.contains(&data),
                "{data} holds Homebrew services' data and is readable through {opened_by:?}"
            );
        }
        // THE ONE FILE LET BACK IN under `etc/` is OpenSSL's config, and it
        // must come AFTER the deny or the deny takes it away again and node
        // stops starting (see `profile`).
        let openssl = p
            .find("(allow file-read* (regex #\"^/(opt/homebrew|usr/local)/etc/openssl@[0-9.]+/openssl\\.cnf$\"))")
            .expect("the openssl.cnf exception");
        assert!(
            deny_data < openssl,
            "openssl.cnf is re-allowed before the deny: {p}"
        );
        assert_eq!(p.matches("(regex ").count(), 1, "another regex rule: {p}");
        // READING IS AN ALLOWLIST (L20): no unfiltered `(allow file-read*)`.
        assert!(
            !p.contains("(allow file-read*)"),
            "file reading is unfiltered again — temp folders and /Users/Shared are readable"
        );
        // THE METADATA ALLOW COMES AFTER THE HOME DENY — the order the
        // `realpathSync` fix (see `profile`) was measured in, when the other
        // order gave node `EPERM: lstat '/Users/...'`. It was swapped by
        // accident while the L20 allowlist was written and nothing went red.
        // HONEST LIMIT: on macOS 26.5.2 (9 Oct 2026) BOTH orders let node lstat
        // the home directory and a sibling in it and run, so this pins the
        // documented order rather than reproducing the failure on this OS.
        let deny_home = p
            .find("(deny file-read* (subpath (param \"HOMEDIR\")))")
            .expect("the home deny");
        let metadata = p
            .find("(allow file-read-metadata)")
            .expect("the metadata allow");
        assert!(
            deny_home < metadata,
            "the home deny comes after the metadata allow — node cannot lstat its own path: {p}"
        );
        // SIGNALS ONLY TO ITSELF (M1).
        assert!(
            p.contains("(allow signal (target self))") && !p.contains("(allow signal)"),
            "the script can signal processes outside the sandbox: {p}"
        );
        // The clipboard: `(allow mach-lookup)` MUST NOT stand unfiltered.
        assert!(
            !p.contains("(allow mach-lookup)"),
            "mach-lookup was left unfiltered — pbpaste becomes reachable"
        );
        assert!(p.contains("(allow mach-lookup (global-name"));
        // Forking is what let a detached grandchild outlive the timeout kill.
        assert!(
            !p.contains("process-fork"),
            "process-fork is back: a detached child can outlive the timeout"
        );
    }

    /// 4a) The fail-closed path check: only what can actually break out of an
    ///     SBPL string is refused, and a folder called `Project (old)` — which
    ///     real users have — must still run.
    #[test]
    fn only_quote_breaking_paths_are_refused() {
        assert!(path_is_shield_safe(Path::new("/Users/x/Project (old)")));
        assert!(path_is_shield_safe(Path::new("/Users/x/a;b")));
        assert!(!path_is_shield_safe(Path::new(
            "/private/tmp/evil\")) (allow default) ;"
        )));
        assert!(!path_is_shield_safe(Path::new("/tmp/back\\slash")));
        assert!(!path_is_shield_safe(Path::new("/tmp/new\nline")));
    }

    /// 4b) IS DISCOVERY REALLY PASSING — `tool_or_skip()` IS A SILENT ESCAPE
    ///     HATCH and this test closes it.
    ///
    /// All the execution tests below print "ok" without measuring anything when
    /// discovery fails. So if `verify_shield` broke (e.g. the measurement script
    /// no longer printed `NET_BLOCKED`) the suite would stay completely green
    /// and the tool would silently leave the catalog in production — the exact
    /// description of this repo's recurring failure. The claim here is built as
    /// a conditional: IF a shield AND an interpreter EXIST on this machine,
    /// discovery MUST succeed.
    ///
    /// The strict claim is UNCONDITIONAL on macOS: under `sandbox-exec`
    /// `connect` returns `EPERM` IMMEDIATELY regardless of the state of the
    /// internet, so the measurement is deterministic. bwrap `--unshare-net`
    /// returns `ENETUNREACH`, which an offline machine also returns, and a
    /// strict claim on every Linux would make this test swing with the
    /// machine's network — and with the machine's kernel, since a distribution
    /// that refuses unprivileged user namespaces cannot start bwrap at all.
    ///
    /// So on Linux it is strict WHERE THE ENVIRONMENT PROMISES IT SHOULD BE:
    /// under `TACET_SANDBOX_MUST_RUN`, which is set only by a CI job that has
    /// already installed bubblewrap and already watched `bwrap ... /bin/echo`
    /// print `SANDBOX_UP` in a preceding step. There a failure here is a defect
    /// in this repository, not a fact about the developer's laptop. Everywhere
    /// else it is still only PRINTED.
    #[test]
    fn when_a_shield_and_an_interpreter_exist_discovery_must_succeed() {
        let Some(shield) = NetworkShield::find() else {
            eprintln!("there is no shield on this machine; the discovery claim cannot be built");
            return;
        };
        if discover_interpreters().is_empty() {
            eprintln!(
                "there is no interpreter on this machine; the discovery claim cannot be built"
            );
            return;
        }
        let discovered = RunCodeTool::discover().is_some();
        if cfg!(target_os = "macos") || sandbox_must_run() {
            assert!(
                discovered,
                "a shield ({}) and an interpreter exist but discovery failed: {}",
                shield.name(),
                RunCodeTool::diagnose()
            );
        } else if !discovered {
            eprintln!(
                "discovery failed ({}): {}",
                shield.name(),
                RunCodeTool::diagnose()
            );
        }
    }

    /// The path to `.github/workflows/ci.yml`. CARGO_MANIFEST_DIR is
    /// `<repo>/crates/tacet-tools`, so the repository root is two parents up.
    fn ci_workflow() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("the crate lives under crates/")
            .join(".github/workflows/ci.yml")
    }

    /// The lines of a YAML file with their comments removed, trimmed, empties
    /// dropped.
    ///
    /// WHY THIS EXISTS RATHER THAN A `contains` OVER THE WHOLE FILE: the first
    /// version of the test below searched the raw text for
    /// `TACET_SANDBOX_MUST_RUN` and for `bubblewrap`, and BOTH strings also
    /// appear in the prose comment that explains the steps. Measured by deleting
    /// the two `env:` lines from the `cargo test` step and, separately, the
    /// `apt-get install` line: after each deletion the file still contained both
    /// strings and the test stayed GREEN — a test named after a guarantee that
    /// did not test the guarantee, which is the exact failure the guarantee is
    /// about.
    ///
    /// IT IS CRUDER THAN YAML AND THAT DIRECTION IS THE SAFE ONE: cutting at the
    /// first `#` would also cut a `#` inside a quoted scalar, so the worst this
    /// can do is FAIL to see a setting that is really there — a red test, never
    /// a green one. (ci.yml has no such scalar today; the shell `#` comments
    /// inside `run:` blocks are exactly what this is meant to drop.)
    fn yaml_settings(text: &str) -> Vec<String> {
        text.lines()
            .map(|line| line.split('#').next().unwrap_or("").trim().to_string())
            .filter(|line| !line.is_empty())
            .collect()
    }

    /// 4b-ii) THE GATE IS ONLY A GATE IF CI STILL SETS IT.
    ///
    /// `sandbox_must_run()` is what turns every `tool_or_skip()` in this crate
    /// from a skip into a failure, and it is switched on by three lines of YAML.
    /// Delete them — or rename the variable on one side only — and 24 tests go
    /// back to passing without measuring anything, exactly as they did before
    /// this work, with nothing anywhere going red. That figure is not asserted
    /// from memory: `the_skip_count_the_documents_quote_is_the_count_in_the_source`
    /// derives it from these two source files and pins this sentence to it.
    ///
    /// SO WHAT IS ASSERTED IS THE SETTING, NOT THE MENTION: the workflow's
    /// comment lines are stripped first (`yaml_settings`), and what must survive
    /// is an assignment of the variable to a non-empty value and an `apt-get
    /// install` line naming the package. Both deletions were simulated while
    /// writing this and each one turns this test red on its own.
    ///
    /// IT PARSES NO YAML, on purpose — the same rule as
    /// `crates/tacet-cli/tests/network_monopoly.rs`, which polices the
    /// zero-dependency claim without pulling in a TOML parser to do it.
    #[test]
    fn ci_sets_the_variable_that_turns_a_skip_into_a_failure() {
        let workflow = ci_workflow();
        let text = std::fs::read_to_string(&workflow)
            .unwrap_or_else(|e| panic!("{} is unreadable: {e}", workflow.display()));
        let settings = yaml_settings(&text);

        let assignment = format!("{SANDBOX_MUST_RUN}:");
        assert!(
            settings.iter().any(|line| {
                // A key with a REAL value. `TACET_SANDBOX_MUST_RUN:` alone is
                // YAML for "null" and `: ''` is the empty string — both of which
                // `sandbox_must_run()` reads as unset, so both would silence
                // every sandbox test on every platform while a naive "is the
                // name here" check stayed green. Quotes and whitespace are
                // dropped before asking whether anything is left; the shape of
                // the expression is deliberately NOT pinned, because a
                // Linux-only job spelling it `1` is equally correct.
                line.strip_prefix(&assignment).is_some_and(|value| {
                    !value
                        .trim()
                        .trim_matches(|c: char| c == '\'' || c == '"' || c.is_whitespace())
                        .is_empty()
                })
            }),
            "no step in {} SETS {SANDBOX_MUST_RUN} any more (a mention in a comment is not a \
             setting): the sandbox tests can skip green again",
            workflow.display()
        );
        assert!(
            settings
                .iter()
                .any(|line| line.contains("apt-get install") && line.contains("bubblewrap")),
            "the Linux runner no longer installs bubblewrap, so {SANDBOX_MUST_RUN} would only \
             make the job fail: {}",
            workflow.display()
        );
    }

    /// How many `#[test]` functions in `text` call `tool_or_skip()`, and how
    /// many of those are macOS-only.
    ///
    /// A line-scanner rather than a parser: an `#[cfg(...)]` line is remembered
    /// until the next `fn`, which is the shape every test in these two files
    /// has. It cannot see a `cfg` written any other way, so it is a lower bound
    /// on "macOS-only" — and a lower bound is the direction that makes the
    /// derived Linux count too HIGH, i.e. red, never quietly right.
    fn skipping_tests(text: &str) -> (usize, usize) {
        let (mut total, mut macos_only) = (0usize, 0usize);
        let mut pending_macos = false;
        let mut in_macos_fn = false;
        for line in text.lines().map(str::trim) {
            if line.starts_with("#[cfg(") {
                pending_macos |= line.contains("target_os = \"macos\"");
            } else if line.starts_with("fn ") {
                in_macos_fn = pending_macos;
                pending_macos = false;
            // THE CALL SHAPE, NOT THE NAME. Matching the name alone made the
            // scanner count its own matcher line and every doc comment that
            // mentions the helper — measured: 16 in this file instead of 15.
            } else if line.starts_with("let Some(tool) =") && line.contains("tool_or_skip") {
                total += 1;
                if in_macos_fn {
                    macos_only += 1;
                }
            }
        }
        (total, macos_only)
    }

    /// 4b-iii) THE NUMBER IN THE PROSE IS THE NUMBER IN THE SOURCE.
    ///
    /// "TWENTY-FOUR tests go back to skipping" was written in four places and
    /// branded COUNTED rather than estimated. It was wrong: the real figure on
    /// Linux is 22 (14 here after excluding the macOS-only
    /// `the_clipboard_is_really_cut`, 8 in write_code.rs). A number a reader is
    /// told was counted is the one number that must not be off, so it is no
    /// longer typed anywhere without this test deriving it from the source and
    /// checking the four documents that quote it. Add a sandbox test tomorrow
    /// and this goes red naming the figure to move.
    #[test]
    fn the_skip_count_the_documents_quote_is_the_count_in_the_source() {
        let (run_total, run_macos) = skipping_tests(include_str!("run_code.rs"));
        let (write_total, write_macos) = skipping_tests(include_str!("write_code.rs"));
        // NOT VACUOUS: a scanner that stopped matching would report zero and
        // every containment check below would then be looking for "0".
        assert!(
            run_total >= 10 && write_total >= 5,
            "the scan found {run_total}/{write_total} skipping tests, far fewer than the files \
             hold — the scanner stopped matching"
        );
        let run_linux = run_total - run_macos;
        let write_linux = write_total - write_macos;
        let total = run_linux + write_linux;

        // Three parents: the workflow is `<repo>/.github/workflows/ci.yml`.
        let root = ci_workflow()
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .expect("the workflow lives under .github/workflows")
            .to_path_buf();
        let claims: [(&str, String); 6] = [
            (".github/workflows/ci.yml", format!("{total} on Linux")),
            (
                ".github/workflows/ci.yml",
                format!("{run_linux} in run_code.rs"),
            ),
            (
                ".github/workflows/ci.yml",
                format!("{write_linux} in write_code.rs"),
            ),
            ("README.md", format!("{total} tests that used to skip")),
            (
                "crates/tacet-tools/src/run_code.rs",
                format!("and {total} tests go"),
            ),
            (
                "crates/tacet-tools/src/write_code.rs",
                format!("{write_linux} here and {run_linux} in run_code.rs on Linux"),
            ),
        ];
        for (document, claim) in &claims {
            let path = root.join(document);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} is unreadable: {e}", path.display()));
            assert!(
                text.contains(claim.as_str()),
                "{document} no longer says {claim:?}. Counted from the source right now: \
                 {run_linux} skipping tests in run_code.rs that compile on Linux ({run_macos} \
                 more are macOS-only), {write_linux} in write_code.rs, {total} in total."
            );
        }
    }

    /// 4c) THE bwrap ARGUMENT ORDER — the only measurable part of the Linux shield.
    ///
    /// The shield itself cannot be measured on this machine (no Linux), but
    /// BUILDING the arguments is a pure function and runs on every platform.
    /// What is measured is the single subtlety in the documentation: `--tmpfs
    /// {home}` must come BEFORE `--bind {sandbox}`. In production the sandbox is
    /// often UNDER the home directory; in the reverse order the tmpfs covers the
    /// sandbox and the script sees its own folder EMPTY. This is a bug that does
    /// not break the build and would never be caught without a test.
    // UNIX ONLY, and not out of convenience: `bwrap_args` guards the home
    // directory with `home.is_absolute()`, and on Windows `/home/user` is NOT
    // absolute (a drive prefix is required), so the argument under test is
    // never produced there. bwrap is a Linux mechanism to begin with; running
    // this on Windows measured the meaning of `is_absolute`, not the sandbox.
    #[cfg(unix)]
    #[test]
    fn the_bwrap_arguments_cover_the_home_directory_before_the_sandbox() {
        let sandbox = Path::new("/home/user/.tacet/sandbox");
        let args: Vec<String> = bwrap_args(sandbox, Some(Path::new("/home/user")), &[])
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();

        assert!(
            args.contains(&"--unshare-net".to_string()),
            "the network was not cut: {args:?}"
        );
        let tmpfs = args
            .iter()
            .position(|a| a == "--tmpfs")
            .expect("the home directory was not covered");
        assert_eq!(args[tmpfs + 1], "/home/user");
        let bind = args
            .iter()
            .position(|a| a == "--bind")
            .expect("the sandbox was not bound");
        assert!(
            tmpfs < bind,
            "the tmpfs covers the sandbox, the order is reversed: {args:?}"
        );
        assert_eq!(args[bind + 1], sandbox.to_string_lossy());

        // The root must be bound READ ONLY: that is what cuts writing outside the sandbox.
        let ro = args
            .iter()
            .position(|a| a == "--ro-bind")
            .expect("the root was not bound");
        assert_eq!(args[ro + 1], "/");
        assert!(
            !args.iter().any(|a| a == "--bind-try"),
            "a try-bind is used, it passes silently: {args:?}"
        );
    }

    /// 4d) `--tmpfs /` IS A DISASTER: it empties the root directory and the
    ///     interpreter does not start at all. `HOME=/` or a broken/missing
    ///     `HOME` must not produce it — and even if the home directory cannot be
    ///     covered, the network and write gates must stay CLOSED. A socket
    ///     directory that resolved to `/` is refused by the same guard.
    #[test]
    fn bwrap_gives_up_on_the_root_when_the_home_directory_is_broken() {
        for home in [Some(Path::new("/")), Some(Path::new("relative/path")), None] {
            let args: Vec<String> =
                bwrap_args(Path::new("/tmp/sandbox"), home, &[Shadow::Dir("/".into())])
                    .into_iter()
                    .map(|a| a.to_string_lossy().into_owned())
                    .collect();
            assert!(
                !args.iter().any(|a| a == "--tmpfs"),
                "a tmpfs was produced for {home:?}: {args:?}"
            );
            assert!(
                args.contains(&"--unshare-net".to_string()),
                "{home:?}: the network was left open"
            );
            assert!(
                args.contains(&"--ro-bind".to_string()),
                "{home:?}: the root was left writable"
            );
        }
    }

    /// 4d-ii) THE SOCKET DIRECTORIES ARE COVERED (audit H7).
    ///
    /// `--unshare-net` does not touch a unix socket that is a FILE, and
    /// `--ro-bind / /` made all of them reachable — `connect` is not a write.
    /// What is checked is the argv, because that is all this machine can run:
    /// each runtime directory gets a tmpfs, a loose socket gets `/dev/null`
    /// bound over it, and both come AFTER the home tmpfs and BEFORE the sandbox
    /// bind (in the reverse order the tmpfs would cover the sandbox when it
    /// lives under `/run` or `/tmp`). The kernel accepting it is measured by
    /// the ubuntu CI leg, where `verify_shield`'s socket probe runs against the
    /// runner's own sockets.
    #[cfg(unix)]
    #[test]
    fn the_bwrap_arguments_cover_the_socket_directories() {
        let sandbox = Path::new("/home/user/project/.tacet-sandbox-x");
        let shadows = [
            Shadow::Dir("/run".into()),
            Shadow::Dir("/tmp/.X11-unix".into()),
            Shadow::Socket("/tmp/tmux-1000/default".into()),
            Shadow::Socket("/var/lib/incus/unix.socket".into()),
        ];
        let args: Vec<String> = bwrap_args(sandbox, Some(Path::new("/home/user")), &shadows)
            .into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        let at = |pair: [&str; 2]| {
            args.windows(2)
                .position(|w| w[0] == pair[0] && w[1] == pair[1])
                .unwrap_or_else(|| panic!("{pair:?} is missing: {args:?}"))
        };
        let home = at(["--tmpfs", "/home/user"]);
        let run = at(["--tmpfs", "/run"]);
        let x11 = at(["--tmpfs", "/tmp/.X11-unix"]);
        let tmux = args
            .windows(3)
            .position(|w| w == ["--ro-bind", "/dev/null", "/tmp/tmux-1000/default"])
            .unwrap_or_else(|| panic!("the tmux socket is not shadowed: {args:?}"));
        let incus = args
            .windows(3)
            .position(|w| w == ["--ro-bind", "/dev/null", "/var/lib/incus/unix.socket"])
            .unwrap_or_else(|| panic!("the Incus socket is not shadowed: {args:?}"));
        let bind = at(["--bind", &sandbox.to_string_lossy()]);
        assert!(
            home < run && run < bind && x11 < bind && tmux < bind && incus < bind,
            "{args:?}"
        );
    }

    /// 4d-iii) THE CANDIDATES BECOME A PLAN: resolved, existing, deduplicated,
    ///         and nothing listed twice. A missing directory MUST be dropped —
    ///         bwrap `mkdir`s a missing `--tmpfs` target inside the read-only
    ///         root and dies — and a socket under a covered directory or under
    ///         the home directory is dropped because it no longer exists there.
    #[cfg(unix)]
    #[test]
    fn the_socket_plan_resolves_dedupes_and_skips_what_is_covered() {
        let root = temp_dir("plan");
        let run = root.join("run");
        let user = run.join("user-1000");
        let x11 = root.join("x11");
        let home = root.join("home");
        let loose = root.join("loose");
        for dir in [&user, &x11, &home, &loose] {
            std::fs::create_dir_all(dir).expect("fixture");
        }
        // `var-run -> run`, the way every current distribution has it.
        std::os::unix::fs::symlink(&run, root.join("var-run")).expect("link");
        // Stand-ins for sockets: the plan only needs them to EXIST.
        for f in [
            loose.join("agent.sock"),
            user.join("bus"),
            home.join("gpg.sock"),
        ] {
            std::fs::write(f, "").expect("fixture");
        }

        let plan = plan_shadows(
            vec![
                root.join("var-run"),
                run.clone(),
                user.clone(),
                x11.clone(),
                root.join("missing"),
            ],
            vec![user.join("bus"), home.join("gpg.sock"), loose.join("gone")],
            Some(loose.join("agent.sock")),
            Some(&home),
        );
        assert_eq!(
            plan,
            vec![
                Shadow::Dir(run.clone()),
                Shadow::Dir(x11.clone()),
                Shadow::Socket(loose.join("agent.sock")),
            ],
            "the plan for {}",
            root.display()
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 4d-iv) `socket_dirs` names the directories the audit lists, and the
    ///        user's `$XDG_RUNTIME_DIR` when there is one.
    #[test]
    fn the_runtime_directories_the_audit_names_are_candidates() {
        let dirs = socket_dirs(Some(std::ffi::OsStr::new("/run/user/1000")));
        for expected in ["/run", "/var/run", "/tmp/.X11-unix", "/run/user/1000"] {
            assert!(
                dirs.contains(&PathBuf::from(expected)),
                "{expected} is not covered: {dirs:?}"
            );
        }
        assert_eq!(socket_dirs(Some(std::ffi::OsStr::new(""))).len(), 3);
    }

    /// 4d-iv-b) THE SOCKETS OUTSIDE /run AND /tmp ARE IN THE PLAN (audit H7,
    ///          review of 10 Oct 2026). LXD's and Incus's API sockets live under
    ///          `/var/snap` and `/var/lib`, and a working folder outside the
    ///          home directory (`/srv/app`) shows its own sockets through
    ///          `--ro-bind / /`; none of them was in the plan. The fixture puts
    ///          a stand-in daemon socket and two working-folder sockets deeper
    ///          than the `/tmp` scan reaches, so only the new sources can find
    ///          them.
    ///
    ///          The sockets are REAL socket files because `scan_sockets` keeps
    ///          only `is_socket` entries: `bind` creates the file and dropping
    ///          the listener at once leaves it behind. Nothing listens and no
    ///          connection is made. `/tmp` rather than `temp_dir()` because a
    ///          socket path has a 104-byte ceiling on macOS and `$TMPDIR` there
    ///          eats half of it.
    #[cfg(unix)]
    #[test]
    fn the_socket_plan_covers_daemon_sockets_and_the_working_folder() {
        let root = PathBuf::from("/tmp").join(format!("tcs-{}", nonce()));
        std::fs::create_dir_all(&root).expect("fixture");
        let root = root.canonicalize().expect("resolved");
        let work = root.join("srv").join("app");
        let sandbox = work.join("run-1");
        let daemon = root.join("var").join("lib").join("incus");
        let home = root.join("home");
        for dir in [&sandbox, &work.join("pg"), &daemon, &home.join("w")] {
            std::fs::create_dir_all(dir).expect("fixture");
        }
        let socket = |path: &Path| {
            drop(std::os::unix::net::UnixListener::bind(path).expect("a socket file"));
        };
        let gunicorn = work.join("g.sock");
        let postgres = work.join("pg").join(".s.PGSQL.5432");
        let incus = daemon.join("unix.socket");
        for path in [&gunicorn, &postgres, &incus] {
            socket(path);
        }

        let host = ShadowHost {
            daemon_sockets: vec![incus.clone(), root.join("absent").join("unix.socket")],
            ..ShadowHost::default()
        };
        let plan = shadows_for(&sandbox, Some(&home), &host);
        for expected in [&gunicorn, &postgres, &incus] {
            assert!(
                plan.contains(&Shadow::Socket(expected.clone())),
                "{} IS REACHABLE UNDER bwrap — it is not in the plan: {plan:?}",
                expected.display()
            );
        }
        assert!(
            !plan
                .iter()
                .any(|s| matches!(s, Shadow::Socket(p) if p.starts_with(root.join("absent")))),
            "a daemon socket this host does not have was planned: {plan:?}"
        );

        // A working folder under the home directory is not scanned: the home
        // tmpfs hides it already, and scanning it would only cost a walk.
        let roots = socket_scan_roots(None, Some(&home.join("w")), Some(&home));
        assert!(!roots.contains(&home.join("w")), "{roots:?}");
        let roots = socket_scan_roots(None, Some(&work), Some(&home));
        assert!(roots.contains(&work), "{roots:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// 4d-iv-c) THE KNOWN DAEMON SOCKETS ARE BOTH SHADOWED AND MEASURED. The
    ///          plan reads them from `ShadowHost::from_env` and the measurement
    ///          from `probe_socket_candidates`; one without the other is either
    ///          a shadow nobody has seen work or a gap `verify_shield` cannot
    ///          see — the reviewer's second point.
    #[cfg(unix)]
    #[test]
    fn the_known_daemon_sockets_are_shadowed_and_probed() {
        let host = ShadowHost::from_env();
        let probed = probe_socket_candidates(&host);
        for expected in [
            "/var/snap/lxd/common/lxd/unix.socket",
            "/var/lib/lxd/unix.socket",
            "/var/lib/incus/unix.socket",
        ] {
            let expected = PathBuf::from(expected);
            assert!(
                host.daemon_sockets.contains(&expected),
                "{} is not shadowed: {:?}",
                expected.display(),
                host.daemon_sockets
            );
            assert!(
                probed.contains(&expected),
                "{} is not measured by verify_shield: {probed:?}",
                expected.display()
            );
        }
    }

    /// 4d-iv-d) A BIG WORKING FOLDER DOES NOT STARVE /tmp (audit H7, review
    ///          of 10 Oct 2026). The scan budget was shared and the working
    ///          folder walked first, so 5,000 files beside the sandbox spent it
    ///          and a socket in /tmp left the plan. The comparison is against
    ///          the same socket beside a SMALL folder, so a /tmp that is itself
    ///          over budget on some machine reads as "skipped", not as a pass.
    #[cfg(unix)]
    #[test]
    fn a_big_working_folder_does_not_starve_the_tmp_scan() {
        let tmp_dir = PathBuf::from("/tmp").join(format!("tcr-{}", nonce()));
        std::fs::create_dir_all(&tmp_dir).expect("fixture");
        let tmp_dir = tmp_dir.canonicalize().expect("resolved");
        let tmp_socket = tmp_dir.join("s.sock");
        drop(std::os::unix::net::UnixListener::bind(&tmp_socket).expect("a socket file"));

        let base = temp_dir("big-workdir");
        let small = base.join("small");
        let big = base.join("big");
        for dir in [
            &small.join("run-1"),
            &big.join("run-1"),
            &big.join("node_modules"),
        ] {
            std::fs::create_dir_all(dir).expect("fixture");
        }
        for i in 0..5_000 {
            std::fs::write(big.join("node_modules").join(format!("f{i}")), b"").expect("fixture");
        }
        let shadowed = |work: &Path| {
            shadows_for(&work.join("run-1"), None, &ShadowHost::default())
                .contains(&Shadow::Socket(tmp_socket.clone()))
        };
        let beside_small = shadowed(&small);
        let beside_big = shadowed(&big);
        std::fs::remove_dir_all(&base).ok();
        std::fs::remove_dir_all(&tmp_dir).ok();
        if !beside_small {
            eprintln!("skipped: /tmp on this machine is over the scan budget on its own");
            return;
        }
        assert!(
            beside_big,
            "a /tmp socket is shadowed beside a small working folder and NOT beside one \
             with 5,000 entries — the folder spent the scan budget"
        );
    }

    /// 4d-iv-e) EVERY SOCKET THE KERNEL LISTS IS IN THE PLAN, wherever it is
    ///          (review of 10 Oct 2026: Multipass and MicroK8s sockets were
    ///          outside every hand-kept rule). The parser reads the fixture's
    ///          format — header, abstract names, a path with a space, a row
    ///          with no path — and a real socket three levels below a folder
    ///          no rule scans is planned once the host lists it.
    #[test]
    fn every_bound_socket_the_kernel_lists_is_shadowed() {
        let fixture = "\
Num       RefCount Protocol Flags    Type St Inode Path
0000000000000000: 00000002 00000000 00010000 0001 01 23456 /run/systemd/private
0000000000000000: 00000002 00000000 00010000 0001 01 23457 @/tmp/.X11-unix/X0
0000000000000000: 00000002 00000000 00010000 0001 01 23458 /var/snap/multipass/common/multipass_socket
0000000000000000: 00000003 00000000 00000000 0001 03 23459
0000000000000000: 00000002 00000000 00010000 0001 01 23460 /srv/my app/run/x.sock
0000000000000000: 00000002 00000000 00010000 0001 01 23456 /run/systemd/private
";
        assert_eq!(
            bound_sockets_from(fixture),
            [
                PathBuf::from("/run/systemd/private"),
                PathBuf::from("/var/snap/multipass/common/multipass_socket"),
                PathBuf::from("/srv/my app/run/x.sock"),
            ]
        );
        assert!(bound_sockets_from("").is_empty());

        #[cfg(unix)]
        {
            let root = PathBuf::from("/tmp").join(format!("tcb-{}", nonce()));
            let deep = root.join("a").join("b").join("c");
            std::fs::create_dir_all(&deep).expect("fixture");
            let deep = deep.canonicalize().expect("resolved");
            let socket = deep.join("d.sock");
            drop(std::os::unix::net::UnixListener::bind(&socket).expect("a socket file"));
            let sandbox = root.join("work").join("run-1");
            std::fs::create_dir_all(&sandbox).expect("fixture");
            let without = shadows_for(&sandbox, None, &ShadowHost::default());
            let with = shadows_for(
                &sandbox,
                None,
                &ShadowHost {
                    bound_sockets: vec![socket.clone()],
                    ..ShadowHost::default()
                },
            );
            std::fs::remove_dir_all(&root).ok();
            assert!(
                !without.contains(&Shadow::Socket(socket.clone())),
                "the fixture must be out of the scan's reach, or it measures the scan"
            );
            assert!(
                with.contains(&Shadow::Socket(socket.clone())),
                "a socket the kernel lists is not in the plan: {with:?}"
            );
        }
    }

    /// 4d-v) THE HOME RULE NAMES THE RESOLVED DIRECTORY (audit M2).
    ///
    /// `sandbox-exec` matches rules against the RESOLVED path of an access, so a
    /// `$HOME` that is a link made `(deny file-read* (subpath HOMEDIR))` match
    /// nothing — measured by the audit with a symlinked home and a dummy secret,
    /// which was read. The fake home here is a link to a directory, the way a
    /// moved home is.
    #[cfg(unix)]
    #[test]
    fn the_home_rule_names_the_resolved_directory() {
        let root = temp_dir("home-link");
        let real = root.join("real-home");
        std::fs::create_dir_all(&real).expect("fixture");
        let alias = root.join("alias-home");
        std::os::unix::fs::symlink(&real, &alias).expect("link");

        assert_eq!(
            canonical_home(Some(alias.clone().into_os_string())),
            real.canonicalize().expect("resolved"),
            "a symlinked $HOME went into the profile unresolved — the deny rule matches nothing"
        );
        // A home behind a link to a link resolves all the way.
        let alias2 = root.join("alias-of-alias");
        std::os::unix::fs::symlink(&alias, &alias2).expect("link");
        assert_eq!(
            canonical_home(Some(alias2.into_os_string())),
            real.canonicalize().expect("resolved")
        );
        // Unusable values fall back to the directory every home lives under.
        assert_eq!(canonical_home(None), PathBuf::from(FALLBACK_HOME));
        assert_eq!(
            canonical_home(Some(OsString::new())),
            PathBuf::from(FALLBACK_HOME)
        );
        assert_eq!(
            canonical_home(Some("relative/home".into())),
            PathBuf::from(FALLBACK_HOME)
        );
        // A home that does not exist keeps its raw value: nothing behind it.
        let ghost = root.join("ghost-home");
        assert_eq!(canonical_home(Some(ghost.clone().into_os_string())), ghost);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 4d-v-b) AND THE COMMAND `run_program` SPAWNS CARRIES IT (review of 10
    ///         Oct 2026). The test above checks `canonical_home` alone; this
    ///         one builds the command through `shielded_command` — the path
    ///         `run_program` takes — with a symlinked raw `$HOME`, for both
    ///         shields, and reads the home back out of the argv. Splicing the
    ///         raw value there again turns this red on a CI machine whose own
    ///         home is not a link.
    #[cfg(unix)]
    #[test]
    fn the_spawned_command_names_the_resolved_home() {
        let root = temp_dir("home-link-argv");
        let real = root.join("real-home");
        std::fs::create_dir_all(&real).expect("fixture");
        let alias = root.join("alias-home");
        std::os::unix::fs::symlink(&real, &alias).expect("link");
        let real = real.canonicalize().expect("resolved");
        let runtime = temp_dir("home-link-argv-sandbox")
            .canonicalize()
            .expect("resolved");

        let argv = |shield: &NetworkShield| -> Vec<OsString> {
            shielded_command(
                shield,
                &runtime,
                Path::new("/usr/bin/true"),
                &[],
                Some(alias.clone().into_os_string()),
            )
            .expect("a safe path builds a command")
            .get_args()
            .map(OsString::from)
            .collect()
        };

        let seatbelt = argv(&NetworkShield::SandboxExec {
            tool: PathBuf::from("/usr/bin/sandbox-exec"),
        });
        let mut want = OsString::from(format!("{PARAM_HOME}="));
        want.push(&real);
        assert!(
            seatbelt.contains(&want),
            "sandbox-exec was not handed the resolved home: {seatbelt:?}"
        );

        let bwrap = argv(&NetworkShield::Bwrap {
            tool: PathBuf::from("/usr/bin/bwrap"),
        });
        assert!(
            bwrap
                .windows(2)
                .any(|w| w[0] == "--tmpfs" && Path::new(&w[1]) == real),
            "bwrap did not cover the resolved home: {bwrap:?}"
        );

        // Neither command names the link: a rule written with it matches
        // nothing the kernel resolves.
        for arg in seatbelt.iter().chain(&bwrap) {
            assert!(
                !arg.to_string_lossy().contains("alias-home"),
                "the symlinked home reached the shield unresolved: {arg:?}"
            );
        }
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&runtime).ok();
    }

    /// 4d-vi) THE VERDICT DEMANDS EVERY GATE. A probe that never printed counts
    ///        as a failure, not a pass, and an "open" marker fails it even next
    ///        to a "blocked" one. Unix sockets (H7) and the home directory (M2)
    ///        are gates on every platform; the clipboard and signals (M1) on
    ///        macOS, where they are the shield's job.
    #[test]
    fn the_verdict_demands_every_gate() {
        let mut all = vec![
            "NET_BLOCKED",
            "UNIX_BLOCKED",
            "HOME_BLOCKED",
            "PB_BLOCKED",
            "SIG_BLOCKED",
        ];
        assert_eq!(shield_verdict(&all.join("\n")), Ok(()));

        let always = ["NET_BLOCKED", "UNIX_BLOCKED", "HOME_BLOCKED"];
        let macos = ["PB_BLOCKED", "SIG_BLOCKED"];
        for marker in always
            .iter()
            .chain(macos.iter().filter(|_| cfg!(target_os = "macos")))
        {
            let without: Vec<&str> = all.iter().copied().filter(|m| m != marker).collect();
            assert!(
                shield_verdict(&without.join("\n")).is_err(),
                "a measurement without {marker} passed"
            );
            let opened = marker.replace("BLOCKED", "OPEN");
            assert!(
                shield_verdict(&format!("{}\n{opened}", all.join("\n"))).is_err(),
                "a measurement that printed {opened} passed"
            );
        }
        // The uncertain answers are failures too.
        all.retain(|m| *m != "SIG_BLOCKED");
        all.push("SIG_UNCERTAIN");
        assert_eq!(
            shield_verdict(&all.join("\n")).is_err(),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            shield_verdict(&["NET_UNCERTAIN", "UNIX_BLOCKED", "HOME_BLOCKED"].join("\n")),
            Err("the network")
        );
    }

    /// 4d-vii) THE HOME SENTINEL IS A NAME THAT IS REALLY THERE, and never the
    ///         component of the measurement directory's own path — bwrap
    ///         recreates that one inside the tmpfs, and finding it would fail a
    ///         working shield.
    #[test]
    fn the_home_sentinel_is_real_and_not_on_the_sandbox_path() {
        let home = temp_dir("sentinel");
        std::fs::create_dir_all(home.join("aaa-tmp/measure")).expect("fixture");
        std::fs::write(home.join("bbb-secret"), "x").expect("fixture");
        let sandbox = home.join("aaa-tmp/measure");
        assert_eq!(
            home_sentinel(&home, &sandbox).as_deref(),
            Some("bbb-secret")
        );
        // Elsewhere, the first name in sorted order.
        assert_eq!(
            home_sentinel(&home, Path::new("/elsewhere")).as_deref(),
            Some("aaa-tmp")
        );
        assert_eq!(home_sentinel(&home.join("missing"), &sandbox), None);
        std::fs::remove_dir_all(&home).ok();
    }

    /// 4d-viii) A HOST PATH NEVER BREAKS OUT OF THE PROBE'S STRING. The probes
    ///          carry host paths into a script; a home or socket path with a
    ///          quote or a newline must arrive as DATA. JSON's string syntax is
    ///          a subset of both languages', so a JSON round trip is the proof.
    #[test]
    fn a_host_path_stays_a_string_in_the_probe() {
        let hostile = "/tmp/a\"b\\c\nd'e";
        let lit = literal(hostile);
        assert_eq!(
            serde_json::from_str::<String>(&lit).expect("valid"),
            hostile
        );
        let list = literal_paths(&[PathBuf::from(hostile), PathBuf::from("/x")]);
        assert_eq!(
            serde_json::from_str::<Vec<String>>(&list).expect("valid"),
            vec![hostile.to_string(), "/x".to_string()]
        );
        // No `null`: it is not Python.
        assert!(!list.contains("null"));
    }

    /// The shield-free stand-in for M5 tests: `/bin/echo` "runs" the command
    /// line it is given and prints it, so the test can read where the sandbox
    /// directory and the script were put without needing a real shield.
    #[cfg(unix)]
    fn echo_shield() -> (NetworkShield, Interpreter) {
        (
            NetworkShield::SandboxExec {
                tool: PathBuf::from("/bin/echo"),
            },
            Interpreter {
                key: "python",
                path: PathBuf::from("/usr/bin/true"),
                extension: "py",
            },
        )
    }

    /// The `SBOX=` value in an echoed command line.
    #[cfg(unix)]
    fn echoed_sandbox(output: &str) -> PathBuf {
        output
            .split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{PARAM_SANDBOX}=")))
            .map(PathBuf::from)
            .unwrap_or_else(|| panic!("no {PARAM_SANDBOX}= in {output}"))
    }

    /// 4d-ix) THE SANDBOX IS A FRESH DIRECTORY, NOT THE USER'S `code/` (M5).
    ///
    /// The root used to be `<cwd>/code`: the user's real `code/` directory was
    /// readable and writable by the model's script, and a `code` that was a
    /// link sent the script into the link's target. Now every call gets a new
    /// `.tacet-sandbox-<nonce>` that is gone afterwards.
    #[cfg(unix)]
    #[test]
    fn the_sandbox_is_a_fresh_directory_not_the_users_code_folder() {
        let (shield, interpreter) = echo_shield();
        let root = temp_dir("fresh-sbx");
        std::fs::create_dir_all(root.join("code")).expect("the user's own folder");
        std::fs::write(root.join("code/notes.txt"), "mine").expect("fixture");

        let mut seen = Vec::new();
        for _ in 0..2 {
            let outcome = run(
                &shield,
                &interpreter,
                &root,
                "print(1)",
                Duration::from_secs(5),
            )
            .expect("echo runs");
            let CodeOutcome::Succeeded { output, .. } = outcome else {
                panic!("{outcome:?}")
            };
            let sandbox = echoed_sandbox(&output);
            assert_ne!(
                sandbox,
                root.join("code"),
                "the user's code/ is the sandbox"
            );
            assert_eq!(
                sandbox.parent(),
                Some(root.as_path()),
                "{}",
                sandbox.display()
            );
            assert!(
                sandbox
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(crate::sandbox_path::SANDBOX_PREFIX)),
                "{}",
                sandbox.display()
            );
            assert!(
                output.contains(&format!("{}/script-", sandbox.display())),
                "the script is not inside the sandbox: {output}"
            );
            assert!(!sandbox.exists(), "the sandbox was left behind");
            seen.push(sandbox);
        }
        assert_ne!(seen[0], seen[1], "the sandbox was reused");
        let names: Vec<_> = std::fs::read_dir(root.join("code"))
            .expect("list")
            .flatten()
            .map(|e| e.file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("notes.txt")]);
        std::fs::remove_dir_all(&root).ok();
    }

    /// 4d-x) A WORKING DIRECTORY BEHIND A LINK IS REFUSED, and nothing is
    ///       created in the link's target (M5, "every component").
    #[cfg(unix)]
    #[test]
    fn a_linked_working_directory_gets_no_sandbox() {
        let (shield, interpreter) = echo_shield();
        let root = temp_dir("linked-cwd");
        let outside = temp_dir("linked-cwd-target");
        std::os::unix::fs::symlink(&outside, root.join("work")).expect("link");

        let result = run(
            &shield,
            &interpreter,
            &root.join("work"),
            "print(1)",
            Duration::from_secs(5),
        );
        assert!(
            matches!(result, Err(ToolError::SandboxViolation(_))),
            "{result:?}"
        );
        assert_eq!(std::fs::read_dir(&outside).expect("list").count(), 0);
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&outside).ok();
    }

    /// 4e) THE SCRIPT WRITE NEVER FOLLOWS A PLANTED LINK.
    ///
    /// The attack it measures: the script running under the shield may create a
    /// symlink inside the sandbox and aim it at the NEXT call's file name; the
    /// parent process (unshielded, full user privileges) then wrote through it
    /// and could overwrite `~/.zshrc`. Both a DANGLING link (the target does not
    /// exist yet, which is the interesting case — the victim file gets CREATED)
    /// and a live one must be refused.
    #[cfg(unix)]
    #[test]
    fn the_script_write_refuses_a_planted_symlink() {
        let root = temp_dir("plant");
        let outside = std::env::temp_dir().join(format!(
            "tacet-plant-victim-{}-{}.txt",
            std::process::id(),
            nonce()
        ));
        std::fs::remove_file(&outside).ok();

        let script = root.join("script-known-name.js");
        std::os::unix::fs::symlink(&outside, &script).expect("plant the link");

        let error = create_script(&script, b"console.log(1)").expect_err("must be refused");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::AlreadyExists,
            "the link must be refused, not followed: {error}"
        );
        assert!(
            !outside.exists(),
            "the file OUTSIDE the sandbox was created: {}",
            outside.display()
        );

        // A live link is refused too, and the target is NOT overwritten.
        let live = root.join("live-target.txt");
        std::fs::write(&live, "original").expect("target");
        let second = root.join("script-live.js");
        std::os::unix::fs::symlink(&live, &second).expect("plant");
        assert!(create_script(&second, b"console.log(2)").is_err());
        assert_eq!(std::fs::read_to_string(&live).expect("read"), "original");

        // A fresh name still works — the gate must not break legitimate writes.
        let fresh = root.join("fresh.js");
        create_script(&fresh, b"console.log(3)").expect("a fresh name must work");
        assert_eq!(
            std::fs::read_to_string(&fresh).expect("read"),
            "console.log(3)"
        );
    }

    /// 4e-ii) PARALLEL DISCOVERIES NEVER SHARE A MEASUREMENT DIRECTORY.
    ///
    /// Sixteen threads released together, the way the test suite starts its
    /// discoveries. With the old `{pid}-{nanoseconds}` name and a microsecond
    /// wall clock, some of them landed in the same directory and one deleted
    /// another's script mid-measurement.
    #[test]
    fn parallel_measurements_never_share_a_directory() {
        let gate = std::sync::Arc::new(std::sync::Barrier::new(16));
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let gate = gate.clone();
                std::thread::spawn(move || {
                    gate.wait();
                    temp_measurement_dir()
                })
            })
            .collect();
        let mut dirs = Vec::new();
        for handle in handles {
            match handle.join().expect("thread") {
                Ok(dir) => dirs.push(dir),
                Err(e) => panic!("a measurement directory could not be made: {e}"),
            }
        }
        let unique: std::collections::HashSet<_> = dirs.iter().collect();
        let shared = dirs.len() - unique.len();
        for dir in &unique {
            std::fs::remove_dir_all(dir).ok();
        }
        assert_eq!(shared, 0, "{shared} measurements shared a directory");
    }

    /// 4e-iii) BOTH DESCRIPTIONS FIT THE CATALOG'S CEILING, and neither
    ///         overclaims. `catalog.rs` only ever measures the description of
    ///         the shield THIS machine discovered, so on macOS nothing would
    ///         notice the bwrap text growing past the budget until the Linux
    ///         leg of CI did. The two phrases asserted absent are the L20
    ///         overclaim ("ANY file") and, for bwrap, the macOS-only promise
    ///         about temp folders, which bwrap does not keep.
    #[test]
    fn both_descriptions_fit_the_ceiling_and_claim_only_what_is_measured() {
        // `catalog.rs`'s estimator and ceiling: 2 bytes per 5 tokens, 420.
        let tokens = |text: &str| text.len().saturating_mul(2).div_ceil(5);
        for text in [DESCRIPTION_SANDBOX_EXEC, DESCRIPTION_BWRAP] {
            assert!(tokens(text) <= 420, "~{} tokens: {text}", tokens(text));
            assert!(!text.contains("ANY file"), "{text}");
        }
        assert!(!DESCRIPTION_BWRAP.contains("NO access to your files"));
        assert!(DESCRIPTION_BWRAP.contains("can still read other system files"));
        // L20, the macOS half: /etc is readable under the profile, so "system
        // libraries" was narrower than what the script can read.
        assert!(
            DESCRIPTION_SANDBOX_EXEC.contains("can read nothing except system files"),
            "{DESCRIPTION_SANDBOX_EXEC}"
        );
        assert!(!DESCRIPTION_SANDBOX_EXEC.contains("system libraries"));
    }

    /// 4e-bis) THE SCRIPT'S `O_NOFOLLOW` IS THE PER-ARCHITECTURE BIT (audit
    ///         L15 review). A source guard, because the defect is a constant
    ///         that is right on the machine running the test: on macOS and
    ///         x86_64 Linux the old `0x20000`-for-every-Linux table gives the
    ///         right bit, and only an aarch64 Linux run would show it wrong.
    #[test]
    fn the_script_belt_uses_the_per_architecture_table() {
        let body = include_str!("run_code.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the file has a body");
        assert!(
            !body.contains("const O_NOFOLLOW"),
            "run_code.rs keeps its own O_NOFOLLOW again; on aarch64 Linux 0x20000 is \
             O_LARGEFILE"
        );
        assert!(
            body.contains("crate::create_document::o_nofollow_for("),
            "create_script no longer takes its bit from the per-architecture table"
        );
    }

    /// 4f) THE SCRIPT NAME IS NOT PREDICTABLE. The old name was
    ///     `verify-{pid}-{code.len()}`, i.e. fully determined by values the
    ///     model controls — which is what let it aim the link above.
    #[test]
    fn the_script_name_does_not_repeat() {
        let names: std::collections::HashSet<String> = (0..64).map(|_| nonce()).collect();
        assert_eq!(names.len(), 64, "the nonce repeated itself");
    }

    /// 5) Truncation works at a character boundary — it DOES NOT SPLIT a multi-byte character.
    #[test]
    fn truncation_does_not_corrupt_utf8() {
        let text = "ğüşiöç".repeat(200);
        let k = truncate(&text, 10);
        assert_eq!(k.chars().count(), 10);
        assert!(k.is_char_boundary(k.len()), "invalid UTF-8 was produced");
        assert_eq!(truncate("short", 100), "short");
    }

    // --- Tests that run a real process ---

    /// 6) The script REALLY runs and its output is captured. "The build is
    ///    green" proves nothing; this test proves it.
    #[test]
    fn the_script_really_runs_and_the_output_is_captured() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("run");
        let mut ctx = context(&root);

        let code = match tool.interpreters()[0].key {
            "python" => "print(6*7)",
            _ => "console.log(6*7)",
        };
        let outcome = hold(tool.run(json!({"code": code}), &mut ctx));
        assert_eq!(outcome.state, ToolState::Read, "{}", outcome.chip_text);
        assert!(outcome.to_model.contains("42"), "{}", outcome.to_model);
        assert!(outcome.to_model.starts_with("ok ("));
        assert!(
            ctx.session_tainted(),
            "an external tool ran, the session must be tainted"
        );
    }

    /// The interface names in a `/proc/net/dev` dump.
    ///
    /// A separate function so the parser the Linux claim below rests on can be
    /// measured on a machine that has no `/proc` at all — see
    /// `the_interface_parser_reads_a_real_proc_net_dev_dump`. The two header
    /// lines carry no `:` and are dropped by `split_once` rather than by a
    /// `skip(2)`: a kernel that adds a third header line would silently shift
    /// a `skip` and turn a real interface into a header.
    fn interface_names(dump: &str) -> Vec<String> {
        dump.lines()
            .filter_map(|line| line.split_once(':'))
            .map(|(name, _)| name.trim().to_string())
            .filter(|name| !name.is_empty())
            .collect()
    }

    /// 6b) The parser above, against a real dump. MEASURED on this machine
    ///     (macOS, 2026-09-04): the text below is a verbatim `/proc/net/dev`
    ///     from a Linux host, pasted in, because macOS has no `/proc` to read
    ///     one from — the parse is the only half of the Linux claim that can be
    ///     checked here at all.
    #[test]
    fn the_interface_parser_reads_a_real_proc_net_dev_dump() {
        let dump = "Inter-|   Receive                                                |  Transmit\n\
                    \x20face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n\
                    \x20   lo:    1234      12    0    0    0     0          0         0     1234      12    0    0    0     0       0          0\n\
                    \x20 eth0: 9876543    4321    0    0    0     0          0         0  1234567    2345    0    0    0     0       0          0\n";
        assert_eq!(interface_names(dump), vec!["lo", "eth0"]);
        // A namespace with nothing but loopback — what the child must report.
        assert_eq!(interface_names("h1\nh2\n  lo: 0 0\n"), vec!["lo"]);
        assert!(interface_names("").is_empty());
    }

    /// 7) THE NETWORK IS REALLY CUT. This test passing is the only ground on
    ///    which the "NO network" sentence in `description` is a fact and not a promise.
    ///
    /// THE SECOND HALF, ON LINUX, ANSWERS THE OBJECTION THIS FILE ALREADY MAKES
    /// AGAINST ITSELF (see `verify_shield`'s doc comment): `NET_BLOCKED` proves
    /// "no connection was opened", not "the shield cut it", because bwrap's
    /// `--unshare-net` and an unplugged cable both give `ENETUNREACH`. A
    /// NAMESPACE FACT can tell them apart where an errno cannot — the host has a
    /// non-`lo` interface in `/proc/net/dev` while the child sees only `lo`. An
    /// offline machine still has `eth0`; a child outside the namespace would
    /// still see it. The host side is read with `std::fs`, never a socket: this
    /// crate is outside the network monopoly and
    /// `crates/tacet-cli/tests/network_monopoly.rs` fails the build over a
    /// `TcpStream` in `crates/*/src`.
    ///
    /// NOT MEASURED ON THIS MACHINE: macOS has no `/proc`, so the child prints
    /// `IFACE_UNREADABLE` here and the whole differential is skipped. Its first
    /// execution anywhere is the CI job that installs bubblewrap.
    #[test]
    fn the_network_is_really_cut() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("network");
        let mut ctx = context(&root);

        // THE INTERFACE PROBE RUNS FIRST, AND THAT ORDER IS LOAD-BEARING: the js
        // socket probe ends every branch in `process.exit(0)` INSIDE a callback,
        // so anything printed after it might never run (the same rule that makes
        // `clipboard_probe` run before `network_probe` in `measurement_script`).
        let code = match tool.interpreters()[0].key {
            "python" => {
                "try:\n\
                 \x20   for line in open('/proc/net/dev').read().splitlines():\n\
                 \x20       if ':' in line:\n\
                 \x20           print('IFACE:' + line.split(':')[0].strip())\n\
                 except Exception:\n\
                 \x20   print('IFACE_UNREADABLE')\n\
                 import socket\n\
                 try:\n\
                 \x20   socket.create_connection(('1.1.1.1', 80), timeout=3)\n\
                 \x20   print('NET_OPEN')\n\
                 except Exception:\n\
                 \x20   print('NET_BLOCKED')\n"
            }
            _ => {
                "try{const d=require('fs').readFileSync('/proc/net/dev','utf8').split('\\n');\
                 for(const r of d){if(r.indexOf(':')<0)continue;\
                 const n=r.split(':')[0].trim();if(n)console.log('IFACE:'+n)}}\
                 catch(e){console.log('IFACE_UNREADABLE')}\n\
                 const s=require('net').connect(80,'1.1.1.1');\
                 s.on('connect',()=>{console.log('NET_OPEN');process.exit(0)});\
                 s.on('error',()=>{console.log('NET_BLOCKED');process.exit(0)});"
            }
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            !output.contains("NET_OPEN"),
            "THE NETWORK WAS LEFT OPEN — the tool must not be in the catalog: {output}"
        );
        assert!(
            output.contains("NET_BLOCKED"),
            "unexpected output: {output} / {}",
            outcome.to_model
        );

        if !cfg!(target_os = "linux") {
            return;
        }
        // `cfg!` rather than `#[cfg]` so this arm is type-checked on macOS too:
        // the one thing that must not happen to Linux-only code in a repository
        // with no Linux is that it stops compiling without anyone noticing.
        let host = interface_names(&std::fs::read_to_string("/proc/net/dev").unwrap_or_default());
        let child: Vec<&str> = output
            .lines()
            .filter_map(|line| line.strip_prefix("IFACE:"))
            .collect();
        assert!(
            !output.contains("IFACE_UNREADABLE"),
            "`--proc /proc` did not give the child a procfs, so the differential \
             measured nothing: {output}"
        );
        assert!(
            child.contains(&"lo"),
            "the child reported no loopback at all, so its /proc/net/dev was not \
             parsed as expected: {output}"
        );
        let leaked: Vec<&&str> = child.iter().filter(|name| **name != "lo").collect();
        assert!(
            leaked.is_empty(),
            "the child is in the HOST's network namespace — it can see {leaked:?}; \
             --unshare-net did not take effect"
        );
        // The host half. It is only informative on a machine that itself has no
        // interface but loopback (a container with no veth): there the two sides
        // agree for a reason that has nothing to do with the shield, so the
        // differential proves nothing and saying so is better than a green tick.
        let host_has_more = host.iter().any(|name| name != "lo");
        assert!(
            host_has_more || !sandbox_must_run(),
            "{SANDBOX_MUST_RUN} is set but the host has only {host:?}, so the \
             namespace differential cannot distinguish a cut network from no \
             network — this job is not measuring what it claims to"
        );
        if !host_has_more {
            eprintln!(
                "the host itself has only {host:?}: the interface differential degenerates here \
                 and only the NET_BLOCKED half was measured"
            );
        }
    }

    /// 8) THE SANDBOX: the script cannot write outside and cannot read the
    ///    user's home directory; but it can write into its own folder (legitimate work must not break).
    ///
    /// THE HOME HALF USED TO MEASURE NOTHING, on any platform, and that is worth
    /// writing down because it looked measured. The python branch listed
    /// `os.path.expanduser('~')` — but `run_program` sets the child's `HOME` to
    /// the sandbox itself (`command.env("HOME", runtime)`), so `~` named the
    /// sandbox and the listing succeeded for a reason that has nothing to do
    /// with the user's files. The js branch read `process.env.HOME_REAL||'/Users'`,
    /// a variable nothing sets and a directory that does not exist on Linux, so
    /// it threw for the wrong reason. And neither `HOME_READ_OK` nor
    /// `HOME_READ_BLOCKED` was ever asserted on. `--tmpfs {home}` (bwrap) and
    /// `(deny file-read* (subpath (param "HOMEDIR")))` (SBPL) were therefore
    /// completely unmeasured; the argument-order test only proved the FLAG was
    /// produced.
    ///
    /// THE CANARY FIXES IT: a file with an unguessable body is planted in the
    /// REAL home, and its absolute path goes into the script AS A LITERAL,
    /// because the child cannot name the user's home any other way.
    ///
    /// THE WRITE HALF IS ASSERTED ON THE HOST, NOT ON THE CHILD'S REPORT, and
    /// that is the correction to the obvious guess: bwrap's `--tmpfs` is a FRESH
    /// WRITABLE filesystem, so a write into the covered home SUCCEEDS inside the
    /// child and is discarded with the namespace. macOS refuses the same write
    /// outright. Two different child-visible outcomes, one guarantee — the file
    /// must not exist on the host afterwards.
    #[test]
    fn the_sandbox_cannot_be_escaped() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("sandbox");
        let mut ctx = context(&root);
        let target = std::env::temp_dir().join(format!("tacet-escape-{}.txt", std::process::id()));
        std::fs::remove_file(&target).ok();

        // Planted in the developer's own home directory, so it is removed on
        // EVERY path — an assert below panicking included, which is why this is
        // a Drop guard and not a call at the end of the function.
        struct Canary(Vec<PathBuf>);
        impl Drop for Canary {
            fn drop(&mut self) {
                for path in &self.0 {
                    std::fs::remove_file(path).ok();
                }
            }
        }

        let body = format!("HOME-CANARY-BODY-{}", nonce());
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|home| home.is_absolute() && home != Path::new("/"));
        let planted = home.as_ref().map(|home| {
            (
                home.join(format!(".tacet-home-canary-{}", std::process::id())),
                home.join(format!(".tacet-home-write-{}", std::process::id())),
            )
        });
        let planted = match planted {
            Some((read_at, write_at)) => {
                std::fs::remove_file(&write_at).ok();
                match std::fs::write(&read_at, &body) {
                    Ok(()) => Some((read_at, write_at)),
                    Err(e) => {
                        eprintln!("$HOME is not writable ({e}), the home half was not measured");
                        None
                    }
                }
            }
            None => {
                eprintln!("$HOME is unset or is `/`, the home half was not measured");
                None
            }
        };
        let _canary = Canary(
            planted
                .iter()
                .flat_map(|(a, b)| [a.clone(), b.clone()])
                .collect(),
        );

        let home_probe = match (&planted, tool.interpreters()[0].key) {
            (None, _) => String::new(),
            (Some((read_at, write_at)), "python") => format!(
                "try:\n\
                 \x20   print('HOME_CANARY_VISIBLE:' + open({read_at:?}).read())\n\
                 except Exception:\n\
                 \x20   print('HOME_CANARY_HIDDEN')\n\
                 try:\n\
                 \x20   open({write_at:?}, 'w').write('x')\n\
                 \x20   print('HOME_WRITE_RETURNED_OK')\n\
                 except Exception:\n\
                 \x20   print('HOME_WRITE_BLOCKED')\n"
            ),
            (Some((read_at, write_at)), _) => format!(
                "try{{console.log('HOME_CANARY_VISIBLE:'+require('fs')\
                 .readFileSync({read_at:?},'utf8'))}}\
                 catch(e){{console.log('HOME_CANARY_HIDDEN')}}\n\
                 try{{require('fs').writeFileSync({write_at:?},'x');\
                 console.log('HOME_WRITE_RETURNED_OK')}}\
                 catch(e){{console.log('HOME_WRITE_BLOCKED')}}\n"
            ),
        };

        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "{home_probe}\
                 try:\n\
                 \x20   open({target:?}, 'w').write('x')\n\
                 \x20   print('OUT_WRITE_OK')\n\
                 except Exception:\n\
                 \x20   print('OUT_WRITE_BLOCKED')\n\
                 open('inside.txt','w').write('y')\n\
                 print('IN_WRITE_OK')\n"
            ),
            _ => format!(
                "{home_probe}\
                 const fs=require('fs');\
                 try{{fs.writeFileSync({target:?},'x');console.log('OUT_WRITE_OK')}}\
                 catch(e){{console.log('OUT_WRITE_BLOCKED')}}\
                 fs.writeFileSync('inside.txt','y');console.log('IN_WRITE_OK')"
            ),
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            output.contains("OUT_WRITE_BLOCKED"),
            "it wrote outside the sandbox: {output}"
        );
        assert!(
            !target.exists(),
            "the file was really created: {}",
            target.display()
        );
        assert!(
            output.contains("IN_WRITE_OK"),
            "it could not write into the sandbox: {output}"
        );

        if let Some((_, write_at)) = &planted {
            assert!(
                !output.contains(&body),
                "THE USER'S HOME DIRECTORY WAS READABLE — the canary's contents \
                 came back through the model's script: {output}"
            );
            assert!(
                output.contains("HOME_CANARY_HIDDEN"),
                "the home directory was not covered: {output}"
            );
            assert!(
                !write_at.exists(),
                "a write from inside the sandbox landed in the user's home directory: {}",
                write_at.display()
            );
        }
    }

    /// THE END STATE: nothing the script started is still running when the tool
    /// says it is done.
    ///
    /// THE HOLE THIS FOUND. The whole process group was killed on the TIMEOUT
    /// path only. A script that exits normally after starting a background
    /// child left that child running — outside every timeout, holding the
    /// sandbox directory and the pipe ends, burning CPU — while the tool
    /// reported success and the turn moved on. `--die-with-parent` covers it on
    /// Linux; macOS `sandbox-exec` has no equivalent, and macOS is where this
    /// project's numbers are measured.
    ///
    /// THE CANARY IS A FILE WITH A DELAY, not a process listing. Reading the
    /// process table would measure this machine's other tenants; a file that
    /// appears three seconds after the tool returned can only have been written
    /// by something the tool left behind.
    #[test]
    fn nothing_survives_the_call() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("survivor");
        let mut ctx = context(&root);
        let canary = std::env::temp_dir().join(format!("tacet-survivor-{}.txt", nonce()));
        std::fs::remove_file(&canary).ok();

        struct Canary(PathBuf);
        impl Drop for Canary {
            fn drop(&mut self) {
                std::fs::remove_file(&self.0).ok();
            }
        }
        let _guard = Canary(canary.clone());

        // Detached, so it is not the process we spawned; it writes OUTSIDE the
        // sandbox on purpose, because a survivor that can only write inside a
        // directory nobody reads again is not the failure being measured.
        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "import subprocess, sys\n\
                 subprocess.Popen([sys.executable, '-c', \
                 \"import time,os;time.sleep(3);open({canary:?},'w').write('alive')\"])\n\
                 print('SPAWNED')\n"
            ),
            _ => format!(
                "const {{spawn}}=require('child_process');\
                 spawn(process.execPath,['-e',\
                 `setTimeout(()=>require('fs').writeFileSync({canary:?},'alive'),3000)`],\
                 {{detached:true,stdio:'ignore'}}).unref();\
                 console.log('SPAWNED')"
            ),
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();

        // TWO ACCEPTABLE END STATES, and this machine takes the stronger one.
        //
        // MEASURED on macOS 15 (7 Sep 2026): the `sandbox-exec` profile refuses
        // the spawn outright — node comes back `Error: spawn EPERM`, python the
        // same — so a survivor is not killed afterwards, it never exists. On
        // Linux `bwrap` gets there differently: `--unshare-pid` plus
        // `--die-with-parent` tears the namespace down with the parent.
        //
        // The test accepts either, because the guarantee is about the END STATE
        // and not about which layer delivers it. What it does not accept is a
        // child that outlives the call.
        let spawned = output.contains("SPAWNED");
        if !spawned {
            let refused = output.contains("EPERM")
                || output.to_lowercase().contains("permission")
                || output.contains("Operation not permitted");
            assert!(
                refused,
                "the probe neither spawned nor was refused, so nothing was \
                 measured — fix the probe rather than the assertion: {output}"
            );
        }

        // Longer than the child's own delay, so a survivor has had its chance.
        std::thread::sleep(std::time::Duration::from_secs(5));
        assert!(
            !canary.exists(),
            "a process the script started outlived the call and wrote {} after \
             the tool had already reported done (spawn {}): {output}",
            canary.display(),
            if spawned { "succeeded" } else { "was refused" }
        );
    }

    /// 9) AN INFINITE LOOP ENDS AT ONE OF TWO THINGS, and both are measured: on
    ///    the timeout the process is KILLED and the duration really is cut at
    ///    the cap (no silent hang); on a STOP (audit M4) it ends within about a
    ///    second, whatever the timeout says. The second half is new — before
    ///    it, nothing read the stop, and an endless loop ran to its full timeout
    ///    while the screen said "cancelling".
    ///
    ///    ONE TEST FOR BOTH, not two: the subject is "what ends a looping
    ///    script", and the setup (the tool, the loop) is the same.
    #[test]
    fn an_infinite_loop_is_killed_on_timeout_or_on_a_stop() {
        use tacet_kernel::context::CancelToken;
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("loop");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "while True: pass",
            _ => "while(true){}",
        };

        let started = Instant::now();
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 2}), &mut ctx));
        let elapsed = started.elapsed();
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
        assert!(
            outcome.to_model.contains("did not finish in time"),
            "{}",
            outcome.to_model
        );
        assert!(
            elapsed < Duration::from_secs(10),
            "the process was not killed: {elapsed:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(2),
            "it returned before the cap: {elapsed:?}"
        );

        // THE STOP. A fresh turn (the tool allows two attempts per turn), the
        // longest timeout the schema permits, and a stop after 300 ms.
        tool.turn_state().new_turn();
        let token = CancelToken::new();
        let mut ctx = context(&root).with_cancel_token(token.clone());
        let stopper = {
            let token = token.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                token.cancel();
            })
        };
        let started = Instant::now();
        let outcome = hold(tool.run(json!({ "code": code, "timeout_s": MAX_TIMEOUT }), &mut ctx));
        let elapsed = started.elapsed();
        stopper.join().expect("stopper");
        assert!(
            elapsed < Duration::from_millis(300) + Duration::from_millis(1500),
            "the stop took {elapsed:?}"
        );
        assert_eq!(
            outcome.state,
            ToolState::Failed(STOPPED.into()),
            "{}",
            outcome.to_model
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// 9b) THE TIMEOUT REALLY BOUNDS THE CALL, even when the script tries to
    ///     leave a process behind that holds the output pipe.
    ///
    /// THE ATTACK: `spawn('/bin/sleep', {detached:true, stdio:[...,'inherit','inherit']})`
    /// plus an endless loop. Killing only the direct child left the grandchild
    /// alive, the stdout pipe never reached EOF, and the unbounded `join` hung
    /// the turn FOREVER — with gate 4 (cancellation) unable to help, because
    /// this runs synchronously inside the async body. Two independent fixes now
    /// stand between that and a hang (no `process-fork` in the profile, and a
    /// bounded join); this test measures the OUTCOME, so it stays honest if
    /// either one is changed.
    #[test]
    fn a_detached_child_cannot_hang_the_turn() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("detach");
        let code = match tool.interpreters()[0].key {
            "python" => {
                "import subprocess\n\
                 try:\n\
                 \x20   subprocess.Popen(['/bin/sleep','600'], start_new_session=True)\n\
                 except Exception:\n\
                 \x20   pass\n\
                 while True: pass\n"
            }
            _ => {
                "try{require('child_process').spawn('/bin/sleep',['600'],\
                 {detached:true,stdio:['ignore','inherit','inherit']}).unref()}catch(e){}\
                 while(true){}"
            }
        };

        let started = Instant::now();
        let outcome = run(
            &tool.shield,
            &tool.interpreters()[0],
            &root,
            code,
            Duration::from_secs(2),
        )
        .expect("run_program must return");
        let elapsed = started.elapsed();

        assert_eq!(outcome, CodeOutcome::Timeout, "{outcome:?}");
        // 2 s timeout + JOIN_GRACE + slack. Before the fix this never returned.
        assert!(
            elapsed < Duration::from_secs(8),
            "the call did not return within its bound: {elapsed:?}"
        );
    }

    /// 9c) A parent that EXITS IMMEDIATELY while a descendant keeps the pipe
    ///     open must not hang the call either. This is the case the timeout path
    ///     never sees (`timed_out == false`), so the process-group kill alone
    ///     would not save it — only the bounded join does.
    #[test]
    fn a_pipe_held_after_the_parent_exits_does_not_hang_the_turn() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("orphan");
        let code = match tool.interpreters()[0].key {
            "python" => {
                "import subprocess\n\
                 try:\n\
                 \x20   subprocess.Popen(['/bin/sleep','600'], start_new_session=True)\n\
                 except Exception:\n\
                 \x20   pass\n\
                 print('PARENT_DONE')\n"
            }
            _ => {
                "try{require('child_process').spawn('/bin/sleep',['600'],\
                 {detached:true,stdio:['ignore','inherit','inherit']}).unref()}catch(e){}\
                 console.log('PARENT_DONE')"
            }
        };

        let started = Instant::now();
        let outcome = run(
            &tool.shield,
            &tool.interpreters()[0],
            &root,
            code,
            Duration::from_secs(20),
        )
        .expect("run_program must return");
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_secs(10),
            "the call hung on a pipe held by a descendant: {elapsed:?} / {outcome:?}"
        );
    }

    /// 9d) THE CLIPBOARD IS REALLY CUT. `(allow mach-lookup)` used to stand
    ///     unfiltered, which opened XPC to the pasteboard: the script could run
    ///     `/usr/bin/pbpaste` and print whatever the user had just copied —
    ///     typically a password or a 2FA code — straight into the model's
    ///     window. The counterpart of `the_network_is_really_cut`; without it
    ///     the "NO access to this device" sentence is a promise, not a fact.
    ///
    /// It reads the GENERAL pasteboard but never writes to it: the test must not
    /// disturb what the developer has on their clipboard.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_clipboard_is_really_cut() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("clipboard");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => {
                "import subprocess\n\
                 try:\n\
                 \x20   subprocess.run(['/usr/bin/pbpaste'], check=True, capture_output=True)\n\
                 \x20   print('PB_OPEN')\n\
                 except Exception:\n\
                 \x20   print('PB_BLOCKED')\n"
            }
            _ => {
                "try{require('child_process').execFileSync('/usr/bin/pbpaste',\
                 {stdio:['ignore','pipe','ignore']});console.log('PB_OPEN')}\
                 catch(e){console.log('PB_BLOCKED')}"
            }
        };

        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            !output.contains("PB_OPEN"),
            "THE CLIPBOARD WAS READABLE — the tool must not be in the catalog: {output}"
        );
        assert!(
            output.contains("PB_BLOCKED"),
            "unexpected output: {output} / {}",
            outcome.to_model
        );
    }

    /// A process OUTSIDE the sandbox that the test owns: killed and reaped on
    /// every path, an assert panicking included.
    #[cfg(unix)]
    struct Outsider(std::process::Child);
    #[cfg(unix)]
    impl Drop for Outsider {
        fn drop(&mut self) {
            self.0.kill().ok();
            self.0.wait().ok();
        }
    }

    /// A unix-socket listener started OUTSIDE the sandbox with the same
    /// interpreter, so the probes have something real to reach. Started with
    /// the interpreter rather than from Rust on purpose: this crate is outside
    /// the network monopoly (`crates/tacet-cli/tests/network_monopoly.rs`), and
    /// a socket opened by its own code — even a local one, even in a test —
    /// is the thing that file exists to keep out.
    #[cfg(unix)]
    fn unix_listener(interpreter: &Interpreter, path: &Path) -> Outsider {
        use std::io::BufRead;
        let code = match interpreter.key {
            "python" => {
                "import socket,sys\n\
                 s=socket.socket(socket.AF_UNIX)\n\
                 s.bind(sys.argv[1])\n\
                 s.listen(8)\n\
                 print('READY',flush=True)\n\
                 while True:\n\
                 \x20   c,_=s.accept()\n\
                 \x20   c.close()\n"
            }
            _ => {
                "require('net').createServer(c=>c.end())\
                 .listen(process.argv[1],()=>console.log('READY'))"
            }
        };
        let flag = if interpreter.key == "python" {
            "-c"
        } else {
            "-e"
        };
        let mut child = Command::new(&interpreter.path)
            .arg(flag)
            .arg(code)
            .arg(path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the listener starts");
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().expect("stdout"))
            .read_line(&mut line)
            .expect("the listener reports");
        assert_eq!(line.trim(), "READY", "the listener did not come up");
        Outsider(child)
    }

    /// 9d-ii) THE PROBES CAN SEE AN OPEN GATE, AND THE SHIELD CLOSES IT.
    ///
    /// A probe that can only ever print "blocked" measures nothing — the
    /// repository rule about a green test that cannot go red applies to
    /// `verify_shield` too. So the local probes (`local_probes`: home
    /// directory, signals, unix sockets) are run TWICE over the same targets:
    /// WITHOUT the shield, where each of them must report OPEN, and under it,
    /// where each must report BLOCKED. The targets are the test's own: a
    /// listening socket and a fake home with a file in it in the temp
    /// directory, and a `sleep` it started — signal 0 delivers nothing. The
    /// network probe is NOT in the unshielded run: it would open a real
    /// connection.
    ///
    /// EVERY INTERPRETER, NOT ONLY THE FIRST. This test used to probe
    /// `interpreters()[0]` — node, on every machine that has it — so the
    /// PYTHON probe never ran anywhere: the review flipped its socket check to
    /// "never open" and this stayed green. On a machine whose only interpreter
    /// is python, that unproven probe is the whole of `verify_shield`. The
    /// loop also takes Homebrew's python when it is installed but not linked
    /// (`probe_interpreters`), because the python discovery finds on a Mac is
    /// usually the Xcode shim, which cannot start under the shield at all.
    ///
    /// macOS ONLY, and why: under bwrap the temp directory is visible, so the
    /// fake home and the listener's socket would be reachable for a reason that
    /// has nothing to do with the gates being measured. The Linux half of H7
    /// is `a_loose_socket_in_tmp_is_shadowed_under_bwrap`, and `verify_shield`
    /// itself, which probes the runner's real sockets under
    /// `TACET_SANDBOX_MUST_RUN`.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_probes_see_an_open_gate_and_the_shield_closes_it() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("probes");
        let fake_home = root.join("fake-home");
        std::fs::create_dir_all(&fake_home).expect("fixture");
        std::fs::write(fake_home.join("TOP-SECRET-DUMMY"), "x").expect("fixture");
        // Short: a unix socket path is limited to 104 bytes on macOS.
        let socket = std::env::temp_dir().join(format!("tacet-us-{}", nonce()));
        let _listener = unix_listener(&tool.interpreters()[0], &socket);
        let sleeper = Outsider(
            Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("an outside process"),
        );
        let targets = ProbeTargets {
            home: vec![fake_home.clone()],
            home_sentinel: Some("TOP-SECRET-DUMMY".into()),
            sockets: vec![socket.clone()],
            outside_pid: sleeper.0.id(),
        };
        let gates = [
            ("HOME_BLOCKED", "HOME_OPEN"),
            ("SIG_BLOCKED", "SIG_OPEN"),
            ("UNIX_BLOCKED", "UNIX_OPEN"),
        ];
        let mut keys_shielded = Vec::new();
        let mut keys_open = Vec::new();
        for interpreter in probe_interpreters(&tool) {
            let (open, shielded) = probe_both_ways(&tool, &interpreter, &targets, &root);
            for (_, opened) in gates {
                assert!(
                    open.contains(opened),
                    "{}: without the shield the probe did not see {opened} — it cannot \
                     detect an open gate: {open}",
                    interpreter.path.display()
                );
            }
            keys_open.push(interpreter.key);
            let Some(shielded) = shielded else { continue };
            for (blocked, opened) in gates {
                assert!(
                    shielded.contains(blocked) && !shielded.contains(opened),
                    "{}: under the shield the gate is open ({blocked} expected): {shielded}",
                    interpreter.path.display()
                );
            }
            keys_shielded.push(interpreter.key);
        }
        // `verify_shield` measured with `interpreters()[0]` for discovery to
        // succeed at all, so that one at least must have been seen shielded.
        assert!(
            keys_shielded.contains(&tool.interpreters()[0].key),
            "the interpreter discovery verified with was never run under the shield"
        );
        eprintln!("probed open: {keys_open:?}, probed shielded: {keys_shielded:?}");
        std::fs::remove_file(&socket).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// The interpreters the probe tests run: every one discovery found, then
    /// Homebrew's python if it is installed but not linked into `bin` (where
    /// discovery does not look). Paths that do not exist are left out.
    #[cfg(unix)]
    fn probe_interpreters(tool: &RunCodeTool) -> Vec<Interpreter> {
        let mut all: Vec<Interpreter> = tool.interpreters().to_vec();
        for prefix in ["/opt/homebrew/opt", "/usr/local/opt"] {
            let Ok(entries) = std::fs::read_dir(prefix) else {
                continue;
            };
            let mut found: Vec<PathBuf> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("python@3"))
                .map(|e| e.path().join("libexec/bin/python3"))
                .filter(|p| p.is_file())
                .collect();
            found.sort();
            if let Some(path) = found.pop()
                && !all.iter().any(|y| y.path == path)
            {
                all.push(Interpreter {
                    key: "python",
                    path,
                    extension: "py",
                });
            }
        }
        all
    }

    /// Runs `local_probes` for one interpreter WITHOUT the shield and then
    /// UNDER it, over the same targets, and returns both outputs.
    ///
    /// The shielded output is `None` only when the interpreter cannot start
    /// under the shield AT ALL — a control script that prints one line fails
    /// too. That is the Xcode shim (`/usr/bin/python3` needs fork, see
    /// `profile`): a process that never starts reads nothing, and its probe has
    /// already been shown to see open gates in the unshielded run. A probe that
    /// fails while the control runs is a panic, not a skip.
    #[cfg(unix)]
    fn probe_both_ways(
        tool: &RunCodeTool,
        interpreter: &Interpreter,
        targets: &ProbeTargets,
        root: &Path,
    ) -> (String, Option<String>) {
        let exit = if interpreter.key == "python" {
            ""
        } else {
            "process.exit(0)"
        };
        let body = local_probes(interpreter.key, targets, exit);
        let script = root.join(format!("probe-{}.{}", nonce(), interpreter.extension));
        std::fs::write(&script, &body).expect("script");
        let open = Command::new(&interpreter.path)
            .arg(&script)
            .output()
            .expect("the unshielded probe runs");
        let open = String::from_utf8_lossy(&open.stdout).into_owned();

        let sandbox = temp_dir("probes-sbx");
        let inside = sandbox.join(format!("probe.{}", interpreter.extension));
        std::fs::write(&inside, &body).expect("copy");
        let run = |path: &Path| {
            run_program(
                &tool.shield,
                &sandbox,
                &interpreter.path,
                &[path.as_os_str().to_os_string()],
                Duration::from_secs(15),
            )
        };
        let shielded = match run(&inside).expect("the shield starts") {
            CodeOutcome::Succeeded { output, .. } => Some(output),
            other => {
                let control = sandbox.join(format!("control.{}", interpreter.extension));
                let line = if interpreter.key == "python" {
                    "print('CONTROL')"
                } else {
                    "console.log('CONTROL')"
                };
                std::fs::write(&control, line).expect("control");
                let started = matches!(
                    run(&control),
                    Ok(CodeOutcome::Succeeded { ref output, .. }) if output.contains("CONTROL")
                );
                assert!(
                    !started,
                    "{} starts under the shield but its probe failed: {other:?}",
                    interpreter.path.display()
                );
                eprintln!(
                    "{} does not start under the shield at all; its shielded half was not measured",
                    interpreter.path.display()
                );
                None
            }
        };
        std::fs::remove_dir_all(&sandbox).ok();
        (open, shielded)
    }

    /// 9d-ii-linux) A LOOSE SOCKET IN /tmp IS SHADOWED UNDER bwrap (audit H7).
    ///
    /// The `Shadow::Socket` branch — `--ro-bind /dev/null <socket>` — had
    /// never run against a real bwrap: on a CI runner whose `/tmp` holds no
    /// socket, nothing exercised it, and if bwrap rejected that bind `run_code`
    /// would leave the catalog for every Linux user with a tmux or ssh-agent
    /// socket in `/tmp` — fail-closed, but unseen. So the test puts one there
    /// itself, at depth 1 where `scan_sockets` must find it, checks that the
    /// unshielded probe REACHES it (so a blocked result means something), and
    /// that under bwrap it does not. Every discovered interpreter is probed.
    /// Under `TACET_SANDBOX_MUST_RUN` — the ubuntu CI leg — a missing bwrap is
    /// a failure, not a skip.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_loose_socket_in_tmp_is_shadowed_under_bwrap() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("probes-linux");
        let socket = PathBuf::from("/tmp").join(format!("tacet-us-{}", nonce()));
        let _listener = unix_listener(&tool.interpreters()[0], &socket);
        let targets = ProbeTargets {
            home: Vec::new(),
            home_sentinel: None,
            sockets: vec![socket.clone()],
            outside_pid: 0,
        };
        let mut measured = 0;
        for interpreter in probe_interpreters(&tool) {
            let (open, shielded) = probe_both_ways(&tool, &interpreter, &targets, &root);
            assert!(
                open.contains("UNIX_OPEN"),
                "{}: without the shield the probe did not reach {} — it cannot detect an \
                 open socket: {open}",
                interpreter.path.display(),
                socket.display()
            );
            let shielded = shielded.unwrap_or_else(|| {
                panic!(
                    "{} did not start under bwrap with a socket in /tmp — the \
                     `--ro-bind /dev/null` shadow is what bwrap refused",
                    interpreter.path.display()
                )
            });
            assert!(
                shielded.contains("UNIX_BLOCKED") && !shielded.contains("UNIX_OPEN"),
                "{}: THE SOCKET IN /tmp IS REACHABLE UNDER bwrap: {shielded}",
                interpreter.path.display()
            );
            measured += 1;
        }
        assert!(measured > 0, "no interpreter was probed");
        std::fs::remove_file(&socket).ok();
        std::fs::remove_dir_all(&root).ok();
    }

    /// 9d-iii) A SIGNAL CANNOT REACH A PROCESS OUTSIDE THE SANDBOX (audit M1).
    ///
    /// The audit's measurement, as a test: a `sleep` started outside, a SIGTERM
    /// sent from inside. Under the old `(allow signal)` the sleep died with 143.
    /// What is asserted is the END STATE — the outside process is still alive —
    /// and not only the script's report of it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_signal_cannot_reach_a_process_outside() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("signal");
        let mut ctx = context(&root);
        let mut victim = Outsider(
            Command::new("/bin/sleep")
                .arg("60")
                .spawn()
                .expect("an outside process"),
        );
        let pid = victim.0.id();
        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "import os, signal\n\
                 try:\n\
                 \x20   os.kill({pid}, signal.SIGTERM)\n\
                 \x20   print('SIGNAL_SENT')\n\
                 except Exception as e:\n\
                 \x20   print('SIGNAL_REFUSED', type(e).__name__)\n"
            ),
            _ => format!(
                "try{{process.kill({pid},'SIGTERM');console.log('SIGNAL_SENT')}}\
                 catch(e){{console.log('SIGNAL_REFUSED '+e.code)}}"
            ),
        };
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        std::thread::sleep(Duration::from_millis(300));
        assert!(
            matches!(victim.0.try_wait(), Ok(None)),
            "THE PROCESS OUTSIDE THE SANDBOX WAS KILLED from inside it: {output}"
        );
        assert!(output.contains("SIGNAL_REFUSED"), "{output}");
    }

    /// 9d-iv) FILES OUTSIDE THE SANDBOX ARE NOT READABLE — not only the home
    ///        directory (audit L20).
    ///
    /// The audit read the user's temp directory (46,797 entries), `/Users/Shared`
    /// and `/private/tmp` from inside the sandbox, while the description said
    /// "NO access to ANY file". A canary with an unguessable body is planted in
    /// the temp directory OUTSIDE the sandbox, and in the WORKING directory —
    /// which, since M5, is outside the sandbox too. `/usr/bin` is the control:
    /// the allowlist still opens the system, so a probe that fails on
    /// everything is caught here instead of passing.
    ///
    /// HOMEBREW'S DATA DIRECTORIES ARE PROBED TOO, wherever they exist: the
    /// review that followed the allowlist read a live postgres database out of
    /// `/opt/homebrew/var` through `(subpath "/opt/homebrew")`, and the four
    /// probes above never looked there. Each existing `var/`/`etc/` is listed,
    /// and one file found inside it (by the HOST, before the script runs) is
    /// opened; only whether that worked is printed, never a byte of it. A
    /// machine without them measures nothing here, and says so.
    #[cfg(target_os = "macos")]
    #[test]
    fn files_outside_the_sandbox_are_not_readable() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("read-outside");
        let mut ctx = context(&root);
        let body = format!("OUTSIDE-CANARY-{}", nonce());
        let canary = std::env::temp_dir()
            .canonicalize()
            .expect("temp")
            .join(format!("tacet-read-canary-{}", nonce()));
        std::fs::write(&canary, &body).expect("canary");
        std::fs::write(root.join("cwd-canary.txt"), &body).expect("canary");
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                std::fs::remove_file(&self.0).ok();
            }
        }
        let _gone = Gone(canary.clone());
        let cwd_canary = root.join("cwd-canary.txt");
        let data = homebrew_data_targets();
        if data.is_empty() {
            eprintln!("no Homebrew var/ or etc/ on this machine, that half was not measured");
        }
        let data_literal = serde_json::to_string(&data).expect("json");

        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "import os\n\
                 for tag, p in [('TMP', {canary:?}), ('CWD', {cwd_canary:?})]:\n\
                 \x20   try:\n\
                 \x20       print(tag + '_READ:' + open(p).read())\n\
                 \x20   except Exception:\n\
                 \x20       print(tag + '_HIDDEN')\n\
                 for tag, p in [('SHARED', '/Users/Shared'), ('PTMP', '/private/tmp'), ('USRBIN', '/usr/bin')]:\n\
                 \x20   try:\n\
                 \x20       os.listdir(p)\n\
                 \x20       print(tag + '_LISTED')\n\
                 \x20   except Exception:\n\
                 \x20       print(tag + '_HIDDEN')\n\
                 for d, f in {data_literal}:\n\
                 \x20   try:\n\
                 \x20       os.listdir(d)\n\
                 \x20       print('DATA_LISTED ' + d)\n\
                 \x20   except Exception:\n\
                 \x20       print('DATA_HIDDEN ' + d)\n\
                 \x20   try:\n\
                 \x20       open(f, 'rb').read(1)\n\
                 \x20       print('DATA_READ ' + f)\n\
                 \x20   except Exception:\n\
                 \x20       print('DATA_HIDDEN ' + f)\n"
            ),
            _ => format!(
                "const fs=require('fs');\
                 for(const [t,p] of [['TMP',{canary:?}],['CWD',{cwd_canary:?}]]){{\
                 try{{console.log(t+'_READ:'+fs.readFileSync(p,'utf8'))}}\
                 catch(e){{console.log(t+'_HIDDEN')}}}}\
                 for(const [t,p] of [['SHARED','/Users/Shared'],['PTMP','/private/tmp'],['USRBIN','/usr/bin']]){{\
                 try{{fs.readdirSync(p);console.log(t+'_LISTED')}}\
                 catch(e){{console.log(t+'_HIDDEN')}}}}\
                 for(const [d,f] of {data_literal}){{\
                 try{{fs.readdirSync(d);console.log('DATA_LISTED '+d)}}\
                 catch(e){{console.log('DATA_HIDDEN '+d)}}\
                 try{{fs.readFileSync(f).subarray(0,1);console.log('DATA_READ '+f)}}\
                 catch(e){{console.log('DATA_HIDDEN '+f)}}}}"
            ),
        };
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 20}), &mut ctx));
        let output = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            !output.contains(&body),
            "A FILE OUTSIDE THE SANDBOX WAS READ — its body came back through the script: {output}"
        );
        for hidden in ["TMP_HIDDEN", "CWD_HIDDEN", "SHARED_HIDDEN", "PTMP_HIDDEN"] {
            assert!(output.contains(hidden), "{hidden} expected: {output}");
        }
        assert!(
            output.contains("USRBIN_LISTED"),
            "the control failed — the probe fails on everything, so it measured nothing: {output}"
        );
        assert!(
            !output.contains("DATA_LISTED") && !output.contains("DATA_READ"),
            "HOMEBREW'S DATA IS READABLE FROM INSIDE THE SANDBOX: {output}"
        );
        for (dir, file) in &data {
            for path in [dir, file] {
                assert!(
                    output.contains(&format!("DATA_HIDDEN {path}")),
                    "the probe of {path} did not run: {output}"
                );
            }
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// The Homebrew data directories that exist here, each with one regular
    /// file found inside it — `[dir, file]` pairs, for
    /// `files_outside_the_sandbox_are_not_readable`. A directory with no file
    /// in reach is left out rather than half-probed. The walk is metadata only
    /// (`symlink_metadata`, nothing followed, nothing opened) and stops at the
    /// first file; `openssl.cnf` is skipped because the profile lets that one
    /// file back in on purpose.
    #[cfg(target_os = "macos")]
    fn homebrew_data_targets() -> Vec<(String, String)> {
        let mut targets = Vec::new();
        for dir in [
            "/opt/homebrew/var",
            "/opt/homebrew/etc",
            "/usr/local/var",
            "/usr/local/etc",
        ] {
            let mut queue = vec![(PathBuf::from(dir), 0usize)];
            let mut file = None;
            while let Some((at, depth)) = queue.pop() {
                let Ok(entries) = std::fs::read_dir(&at) else {
                    continue;
                };
                let mut entries: Vec<_> = entries.flatten().map(|e| e.path()).collect();
                entries.sort();
                for entry in entries {
                    let Ok(meta) = entry.symlink_metadata() else {
                        continue;
                    };
                    if meta.is_file() && entry.file_name() != Some("openssl.cnf".as_ref()) {
                        file = Some(entry);
                        break;
                    }
                    if meta.is_dir() && depth < 3 {
                        queue.push((entry, depth + 1));
                    }
                }
                if file.is_some() {
                    break;
                }
            }
            if let Some(file) = file.and_then(|f| f.to_str().map(str::to_string)) {
                targets.push((dir.to_string(), file));
            }
        }
        targets
    }

    /// 9d-v) A SANDBOX UNDER THE HOME DIRECTORY STILL RUNS.
    ///
    /// The production case — the working folder is somewhere in the user's
    /// home — and the one no other test here exercises: they all put the
    /// sandbox in the temp directory. The read allowlist (L20) and the per-call
    /// folder (M5) both changed what a script under `$HOME` can see; this is
    /// the measurement that it still starts there, which the temp-directory
    /// tests cannot give. The folder is made in the real `$HOME` (the same
    /// place the escape test plants its canary) and removed on every path.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_sandbox_under_the_home_directory_still_runs() {
        let Some(tool) = tool_or_skip() else { return };
        let Some(home) = std::env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|h| h.is_absolute() && h.is_dir())
        else {
            eprintln!("$HOME is unusable, the home case was not measured");
            return;
        };
        let work = home.join(format!(".tacet-home-sandbox-test-{}", nonce()));
        std::fs::create_dir_all(&work).expect("a folder in $HOME");
        struct Gone(PathBuf);
        impl Drop for Gone {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
        let _gone = Gone(work.clone());
        let work = work.canonicalize().expect("resolved");
        let mut ctx = context(&work);
        let code = match tool.interpreters()[0].key {
            "python" => "print(6*7)",
            _ => "console.log(6*7)",
        };
        let outcome = hold(tool.run(json!({"code": code}), &mut ctx));
        assert_eq!(
            outcome.state,
            ToolState::Read,
            "a script in a folder under $HOME did not run: {} / {:?}",
            outcome.to_model,
            outcome.raw_output
        );
        assert!(outcome.to_model.contains("42"), "{}", outcome.to_model);
    }

    /// 9e) A WORKING DIRECTORY WHOSE NAME CAN BREAK OUT OF THE PROFILE STRING
    ///     MUST NOT RUN AT ALL.
    ///
    /// Before the fix the sandbox path was spliced into the SBPL text: a folder
    /// called `evil")) (allow default) ;` closed the quote, added `(allow
    /// default)` and commented out the rest of the profile — network deny
    /// included. MEASURED before the fix, that exact folder gave `NET_OPEN` and
    /// really created a file outside. `verify_shield` could never see it,
    /// because it measures on a CLEAN temp path. Two layers answer it now: the
    /// profile carries no path at all, and this path is refused outright.
    #[test]
    fn a_profile_breaking_directory_name_never_runs() {
        let Some(tool) = tool_or_skip() else { return };
        let root = std::env::temp_dir().join(format!(
            "tacet-evil\")) (allow default) ;-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("temp dir");
        let root = root.canonicalize().expect("resolved");
        let target = std::env::temp_dir().join(format!("tacet-sbpl-{}.txt", std::process::id()));
        std::fs::remove_file(&target).ok();

        let code = match tool.interpreters()[0].key {
            "python" => format!(
                "try:\n\
                 \x20   open({target:?}, 'w').write('x')\n\
                 \x20   print('OUT_WRITE_OK')\n\
                 except Exception:\n\
                 \x20   print('OUT_WRITE_BLOCKED')\n"
            ),
            _ => format!(
                "const fs=require('fs');\
                 try{{fs.writeFileSync({target:?},'x');console.log('OUT_WRITE_OK')}}\
                 catch(e){{console.log('OUT_WRITE_BLOCKED')}}"
            ),
        };

        let result = run(
            &tool.shield,
            &tool.interpreters()[0],
            &root,
            &code,
            Duration::from_secs(10),
        );
        assert!(
            matches!(result, Err(ToolError::SandboxViolation(_))),
            "a path that can break the profile must be refused: {result:?}"
        );
        assert!(
            !target.exists(),
            "the shield was open, the file outside was created: {}",
            target.display()
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// 10) SUCCESS WITH NO OUTPUT IS NOT A SUCCESS — so the model cannot see
    ///     "ok" and invent the result.
    #[test]
    fn empty_output_does_not_stay_silent() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("silent");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "x = 1 + 1",
            _ => "const x = 1 + 1;",
        };
        let outcome = hold(tool.run(json!({"code": code}), &mut ctx));
        assert!(
            matches!(outcome.state, ToolState::Failed(_)),
            "{}",
            outcome.chip_text
        );
    }

    /// 11) HUGE OUTPUT does not eat the memory, it is truncated and the
    ///     truncation IS STATED; large output goes through the bypass channel.
    #[test]
    fn huge_output_is_truncated_and_falls_into_the_bypass_channel() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("huge");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "for i in range(200000): print('line', i)",
            _ => "for(let i=0;i<200000;i++) console.log('line',i)",
        };
        let outcome = hold(tool.run(json!({"code": code, "timeout_s": 25}), &mut ctx));

        let raw = outcome.raw_output.clone().unwrap_or_default();
        assert!(
            raw.len() <= OUTPUT_CAP + 16,
            "the cap was exceeded: {}",
            raw.len()
        );
        // The text going to the model is much shorter: even the full output IS NOT DUMPED on the model.
        assert!(outcome.to_model.len() < 900, "{}", outcome.to_model.len());
        if outcome.state == ToolState::Read {
            assert!(
                outcome.to_model.contains("source_ref"),
                "{}",
                outcome.to_model
            );
            let record = ctx
                .from_store(&SourceRef("code#1".into()))
                .expect("the store record");
            assert!(record.body.len() > MODEL_OUTPUT_CAP);
        }
    }

    /// 12) The third call NEVER SEES THE ENGINE: the tool issues the denial, the
    ///     model is not expected to count.
    #[test]
    fn the_third_call_is_rejected_by_the_tool() {
        let Some(tool) = tool_or_skip() else { return };
        let root = temp_dir("counter");
        let mut ctx = context(&root);
        let code = match tool.interpreters()[0].key {
            "python" => "print(1)",
            _ => "console.log(1)",
        };

        for kind in 1..=2 {
            let s = hold(tool.run(json!({"code": code}), &mut ctx));
            assert_eq!(s.state, ToolState::Read, "attempt {kind} must not fail");
        }
        let third = hold(tool.run(json!({"code": code}), &mut ctx));
        assert!(matches!(third.state, ToolState::Failed(_)));
        assert!(
            third.to_model.starts_with("error_final"),
            "{}",
            third.to_model
        );

        // The turn hook resets the counter; the shell binds this to `ToolExecutor`.
        tool.turn_state().new_turn();
        let new_turn = hold(tool.run(json!({"code": code}), &mut ctx));
        assert_eq!(new_turn.state, ToolState::Read, "the turn was not reset");
    }

    /// 13) The schema contract: the `language` field stands ONLY if there is a
    ///     real choice (it does not open an empty decode slot), and the timeout cap is binding.
    #[test]
    fn the_schema_contract_is_binding() {
        let Some(tool) = tool_or_skip() else { return };
        let schema = tool.schema();
        let js = schema.json_schema();
        assert_eq!(js["required"], json!(["code"]));
        assert_eq!(js["additionalProperties"], json!(false));

        let has_language = js["properties"].get("language").is_some();
        assert_eq!(
            has_language,
            tool.interpreters().len() > 1,
            "the language field must only exist with multiple interpreters"
        );

        assert!(schema.validate(&json!({"code": "x"})).is_ok());
        assert!(
            schema
                .validate(&json!({"code": "x", "timeout_s": 999}))
                .is_err(),
            "the cap"
        );
        assert!(
            schema
                .validate(&json!({"code": "x", "invented": 1}))
                .is_ok(),
            "the schema ignores it"
        );
        if has_language {
            assert!(
                schema
                    .validate(&json!({"code": "x", "language": "cobol"}))
                    .is_err()
            );
        }
    }

    /// 14) The router must be able to pick this tool for a Calc intent.
    #[test]
    fn the_router_hints_hold() {
        use crate::router::{Router, simplify};
        let Some(tool) = tool_or_skip() else { return };
        let text = simplify(&format!("{} {}", tool.name(), tool.description()));
        // The Calc profile hints: "code", "run".
        assert!(text.contains("code") && text.contains("run"));

        let mut catalog = tacet_kernel::ToolCatalog::new();
        catalog.add(Arc::new(crate::time::TimeTool::new()));
        catalog.add(Arc::new(RunCodeTool::discover().expect("discovered")));
        let chosen = Router::new()
            .max(1)
            .select("calculate this with python", &catalog);
        assert_eq!(chosen[0].name(), "run_code");
    }

    // --- The language guess ---

    /// When the source speaks PLAINLY it gives the right language.
    #[test]
    fn language_from_source_recognises_an_explicit_source() {
        assert_eq!(
            language_from_source("const a = [];\nlet b = 1;"),
            Some("js")
        );
        assert_eq!(language_from_source("console.log(42)"), Some("js"));
        assert_eq!(
            language_from_source("print(sum(range(10)))"),
            Some("python")
        );
        assert_eq!(
            language_from_source("import math\nprint(math.pi)"),
            Some("python")
        );
    }

    /// Syntax valid in both languages DOES NOT MAKE the guess speak; the
    /// decision is left to the declaration/preference order.
    #[test]
    fn language_from_source_stays_silent_on_shared_syntax() {
        assert_eq!(language_from_source("for i in x:\n    pass"), None);
        assert_eq!(language_from_source("1 + 2"), None);
        assert_eq!(language_from_source(""), None);
    }

    /// THE SOURCE OVERRIDES THE DECLARATION. The failure measured in the field:
    /// the model wrote JS and sent `language:"python"`, and the script failed with a syntax error.
    #[test]
    fn the_source_overrides_a_wrong_declaration() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return; // nothing to measure on a machine with a single interpreter
        }
        let chosen = tool.resolve_interpreter(Some("python"), "console.log(1)");
        assert_eq!(chosen.key, "js");
        let chosen = tool.resolve_interpreter(Some("js"), "print(1)");
        assert_eq!(chosen.key, "python");
    }

    /// If the source is silent the declaration holds.
    #[test]
    fn when_the_source_is_silent_the_declaration_holds() {
        let Some(tool) = tool_or_skip() else { return };
        if tool.interpreters.len() < 2 {
            return;
        }
        assert_eq!(
            tool.resolve_interpreter(Some("python"), "1 + 2").key,
            "python"
        );
    }
}
