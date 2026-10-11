//! A counting listener on `127.0.0.1` for the tests that must prove a request
//! did NOT arrive somewhere. Shared so the two files that need it (and run in
//! separate processes on purpose) measure with the same instrument.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::thread;
use std::time::Duration;

/// A listener that answers every request with `answer` and counts what came.
pub struct Listener {
    pub port: u16,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    job: thread::JoinHandle<()>,
}

impl Listener {
    pub fn start(answer: impl Fn() -> String + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("port");
        let port = listener.local_addr().expect("address").port();
        listener.set_nonblocking(true).expect("non-blocking");
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (hits_job, stop_job) = (Arc::clone(&hits), Arc::clone(&stop));
        let job = thread::spawn(move || {
            while !stop_job.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream.set_nonblocking(false).expect("blocking stream");
                        hits_job.fetch_add(1, Ordering::Relaxed);
                        let _ = serve(stream, &answer());
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
            hits,
            stop,
            job,
        }
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::Relaxed)
    }

    pub fn shutdown(self) {
        self.stop.store(true, Ordering::Relaxed);
        self.job.join().ok();
    }
}

/// Reads one request (headers and a `Content-Length` body) and answers it,
/// then closes — one request per connection keeps the count honest.
fn serve(stream: TcpStream, answer: &str) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(());
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body)?;
    let mut stream = stream;
    stream.write_all(answer.as_bytes())?;
    stream.flush()
}
