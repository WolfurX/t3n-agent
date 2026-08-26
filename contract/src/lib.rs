//! z-tenant-ar v0.1.0 — accounts-receivable reminder agent.
//!
//! Demonstrates the z-space tenant model on a dunning workflow:
//!   - `upsert-invoice`: writes an invoice record (business data only) into
//!     the tenant `invoices` KV map. Input structs deny unknown fields, so a
//!     payload that tries to smuggle customer PII fails at parse time.
//!   - `list-invoices`: scans the map; "overdue" is computed against the
//!     cluster timestamp, never a wall clock.
//!   - `send-reminder`: emails the customer via the host's
//!     `http-with-placeholders` interface. The customer's name and email are
//!     NEVER contract arguments: the contract templates `{{profile.<field>}}`
//!     markers into the email payload and the host resolves them from the
//!     calling user's profile at dispatch time, so plaintext PII never enters
//!     WASM.
//!
//! The email gateway endpoint and its API key are read from the z: KV map
//! `secrets` (keys: `email_endpoint`, `email_api_key`). Both maps are created
//! and populated by the tenant SDK before the contract runs.
#![warn(clippy::style, missing_debug_implementations)]
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

extern crate alloc;

pub const CONTRACT_VERSION: &str = "0.1.0";

wit_bindgen::generate!({
    world: "tenant-ar",
    path: "wit",
    additional_derives: [
        serde::Deserialize,
        serde::Serialize,
    ],
    generate_all,
});

mod invoices;
mod remind;

struct Component;

#[cfg(target_arch = "wasm32")]
impl exports::z::tenant_ar::contracts::Guest for Component {
    fn upsert_invoice(
        req: exports::z::tenant_ar::contracts::GenericInput,
    ) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        let input = req.input.ok_or("upsert-invoice: missing input")?;
        invoices::upsert_invoice(&input)
    }

    fn list_invoices(
        req: exports::z::tenant_ar::contracts::GenericInput,
    ) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        let input = req.input.unwrap_or_else(|| b"{}".to_vec());
        invoices::list_invoices(&input)
    }

    fn send_reminder(
        req: exports::z::tenant_ar::contracts::GenericInput,
    ) -> Result<alloc::vec::Vec<u8>, alloc::string::String> {
        let input = req.input.ok_or("send-reminder: missing input")?;
        remind::send_reminder(&input)
    }
}

#[cfg(target_arch = "wasm32")]
export!(Component);

#[cfg(test)]
mod tests {
    use super::CONTRACT_VERSION;

    #[test]
    fn contract_version_is_semver() {
        let parts: Vec<&str> = CONTRACT_VERSION.split('.').collect();
        assert_eq!(parts.len(), 3, "CONTRACT_VERSION must be MAJOR.MINOR.PATCH");
        for part in parts {
            assert!(part.parse::<u32>().is_ok(), "each part must be a number");
        }
    }
}
