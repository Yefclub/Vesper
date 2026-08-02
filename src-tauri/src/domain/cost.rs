//! What a meeting cost, in nano-USD.
//!
//! OpenRouter reports the price of every call in its `usage.cost` field, so
//! nothing here derives a price from token counts — the provider already did the
//! arithmetic, including whatever discount or free tier applies to that key.
//!
//! The unit is an integer billionth of a dollar. A meeting is dozens of
//! transcription chunks at a fraction of a cent each, and adding those as `f64`
//! loses money in the last digits for no reason: `i64` nano-USD reaches about
//! 9.2 billion dollars, and any real total stays well inside the 2^53 a JSON
//! number can carry exactly to the WebView.

/// One US dollar.
pub const NANO_PER_USD: f64 = 1_000_000_000.0;

/// The provider's float, converted once, at the edge.
///
/// Everything downstream is integer addition. Negative and non-finite are
/// treated as "no charge reported" rather than propagated: a provider that
/// answers `NaN` should not be able to poison a meeting's total.
pub fn usd_to_nano(usd: f64) -> Option<i64> {
    if !usd.is_finite() || !(0.0..=MAX_CALL_USD).contains(&usd) {
        return None;
    }
    Some((usd * NANO_PER_USD).round() as i64)
}

/// The most one call can plausibly cost, above which the number is a bug rather
/// than a charge.
///
/// A transcription chunk is a fraction of a cent and the most expensive
/// completion in the catalogue does not reach a dollar for a meeting-sized
/// prompt. Accepting anything up to `i64::MAX` meant a malformed `9000000000.0`
/// became a nine-billion-dollar meeting, past what JSON carries exactly and
/// impossible to correct without editing the database.
pub const MAX_CALL_USD: f64 = 100.0;

/// How a cost reads to someone deciding whether to keep using a cloud model.
///
/// Three bands, because one format cannot serve both ends: `$0.00` for a real
/// charge reads as free, and `$0.000000512` for a whole meeting reads as noise.
/// Below a hundredth of a cent the number stops being useful and the sentence
/// becomes the point.
///
/// Exactly zero prints `$0.00` — a `:free` model really did cost nothing, and
/// that is worth saying. A meeting that never called OpenRouter has no cost at
/// all and must not reach this function; the UI renders nothing for it.
pub fn format_cost(nano: i64) -> String {
    let usd = nano as f64 / NANO_PER_USD;
    if nano == 0 {
        return "$0.00".to_string();
    }
    if usd >= 1.0 {
        return format!("${usd:.2}");
    }
    if usd >= 0.0001 {
        return format!("${usd:.4}");
    }
    "< $0.0001".to_string()
}

/// The per-million-token price of a model, as the picker shows it.
///
/// OpenRouter quotes prices per token as decimal strings, which are unreadable
/// at that scale — `0.00000015` says nothing, `$0.15/M` is a number someone can
/// compare. A model quoting `0` is free and says so.
///
/// `None` when the field is missing or unparseable rather than a zero: claiming
/// a model is free because its price did not arrive is the one wrong answer.
pub fn format_price_per_mtok(prompt: Option<&str>, completion: Option<&str>) -> Option<String> {
    let parse = |v: Option<&str>| v.and_then(|s| s.trim().parse::<f64>().ok());
    let (p, c) = (parse(prompt), parse(completion));
    let (p, c) = match (p, c) {
        (None, None) => return None,
        (a, b) => (a.unwrap_or(0.0), b.unwrap_or(0.0)),
    };
    if p == 0.0 && c == 0.0 {
        return Some("free".to_string());
    }
    Some(format!(
        "${}/M in · ${}/M out",
        trim_price(p * 1_000_000.0),
        trim_price(c * 1_000_000.0)
    ))
}

/// Two decimals, without the trailing zeros that make a price list ragged.
fn trim_price(v: f64) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() {
        "0".to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reported_price_becomes_an_exact_integer() {
        assert_eq!(usd_to_nano(0.000508), Some(508_000));
        assert_eq!(usd_to_nano(1.0), Some(1_000_000_000));
        assert_eq!(usd_to_nano(0.0), Some(0));
    }

    /// A provider answering nonsense must not be able to poison a total.
    #[test]
    fn nonsense_is_not_a_charge() {
        assert_eq!(usd_to_nano(f64::NAN), None);
        assert_eq!(usd_to_nano(f64::INFINITY), None);
        assert_eq!(usd_to_nano(-0.5), None);
        assert_eq!(usd_to_nano(1e30), None);
        // Finite, positive and still not a price anyone was charged.
        assert_eq!(usd_to_nano(9_000_000_000.0), None);
        assert_eq!(usd_to_nano(MAX_CALL_USD), Some(100_000_000_000));
    }

    /// The reason for integers: forty chunks at a fraction of a cent have to add
    /// up to what was actually spent.
    #[test]
    fn many_small_charges_add_up_exactly() {
        let one = usd_to_nano(0.000_123).unwrap();
        let total: i64 = (0..40).map(|_| one).sum();
        assert_eq!(total, 4_920_000);
        assert_eq!(format_cost(total), "$0.0049");
    }

    #[test]
    fn each_band_says_something_useful() {
        assert_eq!(format_cost(12_340_000_000), "$12.34");
        assert_eq!(format_cost(500_000), "$0.0005");
        // Real money, far below a hundredth of a cent. `$0.00` would read as free.
        assert_eq!(format_cost(12), "< $0.0001");
        // And a free model really is free.
        assert_eq!(format_cost(0), "$0.00");
    }

    #[test]
    fn prices_are_quoted_per_million_tokens() {
        assert_eq!(
            format_price_per_mtok(Some("0.00000015"), Some("0.0000006")),
            Some("$0.15/M in · $0.6/M out".to_string())
        );
        assert_eq!(
            format_price_per_mtok(Some("0"), Some("0")),
            Some("free".to_string())
        );
    }

    /// A missing price is unknown, never free — the one wrong answer here.
    #[test]
    fn an_absent_price_is_not_zero() {
        assert_eq!(format_price_per_mtok(None, None), None);
        assert_eq!(format_price_per_mtok(Some("not a number"), None), None);
    }
}
