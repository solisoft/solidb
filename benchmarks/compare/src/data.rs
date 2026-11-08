//! The dataset, generated the same way for every engine: document `i` is a
//! pure function of `i`, so any engine, any worker and the reference counts
//! all agree without sharing state.

use serde_json::{json, Value};

/// 2024-01-01T00:00:00Z. Orders are spread over the two years after it.
pub const EPOCH: u64 = 1_704_067_200;
pub const SPAN_SECS: u64 = 2 * 365 * 86_400;
/// The width of the `created_at` window the range and join scenarios read:
/// one day, about 1,370 orders at a million documents, of which they keep 20.
pub const WINDOW_SECS: u64 = 86_400;
pub const RECENT_LIMIT: usize = 20;

pub const STATUSES: [(&str, u64); 5] = [
    ("delivered", 70),
    ("shipped", 15),
    ("pending", 8),
    ("cancelled", 5),
    ("returned", 2),
];
const COUNTRIES: [&str; 20] = [
    "FR", "DE", "ES", "IT", "GB", "NL", "BE", "CH", "PT", "SE", "NO", "DK", "PL", "AT", "IE", "US",
    "CA", "BR", "JP", "AU",
];
const CITIES: [&str; 16] = [
    "Lyon",
    "Paris",
    "Lille",
    "Nantes",
    "Bordeaux",
    "Toulouse",
    "Nice",
    "Rennes",
    "Berlin",
    "Madrid",
    "Milan",
    "London",
    "Amsterdam",
    "Brussels",
    "Geneva",
    "Lisbon",
];
const STREETS: [&str; 8] = [
    "rue des Lilas",
    "avenue Jean Jaurès",
    "boulevard Voltaire",
    "rue de la République",
    "place Bellecour",
    "quai Saint-Antoine",
    "rue Victor Hugo",
    "chemin des Vignes",
];
const TAGS: [&str; 12] = [
    "gift",
    "express",
    "promo",
    "first-order",
    "b2b",
    "fragile",
    "bulky",
    "subscription",
    "loyalty",
    "marketplace",
    "pickup",
    "eco",
];
const CHANNELS: [&str; 4] = ["web", "mobile", "store", "phone"];
const TIERS: [&str; 4] = ["bronze", "silver", "gold", "platinum"];
const NOTES: [&str; 4] = [
    "Leave the parcel with the concierge if nobody answers the door.",
    "Call before delivery; the intercom does not work.",
    "Deliver to the side entrance, next to the bakery.",
    "",
];

/// SplitMix64: a well-mixed 64-bit value per (seed, i).
pub fn mix(seed: u64, i: u64) -> u64 {
    let mut z = seed ^ i.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// The xorshift generator of `clients/rust-client/examples/engine_bench.rs`,
/// for the parameters each worker draws.
#[derive(Clone)]
pub struct Rng(pub u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(mix(seed, 0x5eed) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

pub fn order_key(i: u64) -> String {
    format!("o{i:07}")
}

pub fn customer_key(c: u64) -> String {
    format!("c{c:05}")
}

/// Days since 1970-01-01 to (year, month, day), Howard Hinnant's algorithm.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

pub fn iso(secs: u64) -> String {
    let (y, m, d) = civil((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        s / 3600,
        s / 60 % 60,
        s % 60
    )
}

/// The parts of order `i` that the reference counts depend on.
pub struct OrderFacts {
    pub customer: u64,
    pub status: usize,
    pub amount_cents: u64,
    pub created: u64,
}

pub fn order_facts(i: u64, customers: u64) -> OrderFacts {
    let h = mix(1, i);
    let roll = mix(2, i) % 100;
    let mut acc = 0;
    let mut status = 0;
    for (n, (_, weight)) in STATUSES.iter().enumerate() {
        acc += weight;
        if roll < acc {
            status = n;
            break;
        }
    }
    let amount_cents = (0..3).map(|k| item(i, k)).map(|(_, q, p)| q * p).sum();
    OrderFacts {
        customer: h % customers,
        status,
        amount_cents,
        created: EPOCH + mix(3, i) % SPAN_SECS,
    }
}

/// (sku, quantity, unit price in cents) of item `k` of order `i`.
fn item(i: u64, k: u64) -> (u64, u64, u64) {
    let h = mix(10 + k, i);
    (h % 50_000, 1 + (h >> 20) % 4, 199 + (h >> 32) % 9_800)
}

/// Order `i` as a JSON document, keyed by `key`. Every engine stores this
/// object, renaming `key` to its own identifier field.
pub fn order(i: u64, key: &str, customers: u64) -> Value {
    let f = order_facts(i, customers);
    let h = mix(4, i);
    let items: Vec<Value> = (0..3)
        .map(|k| {
            let (sku, qty, price) = item(i, k);
            json!({"sku": format!("SKU-{sku:05}"), "qty": qty, "price": price as f64 / 100.0})
        })
        .collect();
    json!({
        "key": key,
        "customer_id": customer_key(f.customer),
        "status": STATUSES[f.status].0,
        "amount": f.amount_cents as f64 / 100.0,
        "currency": "EUR",
        "created_at": iso(f.created),
        "country": COUNTRIES[(h % 20) as usize],
        "channel": CHANNELS[(h >> 8) as usize % 4],
        "tags": [TAGS[(h >> 12) as usize % 12], TAGS[(h >> 16) as usize % 12], TAGS[(h >> 20) as usize % 12]],
        "items": items,
        "address": {
            "street": format!("{} {}", 1 + (h >> 24) % 180, STREETS[(h >> 32) as usize % 8]),
            "city": CITIES[(h >> 36) as usize % 16],
            "zip": format!("{:05}", 1000 + (h >> 40) % 94_000),
        },
        "note": NOTES[(h >> 48) as usize % 4],
    })
}

pub fn customer(c: u64) -> Value {
    let h = mix(5, c);
    json!({
        "key": customer_key(c),
        "name": format!("Customer {c}"),
        "email": format!("customer{c}@example.com"),
        "tier": TIERS[(h % 4) as usize],
        "country": COUNTRIES[(h >> 8) as usize % 20],
        "since": iso(EPOCH - (h >> 16) % (3 * 365 * 86_400)),
    })
}

/// What every engine must answer, computed from the generator alone.
pub struct Reference {
    pub per_customer: Vec<u32>,
    /// Per status, in `STATUSES` order: (count, sum of amounts in cents).
    pub per_status: Vec<(u64, u64)>,
    /// Every `created_at`, sorted, to count a window.
    pub created: Vec<u64>,
}

impl Reference {
    pub fn build(docs: u64, customers: u64) -> Self {
        let mut per_customer = vec![0u32; customers as usize];
        let mut per_status = vec![(0u64, 0u64); STATUSES.len()];
        let mut created = Vec::with_capacity(docs as usize);
        for i in 0..docs {
            let f = order_facts(i, customers);
            per_customer[f.customer as usize] += 1;
            per_status[f.status].0 += 1;
            per_status[f.status].1 += f.amount_cents;
            created.push(f.created);
        }
        created.sort_unstable();
        Reference {
            per_customer,
            per_status,
            created,
        }
    }

    /// Rows in `[lo, hi)`, capped at the scenarios' LIMIT.
    pub fn recent_rows(&self, lo: u64, hi: u64) -> usize {
        let a = self.created.partition_point(|&t| t < lo);
        let b = self.created.partition_point(|&t| t < hi);
        (b - a).min(RECENT_LIMIT)
    }
}

/// A random one-day window, as the ISO bounds the queries take.
pub fn window(r: &mut Rng) -> (u64, u64) {
    let lo = EPOCH + r.below(SPAN_SECS - WINDOW_SECS);
    (lo, lo + WINDOW_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso(EPOCH), "2024-01-01T00:00:00Z");
        assert_eq!(iso(EPOCH + 59 * 86_400 + 3661), "2024-02-29T01:01:01Z");
        assert_eq!(iso(1_735_689_599), "2024-12-31T23:59:59Z");
    }

    #[test]
    fn document_size() {
        let len = serde_json::to_vec(&order(42, &order_key(42), 100_000))
            .unwrap()
            .len();
        assert!((400..700).contains(&len), "{len}");
    }
}
