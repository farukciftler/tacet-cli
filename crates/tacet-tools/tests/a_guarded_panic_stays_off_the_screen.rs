//! RG-04 review: a reader that panics outside a tool — `--from`,
//! `/resume-from`, `tacet handoff` — must not put its panic payload on the
//! user's terminal. The payload may quote the foreign line that caused it.
//!
//! `catch_unwind` alone did not do that. It kept the payload out of the
//! returned error, but std's hook runs before the unwind is caught and had
//! already printed "thread 'main' panicked at ...: <payload>" on stderr. The
//! reviewer saw the payload on stderr with `--nocapture` while every test was
//! green, because those tests checked only the returned string.
//!
//! Hooks are process-wide, so the check runs in a child process: this test
//! binary again, running this test only, with the hook `main` installs. Its own
//! file, so the `RUNTIME_SPAWN_SITES` row in `network_monopoly.rs` that allows
//! the re-exec covers this file and not the reader's.

use tacet_tools::executor::quiet_tool_panics;
use tacet_tools::sessions_tool::guarded;

const CHILD: &str = "TACET_TEST_GUARDED_PANIC_CHILD";

/// Two panics outside the guard must still reach stderr: one on another thread,
/// and one on the same thread after the guard returns. Without them, silence
/// would prove nothing. A child that printed nothing at all would pass a
/// check for absence alone.
#[test]
fn a_guarded_panic_does_not_reach_stderr() {
    if std::env::var_os(CHILD).is_some() {
        quiet_tool_panics();
        assert!(guarded(|| panic!("PAYLOAD-IN-GUARD")).is_err());
        let _ = std::thread::Builder::new()
            .name("unguarded-probe".into())
            .spawn(|| panic!("PAYLOAD-ON-ANOTHER-THREAD"))
            .expect("thread")
            .join();
        let _ = std::panic::catch_unwind(|| panic!("PAYLOAD-AFTER-GUARD"));
        return;
    }
    let out = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([
            "--exact",
            "a_guarded_panic_does_not_reach_stderr",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .output()
        .expect("the test binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("1 passed"),
        "the child did not run the test: {stdout}\n{stderr}"
    );
    assert!(
        stderr.contains("PAYLOAD-ON-ANOTHER-THREAD") && stderr.contains("PAYLOAD-AFTER-GUARD"),
        "a panic outside the guard was silenced too: {stderr}"
    );
    assert!(
        !stderr.contains("PAYLOAD-IN-GUARD"),
        "a guarded panic's payload reached stderr: {stderr}"
    );
}
