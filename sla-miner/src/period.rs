//! Parse a free-text period ("last 30 days", "past 2 months", "last week", "30d") into days.

pub const MIN_DAYS: i64 = 1;
pub const MAX_DAYS: i64 = 90;

fn word_number(w: &str) -> Option<i64> {
    Some(match w {
        "a" | "an" | "one" => 1,
        "two" => 2,
        "three" => 3,
        "four" => 4,
        "five" => 5,
        "six" => 6,
        "seven" => 7,
        "eight" => 8,
        "nine" => 9,
        "ten" => 10,
        "eleven" => 11,
        "twelve" => 12,
        "fourteen" => 14,
        "thirty" => 30,
        "sixty" => 60,
        "ninety" => 90,
        _ => return None,
    })
}

fn unit_days(u: &str) -> Option<i64> {
    Some(match u {
        "d" | "day" | "days" => 1,
        "w" | "wk" | "wks" | "week" | "weeks" => 7,
        "fortnight" | "fortnights" => 14,
        "m" | "mo" | "mos" | "month" | "months" => 30,
        "q" | "quarter" | "quarters" => 90,
        "y" | "yr" | "yrs" | "year" | "years" => 365,
        _ => return None,
    })
}

/// Returns the number of days (clamped to 1..=90), or `None` if no number or unit
/// can be found. A bare number means days; a bare unit means one of it ("last week").
pub fn parse_period(p: &str) -> Option<i64> {
    let p = p.trim().to_lowercase();
    if matches!(p.as_str(), "today" | "yesterday" | "24h" | "24 hours") {
        return Some(1);
    }
    let mut num: Option<i64> = None;
    let mut unit: Option<i64> = None;
    for tok in p.split(|c: char| !c.is_ascii_alphanumeric()).filter(|t| !t.is_empty()) {
        let digits: String = tok.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            if num.is_none() {
                num = digits.parse().ok();
            }
            let suffix = &tok[digits.len()..];
            if !suffix.is_empty() && unit.is_none() {
                unit = unit_days(suffix);
            }
        } else if let Some(n) = word_number(tok) {
            num.get_or_insert(n);
        } else if let Some(u) = unit_days(tok) {
            unit.get_or_insert(u);
        } else if tok == "hours" || tok == "hour" || tok == "h" {
            unit.get_or_insert(0); // sub-day: rounds up to the 1-day minimum
        }
    }
    let days = match (num, unit) {
        (None, None) => return None,
        (Some(n), None) => n,
        (n, Some(u)) => n.unwrap_or(1).saturating_mul(u),
    };
    Some(days.clamp(MIN_DAYS, MAX_DAYS))
}

/// `days` param (integer as string) wins over `period`; neither -> 30.
pub fn resolve_days(days: Option<&str>, period: Option<&str>) -> Result<i64, String> {
    if let Some(d) = days.map(str::trim).filter(|d| !d.is_empty()) {
        return d
            .parse::<i64>()
            .map(|d| d.clamp(MIN_DAYS, MAX_DAYS))
            .or_else(|_| parse_period(d).ok_or(()))
            .map_err(|_| format!("could not parse days '{d}'"));
    }
    match period.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(30),
        Some(p) => parse_period(p).ok_or_else(|| {
            format!("could not parse period '{p}'. Examples: 'last 30 days', 'last week', 'past 2 months', '30d'")
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_phrases() {
        assert_eq!(parse_period("last 30 days"), Some(30));
        assert_eq!(parse_period("Last 7 Days"), Some(7));
        assert_eq!(parse_period("last week"), Some(7));
        assert_eq!(parse_period("past 2 weeks"), Some(14));
        assert_eq!(parse_period("past 2 months"), Some(60));
        assert_eq!(parse_period("last month"), Some(30));
        assert_eq!(parse_period("last quarter"), Some(90));
        assert_eq!(parse_period("a fortnight"), Some(14));
        assert_eq!(parse_period("two weeks"), Some(14));
        assert_eq!(parse_period("today"), Some(1));
        assert_eq!(parse_period("last 24 hours"), Some(1));
    }

    #[test]
    fn compact_forms() {
        assert_eq!(parse_period("30d"), Some(30));
        assert_eq!(parse_period("2w"), Some(14));
        assert_eq!(parse_period("3mo"), Some(90));
        assert_eq!(parse_period("45"), Some(45));
    }

    #[test]
    fn clamps() {
        assert_eq!(parse_period("last year"), Some(90));
        assert_eq!(parse_period("last 6 months"), Some(90));
        assert_eq!(parse_period("0 days"), Some(1));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_period("whenever"), None);
        assert_eq!(parse_period(""), None);
    }

    #[test]
    fn resolve_precedence() {
        assert_eq!(resolve_days(Some("14"), Some("last 30 days")), Ok(14));
        assert_eq!(resolve_days(None, Some("last week")), Ok(7));
        assert_eq!(resolve_days(None, None), Ok(30));
        assert_eq!(resolve_days(Some("500"), None), Ok(90));
        assert!(resolve_days(None, Some("soon")).is_err());
    }
}
