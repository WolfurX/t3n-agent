//! send_reminder: emails the customer of one open invoice via the host's
//! `http-with-placeholders` interface.
//!
//! The recipient address and greeting name are `{{profile.<field>}}` markers
//! resolved host-side from the calling user's profile — the contract argument
//! is just an invoice_id, and plaintext PII never enters WASM memory. Egress
//! and profile access are both gated by the user's `agent-auth-update` grant.

use alloc::format;

use crate::invoices::{self, Status};

/// Refuse a second reminder inside this window, so a double-fired cron or a
/// retried agent run cannot spam the customer.
pub const REMINDER_COOLDOWN_SECS: u64 = 20 * 3600;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct RemindReq {
    invoice_id: String,
}

#[derive(serde::Serialize)]
struct RemindResp {
    invoice_id: String,
    email_status: u16,
    reminder_count: u32,
}

pub fn send_reminder(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: RemindReq = serde_json::from_slice(input)
        .map_err(|e| format!("send-reminder: bad input: {e}"))?;
    if req.invoice_id.is_empty() {
        return Err("send-reminder: bad input: invoice_id must not be empty".to_string());
    }

    #[cfg(target_arch = "wasm32")]
    {
        let resp = send_reminder_wasm(req)?;
        serde_json::to_vec(&resp).map_err(|e| e.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("send_reminder is only implemented on the wasm32 target".to_string())
    }
}

#[cfg(target_arch = "wasm32")]
use crate::host::{
    interfaces::{http_with_placeholders as hwp, kv_store, logging},
    tenant::tenant_context,
};

#[cfg(target_arch = "wasm32")]
fn send_reminder_wasm(req: RemindReq) -> Result<RemindResp, String> {
    use serde_json::json;

    let now = tenant_context::cluster_timestamp_secs();
    let mut inv = invoices::load_invoice(&req.invoice_id)?
        .ok_or_else(|| format!("send-reminder: invoice {} not found", req.invoice_id))?;

    if inv.status != Status::Open {
        return Err(format!(
            "send-reminder: invoice {} is not open — no reminder sent",
            inv.invoice_id
        ));
    }
    if let Some(last) = inv.reminded_at_epoch_secs {
        let since = now.saturating_sub(last);
        if since < REMINDER_COOLDOWN_SECS {
            return Err(format!(
                "send-reminder: invoice {} was reminded {}h ago — cooldown is {}h",
                inv.invoice_id,
                since / 3600,
                REMINDER_COOLDOWN_SECS / 3600
            ));
        }
    }

    let endpoint = get_secret("email_endpoint")?
        .ok_or("email_endpoint not found in z:<tid>:secrets — populate it via the tenant SDK")?;
    let api_key = get_secret("email_api_key")?;
    let from = get_secret("email_from")?;

    // Recipient and greeting are host-resolved markers; everything else is
    // the invoice's business data.
    let overdue = invoices::is_overdue(&inv, now);
    let subject = format!(
        "Payment reminder: invoice {} ({} {})",
        inv.invoice_id, inv.amount, inv.currency
    );
    let text = format!(
        "Dear {{{{profile.first_name}}}},\n\n\
         This is a friendly reminder that invoice {} for {} {} {} on {}.\n\n\
         If you have already made this payment, please disregard this message.\n\n\
         Thank you,\nAccounts Receivable",
        inv.invoice_id,
        inv.amount,
        inv.currency,
        if overdue { "was due" } else { "is due" },
        inv.due_date,
    );
    let mut body = json!({
        "to": ["{{profile.verified_contacts.email.value}}"],
        "subject": subject,
        "text": text,
    });
    if let Some(from) = from {
        body["from"] = json!(from);
    }

    let mut headers = vec![("Content-Type".to_string(), "application/json".to_string())];
    if let Some(key) = api_key {
        headers.push(("Authorization".to_string(), format!("Bearer {key}")));
    }

    let resp = hwp::call(&hwp::Request {
        method: hwp::Verb::Post,
        url: endpoint,
        headers: Some(headers),
        payload: Some(serde_json::to_vec(&body).map_err(|e| e.to_string())?),
    })
    .map_err(|e| format!("send-reminder: email gateway: {}", format_http_error(e)))?;

    // The gateway response can echo the resolved recipient address, so the
    // body never crosses the WIT boundary — only the status code does.
    if !(200..300).contains(&resp.code) {
        let _ = logging::error(&format!(
            "send-reminder: {} gateway returned HTTP {}",
            inv.invoice_id, resp.code
        ));
        return Err(format!(
            "send-reminder: email gateway returned HTTP {} — check the gateway credentials \
             and payload shape (response body withheld: it can echo recipient details)",
            resp.code
        ));
    }

    inv.reminded_at_epoch_secs = Some(now);
    inv.reminder_count += 1;
    invoices::store_invoice(&inv)?;
    let _ = logging::info(&format!(
        "send-reminder: {} reminder #{} accepted (HTTP {})",
        inv.invoice_id, inv.reminder_count, resp.code
    ));

    Ok(RemindResp {
        invoice_id: inv.invoice_id,
        email_status: resp.code,
        reminder_count: inv.reminder_count,
    })
}

#[cfg(target_arch = "wasm32")]
fn get_secret(key: &str) -> Result<Option<String>, String> {
    let map = format!("z:{}:secrets", hex::encode(tenant_context::tenant_did()));
    let bytes = kv_store::get(&map, key.as_bytes()).map_err(|e| format!("kv read: {e}"))?;
    match bytes {
        None => Ok(None),
        Some(b) => String::from_utf8(b)
            .map(Some)
            .map_err(|e| format!("secret {key} is not UTF-8: {e}")),
    }
}

#[cfg(target_arch = "wasm32")]
fn format_http_error(e: hwp::HttpError) -> String {
    match e {
        hwp::HttpError::EgressDenied(host) => format!("egress denied for host {host}"),
        hwp::HttpError::PlaceholderDenied(marker) => {
            format!("placeholder not permitted: {marker}")
        }
        hwp::HttpError::PlaceholderUnknown(field) => {
            format!("user profile missing field: {field}")
        }
        hwp::HttpError::PlaceholderNoUserContext => {
            "no user context bound for placeholder resolution".to_string()
        }
        hwp::HttpError::UpstreamError(reason) => format!("upstream: {reason}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remind_rejects_smuggled_recipient() {
        // The recipient comes from the user profile, never the input.
        let err = send_reminder(br#"{"invoice_id":"INV-1","to":"jane@example.com"}"#)
            .unwrap_err();
        assert!(err.contains("bad input"), "{err}");
    }

    #[test]
    fn remind_rejects_non_json_and_empty_id() {
        assert!(send_reminder(b"not json").unwrap_err().contains("bad input"));
        assert!(send_reminder(br#"{"invoice_id":""}"#)
            .unwrap_err()
            .contains("must not be empty"));
    }
}
