//! The ONE place a `ureq` agent configuration is started.
//!
//! WHY A SHARED STARTING POINT — THE ENVIRONMENT PROXY. `ureq` 3 builds its
//! default configuration with `Proxy::try_from_env()`, which reads `ALL_PROXY`,
//! `HTTPS_PROXY` and `HTTP_PROXY` (either case) and silently routes every
//! request through whatever they name. No agent in this crate turned that off,
//! and it was measured, not guessed (audit M12, 9 Oct 2026): with `ALL_PROXY`
//! pointing at a CONNECT harness, the plain-text request line
//! `GET /search?q=symptoms%20of%20my%20diagnosis` reached the harness on its
//! way to a LOCAL SearXNG. A variable set once in a shell profile for some
//! other tool was enough to hand a third process every query.
//!
//! The `Cargo.toml` comment that justified the dependency said the "proxy
//! feature" was not pulled in. That was true of the SOCKS feature and false of
//! the HTTP CONNECT support, which is compiled in unconditionally — the
//! comment described a switch that does not exist.
//!
//! ONE FUNCTION, NOT A FLAG AT FIVE CALL SITES: the five agents in this crate
//! were written at different times, and a sixth written next month would copy
//! whichever neighbour it was next to. Two tests keep the rule honest:
//! `tests/no_env_proxy.rs` routes real requests past a listening proxy, and
//! `tests/agent_doors.rs` fails if any file in `src/` names a `ureq` path
//! outside a short allow-list of types that cannot build an agent.
//!
//! WHAT IS GIVEN UP: a user who genuinely must go through a corporate proxy
//! cannot, today. That is a capability to add with an explicit setting the
//! user types, not one to inherit from whatever the environment happens to
//! carry. An offline-first tool that leaks queries to a proxy it was never
//! told about has the default the wrong way round.

use ureq::config::ConfigBuilder;
use ureq::typestate::AgentScope;

/// A configuration builder with the environment proxy switched OFF. Every
/// agent in this crate starts here; see the top of the file.
pub(crate) fn builder() -> ConfigBuilder<AgentScope> {
    ureq::Agent::config_builder().proxy(None)
}

/// Every agent this crate builds, by name, configured exactly as the call
/// sites configure them.
///
/// HIDDEN AND NOT AN API: it exists so `tests/no_env_proxy.rs` — a separate
/// process, the only safe place to set `ALL_PROXY` — can ask each agent whether
/// it would use a proxy. Two of the five (`page_text`, the `http` tool) refuse
/// every loopback address by design, so no local listener can stand in for
/// their far end; their configuration is the only thing a test can observe.
#[doc(hidden)]
pub fn every_agent() -> Vec<(&'static str, ureq::Agent)> {
    let timeout = std::time::Duration::from_secs(5);
    vec![
        (
            "client::search",
            crate::client::search_agent("https://example.test", timeout),
        ),
        ("client::page_text", crate::client::public_agent(timeout)),
        ("http::call", crate::http::call_agent(timeout)),
        ("download", crate::download::download_agent(Some(1024))),
        ("release", crate::release::release_agent(timeout)),
    ]
}
