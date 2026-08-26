//! upsert_invoice / list_invoices: invoice records in the tenant `invoices`
//! KV map. Records hold business data only — the input struct denies unknown
//! fields, so payloads carrying customer names or email addresses fail at
//! parse time. The customer is identified by `customer_ref`, an opaque
//! business reference.

use alloc::format;

/// One invoice record, stored as JSON bytes under key = invoice_id.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Invoice {
    pub invoice_id: String,
    pub customer_ref: String,
    /// Decimal string ("1250.00") — never a float, so amounts round-trip exactly.
    pub amount: String,
    pub currency: String,
    /// YYYY-MM-DD.
    pub due_date: String,
    pub status: Status,
    pub created_at_epoch_secs: u64,
    pub reminded_at_epoch_secs: Option<u64>,
    pub reminder_count: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Open,
    Paid,
    Void,
}

/// `deny_unknown_fields` is the PII gate: {"email": …} or {"customer_name": …}
/// is a parse error, not a silently-stored field.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct UpsertReq {
    invoice_id: String,
    customer_ref: String,
    amount: String,
    currency: String,
    due_date: String,
    status: Option<Status>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ListReq {
    filter: Option<String>,
}

#[derive(serde::Serialize)]
struct UpsertResp {
    invoice_id: String,
    created: bool,
}

#[derive(serde::Serialize)]
struct ListResp {
    invoices: Vec<Invoice>,
    as_of_epoch_secs: u64,
}

pub fn upsert_invoice(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: UpsertReq = serde_json::from_slice(input)
        .map_err(|e| format!("upsert-invoice: bad input: {e}"))?;
    validate(&req)?;

    #[cfg(target_arch = "wasm32")]
    {
        let resp = upsert_invoice_wasm(req)?;
        serde_json::to_vec(&resp).map_err(|e| e.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("upsert_invoice is only implemented on the wasm32 target".to_string())
    }
}

pub fn list_invoices(input: &[u8]) -> Result<Vec<u8>, String> {
    let req: ListReq = serde_json::from_slice(input)
        .map_err(|e| format!("list-invoices: bad input: {e}"))?;
    let filter = req.filter.unwrap_or_else(|| "all".to_string());
    if !matches!(filter.as_str(), "all" | "open" | "overdue") {
        return Err(format!(
            "list-invoices: bad input: filter must be all|open|overdue, got \"{filter}\""
        ));
    }

    #[cfg(target_arch = "wasm32")]
    {
        let resp = list_invoices_wasm(&filter)?;
        serde_json::to_vec(&resp).map_err(|e| e.to_string())
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        Err("list_invoices is only implemented on the wasm32 target".to_string())
    }
}

fn validate(req: &UpsertReq) -> Result<(), String> {
    let id_ok = !req.invoice_id.is_empty()
        && req.invoice_id.len() <= 64
        && req
            .invoice_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-');
    if !id_ok {
        return Err("upsert-invoice: bad input: invoice_id must be 1-64 chars of [A-Za-z0-9._-]"
            .to_string());
    }
    if req.customer_ref.is_empty() || req.customer_ref.len() > 128 {
        return Err("upsert-invoice: bad input: customer_ref must be 1-128 chars".to_string());
    }
    if req.customer_ref.contains('@') {
        return Err(
            "upsert-invoice: bad input: customer_ref must be an opaque business reference, \
             not an email address — customer contact details live in the user profile and \
             never enter this contract"
                .to_string(),
        );
    }
    let amount_ok = match req.amount.split_once('.') {
        None => !req.amount.is_empty() && req.amount.bytes().all(|b| b.is_ascii_digit()),
        Some((whole, frac)) => {
            !whole.is_empty()
                && whole.bytes().all(|b| b.is_ascii_digit())
                && (1..=4).contains(&frac.len())
                && frac.bytes().all(|b| b.is_ascii_digit())
        }
    } && req.amount.len() <= 20;
    if !amount_ok {
        return Err(format!(
            "upsert-invoice: bad input: amount must be a decimal string like \"1250.00\", got \"{}\"",
            req.amount
        ));
    }
    if req.currency.len() != 3 || !req.currency.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(format!(
            "upsert-invoice: bad input: currency must be a 3-letter uppercase code, got \"{}\"",
            req.currency
        ));
    }
    parse_due_date(&req.due_date).map_err(|e| format!("upsert-invoice: bad input: {e}"))?;
    Ok(())
}

/// Parse YYYY-MM-DD into days since the Unix epoch. Rejects impossible dates.
pub fn parse_due_date(s: &str) -> Result<i64, String> {
    let bad = || format!("bad due_date \"{s}\": must be a valid YYYY-MM-DD");
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return Err(bad());
    }
    let num = |r: core::ops::Range<usize>| -> Result<i64, String> {
        let part = &s[r];
        if !part.bytes().all(|c| c.is_ascii_digit()) {
            return Err(bad());
        }
        part.parse::<i64>().map_err(|_| bad())
    };
    let (y, m, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1970..=2200).contains(&y) || !(1..=12).contains(&m) {
        return Err(bad());
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days_in_month = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&d) {
        return Err(bad());
    }
    Ok(days_from_civil(y, m as u32, d as u32))
}

/// Howard Hinnant's civil-date algorithm: (y, m, d) → days since 1970-01-01.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = (y - era * 400) as u64;
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp as u64 + 2) / 5 + d as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

pub fn is_overdue(inv: &Invoice, now_epoch_secs: u64) -> bool {
    inv.status == Status::Open
        && match parse_due_date(&inv.due_date) {
            Ok(due_day) => due_day < (now_epoch_secs / 86400) as i64,
            Err(_) => false, // stored dates were validated at upsert; be conservative
        }
}

// ---------------------------------------------------------------------------
// wasm-only: KV map access shared with remind.rs
// ---------------------------------------------------------------------------

#[cfg(target_arch = "wasm32")]
use crate::host::{
    interfaces::{kv_store, logging},
    tenant::tenant_context,
};

#[cfg(target_arch = "wasm32")]
pub fn invoices_map() -> String {
    format!("z:{}:invoices", hex::encode(tenant_context::tenant_did()))
}

#[cfg(target_arch = "wasm32")]
pub fn load_invoice(invoice_id: &str) -> Result<Option<Invoice>, String> {
    let bytes = kv_store::get(&invoices_map(), invoice_id.as_bytes())
        .map_err(|e| format!("kv read: {e}"))?;
    match bytes {
        None => Ok(None),
        Some(b) => serde_json::from_slice(&b)
            .map(Some)
            .map_err(|e| format!("stored invoice {invoice_id} is corrupt: {e}")),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn store_invoice(inv: &Invoice) -> Result<(), String> {
    let bytes = serde_json::to_vec(inv).map_err(|e| e.to_string())?;
    kv_store::put(&invoices_map(), inv.invoice_id.as_bytes(), &bytes)
        .map_err(|e| format!("kv write: {e}"))
}

#[cfg(target_arch = "wasm32")]
fn upsert_invoice_wasm(req: UpsertReq) -> Result<UpsertResp, String> {
    let now = tenant_context::cluster_timestamp_secs();
    let existing = load_invoice(&req.invoice_id)?;
    let created = existing.is_none();
    let inv = match existing {
        // Business fields update; bookkeeping (created_at, reminder history) survives.
        Some(prev) => Invoice {
            customer_ref: req.customer_ref,
            amount: req.amount,
            currency: req.currency,
            due_date: req.due_date,
            status: req.status.unwrap_or(prev.status),
            ..prev
        },
        None => Invoice {
            invoice_id: req.invoice_id,
            customer_ref: req.customer_ref,
            amount: req.amount,
            currency: req.currency,
            due_date: req.due_date,
            status: req.status.unwrap_or(Status::Open),
            created_at_epoch_secs: now,
            reminded_at_epoch_secs: None,
            reminder_count: 0,
        },
    };
    store_invoice(&inv)?;
    let _ = logging::info(&format!(
        "upsert-invoice: {} ({})",
        inv.invoice_id,
        if created { "created" } else { "updated" }
    ));
    Ok(UpsertResp {
        invoice_id: inv.invoice_id,
        created,
    })
}

#[cfg(target_arch = "wasm32")]
fn list_invoices_wasm(filter: &str) -> Result<ListResp, String> {
    const BATCH: u32 = 500;
    let map = invoices_map();
    let now = tenant_context::cluster_timestamp_secs();
    let mut out: Vec<Invoice> = Vec::new();
    let mut start: Vec<u8> = Vec::new();
    loop {
        // Half-open scan; the end key 0xFF is above any ASCII invoice_id.
        let batch = kv_store::scan(&map, &start, &[0xFFu8], BATCH)
            .map_err(|e| format!("kv scan: {e}"))?;
        let batch_len = batch.len();
        for (key, value) in batch {
            let inv: Invoice = serde_json::from_slice(&value).map_err(|e| {
                format!(
                    "stored invoice {} is corrupt: {e}",
                    String::from_utf8_lossy(&key)
                )
            })?;
            let keep = match filter {
                "open" => inv.status == Status::Open,
                "overdue" => is_overdue(&inv, now),
                _ => true,
            };
            if keep {
                out.push(inv);
            }
            // Scan is one-shot with no cursor: resume just past the last key.
            start = key;
            start.push(0);
        }
        if (batch_len as u32) < BATCH {
            break;
        }
    }
    Ok(ListResp {
        invoices: out,
        as_of_epoch_secs: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> serde_json::Value {
        serde_json::json!({
            "invoice_id": "INV-2026-001",
            "customer_ref": "ACME-0042",
            "amount": "1250.00",
            "currency": "USD",
            "due_date": "2026-09-30",
        })
    }

    #[test]
    fn upsert_rejects_smuggled_pii_fields() {
        let mut input = valid();
        input["customer_email"] = "jane@example.com".into();
        let err = upsert_invoice(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(err.contains("bad input"), "{err}");
    }

    #[test]
    fn upsert_rejects_email_as_customer_ref() {
        let mut input = valid();
        input["customer_ref"] = "jane@example.com".into();
        let err = upsert_invoice(&serde_json::to_vec(&input).unwrap()).unwrap_err();
        assert!(err.contains("not an email address"), "{err}");
    }

    #[test]
    fn upsert_rejects_non_json() {
        assert!(upsert_invoice(b"not json").unwrap_err().contains("bad input"));
    }

    #[test]
    fn upsert_rejects_float_amounts_and_bad_currency() {
        for (field, bad_value) in [
            ("amount", serde_json::json!("12,50")),
            ("amount", serde_json::json!("")),
            ("amount", serde_json::json!("1.23456")),
            ("currency", serde_json::json!("usd")),
            ("currency", serde_json::json!("USDC")),
            ("due_date", serde_json::json!("2026-02-30")),
            ("due_date", serde_json::json!("30-09-2026")),
        ] {
            let mut input = valid();
            input[field] = bad_value.clone();
            let err = upsert_invoice(&serde_json::to_vec(&input).unwrap()).unwrap_err();
            assert!(err.contains("bad input"), "{field}={bad_value}: {err}");
        }
    }

    #[test]
    fn list_rejects_unknown_filter() {
        let err = list_invoices(br#"{"filter":"unpaid"}"#).unwrap_err();
        assert!(err.contains("all|open|overdue"), "{err}");
    }

    // Anchors verified independently: 1970-01-01 is epoch day 0;
    // 2000-01-01 is day 10957 (30*365 + 7 leap days); 2000 is a leap year,
    // so 2000-03-01 = 10957 + 31 + 29 = 11017.
    #[test]
    fn civil_date_anchors() {
        assert_eq!(parse_due_date("1970-01-01").unwrap(), 0);
        assert_eq!(parse_due_date("2000-01-01").unwrap(), 10957);
        assert_eq!(parse_due_date("2000-03-01").unwrap(), 11017);
    }

    #[test]
    fn overdue_is_strictly_past_due_date() {
        let mut inv: Invoice = serde_json::from_value(serde_json::json!({
            "invoice_id": "INV-1", "customer_ref": "ACME", "amount": "10.00",
            "currency": "USD", "due_date": "2000-03-01", "status": "open",
            "created_at_epoch_secs": 0, "reminded_at_epoch_secs": null,
            "reminder_count": 0,
        }))
        .unwrap();
        let due_midnight = 11017u64 * 86400;
        assert!(!is_overdue(&inv, due_midnight + 86399)); // still the due day
        assert!(is_overdue(&inv, due_midnight + 86400)); // first second past it
        inv.status = Status::Paid;
        assert!(!is_overdue(&inv, due_midnight + 86400));
    }
}
