//! Synthetic data generator with a known solution, shared by the integration tests.

// Each integration test file compiles this module as its own crate and uses only part of it.
#![allow(dead_code)]

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
    /// An invoice joins a batch of 2 to 4 invoices that its customer pays with one payment.
    pub grouped: f64,
    /// An invoice is paid by 2 or 3 payments that add up to its amount.
    pub partial: f64,
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
            grouped: 0.0,
            partial: 0.0,
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
            grouped: 0.08,
            partial: 0.05,
        }
    }
}

/// Generated invoices and payments plus the correct pairing.
pub struct Dataset {
    pub invoices: Vec<Invoice>,
    pub payments: Vec<Payment>,
    /// The true groups as (payment ids, invoice ids), each list sorted ascending.
    /// A one-to-one payment is a group of one payment and one invoice.
    pub solution: Vec<(Vec<String>, Vec<String>)>,
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

/// A customer's open batch of invoices (indices into the invoice list) and its target size.
struct Batch {
    members: Vec<usize>,
    target: usize,
}

fn shift(date: NaiveDate, days: i64) -> NaiveDate {
    if days >= 0 {
        date + Days::new(days as u64)
    } else {
        date - Days::new(days.unsigned_abs())
    }
}

/// Payer name as the bank reports it: abbreviated with the given probability.
fn payer(rng: &mut Rng, c: &Customer, noise: &Noise) -> String {
    if rng.chance(noise.abbreviated_name) {
        c.bank_name.clone()
    } else {
        c.name.clone()
    }
}

/// First word of the customer's bank name, used in one reference style.
fn short_name(c: &Customer) -> String {
    c.bank_name.split(' ').next().unwrap_or_default().to_owned()
}

/// Appends a payment of the customer and returns its id.
fn push_payment(
    payments: &mut Vec<Payment>,
    c: &Customer,
    amount_minor: i64,
    value_date: NaiveDate,
    reference: String,
    counterparty: String,
) -> String {
    let id = format!("P-{:05}", payments.len());
    payments.push(Payment {
        id: id.clone(),
        amount_minor,
        currency: c.currency.to_owned(),
        value_date,
        reference,
        counterparty,
        scope: c.scope.to_owned(),
    });
    id
}

/// Value date: usually close to the due date, sometimes very late.
fn late_offset(rng: &mut Rng) -> i64 {
    if rng.chance(0.05) {
        rng.range(0, 60)
    } else {
        rng.range(-10, 25)
    }
}

/// Pays one invoice with one payment.
fn pay_single(
    rng: &mut Rng,
    c: &Customer,
    noise: &Noise,
    invoice: &Invoice,
    payments: &mut Vec<Payment>,
    solution: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let paid = if rng.chance(noise.fee) {
        invoice.amount_minor - rng.range(1, 300)
    } else {
        invoice.amount_minor
    };
    let offset = late_offset(rng);
    let reference = reference(rng, &invoice.number, &short_name(c), noise);
    let name = payer(rng, c, noise);
    let id = push_payment(
        payments,
        c,
        paid,
        shift(invoice.due_date, offset),
        reference,
        name,
    );
    solution.push((vec![id], vec![invoice.id.clone()]));
}

/// Pays one invoice with 2 or 3 payments, each at least 10% of the amount.
///
/// The fee, when it fires, is taken from the last part (capped so the part stays positive).
fn pay_partial(
    rng: &mut Rng,
    c: &Customer,
    noise: &Noise,
    invoice: &Invoice,
    payments: &mut Vec<Payment>,
    solution: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let parts = 2 + rng.below(2);
    let min_part = (invoice.amount_minor + 9) / 10;
    let mut remaining = invoice.amount_minor;
    let mut amounts = Vec::new();
    for left in (1..parts).rev() {
        let part = rng.range(min_part, remaining - min_part * left as i64);
        amounts.push(part);
        remaining -= part;
    }
    amounts.push(remaining);
    if rng.chance(noise.fee) {
        let last = amounts.last_mut().unwrap_or(&mut remaining);
        *last -= rng.range(1, 300.min(*last - 1));
    }
    let mut ids = Vec::new();
    for amount in amounts {
        let offset = rng.range(-10, 25);
        let reference = reference(rng, &invoice.number, &short_name(c), noise);
        let name = payer(rng, c, noise);
        ids.push(push_payment(
            payments,
            c,
            amount,
            shift(invoice.due_date, offset),
            reference,
            name,
        ));
    }
    ids.sort();
    solution.push((ids, vec![invoice.id.clone()]));
}

/// Pays a batch of invoices with one payment that lists every number.
fn pay_batch(
    rng: &mut Rng,
    c: &Customer,
    noise: &Noise,
    members: &[&Invoice],
    payments: &mut Vec<Payment>,
    solution: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let mut total: i64 = members.iter().map(|i| i.amount_minor).sum();
    if rng.chance(noise.fee) {
        total -= rng.range(1, 300);
    }
    let last_due = members.iter().map(|i| i.due_date).max().unwrap_or_default();
    let offset = late_offset(rng);
    let reference = if rng.chance(noise.missing_reference) {
        format!("TRANSFER {:06}", rng.below(1_000_000))
    } else {
        let style = rng.below(3);
        let numbers: Vec<String> = members
            .iter()
            .map(|i| {
                let mut compact: Vec<char> = i.number.chars().filter(|c| *c != '-').collect();
                if rng.chance(noise.typo) {
                    typo(rng, &mut compact);
                }
                render_number(&compact, style)
            })
            .collect();
        format!("Payment {}", numbers.join(" "))
    };
    let name = payer(rng, c, noise);
    let id = push_payment(payments, c, total, shift(last_due, offset), reference, name);
    let mut invoice_ids: Vec<String> = members.iter().map(|i| i.id.clone()).collect();
    invoice_ids.sort();
    solution.push((vec![id], invoice_ids));
}

/// Closes a batch: one payment for two or more members, a plain single payment for one.
fn close_batch(
    rng: &mut Rng,
    c: &Customer,
    noise: &Noise,
    invoices: &[Invoice],
    batch: &Batch,
    payments: &mut Vec<Payment>,
    solution: &mut Vec<(Vec<String>, Vec<String>)>,
) {
    let members: Vec<&Invoice> = batch.members.iter().map(|&k| &invoices[k]).collect();
    if let [only] = members.as_slice() {
        pay_single(rng, c, noise, only, payments, solution);
    } else {
        pay_batch(rng, c, noise, &members, payments, solution);
    }
}

/// Generates a reproducible dataset with the given noise; the same seed gives the same data.
///
/// Per invoice, in order: unpaid, else paid in parts, else joined to a batch, else paid by
/// one payment.
pub fn generate(seed: u64, n_invoices: usize, noise: &Noise) -> Dataset {
    let mut rng = Rng(seed);
    let customers: Vec<Customer> = (0..(n_invoices / 10).max(1))
        .map(|_| customer(&mut rng))
        .collect();
    let base = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();

    let mut invoices: Vec<Invoice> = Vec::new();
    let mut payments = Vec::new();
    let mut solution = Vec::new();
    let mut batches: Vec<Option<Batch>> = customers.iter().map(|_| None).collect();
    for n in 0..n_invoices {
        let ci = rng.below(customers.len());
        let c = &customers[ci];
        let unpaid = rng.chance(noise.unpaid);
        let partial = !unpaid && rng.chance(noise.partial);
        let grouped = !unpaid && !partial && rng.chance(noise.grouped);
        let due = match &batches[ci] {
            Some(batch) if grouped => shift(invoices[batch.members[0]].due_date, rng.range(0, 20)),
            _ => shift(base, rng.range(0, 364)),
        };
        let amount = c
            .fixed_amount
            .unwrap_or_else(|| rng.range(1_000, 5_000_000));
        invoices.push(Invoice {
            id: format!("I-{n:05}"),
            number: format!("INV-2024-{:05}", n + 1),
            amount_minor: amount,
            currency: c.currency.to_owned(),
            issue_date: shift(due, -30),
            due_date: due,
            counterparty: c.name.clone(),
            scope: c.scope.to_owned(),
        });
        let invoice = &invoices[n];
        if unpaid {
            continue;
        }
        if partial {
            pay_partial(&mut rng, c, noise, invoice, &mut payments, &mut solution);
        } else if grouped {
            let batch = batches[ci].get_or_insert_with(|| Batch {
                members: Vec::new(),
                target: 2 + rng.below(3),
            });
            batch.members.push(n);
            let full = batch.members.len() == batch.target;
            if let Some(batch) = batches[ci].take_if(|_| full) {
                close_batch(
                    &mut rng,
                    c,
                    noise,
                    &invoices,
                    &batch,
                    &mut payments,
                    &mut solution,
                );
            }
        } else {
            pay_single(&mut rng, c, noise, invoice, &mut payments, &mut solution);
        }
    }
    for (ci, batch) in batches.into_iter().enumerate() {
        if let Some(batch) = batch {
            let c = &customers[ci];
            close_batch(
                &mut rng,
                c,
                noise,
                &invoices,
                &batch,
                &mut payments,
                &mut solution,
            );
        }
    }
    for _ in 0..(n_invoices as f64 * noise.unrelated).round() as usize {
        let c = &customers[rng.below(customers.len())];
        let amount = rng.range(1_000, 5_000_000);
        let date = shift(base, rng.range(0, 364));
        let reference = format!("TRANSFER {:06}", rng.below(1_000_000));
        push_payment(&mut payments, c, amount, date, reference, c.name.clone());
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

/// Counterparty names of the dense generator; three pairs differ only by a legal suffix.
const DENSE_NAMES: [&str; 8] = [
    "Acme Trading",
    "Acme Trading Ltd",
    "Borealis Foods",
    "Borealis Foods S.L.",
    "Cobalt Works",
    "Delta Freight GmbH",
    "Delta Freight",
    "Evergreen Supply",
];

/// The few distinct amounts of the dense generator, in minor units.
const DENSE_AMOUNTS: [i64; 12] = [
    5_000, 7_500, 10_000, 12_500, 15_000, 20_000, 25_000, 25_050, 40_000, 50_000, 75_000, 100_000,
];

/// Returns the index of the name that differs from `name` only by a legal suffix, if any.
fn dense_twin(name: usize) -> usize {
    match name {
        0 => 1,
        1 => 0,
        2 => 3,
        3 => 2,
        5 => 6,
        6 => 5,
        other => other,
    }
}

/// Returns the invoice number of pool entry `k`; every eighth number is a bare digit run.
fn dense_number(k: usize) -> String {
    if k.is_multiple_of(8) {
        format!("{k:04}")
    } else {
        format!("INV-{k:04}")
    }
}

/// Writes a payment reference naming the given pool numbers in one of three styles.
///
/// Up to three numbers are named; sometimes none, sometimes an extra one that the payment
/// does not settle.
fn dense_reference(rng: &mut Rng, settled: &[usize], pool: usize) -> String {
    if rng.chance(0.1) {
        return String::new();
    }
    let mut named: Vec<usize> = if rng.chance(0.25) {
        Vec::new()
    } else {
        settled.iter().take(3).copied().collect()
    };
    if named.len() < 3 && rng.chance(0.15) {
        named.push(1 + rng.below(pool));
    }
    if named.is_empty() {
        return format!("TRANSFER {:06}", rng.below(1_000_000));
    }
    let style = rng.below(3);
    let parts: Vec<String> = named
        .iter()
        .map(|&k| {
            let mut text = match style {
                0 => dense_number(k),
                1 => dense_number(k).to_lowercase().replace('-', " "),
                _ => dense_number(k).replace('-', ""),
            };
            if rng.chance(0.1) {
                // One edit: an inserted digit, a dropped character or a changed digit.
                let at = text.len() - 1 - rng.below(3);
                match rng.below(3) {
                    0 => text.insert(at, '7'),
                    1 => {
                        text.remove(at);
                    }
                    _ => text.replace_range(at..=at, "3"),
                }
            }
            text
        })
        .collect();
    format!("Payment {}", parts.join(" "))
}

/// Generates a reproducible dataset built for collisions; the same seed gives the same data.
///
/// Few distinct amounts, eight counterparty names (three pairs differ only by a legal
/// suffix), a small pool of shared invoice numbers, due dates inside about 60 days, two
/// scopes, two currencies (sometimes in lower case) and some non-positive amounts. Payments
/// are single, partial (two payments for one invoice) or grouped (one payment for two or
/// three invoices of one partition), and references name zero to three numbers. The
/// dataset has no known solution, so `solution` is empty.
pub fn generate_dense(seed: u64, n_invoices: usize) -> Dataset {
    let mut rng = Rng(seed);
    let base = NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    let pool = (n_invoices / 8).clamp(8, 400);

    let mut invoices: Vec<Invoice> = Vec::new();
    let mut names: Vec<usize> = Vec::new();
    for n in 0..n_invoices {
        let name = rng.below(DENSE_NAMES.len());
        let mut amount = *rng.pick(&DENSE_AMOUNTS);
        match rng.below(40) {
            0 => amount = 0,
            1 => amount = -amount,
            _ if rng.chance(0.1) => amount += rng.range(1, 250),
            _ => {}
        }
        let due = shift(base, rng.range(0, 60));
        let currency = *rng.pick(&CURRENCIES);
        invoices.push(Invoice {
            id: format!("I-{n:05}"),
            number: dense_number(1 + rng.below(pool)),
            amount_minor: amount,
            currency: if rng.chance(0.1) {
                currency.to_lowercase()
            } else {
                currency.to_owned()
            },
            issue_date: shift(due, -30),
            due_date: due,
            counterparty: DENSE_NAMES[name].to_owned(),
            scope: (*rng.pick(&SCOPES)).to_owned(),
        });
        names.push(name);
    }

    let key = |i: &Invoice| (i.scope.clone(), i.currency.to_uppercase());
    let mut payments: Vec<Payment> = Vec::new();
    let mut push = |rng: &mut Rng, anchor: usize, amount: i64, numbers: &[usize], pool| {
        let inv = &invoices[anchor];
        let name = if rng.chance(0.8) {
            names[anchor]
        } else if rng.chance(0.5) {
            dense_twin(names[anchor])
        } else {
            rng.below(DENSE_NAMES.len())
        };
        let date = shift(inv.due_date, rng.range(-8, 12)).clamp(base, shift(base, 62));
        payments.push(Payment {
            id: format!("P-{:05}", payments.len()),
            amount_minor: amount,
            currency: inv.currency.clone(),
            value_date: date,
            reference: dense_reference(rng, numbers, pool),
            counterparty: DENSE_NAMES[name].to_owned(),
            scope: inv.scope.clone(),
        });
    };
    let number_of = |i: &Invoice| {
        i.number
            .trim_start_matches("INV-")
            .parse::<usize>()
            .unwrap_or_default()
    };
    for n in 0..n_invoices {
        let inv = &invoices[n];
        if inv.amount_minor <= 0 {
            continue;
        }
        let roll = rng.below(100);
        if roll < 5 {
            continue;
        } else if roll < 20 && inv.amount_minor >= 2 {
            let first = rng.range(1, inv.amount_minor - 1);
            push(&mut rng, n, first, &[number_of(inv)], pool);
            push(
                &mut rng,
                n,
                inv.amount_minor - first,
                &[number_of(inv)],
                pool,
            );
        } else if roll < 35 {
            let mut members = vec![n];
            for _ in 0..rng.below(3).max(1) {
                let j = rng.below(n_invoices);
                if key(&invoices[j]) == key(inv)
                    && invoices[j].amount_minor > 0
                    && !members.contains(&j)
                {
                    members.push(j);
                }
            }
            let total: i64 = members.iter().map(|&j| invoices[j].amount_minor).sum();
            let numbers: Vec<usize> = members.iter().map(|&j| number_of(&invoices[j])).collect();
            push(&mut rng, n, total, &numbers, pool);
        } else {
            let fee = if rng.chance(0.1) {
                rng.range(1, 250)
            } else {
                0
            };
            push(
                &mut rng,
                n,
                (inv.amount_minor - fee).max(1),
                &[number_of(inv)],
                pool,
            );
        }
    }
    for _ in 0..n_invoices / 30 {
        let anchor = rng.below(n_invoices);
        let amount = if rng.chance(0.5) {
            0
        } else {
            -*rng.pick(&DENSE_AMOUNTS)
        };
        push(&mut rng, anchor, amount, &[], pool);
    }
    for i in (1..payments.len()).rev() {
        payments.swap(i, rng.below(i + 1));
    }
    Dataset {
        invoices,
        payments,
        solution: Vec::new(),
    }
}
