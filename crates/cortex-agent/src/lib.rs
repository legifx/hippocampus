//! The Agent Surface's safety layer: what an agent may do, and the record that it did it.
//!
//! Three things live here, and they are separate on purpose:
//!
//! - [`policy`] decides. It is a pure function of configuration and a path — no I/O, no state,
//!   so a refusal can be tested without a daemon and cannot depend on the order of calls.
//! - [`surface`] holds the kill switch (rule R7), the one piece of policy that is runtime
//!   state rather than configuration, and therefore the one piece that has to be persisted.
//! - [`audit`] records every decision, allowed or refused (rule R8).
//!
//! The reason for the layer is not paranoia. An agent with unchecked `fs.move` over an indexed
//! home directory does more damage in 200 ms than a nightly backup undoes.

pub mod audit;
pub mod brain_tools;
pub mod fs_tools;
pub mod plans;
pub mod policy;
pub mod surface;
pub mod transfer;
pub mod trash;

pub use audit::{redact, Auditor};
pub use fs_tools::{Surface, ToolError, ToolResult};
pub use plans::PlanBook;
pub use policy::{Policy, Refusal};
pub use surface::KillSwitch;

/// Actor names the surface accepts from a client.
///
/// An agent identifies itself; nothing verifies the claim, and nothing should pretend to. The
/// value of the name is in the audit log, where "which agent did this" is answerable, not in
/// access control — a local Unix socket with `0600` on it is the access control.
pub fn actor_from_client(name: Option<&str>) -> cortex_core::model::Actor {
    match name {
        None => cortex_core::model::Actor::Human,
        Some(n) if n == "human" => cortex_core::model::Actor::Human,
        Some(n) if n == "system" => cortex_core::model::Actor::System,
        Some(n) => cortex_core::model::Actor::Agent(
            n.strip_prefix("agent:").unwrap_or(n).to_string(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::model::Actor;

    #[test]
    fn client_names_map_to_actors() {
        assert_eq!(actor_from_client(None), Actor::Human);
        assert_eq!(actor_from_client(Some("human")), Actor::Human);
        assert_eq!(actor_from_client(Some("system")), Actor::System);
        assert_eq!(
            actor_from_client(Some("agent:claude")),
            Actor::Agent("claude".into())
        );
        assert_eq!(
            actor_from_client(Some("claude")),
            Actor::Agent("claude".into()),
            "a bare name is an agent name, not a system claim"
        );
    }
}
