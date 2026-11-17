//! An exact, order-independent sum of `f64` values, for `SUM` and `AVG`.
//!
//! `COLLECT … AGGREGATE s = SUM(x)` used to add the values one by one into
//! an `f64`. That rounds after every addition, so the answer depended on the
//! order the documents were read in — and a scan split into partitions read
//! on several threads, then merged, reads them in another order. Rather than
//! keep the scan sequential, the sum is made exact: every value is added
//! without rounding into a wide fixed-point integer (units of 2^-1074, the
//! smallest subnormal, so every finite double is an integer there), and the
//! total is rounded to the nearest double once, ties to even. The result is
//! the correctly rounded sum of the inputs: the same for any order and any
//! partitioning, and never less accurate than the running `f64` (it differs
//! from it only in the last bits, where the running sum had drifted).
//!
//! The integer is held in 32-bit chunks, each in an `i64`, so an addition
//! touches three chunks and never carries: a chunk moves by less than 2^32
//! per value, and the carries are propagated every 2^30 values (and before a
//! merge or the final rounding). Adding a value costs a few integer
//! operations, about what the `f64` addition and its bookkeeping did. Only
//! the chunks the values reach are kept — a window, usually four or five
//! chunks for values of similar magnitude — so a group's `SUM` holds a few
//! dozen bytes, not the 544 the whole double range would take.

/// Bits of a double's position range: a finite double is `m · 2^p · 2^-1074`
/// with `m < 2^53` and `p ≤ 2045`, so the largest needs bits up to 2097.
/// Two more chunks hold the carries of 2^64 additions.
const CHUNKS: usize = 68;
const CHUNK_BITS: u32 = 32;
const LOW_MASK: i64 = (1 << CHUNK_BITS) - 1;
/// Additions between carry propagations: each moves a chunk by < 2^32, so
/// 2^30 of them keep every chunk well inside an `i64`.
const ADDS_BEFORE_CARRY: u32 = 1 << 30;
/// Chunks a window keeps from the lowest an addition reaches: the three it
/// touches, and one above for the carries.
const SPAN: usize = 4;

#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ExactSum {
    /// Chunk `i` of the integer is `chunks[i - lo]`; those outside the
    /// window are 0. The window always ends at least one chunk above the
    /// highest chunk an addition touched, so that chunk takes the carries
    /// (and the sign).
    lo: usize,
    chunks: Vec<i64>,
    /// Additions since the carries were last propagated.
    pending: u32,
    /// A non-finite input (none can come from JSON; kept for safety): the
    /// sum is then not a number.
    non_finite: bool,
    /// The sum of the integers added with [`Self::add_number`], while every
    /// value added was one: `SUM` of integers is an integer, as in SQL, and
    /// not `15.0` for 5 + 10. `not_integers` once a float was added or the
    /// total left an `i128`.
    integers: i128,
    not_integers: bool,
}

impl std::fmt::Debug for ExactSum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ExactSum({})", self.value())
    }
}

impl ExactSum {
    /// Add `x`, exactly. The sum is a float from then on.
    #[inline]
    pub(crate) fn add(&mut self, x: f64) {
        self.not_integers = true;
        self.accumulate(x);
    }

    /// Add a JSON number: an integer keeps the sum an integer.
    #[inline]
    pub(crate) fn add_number(&mut self, n: &serde_json::Number) {
        let integer = n
            .as_i64()
            .map(i128::from)
            .or_else(|| n.as_u64().map(i128::from));
        match integer.and_then(|i| self.integers.checked_add(i)) {
            Some(total) if !self.not_integers => self.integers = total,
            _ => self.not_integers = true,
        }
        if let Some(x) = n.as_f64() {
            self.accumulate(x);
        }
    }

    /// [`Self::add_number`] of the integer `i`.
    #[inline]
    pub(crate) fn add_i64(&mut self, i: i64) {
        match self.integers.checked_add(i128::from(i)) {
            Some(total) if !self.not_integers => self.integers = total,
            _ => self.not_integers = true,
        }
        self.accumulate(i as f64);
    }

    /// [`Self::add`] of each of `xs`. Their sum in floating point, and
    /// the total of what its additions rounded away, are two floats that
    /// add up to their exact sum for as long as adding the errors rounds
    /// nothing itself: those two are then added in their place.
    pub(crate) fn add_slice(&mut self, xs: &[f64]) {
        if xs.is_empty() {
            return;
        }
        let (mut sum, mut lost, mut exact) = (0.0f64, 0.0f64, true);
        for &x in xs {
            let next = sum + x;
            let virtual_x = next - sum;
            let error = (sum - (next - virtual_x)) + (x - virtual_x);
            sum = next;
            let kept = lost + error;
            let virtual_e = kept - lost;
            exact &= (lost - (kept - virtual_e)) + (error - virtual_e) == 0.0;
            lost = kept;
        }
        if exact && sum.is_finite() && lost.is_finite() {
            self.add(sum);
            if lost != 0.0 {
                self.add(lost);
            }
        } else {
            for &x in xs {
                self.add(x);
            }
        }
    }

    /// Add `x · x`, exactly: the product of two doubles is the sum of two,
    /// its rounding and what that rounding left (exact unless the product
    /// leaves the range of a double).
    #[inline]
    pub(crate) fn add_square(&mut self, x: f64) {
        self.add_product(x, x);
    }

    /// Add `a · b`, as [`Self::add_square`] adds a square.
    #[inline]
    pub(crate) fn add_product(&mut self, a: f64, b: f64) {
        let high = a * b;
        self.add(high);
        self.add(a.mul_add(b, -high));
    }

    /// The sum as up to four doubles that add up to it, the largest first
    /// (zeros after the last): each is what is left, rounded. Exact when
    /// the sum fits their 212 bits, else its leading bits. `None` when the
    /// sum is not a finite double.
    pub(crate) fn parts(&self) -> Option<[f64; 4]> {
        let mut rest = self.clone();
        let mut parts = [0.0; 4];
        for part in &mut parts {
            let value = rest.value();
            if !value.is_finite() {
                return None;
            }
            if value == 0.0 {
                break;
            }
            *part = value;
            rest.add(-value);
        }
        Some(parts)
    }

    /// The sum as an integer, when every value added was one (`0` for no
    /// value at all) and it fits a JSON integer; `None` for a float sum.
    pub(crate) fn integer(&self) -> Option<serde_json::Value> {
        if self.not_integers || self.non_finite {
            return None;
        }
        i64::try_from(self.integers)
            .map(serde_json::Value::from)
            .or_else(|_| u64::try_from(self.integers).map(serde_json::Value::from))
            .ok()
    }

    #[inline]
    fn accumulate(&mut self, x: f64) {
        let bits = x.to_bits();
        let exp = ((bits >> 52) & 0x7ff) as u32;
        let mut mantissa = bits & ((1u64 << 52) - 1);
        if exp == 0x7ff {
            self.non_finite = true;
            return;
        }
        // x = mantissa · 2^(shift - 1074).
        let shift = if exp == 0 {
            if mantissa == 0 {
                return;
            }
            0
        } else {
            mantissa |= 1u64 << 52;
            exp - 1
        };
        let index = (shift / CHUNK_BITS) as usize;
        let offset = shift % CHUNK_BITS;
        // mantissa << offset is at most 84 bits: split it in three chunks.
        let low = ((mantissa << offset) & LOW_MASK as u64) as i64;
        let high = mantissa >> (CHUNK_BITS - offset);
        let mid = (high & LOW_MASK as u64) as i64;
        let top = (high >> CHUNK_BITS) as i64;
        if index < self.lo || index + SPAN > self.lo + self.chunks.len() {
            self.widen(index, index + SPAN);
        }
        let at = index - self.lo;
        let chunks = &mut self.chunks[at..at + 3];
        if bits >> 63 == 0 {
            chunks[0] += low;
            chunks[1] += mid;
            chunks[2] += top;
        } else {
            chunks[0] -= low;
            chunks[1] -= mid;
            chunks[2] -= top;
        }
        self.pending += 1;
        if self.pending == ADDS_BEFORE_CARRY {
            self.carry();
        }
    }

    /// Make the window cover chunks `[lo, hi)` too.
    #[cold]
    fn widen(&mut self, lo: usize, hi: usize) {
        if self.chunks.is_empty() {
            self.lo = lo;
            self.chunks = vec![0; hi - lo];
            return;
        }
        if lo < self.lo {
            let more = self.lo - lo;
            self.chunks.splice(0..0, std::iter::repeat_n(0, more));
            self.lo = lo;
        }
        let end = self.lo + self.chunks.len();
        if hi > end {
            self.chunks.resize(self.chunks.len() + (hi - end), 0);
        }
    }

    /// Add everything `other` holds: the sum of both sets of values, exactly.
    pub(crate) fn merge(&mut self, other: &ExactSum) {
        self.non_finite |= other.non_finite;
        match self.integers.checked_add(other.integers) {
            Some(total) if !self.not_integers && !other.not_integers => self.integers = total,
            _ => self.not_integers = true,
        }
        if other.chunks.is_empty() {
            return;
        }
        // A sum whose carries are propagated ([`Self::settle`]) holds
        // chunks below 2^32, and a top chunk of about the number of values
        // added: it goes in as an addition does, with no copy and no carry
        // until enough of them have piled up.
        if other.pending == 0 {
            if self.pending >= ADDS_BEFORE_CARRY {
                self.carry();
            }
            self.widen(other.lo, other.lo + other.chunks.len());
            let at = other.lo - self.lo;
            for (a, b) in self.chunks[at..].iter_mut().zip(&other.chunks) {
                *a += b;
            }
            self.pending += 1;
            return;
        }
        self.carry();
        let mut other = other.clone();
        other.carry();
        // The union of both windows. Its top chunk takes the carries of the
        // sum: after `carry` each top is at most about the number of values
        // added, so the two add up without overflow.
        self.widen(other.lo, other.lo + other.chunks.len());
        let at = other.lo - self.lo;
        for (a, b) in self.chunks[at..].iter_mut().zip(&other.chunks) {
            *a += b;
        }
        self.carry();
    }

    /// Propagate the carries, so that merging this sum into another costs
    /// what one addition does. For a sum that is kept.
    pub(crate) fn settle(&mut self) {
        self.carry();
    }

    /// Propagate the carries: every chunk of the window but the last in
    /// `[0, 2^32)`, the last holding the sign.
    fn carry(&mut self) {
        for i in 0..self.chunks.len().saturating_sub(1) {
            let carry = self.chunks[i] >> CHUNK_BITS;
            self.chunks[i] &= LOW_MASK;
            self.chunks[i + 1] += carry;
        }
        self.pending = 0;
    }

    /// The sum, rounded to the nearest double (ties to even); infinite when
    /// its magnitude rounds past the largest double, NaN after a non-finite
    /// input.
    pub(crate) fn value(&self) -> f64 {
        if self.non_finite {
            return f64::NAN;
        }
        // The window in place, and two chunks above it for what its top
        // chunk carries (it holds up to 2^63) and for the sign: the chunks
        // below and above are zero and stay so.
        let mut chunks = [0i64; CHUNKS];
        chunks[self.lo..self.lo + self.chunks.len()].copy_from_slice(&self.chunks);
        let (lo, hi) = (self.lo, (self.lo + self.chunks.len() + 2).min(CHUNKS));
        let carry = |chunks: &mut [i64; CHUNKS]| {
            for i in lo..hi - 1 {
                let c = chunks[i] >> CHUNK_BITS;
                chunks[i] &= LOW_MASK;
                chunks[i + 1] += c;
            }
        };
        carry(&mut chunks);
        let negative = chunks[hi - 1] < 0;
        if negative {
            for c in chunks[lo..hi].iter_mut() {
                *c = -*c;
            }
            carry(&mut chunks);
        }
        // Now a non-negative integer, every chunk in [0, 2^32).
        let Some(top) = chunks[..hi].iter().rposition(|&c| c != 0) else {
            return 0.0;
        };
        let top_bit = top as u32 * CHUNK_BITS + (63 - (chunks[top] as u64).leading_zeros());
        let bit = |pos: u32| -> u64 {
            (chunks[(pos / CHUNK_BITS) as usize] as u64 >> (pos % CHUNK_BITS)) & 1
        };
        let magnitude = if top_bit < 53 {
            // Below 2^53 units: the integer itself is the double's bit
            // pattern (a subnormal, or the smallest normal binade).
            (chunks[0] as u64) | ((chunks[1] as u64) << CHUNK_BITS)
        } else {
            // The 53 bits from top_bit down, then round on the rest.
            let low_bit = top_bit - 52;
            let mut mantissa = 0u64;
            for pos in (low_bit..=top_bit).rev() {
                mantissa = (mantissa << 1) | bit(pos);
            }
            let round = bit(low_bit - 1) == 1;
            let sticky = {
                let below = low_bit - 1;
                let whole = (below / CHUNK_BITS) as usize;
                chunks[lo.min(whole)..whole].iter().any(|&c| c != 0)
                    || (chunks[whole] as u64) & ((1u64 << (below % CHUNK_BITS)) - 1) != 0
            };
            let mut exponent = top_bit - 51;
            if round && (sticky || mantissa & 1 == 1) {
                mantissa += 1;
                if mantissa == 1 << 53 {
                    mantissa >>= 1;
                    exponent += 1;
                }
            }
            if exponent >= 0x7ff {
                f64::INFINITY.to_bits()
            } else {
                ((exponent as u64) << 52) | (mantissa & ((1u64 << 52) - 1))
            }
        };
        let value = f64::from_bits(magnitude);
        if negative {
            -value
        } else {
            value
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sum(values: &[f64]) -> f64 {
        let mut s = ExactSum::default();
        for &v in values {
            s.add(v);
        }
        s.value()
    }

    /// A small deterministic generator, so the tests need no dependency.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 11
        }
    }

    #[test]
    fn exact_where_the_running_sum_drifts() {
        assert_eq!(sum(&[0.1; 10]), 1.0);
        assert_ne!([0.1f64; 10].iter().sum::<f64>(), 1.0);
        assert_eq!(sum(&[1e308, 1e308, -1e308]), 1e308);
        assert_eq!(sum(&[1e308, -1e308, 1e308]), 1e308);
        assert_eq!(sum(&[1.0, 1e-30, -1.0]), 1e-30);
        assert_eq!(sum(&[1e100, 1.0, -1e100]), 1.0);
        assert_eq!(sum(&[]), 0.0);
        assert_eq!(sum(&[-0.0]).to_bits(), 0.0f64.to_bits());
        assert_eq!(sum(&[3.5, -3.5]), 0.0);
        assert_eq!(sum(&[-2.5, 1.0]), -1.5);
    }

    #[test]
    fn overflow_and_the_extremes() {
        assert_eq!(sum(&[f64::MAX, f64::MAX]), f64::INFINITY);
        assert_eq!(sum(&[-f64::MAX, -f64::MAX]), f64::NEG_INFINITY);
        assert_eq!(sum(&[f64::MAX, f64::MAX, -f64::MAX]), f64::MAX);
        assert_eq!(sum(&[1e308; 6]), f64::INFINITY);
        let tiny = f64::from_bits(1);
        assert_eq!(sum(&[tiny, tiny, tiny]), f64::from_bits(3));
        assert_eq!(
            sum(&[f64::MIN_POSITIVE, -tiny]),
            f64::from_bits((1 << 52) - 1)
        );
        assert_eq!(sum(&[f64::MAX]), f64::MAX);
        assert_eq!(sum(&[-f64::MIN_POSITIVE]), -f64::MIN_POSITIVE);
        assert!(sum(&[f64::NAN]).is_nan());
    }

    #[test]
    fn rounds_once_to_nearest_even() {
        let two53 = 9007199254740992.0;
        // 2^53 + 1 is a tie: to even, 2^53.
        assert_eq!(sum(&[two53, 1.0]), two53);
        // 2^53 + 3 is a tie: to even, 2^53 + 4.
        assert_eq!(sum(&[two53, 3.0]), two53 + 4.0);
        // Just above the tie rounds up.
        assert_eq!(sum(&[two53, 1.0, f64::from_bits(1)]), two53 + 2.0);
        // Rounding up into the next binade.
        let below = f64::from_bits(two53.to_bits() - 1); // 2^53 - 1
        assert_eq!(sum(&[below, 0.5]), two53);
    }

    /// Against an exact reference: values k · 2^-20 with |k| < 2^40, whose
    /// sum is an exact `i128`; Rust's int-to-float conversion rounds to
    /// nearest, ties to even, and the scaling is exact.
    #[test]
    fn matches_an_exact_reference_in_any_order() {
        let mut rng = Lcg(42);
        let scale = 2f64.powi(-20);
        for round in 0..200 {
            let len = 1 + (rng.next() % 300) as usize;
            let ks: Vec<i64> = (0..len)
                .map(|_| (rng.next() % (1 << 40)) as i64 - (1 << 39))
                .collect();
            let values: Vec<f64> = ks.iter().map(|&k| k as f64 * scale).collect();
            let exact = ks.iter().map(|&k| k as i128).sum::<i128>() as f64 * scale;
            assert_eq!(sum(&values), exact, "round {round}");
            let mut reversed = values.clone();
            reversed.reverse();
            assert_eq!(sum(&reversed), exact);
            // Split anywhere, summed apart, merged: the same double.
            let cut = (rng.next() as usize) % (len + 1);
            let (mut a, mut b) = (ExactSum::default(), ExactSum::default());
            values[..cut].iter().for_each(|&v| a.add(v));
            values[cut..].iter().for_each(|&v| b.add(v));
            a.merge(&b);
            assert_eq!(a.value(), exact);
        }
    }

    /// Wide magnitudes, in many orders and partitions: always one answer.
    #[test]
    fn order_and_partitioning_never_change_the_result() {
        let mut rng = Lcg(7);
        for _ in 0..100 {
            let values: Vec<f64> = (0..200)
                .map(|_| {
                    let mantissa = (rng.next() % (1 << 52)) as f64 / (1u64 << 52) as f64 + 1.0;
                    let exp = (rng.next() % 120) as i32 - 60;
                    let sign = if rng.next().is_multiple_of(2) {
                        1.0
                    } else {
                        -1.0
                    };
                    sign * mantissa * 2f64.powi(exp)
                })
                .collect();
            let reference = sum(&values);
            for parts in [2usize, 3, 7, 16] {
                let mut merged = ExactSum::default();
                for chunk in values.chunks(values.len().div_ceil(parts)).rev() {
                    let mut s = ExactSum::default();
                    chunk.iter().for_each(|&v| s.add(v));
                    merged.merge(&s);
                }
                assert_eq!(merged.value().to_bits(), reference.to_bits());
            }
        }
    }

    #[test]
    fn integers_add_as_integers() {
        let ints: Vec<f64> = (1..=100_000).map(|i| i as f64).collect();
        assert_eq!(sum(&ints), 5_000_050_000.0);
        assert_eq!(sum(&[-5.0, 3.0, 2.0]), 0.0);
    }

    #[test]
    fn the_window_stays_small_for_similar_values() {
        let mut s = ExactSum::default();
        for i in 0..10_000 {
            s.add((i % 997) as f64 / 100.0);
        }
        assert!(s.chunks.len() <= 5, "{}", s.chunks.len());
        let mut wide = ExactSum::default();
        wide.add(1e300);
        wide.add(1e-300);
        wide.add(-1e300);
        assert_eq!(wide.value(), 1e-300);
        let mut empty = ExactSum::default();
        empty.merge(&ExactSum::default());
        assert_eq!(empty.value(), 0.0);
        empty.merge(&wide);
        assert_eq!(empty.value(), 1e-300);
    }

    /// Merging partial sums of the largest values many times over never
    /// outgrows the double range's chunks.
    #[test]
    fn merges_of_extremes_stay_in_range() {
        let mut total = ExactSum::default();
        for _ in 0..50 {
            let mut part = ExactSum::default();
            part.add(f64::MAX);
            part.add(-f64::MAX);
            part.add(1e308);
            total.merge(&part);
        }
        assert_eq!(total.value(), f64::INFINITY);
        let mut back = total.clone();
        for _ in 0..50 {
            back.add(-1e308);
        }
        assert_eq!(back.value(), 0.0);
        assert!(back.chunks.len() <= CHUNKS);
    }

    #[test]
    fn carries_survive_many_additions() {
        let mut s = ExactSum::default();
        for _ in 0..3_000_000 {
            s.add(0.75);
            s.add(-0.25);
        }
        assert_eq!(s.value(), 1_500_000.0);
    }
    /// Settled sums (what a column block keeps of its rows) merged into
    /// one, and slices added with [`ExactSum::add_slice`], give what adding
    /// every value gives: the same float, and an integer when all were.
    #[test]
    fn settled_parts_and_slices_add_up_to_the_whole() {
        let mut rng = Lcg(7);
        for round in 0..40 {
            let values: Vec<f64> = (0..3_000)
                .map(|_| {
                    let n = rng.next();
                    match round % 4 {
                        0 => (n % 1000) as f64,
                        1 => (n % 1000) as f64 / 3.0,
                        2 => f64::from_bits(n.rotate_left(11)).clamp(-1e300, 1e300),
                        _ => ((n % 2001) as f64 - 1000.0) * 1e-7,
                    }
                })
                .filter(|v| v.is_finite())
                .collect();
            let mut whole = ExactSum::default();
            let mut ints = ExactSum::default();
            for &v in &values {
                whole.add(v);
                ints.add_i64(v as i64);
            }
            let (mut parts, mut int_parts, mut slices) = (
                ExactSum::default(),
                ExactSum::default(),
                ExactSum::default(),
            );
            for chunk in values.chunks(97) {
                let (mut part, mut int_part) = (ExactSum::default(), ExactSum::default());
                for &v in chunk {
                    part.add(v);
                    int_part.add_i64(v as i64);
                }
                part.settle();
                int_part.settle();
                parts.merge(&part);
                int_parts.merge(&int_part);
                slices.add_slice(chunk);
            }
            assert_eq!(
                parts.value().to_bits(),
                whole.value().to_bits(),
                "round {round}"
            );
            assert_eq!(
                slices.value().to_bits(),
                whole.value().to_bits(),
                "round {round}"
            );
            assert_eq!(parts.integer(), None);
            assert_eq!(int_parts.integer(), ints.integer(), "round {round}");
            // (An integer sum too large for a JSON integer is `None` both ways.)
            assert_eq!(ints.integer().is_some(), round % 4 != 2, "round {round}");
            assert_eq!(
                int_parts.value().to_bits(),
                ints.value().to_bits(),
                "round {round}"
            );
        }
    }
}
