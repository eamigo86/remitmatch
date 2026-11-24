//! Synthetic data generator with a known solution, shared by the integration tests.

// Each integration test file compiles this module as its own crate and uses only part of it.
#![allow(dead_code)]

use std::collections::BTreeMap;

use chrono::{Days, NaiveDate};
use remitmatch::{Invoice, Payment};

/// Probabilities of each kind of noise, each in the range 0 to 1.
pub struct Noise {
    /// A payment is short by a bank fee of 1 to 300 minor units.
    pub fee: f64,
    /// The invoice number in the reference has exactly one edit.
    pub typo: f64,
    /// The reference carries no invoice number.
    pub missing_reference: f64,
    /// The payer name is a bank-style variant of the customer name.
    pub abbreviated_name: f64,
    /// An invoice has no payment.
    pub unpaid: f64,
    /// Extra payments (as a fraction of the invoice count) that settle nothing.
    pub unrelated: f64,
}

impl Noise {
    /// No noise at all.
    pub fn none() -> Self {
        Self {
            fee: 0.0,
            typo: 0.0,
            missing_reference: 0.0,
            abbreviated_name: 0.0,
            unpaid: 0.0,
            unrelated: 0.0,
        }
    }

    /// The noise level used for the accuracy targets.
    pub fn realistic() -> Self {
        Self {
            fee: 0.15,
            typo: 0.05,
            missing_reference: 0.05,
            abbreviated_name: 0.30,
            unpaid: 0.10,
            unrelated: 0.05,
        }
    }
}

/// Generated invoices and payments plus the correct pairing.
pub struct Dataset {
    pub invoices: Vec<Invoice>,
    pub payments: Vec<Payment>,
    /// Payment id to invoice id.
    pub solution: BTreeMap<String, String>,
}

/// SplitMix64, enough for reproducible test data without a `rand` dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform value in `0..n`; the modulo bias is irrelevant for test data.
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    /// Uniform value in `lo..=hi`.
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next() % (hi - lo + 1) as u64) as i64
    }

    fn chance(&mut self, p: f64) -> bool {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 <= p && p > 0.0
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

const SCOPES: [&str; 2] = ["alpha", "beta"];
const CURRENCIES: [&str; 2] = ["EUR", "USD"];
const FIRST: [&str; 8] = [
    "Construcciones",
    "Transportes",
    "Industrias",
    "Servicios",
    "Distribuciones",
    "Maderas",
    "Aceros",
    "Suministros",
];
const SURNAMES: [&str; 8] = [
    "Ibáñez",
    "Pérez",
    "García",
    "Núñez",
    "Martín",
    "Fernández",
    "López",
    "Muñoz",
];
const CITIES: [&str; 6] = ["Valencia", "Sevilla", "Bilbao", "Málaga", "Girona", "León"];
const SUFFIXES: [&str; 6] = ["S.L.", "S.A.", "LLC", "Inc.", "GmbH", "Ltd"];

struct Customer {
    name: String,
    bank_name: String,
    scope: &'static str,
    currency: &'static str,
    fixed_amount: Option<i64>,
}

fn strip_accents(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' => 'u',
            'ñ' => 'n',
            'Á' => 'A',
            'É' => 'E',
            'Í' => 'I',
            'Ó' => 'O',
            'Ú' => 'U',
            'Ñ' => 'N',
            other => other,
        })
        .collect()
}

fn customer(rng: &mut Rng) -> Customer {
    let words = [*rng.pick(&FIRST), *rng.pick(&SURNAMES), *rng.pick(&CITIES)];
    let suffix = *rng.pick(&SUFFIXES);
    let name = format!("{} {suffix}", words.join(" "));
    // Bank style: uppercase, no accents, suffix dropped and/or words truncated.
    let mut bank: Vec<String> = words
        .iter()
        .map(|w| strip_accents(w).to_uppercase())
        .collect();
    if rng.chance(0.5) {
        bank.iter_mut().for_each(|w| w.truncate(8));
    }
    if rng.chance(0.5) {
        bank.push(strip_accents(suffix).to_uppercase());
    }
    Customer {
        name,
        bank_name: bank.join(" "),
        scope: rng.pick(&SCOPES),
        currency: rng.pick(&CURRENCIES),
        fixed_amount: rng.chance(0.2).then(|| rng.range(1_000, 5_000_000)),
    }
}

/// Renders the compact number (for example `INV202400017`) in one of three styles.
fn render_number(compact: &[char], style: usize) -> String {
    let mut out = String::new();
    for (i, c) in compact.iter().enumerate() {
        if style < 2 && (i == 3 || i == 7) {
            out.push(if style == 0 { '-' } else { ' ' });
        }
        out.push(if style == 1 {
            c.to_ascii_lowercase()
        } else {
            *c
        });
    }
    out
}

/// Applies exactly one edit to the digit part of the compact number.
fn typo(rng: &mut Rng, compact: &mut Vec<char>) {
    let pos = 3 + rng.below(compact.len() - 3);
    match rng.below(3) {
        0 => {
            let old = compact[pos];
            compact[pos] = if old == '9' {
                '0'
            } else {
                (old as u8 + 1) as char
            };
        }
        1 => {
            // Swap two different neighbours; fall back to a drop when all are equal.
            let swap = (3..compact.len() - 1).find(|&i| compact[i] != compact[i + 1]);
            match swap {
                Some(i) => compact.swap(i, i + 1),
                None => {
                    compact.remove(pos);
                }
            }
        }
        _ => {
            compact.remove(pos);
        }
    }
}

fn reference(rng: &mut Rng, number: &str, short_name: &str, noise: &Noise) -> String {
    if rng.chance(noise.missing_reference) {
        return format!("TRANSFER {:06}", rng.below(1_000_000));
    }
    let mut compact: Vec<char> = number.chars().filter(|c| *c != '-').collect();
    if rng.chance(noise.typo) {
        typo(rng, &mut compact);
    }
    match rng.below(3) {
        0 => format!("Payment {}", render_number(&compact, 0)),
        1 => format!("fact {} gracias", render_number(&compact, 1)),
        _ => format!("TRANSFER/{}/{short_name}", render_number(&compact, 2)),
    }
}

/// Generates a reproducible dataset with the given noise; the same seed gives the same data.
pub fn generate(seed: u64, n_invoices: usize, noise: &Noise) -> Dataset {
    let mut rng = Rng(seed);
    let customers: Vec<Customer> = (0..(n_invoices / 10).max(1))
        .map(|_| customer(&mut rng))
        .collect();
    let base = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let shift = |date: NaiveDate, days: i64| {
        if days >= 0 {
            date + Days::new(days as u64)
        } else {
            date - Days::new(days.unsigned_abs())
        }
    };

    let mut invoices = Vec::new();
    let mut payments = Vec::new();
    let mut solution = BTreeMap::new();
    for n in 0..n_invoices {
        let c = &customers[rng.below(customers.len())];
        let due = shift(base, rng.range(0, 364));
        let amount = c
            .fixed_amount
            .unwrap_or_else(|| rng.range(1_000, 5_000_000));
        let invoice = Invoice {
            id: format!("I-{n:05}"),
            number: format!("INV-2024-{:05}", n + 1),
            amount_minor: amount,
            currency: c.currency.to_owned(),
            issue_date: shift(due, -30),
            due_date: due,
            counterparty: c.name.clone(),
            scope: c.scope.to_owned(),
        };
        if !rng.chance(noise.unpaid) {
            let paid = if rng.chance(noise.fee) {
                amount - rng.range(1, 300)
            } else {
                amount
            };
            let offset = if rng.chance(0.05) {
                rng.range(0, 60)
            } else {
                rng.range(-10, 25)
            };
            let short = c.bank_name.split(' ').next().unwrap_or_default().to_owned();
            let payment = Payment {
                id: format!("P-{:05}", payments.len()),
                amount_minor: paid,
                currency: c.currency.to_owned(),
                value_date: shift(due, offset),
                reference: reference(&mut rng, &invoice.number, &short, noise),
                counterparty: if rng.chance(noise.abbreviated_name) {
                    c.bank_name.clone()
                } else {
                    c.name.clone()
                },
                scope: c.scope.to_owned(),
            };
            solution.insert(payment.id.clone(), invoice.id.clone());
            payments.push(payment);
        }
        invoices.push(invoice);
    }
    for _ in 0..(n_invoices as f64 * noise.unrelated).round() as usize {
        let c = &customers[rng.below(customers.len())];
        payments.push(Payment {
            id: format!("P-{:05}", payments.len()),
            amount_minor: rng.range(1_000, 5_000_000),
            currency: c.currency.to_owned(),
            value_date: shift(base, rng.range(0, 364)),
            reference: format!("TRANSFER {:06}", rng.below(1_000_000)),
            counterparty: c.name.clone(),
            scope: c.scope.to_owned(),
        });
    }
    // Fisher-Yates, so input order carries no information.
    for i in (1..payments.len()).rev() {
        payments.swap(i, rng.below(i + 1));
    }
    Dataset {
        invoices,
        payments,
        solution,
    }
}
