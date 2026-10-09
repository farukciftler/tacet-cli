//! NO AGENT IN THIS CRATE ROUTES THROUGH A PROXY IT WAS NOT TOLD ABOUT
//! (audit M12).
//!
//! `ureq` 3 reads `ALL_PROXY` / `HTTPS_PROXY` / `HTTP_PROXY` into every default
//! configuration. The audit pointed those variables at a CONNECT harness and
//! watched a local SearXNG query cross it in plain text
//! (`GET /search?q=symptoms%20of%20my%20diagnosis`). Here the "proxy" is a
//! listener on 127.0.0.1 that only counts, and the assertion is that it is
//! never touched while real requests go to their real (local) destinations.
//!
//! A FILE OF ITS OWN, AND ONE TEST THAT TOUCHES THE ENVIRONMENT: `set_var` is
//! process-wide. In the library's test binary it would race every other test;
//! here this binary is the whole process and the variables are set before any
//! agent exists.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tacet_web::{DownloadApproval, DownloadPlan, SilentProgress, WebSearchClient};

/// A listener that records every connection and the first bytes sent on it,
/// then answers with `reply` (or closes, if `reply` is empty).
struct Listener {
    port: u16,
    connections: Arc<AtomicUsize>,
    seen: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
}

impl Listener {
    fn start(reply: &'static [u8]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port");
        let port = listener.local_addr().expect("address").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let connections = Arc::new(AtomicUsize::new(0));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (counter, log, halt) = (
            Arc::clone(&connections),
            Arc::clone(&seen),
            Arc::clone(&stop),
        );
        thread::spawn(move || {
            while !halt.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        counter.fetch_add(1, Ordering::Relaxed);
                        stream.set_nonblocking(false).ok();
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .ok();
                        let mut head = Vec::new();
                        let mut byte = [0u8; 1];
                        while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                            match stream.read(&mut byte) {
                                Ok(1) => head.push(byte[0]),
                                _ => break,
                            }
                        }
                        log.lock()
                            .unwrap()
                            .push(String::from_utf8_lossy(&head).into_owned());
                        if !reply.is_empty() {
                            let _ = stream.write_all(reply);
                        }
                        let _ = stream.shutdown(Shutdown::Both);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(_) => return,
                }
            }
        });
        Self {
            port,
            connections,
            seen,
            stop,
        }
    }

    fn connections(&self) -> usize {
        // Give a connection that WAS made time to be accepted, so a zero here
        // is a measurement and not a race.
        thread::sleep(Duration::from_millis(50));
        self.connections.load(Ordering::Relaxed)
    }

    fn seen(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

const SEARXNG_REPLY: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 81\r\nConnection: close\r\n\r\n{\"results\":[{\"url\":\"https://a.test/x\",\"title\":\"A title\",\"content\":\"some text.\"}]}";

struct Approve;
impl DownloadApproval for Approve {
    fn approve(&self, _plan: &DownloadPlan, _existing: u64) -> bool {
        true
    }
}

#[test]
fn no_agent_routes_through_an_environment_proxy() {
    let proxy = Listener::start(b"");
    let searxng = Listener::start(SEARXNG_REPLY);
    // A TLS endpoint stand-in: it accepts and hangs up, so the download fails
    // at the handshake — after the connection, which is the thing measured.
    let mirror = Listener::start(b"");

    let proxy_url = format!("http://127.0.0.1:{}", proxy.port);
    // SAFETY: this binary has one test and no other thread reads the
    // environment while these are set (the listeners do not).
    unsafe {
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ] {
            std::env::set_var(name, &proxy_url);
        }
        std::env::remove_var("NO_PROXY");
        std::env::remove_var("no_proxy");
    }

    // THE ENVIRONMENT IS LIVE: an agent built the default way picks the proxy
    // up. Without this line a typo in the variable names would make the rest of
    // the test pass for the wrong reason.
    let default_agent: ureq::Agent = ureq::Agent::config_builder().build().into();
    assert!(
        default_agent.config().proxy().is_some(),
        "the harness is broken: ureq did not read the proxy variables"
    );

    // 1. SEARCH — the audit's exact case: a local SearXNG, a private query.
    let results = WebSearchClient::with_address(format!("http://127.0.0.1:{}", searxng.port))
        .search("symptoms of my diagnosis", None)
        .expect("the local search must answer");
    assert_eq!(results.len(), 1);
    assert!(
        searxng
            .seen()
            .iter()
            .any(|h| h.contains("symptoms%20of%20my%20diagnosis")),
        "the query did not reach the search server: {:?}",
        searxng.seen()
    );

    // 2. DOWNLOAD — https-only, so the far end is a socket that hangs up; the
    // connection still has to go to IT and not to the proxy.
    let dir = std::env::temp_dir().join(format!("tacet-no-env-proxy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let plan = DownloadPlan {
        name: "m.bin".into(),
        url: format!("https://127.0.0.1:{}/m.bin", mirror.port),
        target: dir.join("m.bin"),
        expected_bytes: Some(10),
        expected_sha256: None,
    };
    assert!(tacet_web::download(&plan, &Approve, &SilentProgress).is_err());
    assert!(
        mirror.connections() >= 1,
        "the download never reached its source"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // 3. THE REST — `page_text` and the `http` tool refuse every loopback
    // target before connecting, and the release check has a fixed public
    // address; their configuration is what can be observed.
    for (site, agent) in tacet_web::net::every_agent() {
        assert!(
            agent.config().proxy().is_none(),
            "{site} would route through {proxy_url}"
        );
    }

    assert_eq!(
        proxy.connections(),
        0,
        "the proxy saw traffic: {:?}",
        proxy.seen()
    );
}

/// EVERY AGENT STARTS IN `net.rs`. The test above checks the agents that
/// exist; this one stops the next one from being written somewhere else with
/// a bare `ureq::Agent::config_builder()` — the way all five were before.
///
/// THE NEEDLES ARE EVERY DOOR INTO A DEFAULT CONFIG, not only the one the five
/// used. The first version looked for four spellings and read `src/` one
/// level deep; a review on 10 Oct 2026 pointed out that
/// `Agent::new_with_defaults()`, `Config::default()` and the `ureq::put` /
/// `head` / `delete` family each build an agent that reads `ALL_PROXY` again,
/// and that a module moved into `src/<dir>/` was never opened. Each needle was
/// proved by a throwaway line in a real source file before it was kept.
#[test]
fn no_source_file_builds_an_agent_outside_net_rs() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let net = src.join("net.rs");
    let mut offenders = Vec::new();
    let mut seen = 0usize;
    for path in rust_files(&src) {
        if path == net {
            continue;
        }
        seen += 1;
        let text = std::fs::read_to_string(&path).expect("read");
        for needle in AGENT_DOORS {
            if text.contains(needle) {
                offenders.push(format!("{}: {needle}", path.display()));
            }
        }
    }
    // A walk that found nothing would pass vacuously.
    assert!(
        seen >= 5,
        "only {seen} source files were read under {src:?}"
    );
    assert!(
        offenders.is_empty(),
        "an agent is configured outside net.rs, where the proxy rule lives: {offenders:?}"
    );
}

/// Every spelling that yields a `ureq` agent with the environment proxy left
/// on. `ureq::request` covers `ureq::request(` and `ureq::request_uri(`.
const AGENT_DOORS: &[&str] = &[
    "config_builder(",
    "Config::builder(",
    "Config::default(",
    "new_with_defaults(",
    "new_with_config(",
    "Agent::default(",
    "ureq::get(",
    "ureq::post(",
    "ureq::put(",
    "ureq::delete(",
    "ureq::head(",
    "ureq::patch(",
    "ureq::options(",
    "ureq::trace(",
    "ureq::request",
];

/// Every `.rs` file under `dir`, at any depth.
fn rust_files(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).expect("read_dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
    out
}

/// The walk must reach a nested module — the case the first version missed.
#[test]
fn the_source_walk_reaches_a_nested_directory() {
    let root = std::env::temp_dir().join(format!("tacet-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a/b")).unwrap();
    std::fs::write(root.join("top.rs"), "").unwrap();
    std::fs::write(root.join("a/b/deep.rs"), "").unwrap();
    std::fs::write(root.join("a/notes.txt"), "").unwrap();
    let mut found: Vec<_> = rust_files(&root)
        .into_iter()
        .map(|p| p.strip_prefix(&root).unwrap().to_path_buf())
        .collect();
    found.sort();
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(
        found,
        vec![
            std::path::PathBuf::from("a/b/deep.rs"),
            std::path::PathBuf::from("top.rs")
        ]
    );
}
