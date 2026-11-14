//! The graph every engine loads, and the answers it must give.
//!
//! People who know each other (directed, about ten edges each, most of them
//! to people with nearby numbers and one in five to a few hubs) and who
//! bought products. Everything is a function of the vertex numbers, so the
//! harness holds the whole graph and computes each expected answer itself.

use std::collections::{HashSet, VecDeque};

pub const COUNTRIES: [&str; 20] = [
    "FR", "DE", "ES", "IT", "PT", "NL", "BE", "CH", "AT", "SE", "NO", "DK", "FI", "PL", "CZ",
    "IE", "GB", "US", "CA", "JP",
];

fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn h(a: u64, b: u64) -> u64 {
    mix(mix(a) ^ b.wrapping_mul(0xD6E8_FEB8_6659_FD93))
}

pub fn country(n: u32) -> &'static str {
    COUNTRIES[(h(n as u64, 1) % 20) as usize]
}

pub fn age(n: u32) -> i64 {
    18 + (h(n as u64, 4) % 60) as i64
}

/// The year an edge dates from, 2000 to 2024.
pub fn since(from: u32, to: u32) -> i64 {
    2000 + (h(((from as u64) << 32) | to as u64, 5) % 25) as i64
}

pub fn weight(from: u32, to: u32) -> i64 {
    1 + (h(((from as u64) << 32) | to as u64, 6) % 9) as i64
}

pub struct Graph {
    pub people: u32,
    pub products: u32,
    /// Whom each person knows, sorted, without duplicates or themselves.
    pub knows: Vec<Vec<u32>>,
    /// What each person bought, sorted.
    pub bought: Vec<Vec<u32>>,
}

impl Graph {
    pub fn new(people: u32, products: u32) -> Self {
        let mut knows = Vec::with_capacity(people as usize);
        let mut bought = Vec::with_capacity(people as usize);
        for i in 0..people {
            let degree = 2 + h(i as u64, 2) % 17;
            let mut to: Vec<u32> = (0..degree)
                .map(|j| {
                    let r = h(i as u64, 100 + j);
                    if r % 5 == 0 {
                        // A hub: low numbers are known by many.
                        let u = (r >> 11) as f64 / (1u64 << 53) as f64;
                        (people as f64 * u * u * u) as u32
                    } else {
                        ((i as u64 + 1 + (r >> 8) % 2000) % people as u64) as u32
                    }
                })
                .filter(|t| *t != i && *t < people)
                .collect();
            to.sort_unstable();
            to.dedup();
            knows.push(to);
            let mut items: Vec<u32> = (0..h(i as u64, 3) % 4)
                .map(|j| (h(i as u64, 200 + j) % products as u64) as u32)
                .collect();
            items.sort_unstable();
            items.dedup();
            bought.push(items);
        }
        Graph {
            people,
            products,
            knows,
            bought,
        }
    }

    pub fn edges(&self) -> u64 {
        self.knows.iter().map(|k| k.len() as u64).sum()
    }

    pub fn purchases(&self) -> u64 {
        self.bought.iter().map(|k| k.len() as u64).sum()
    }

    /// The people within `depth` hops of `start`, other than `start`.
    pub fn within(&self, start: u32, depth: u32) -> HashSet<u32> {
        let mut seen: HashSet<u32> = HashSet::from([start]);
        let mut queue = VecDeque::from([(start, 0)]);
        while let Some((at, d)) = queue.pop_front() {
            if d == depth {
                continue;
            }
            for &next in &self.knows[at as usize] {
                if seen.insert(next) {
                    queue.push_back((next, d + 1));
                }
            }
        }
        seen.remove(&start);
        seen
    }

    /// People, other than `start`, two hops away through a second edge
    /// dating from `year` or later, and living in `country`.
    pub fn filtered(&self, start: u32, year: i64, in_country: &str) -> HashSet<u32> {
        let mut out = HashSet::new();
        for &mid in &self.knows[start as usize] {
            for &end in &self.knows[mid as usize] {
                if end != start && since(mid, end) >= year && country(end) == in_country {
                    out.insert(end);
                }
            }
        }
        out
    }

    /// People within two hops of `start` who bought `product`.
    pub fn fof_bought(&self, start: u32, product: u32) -> HashSet<u32> {
        self.within(start, 2)
            .into_iter()
            .filter(|p| self.bought[*p as usize].binary_search(&product).is_ok())
            .collect()
    }

    /// Someone exactly `hops` away from `start`, following edges forward.
    pub fn someone_at(&self, start: u32, hops: u32, pick: u64) -> Option<u32> {
        let mut seen: HashSet<u32> = HashSet::from([start]);
        let mut frontier = vec![start];
        for _ in 0..hops {
            let mut next = Vec::new();
            for at in frontier {
                for &to in &self.knows[at as usize] {
                    if seen.insert(to) {
                        next.push(to);
                    }
                }
            }
            frontier = next;
        }
        frontier.sort_unstable();
        (!frontier.is_empty()).then(|| frontier[(pick % frontier.len() as u64) as usize])
    }
}

/// `(count, sum of the numbers)`: what every engine returns for a set of
/// people, and what the harness compares.
pub fn tally(set: &HashSet<u32>) -> (u64, u64) {
    (set.len() as u64, set.iter().map(|n| *n as u64).sum())
}

/// A deterministic stream of numbers for choosing parameters.
pub fn pick(seed: u64, i: u64) -> u64 {
    h(seed, i)
}
