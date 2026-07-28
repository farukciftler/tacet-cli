//! THE SSRF GATE, MEASURED WITH A REAL SOCKET — and the measurement is that
//! THE SOCKET IS NEVER TOUCHED.
//!
//! WHY NOT `#[ignore]`: this test does not go online. The listener is born
//! inside the test on `127.0.0.1`, on a random port, and dies with it; no DNS,
//! no internet, no external dependency.
//!
//! WHY IT EXISTS: the unit tests in `client.rs` prove that `target_is_public`
//! returns an error. That is not the same claim as "no connection was made" —
//! a gate placed AFTER the request would pass those tests just as well. Here a
//! real listener sits at the address the model would name (a local service on
//! loopback: `ollama`, a database admin panel, the metadata service's stand-in)
//! and the assertion is that it never sees a connection.

use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;
use tacet_web::WebSearchClient;

struct Listener {
    port: u16,
    connections: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    job: thread::JoinHandle<()>,
}

impl Listener {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port");
        let port = listener.local_addr().expect("address").port();
        // Non-blocking accept: otherwise the thread would sleep inside
        // `accept()` and never learn about the shutdown request.
        listener.set_nonblocking(true).expect("non-blocking");
        let connections = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let counter = Arc::clone(&connections);
        let stop_job = Arc::clone(&stop);
        let job = thread::spawn(move || {
            while !stop_job.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok(_) => {
                        counter.fetch_add(1, Ordering::Relaxed);
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
            stop,
            job,
        }
    }

    fn shutdown(self) -> usize {
        // Give a connection that WAS made time to be accepted, so a failure
        // here is a real failure and not a race.
        thread::sleep(Duration::from_millis(50));
        self.stop.store(true, Ordering::Relaxed);
        self.job.join().ok();
        self.connections.load(Ordering::Relaxed)
    }
}

/// A service listening on loopback is exactly what an SSRF is aimed at: it is
/// reachable from this machine and from nowhere else, so it usually has no
/// authentication at all.
#[test]
fn a_loopback_target_is_refused_without_a_connection_being_made() {
    let listener = Listener::start();
    let client = WebSearchClient::new();

    for address in [
        format!("https://127.0.0.1:{}/api/tags", listener.port),
        format!("https://[::1]:{}/api/tags", listener.port),
        // Plain http: refused by the scheme gate before anything else.
        format!("http://127.0.0.1:{}/api/tags", listener.port),
        // The name resolves to loopback; the gate judges the RESOLVED address,
        // not the spelling.
        format!("https://localhost:{}/api/tags", listener.port),
    ] {
        let outcome = client.page_text(&address);
        assert!(outcome.is_err(), "{address} was fetched");
    }

    assert_eq!(
        listener.shutdown(),
        0,
        "the gate must refuse BEFORE the socket: the local service saw a connection"
    );
}
