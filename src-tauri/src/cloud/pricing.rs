//! Published prices of the storage providers, as data, and the cost
//! estimate sotyware computes from them (ADR-041).
//!
//! Prices change on their own schedule and must carry their source and the
//! date they were read, so they are not in the provider profiles (ADR-016)
//! and not in code: each provider with a price list has one JSON file in
//! `src-tauri/pricing/` (format `sotyware-pricing`, version 1), compiled in
//! and checked by the tests below. Updating a price means editing that file
//! (the number exactly as the provider publishes it, in the provider's own
//! unit) and its `checkedOn` date. A provider without a file (a generic
//! S3-compatible service) gets usage figures but no estimate.
//!
//! Everything computed here is an **estimate** from list prices and
//! measured usage, never a bill: neither Backblaze nor Hetzner offers an API
//! for billing data, the stored amount is assumed constant for the period,
//! downloads by other programs cannot be measured (the user states an
//! amount), and taxes are not included. Each estimate lists its components
//! and the assumptions behind them.

use serde::{Deserialize, Serialize};

/// The period every estimate covers.
pub const PERIOD_HOURS: f64 = 720.0;
pub const PERIOD_LABEL: &str = "30 days";

/// The size below which a stored object counts as small for providers that
/// bill a minimum object size; measurements count small objects at this
/// threshold (`usage::Measurement::small_objects`).
pub const SMALL_OBJECT_BYTES: u64 = 64_000;

/// The price lists compiled into sotyware: `(provider id, JSON)`.
const FILES: &[(&str, &str)] = &[
    (
        "backblaze-b2",
        include_str!("../../pricing/backblaze-b2.json"),
    ),
    ("hetzner", include_str!("../../pricing/hetzner.json")),
];

/// The unit a price is published in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Per {
    /// Per TB stored for 30 days (Backblaze).
    #[serde(rename = "tb-30-days")]
    Tb30Days,
    /// Per TB stored for one hour (Hetzner).
    TbHour,
    /// Per GB transferred.
    Gb,
    /// Per TB transferred.
    Tb,
    /// A fixed price per month.
    Month,
}

/// Which buckets share one provider account's allowances and base fee.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountScope {
    /// One account per region (Backblaze: an account lives in one region).
    Region,
    /// One account for every bucket at the provider (Hetzner: the base fee
    /// is per account, whatever the projects and locations).
    Provider,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StoragePrice {
    pub price: f64,
    pub per: Per,
    pub free_bytes: u64,
    pub minimum_object_bytes: Option<u64>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BaseFee {
    pub price: f64,
    pub per: Per,
    pub included_storage_bytes: u64,
    pub included_egress_bytes: u64,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EgressPrice {
    pub price: f64,
    pub per: Per,
    /// Free egress as a multiple of the stored data (Backblaze: 3).
    pub free_multiple_of_stored: Option<f64>,
    pub included_bytes: u64,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransactionPrice {
    pub note: String,
}

/// One provider's published prices (a `src-tauri/pricing/*.json` file).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Pricing {
    pub format: String,
    pub version: u32,
    pub provider: String,
    pub name: String,
    /// ISO 4217 code.
    pub currency: String,
    /// `excluded`: the prices are without VAT or sales tax.
    pub taxes: String,
    /// When the prices were read from the sources, `YYYY-MM-DD`.
    pub checked_on: String,
    pub sources: Vec<String>,
    pub account_scope: AccountScope,
    /// What the provider's "TB" is taken to be.
    pub tb_bytes: u64,
    pub storage: StoragePrice,
    pub base_fee: Option<BaseFee>,
    pub egress: EgressPrice,
    pub transactions: TransactionPrice,
    pub notes: Vec<String>,
}

/// Why a price list is not usable (a defect in a compiled-in file).
fn check(p: &Pricing, id: &str) -> Result<(), String> {
    let bad = |what: &str| Err(format!("pricing {id}: {what}"));
    if p.format != "sotyware-pricing" || p.version != 1 {
        return bad("unknown format or version");
    }
    if p.provider != id {
        return bad("provider id does not match the file");
    }
    if p.currency.len() != 3 || !p.currency.chars().all(|c| c.is_ascii_uppercase()) {
        return bad("currency is not an ISO 4217 code");
    }
    if p.taxes != "excluded" {
        return bad("only prices without taxes are supported");
    }
    let date = p.checked_on.as_bytes();
    if date.len() != 10 || date[4] != b'-' || date[7] != b'-' {
        return bad("checkedOn is not YYYY-MM-DD");
    }
    if p.sources.is_empty() || p.sources.iter().any(|s| !s.starts_with("https://")) {
        return bad("every source must be an https:// link");
    }
    if p.tb_bytes == 0 {
        return bad("tbBytes must be positive");
    }
    let prices = [
        Some(p.storage.price),
        p.base_fee.as_ref().map(|b| b.price),
        Some(p.egress.price),
    ];
    if prices
        .into_iter()
        .flatten()
        .any(|v| !v.is_finite() || v < 0.0)
    {
        return bad("prices must be finite and not negative");
    }
    if !matches!(p.storage.per, Per::Tb30Days | Per::TbHour) {
        return bad("storage is priced per TB for 30 days or per TB-hour");
    }
    if !matches!(p.egress.per, Per::Gb | Per::Tb) {
        return bad("egress is priced per GB or per TB");
    }
    if p.base_fee.as_ref().is_some_and(|b| b.per != Per::Month) {
        return bad("a base fee is priced per month");
    }
    if p.storage
        .minimum_object_bytes
        .is_some_and(|m| m != SMALL_OBJECT_BYTES)
    {
        return bad(
            "a minimum object size must equal SMALL_OBJECT_BYTES, the threshold measurements count",
        );
    }
    if p.egress
        .free_multiple_of_stored
        .is_some_and(|m| !m.is_finite() || m < 0.0)
    {
        return bad("freeMultipleOfStored must be finite and not negative");
    }
    Ok(())
}

/// The price list for `provider`, if sotyware has one.
pub fn for_provider(provider: &str) -> Option<Pricing> {
    let (id, text) = FILES.iter().find(|(id, _)| *id == provider)?;
    match serde_json::from_str::<Pricing>(text)
        .map_err(|e| format!("pricing {id}: {e}"))
        .and_then(|p| check(&p, id).map(|()| p))
    {
        Ok(p) => Some(p),
        Err(e) => {
            // The tests make this unreachable for the files shipped.
            eprintln!("sotyware: {e}");
            None
        }
    }
}

/// What an estimate is computed from: one provider account's measured
/// usage (the sum of its measured buckets) and the user's assumption about
/// downloads.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct UsageInput {
    /// Everything stored: current and older versions, unfinished uploads.
    pub stored_bytes: u64,
    /// Stored objects (versions) smaller than [`SMALL_OBJECT_BYTES`], and
    /// their total size.
    pub small_objects: u64,
    pub small_bytes: u64,
    /// Downloads per 30 days, as the user assumes them.
    pub egress_bytes: u64,
}

/// One part of an estimate.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Component {
    /// `base-fee`, `storage`, `egress` or `transactions`.
    pub id: &'static str,
    pub label: String,
    pub amount: f64,
    pub detail: String,
}

/// An estimate for 30 days, in the price list's currency, without taxes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Estimate {
    pub currency: String,
    pub period: &'static str,
    pub total: f64,
    pub components: Vec<Component>,
    /// What the estimate takes for granted, in the user's words.
    pub assumptions: Vec<String>,
}

/// `bytes` as the dashboard writes sizes (1000-based, like the providers).
pub fn shown_bytes(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["bytes", "kB", "MB", "GB", "TB", "PB"];
    if bytes < 1000 {
        return format!("{bytes} bytes");
    }
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1000.0 && unit < UNITS.len() - 1 {
        v /= 1000.0;
        unit += 1;
    }
    let digits = if v < 10.0 {
        2
    } else if v < 100.0 {
        1
    } else {
        0
    };
    format!("{v:.digits$} {}", UNITS[unit])
}

fn money(p: &Pricing, amount: f64) -> String {
    format!("{} {amount:.2}", p.currency)
}

/// The cost for 30 days of storing `bytes` beyond `free` at `p`'s storage price.
fn storage_cost(p: &Pricing, billable: u64, free: u64) -> f64 {
    let tb = billable.saturating_sub(free) as f64 / p.tb_bytes as f64;
    match p.storage.per {
        Per::Tb30Days => tb * p.storage.price,
        Per::TbHour => tb * p.storage.price * PERIOD_HOURS,
        // Excluded by `check`.
        _ => 0.0,
    }
}

fn egress_cost(p: &Pricing, bytes: u64) -> f64 {
    match p.egress.per {
        Per::Gb => bytes as f64 / 1e9 * p.egress.price,
        Per::Tb => bytes as f64 / p.tb_bytes as f64 * p.egress.price,
        _ => 0.0,
    }
}

/// The 30-day estimate for one provider account's usage under `p`.
pub fn estimate(p: &Pricing, input: &UsageInput) -> Estimate {
    let mut components = Vec::new();
    let mut assumptions = vec![
        format!(
            "Prices of {} as published on {} ({}), without taxes; a contract or promotion may differ.",
            p.name,
            p.checked_on,
            p.sources.join(", ")
        ),
        format!(
            "The stored amount stays as measured for the whole {PERIOD_LABEL}; 1 TB is taken as {} bytes.",
            p.tb_bytes
        ),
    ];

    // Small objects billed as the minimum size.
    let mut billable = input.stored_bytes;
    let mut padding_note = String::new();
    if let Some(min) = p.storage.minimum_object_bytes {
        let padding = input
            .small_objects
            .saturating_mul(min)
            .saturating_sub(input.small_bytes);
        if input.small_objects > 0 {
            billable = billable.saturating_add(padding);
            padding_note = format!(
                "; {} object(s) under {} billed as {} each (+{})",
                input.small_objects,
                shown_bytes(min),
                shown_bytes(min),
                shown_bytes(padding)
            );
        }
    }

    let (included_storage, included_egress) = match &p.base_fee {
        Some(b) => {
            components.push(Component {
                id: "base-fee",
                label: "Base price".into(),
                amount: b.price,
                detail: format!(
                    "{} a month, including {} of storage and {} of egress. {}",
                    money(p, b.price),
                    shown_bytes(b.included_storage_bytes),
                    shown_bytes(b.included_egress_bytes),
                    b.note
                ),
            });
            assumptions.push(format!("A month is taken as {PERIOD_LABEL}."));
            (b.included_storage_bytes, b.included_egress_bytes)
        }
        None => (0, 0),
    };

    let free = p.storage.free_bytes.saturating_add(included_storage);
    let storage = storage_cost(p, billable, free);
    let rate = match p.storage.per {
        Per::Tb30Days => format!("{} per TB for 30 days", money(p, p.storage.price)),
        _ => format!(
            "{} per TB-hour ({} per TB for 30 days)",
            money(p, p.storage.price),
            money(p, p.storage.price * PERIOD_HOURS)
        ),
    };
    let free_note = match (p.storage.free_bytes, included_storage) {
        (0, 0) => String::new(),
        (f, 0) => format!(", the first {} free", shown_bytes(f)),
        (0, i) => format!(", {} included in the base price", shown_bytes(i)),
        (f, i) => format!(", {} free and {} included", shown_bytes(f), shown_bytes(i)),
    };
    components.push(Component {
        id: "storage",
        label: if included_storage > 0 {
            format!(
                "Storage above the included {}",
                shown_bytes(included_storage)
            )
        } else {
            "Storage".into()
        },
        amount: storage,
        detail: format!(
            "{} stored{free_note}{padding_note}, at {rate}.",
            shown_bytes(input.stored_bytes)
        ),
    });

    // Downloads: what the user assumes, beyond what is free.
    let free_egress = p
        .egress
        .free_multiple_of_stored
        .map_or(0, |m| (input.stored_bytes as f64 * m) as u64)
        .saturating_add(p.egress.included_bytes)
        .saturating_add(included_egress);
    let paid_egress = input.egress_bytes.saturating_sub(free_egress);
    let unit = match p.egress.per {
        Per::Gb => format!("{} per GB", money(p, p.egress.price)),
        _ => format!("{} per TB", money(p, p.egress.price)),
    };
    components.push(Component {
        id: "egress",
        label: "Downloads (egress)".into(),
        amount: egress_cost(p, paid_egress),
        detail: format!(
            "{} assumed for {PERIOD_LABEL}; {} free{}, then {unit}.",
            shown_bytes(input.egress_bytes),
            shown_bytes(free_egress),
            p.egress
                .free_multiple_of_stored
                .map_or(String::new(), |m| format!(" ({m} times the stored data)"))
        ),
    });
    assumptions.push(format!(
        "Downloads of {} for {PERIOD_LABEL}, as entered: sotyware cannot measure what other programs download.",
        shown_bytes(input.egress_bytes)
    ));

    components.push(Component {
        id: "transactions",
        label: "API calls".into(),
        amount: 0.0,
        detail: p.transactions.note.clone(),
    });

    let total = components.iter().map(|c| c.amount).sum();
    Estimate {
        currency: p.currency.clone(),
        period: PERIOD_LABEL,
        total,
        components,
        assumptions,
    }
}

/// What the dashboard shows about a price list.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingInfo {
    pub name: String,
    pub currency: String,
    pub checked_on: String,
    pub sources: Vec<String>,
    pub notes: Vec<String>,
    /// Storage included in a base price (Hetzner: 1 TB), for a meter.
    pub included_storage_bytes: Option<u64>,
    /// Storage free of charge (Backblaze: 10 GB).
    pub free_storage_bytes: u64,
}

impl From<&Pricing> for PricingInfo {
    fn from(p: &Pricing) -> Self {
        let mut notes = vec![p.storage.note.clone(), p.egress.note.clone()];
        if let Some(b) = &p.base_fee {
            notes.insert(0, b.note.clone());
        }
        notes.extend(p.notes.iter().cloned());
        Self {
            name: p.name.clone(),
            currency: p.currency.clone(),
            checked_on: p.checked_on.clone(),
            sources: p.sources.clone(),
            notes,
            included_storage_bytes: p.base_fee.as_ref().map(|b| b.included_storage_bytes),
            free_storage_bytes: p.storage.free_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GB: u64 = 1_000_000_000;
    const TB: u64 = 1_000 * GB;

    fn b2() -> Pricing {
        for_provider("backblaze-b2").expect("compiled in")
    }

    fn hetzner() -> Pricing {
        for_provider("hetzner").expect("compiled in")
    }

    fn component<'e>(e: &'e Estimate, id: &str) -> &'e Component {
        e.components.iter().find(|c| c.id == id).unwrap()
    }

    fn cents(v: f64) -> i64 {
        (v * 100.0).round() as i64
    }

    #[test]
    fn every_compiled_in_price_list_is_valid() {
        for (id, text) in FILES {
            let p: Pricing = serde_json::from_str(text).unwrap();
            check(&p, id).unwrap();
            assert_eq!(for_provider(id).as_ref(), Some(&p));
        }
        assert_eq!(for_provider("s3-compatible"), None);
        assert_eq!(for_provider("nope"), None);
    }

    /// Every price list belongs to a provider profile sotyware knows.
    #[test]
    fn price_lists_belong_to_known_providers() {
        for (id, _) in FILES {
            assert!(
                crate::cloud::provider::profile(id).is_some(),
                "no provider profile for {id}"
            );
        }
    }

    /// The figures read on 2026-11-15 from the providers' pages; changing a
    /// price list without updating these is a mistake this test catches.
    #[test]
    fn the_published_prices_as_read_on_2026_11_15() {
        let p = b2();
        assert_eq!(
            (p.storage.price, p.storage.per, p.storage.free_bytes),
            (6.95, Per::Tb30Days, 10 * GB)
        );
        assert_eq!((p.egress.price, p.egress.per), (0.01, Per::Gb));
        assert_eq!(p.egress.free_multiple_of_stored, Some(3.0));
        assert_eq!(
            (p.currency.as_str(), p.checked_on.as_str()),
            ("USD", "2026-11-15")
        );
        assert_eq!(p.base_fee, None);
        assert_eq!(p.account_scope, AccountScope::Region);

        let p = hetzner();
        let base = p.base_fee.as_ref().unwrap();
        assert_eq!(
            (
                base.price,
                base.included_storage_bytes,
                base.included_egress_bytes
            ),
            (6.49, TB, TB)
        );
        assert_eq!((p.storage.price, p.storage.per), (0.0087, Per::TbHour));
        assert_eq!(p.storage.minimum_object_bytes, Some(64_000));
        assert_eq!((p.egress.price, p.egress.per), (1.0, Per::Tb));
        assert_eq!(
            (p.currency.as_str(), p.checked_on.as_str()),
            ("EUR", "2026-11-15")
        );
        assert_eq!(p.account_scope, AccountScope::Provider);
    }

    #[test]
    fn backblaze_storage_after_the_free_10_gb() {
        let e = estimate(
            &b2(),
            &UsageInput {
                stored_bytes: 210 * GB,
                ..UsageInput::default()
            },
        );
        // 200 GB billable × $6.95/TB = $1.39.
        assert_eq!(cents(component(&e, "storage").amount), 139);
        assert_eq!(cents(e.total), 139);
        assert_eq!(e.currency, "USD");
        assert_eq!(e.period, "30 days");
        assert!(
            component(&e, "storage")
                .detail
                .contains("the first 10.0 GB free"),
            "{:?}",
            component(&e, "storage")
        );
        assert!(e.components.iter().all(|c| c.id != "base-fee"));
        // Below the free amount: nothing.
        let e = estimate(
            &b2(),
            &UsageInput {
                stored_bytes: 5 * GB,
                ..UsageInput::default()
            },
        );
        assert_eq!(cents(e.total), 0);
    }

    #[test]
    fn backblaze_downloads_are_free_up_to_three_times_the_stored_data() {
        let input = UsageInput {
            stored_bytes: TB,
            egress_bytes: 3 * TB,
            ..UsageInput::default()
        };
        let e = estimate(&b2(), &input);
        assert_eq!(cents(component(&e, "egress").amount), 0);
        // 1 TB more than the allowance: 1,000 GB × $0.01.
        let e = estimate(
            &b2(),
            &UsageInput {
                egress_bytes: 4 * TB,
                ..input
            },
        );
        assert_eq!(cents(component(&e, "egress").amount), 1_000);
        // Storage: (1 TB − 10 GB) × $6.95 = $6.88; total $16.88.
        assert_eq!(cents(component(&e, "storage").amount), 688);
        assert_eq!(cents(e.total), 1_688);
        assert_eq!(component(&e, "transactions").amount, 0.0);
    }

    #[test]
    fn hetzner_charges_the_base_price_even_for_an_empty_bucket() {
        let e = estimate(&hetzner(), &UsageInput::default());
        assert_eq!(cents(component(&e, "base-fee").amount), 649);
        assert_eq!(cents(component(&e, "storage").amount), 0);
        assert_eq!(cents(e.total), 649);
        assert_eq!(e.currency, "EUR");
    }

    #[test]
    fn hetzner_storage_beyond_the_included_terabyte_is_priced_per_tb_hour() {
        // 2 TB: 1 TB above the included one × €0.0087 × 720 h = €6.26.
        let e = estimate(
            &hetzner(),
            &UsageInput {
                stored_bytes: 2 * TB,
                ..UsageInput::default()
            },
        );
        assert_eq!(cents(component(&e, "storage").amount), 626);
        assert_eq!(cents(e.total), 649 + 626);
        assert!(component(&e, "storage").label.contains("included 1.00 TB"));
        // Egress beyond the included 1 TB: 2.5 TB more at €1/TB.
        let e = estimate(
            &hetzner(),
            &UsageInput {
                stored_bytes: TB,
                egress_bytes: 3_500 * GB,
                ..UsageInput::default()
            },
        );
        assert_eq!(cents(component(&e, "egress").amount), 250);
    }

    #[test]
    fn hetzner_bills_small_objects_as_64_kb() {
        // 1.1 TB stored, of which 10 million objects of 1 kB: each is billed
        // as 64 kB, adding 630 GB.
        let input = UsageInput {
            stored_bytes: 1_100 * GB,
            small_objects: 10_000_000,
            small_bytes: 10 * GB,
            egress_bytes: 0,
        };
        let e = estimate(&hetzner(), &input);
        // Billable 1,730 GB: 730 GB above the included TB × 6.264 = €4.57.
        assert_eq!(cents(component(&e, "storage").amount), 457);
        assert!(
            component(&e, "storage")
                .detail
                .contains("10000000 object(s) under 64.0 kB")
        );
        // Backblaze has no minimum size: the small objects change nothing.
        let e = estimate(&b2(), &input);
        assert_eq!(cents(component(&e, "storage").amount), cents(1.09 * 6.95));
    }

    #[test]
    fn an_estimate_states_its_assumptions_and_sources() {
        let e = estimate(
            &hetzner(),
            &UsageInput {
                egress_bytes: 50 * GB,
                ..UsageInput::default()
            },
        );
        let all = e.assumptions.join(" ");
        assert!(all.contains("as published on 2026-11-15"), "{all}");
        assert!(
            all.contains("https://www.hetzner.com/storage/object-storage/"),
            "{all}"
        );
        assert!(all.contains("without taxes"), "{all}");
        assert!(all.contains("50.0 GB"), "{all}");
        assert!(all.contains("A month is taken as 30 days"), "{all}");
        let info = PricingInfo::from(&hetzner());
        assert_eq!(info.checked_on, "2026-11-15");
        assert_eq!(
            (info.included_storage_bytes, info.free_storage_bytes),
            (Some(TB), 0)
        );
        let b2_info = PricingInfo::from(&b2());
        assert_eq!(
            (b2_info.included_storage_bytes, b2_info.free_storage_bytes),
            (None, 10 * GB)
        );
        assert!(info.notes.iter().any(|n| n.contains("Metadata")));
    }

    #[test]
    fn a_broken_price_list_is_refused() {
        let good = b2();
        type Change = Box<dyn Fn(&mut Pricing)>;
        let cases: Vec<(&str, Change)> = vec![
            ("format", Box::new(|p| p.format = "x".into())),
            ("provider", Box::new(|p| p.provider = "hetzner".into())),
            ("currency", Box::new(|p| p.currency = "usd".into())),
            ("taxes", Box::new(|p| p.taxes = "included".into())),
            ("date", Box::new(|p| p.checked_on = "15.11.2026".into())),
            ("source", Box::new(|p| p.sources = vec!["http://x".into()])),
            ("negative", Box::new(|p| p.storage.price = -1.0)),
            ("nan", Box::new(|p| p.egress.price = f64::NAN)),
            ("storage unit", Box::new(|p| p.storage.per = Per::Gb)),
            ("egress unit", Box::new(|p| p.egress.per = Per::Month)),
            (
                "minimum",
                Box::new(|p| p.storage.minimum_object_bytes = Some(4096)),
            ),
            ("tb", Box::new(|p| p.tb_bytes = 0)),
        ];
        for (name, change) in cases {
            let mut p = good.clone();
            change(&mut p);
            assert!(check(&p, "backblaze-b2").is_err(), "{name}");
        }
        // Unknown fields are refused too (a typo would otherwise be ignored).
        let text = FILES[0].1.replacen("\"notes\"", "\"notess\"", 1);
        assert!(serde_json::from_str::<Pricing>(&text).is_err());
    }

    #[test]
    fn sizes_are_written_like_the_providers_write_them() {
        assert_eq!(shown_bytes(0), "0 bytes");
        assert_eq!(shown_bytes(999), "999 bytes");
        assert_eq!(shown_bytes(64_000), "64.0 kB");
        assert_eq!(shown_bytes(10 * GB), "10.0 GB");
        assert_eq!(shown_bytes(1_234_567_890_123), "1.23 TB");
    }
}
