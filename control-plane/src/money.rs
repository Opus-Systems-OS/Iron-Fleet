//! Whole US cents, carried as a string on the wire.
//!
//! Managed Agents budgets (`max_list_cost.amount`) and reported `usage.list_cost`
//! are integer strings of cents: `"500"` is $5.00. The API rejects decimals and
//! numbers. `Cents` is the only type that can occupy those fields, and the only
//! way to construct one is the validating parser, so no float — and no unchecked
//! integer — can reach a request body.
//!
//! `"0"` is valid: a freshly created session reports `list_cost.amount: "0"`.
//! A budget cap must be > 0 — that is enforced where caps are defined
//! (`registry::Policy`), not here.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Cents(u64);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CentsParseError {
    #[error("amount is empty")]
    Empty,
    #[error("amount must be whole cents as a string of digits, got {0:?}")]
    NotDigits(String),
    #[error("amount must not have leading zeros, got {0:?}")]
    LeadingZero(String),
    #[error("amount is too large")]
    Overflow,
}

impl Cents {
    pub fn get(self) -> u64 {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == 0
    }
}

impl FromStr for Cents {
    type Err = CentsParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(CentsParseError::Empty);
        }
        if !s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(CentsParseError::NotDigits(s.to_owned()));
        }
        if s != "0" && s.starts_with('0') {
            return Err(CentsParseError::LeadingZero(s.to_owned()));
        }
        s.parse::<u64>()
            .map(Cents)
            .map_err(|_| CentsParseError::Overflow)
    }
}

impl TryFrom<String> for Cents {
    type Error = CentsParseError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl From<Cents> for String {
    fn from(c: Cents) -> String {
        c.0.to_string()
    }
}

impl fmt::Display for Cents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_whole_cents_strings() {
        assert_eq!("500".parse::<Cents>().unwrap().get(), 500);
        assert_eq!("1".parse::<Cents>().unwrap().get(), 1);
        assert_eq!("1000".parse::<Cents>().unwrap().get(), 1000);
        // The API reports "0" on a fresh session; it must parse.
        assert!("0".parse::<Cents>().unwrap().is_zero());
    }

    #[test]
    fn rejects_everything_else() {
        assert_eq!("".parse::<Cents>(), Err(CentsParseError::Empty));
        assert_eq!(
            "00".parse::<Cents>(),
            Err(CentsParseError::LeadingZero("00".into()))
        );
        assert!(matches!(
            "050".parse::<Cents>(),
            Err(CentsParseError::LeadingZero(_))
        ));
        assert!(matches!(
            "5.00".parse::<Cents>(),
            Err(CentsParseError::NotDigits(_))
        ));
        assert!(matches!(
            "-5".parse::<Cents>(),
            Err(CentsParseError::NotDigits(_))
        ));
        assert!(matches!(
            "5 ".parse::<Cents>(),
            Err(CentsParseError::NotDigits(_))
        ));
        assert!(matches!(
            "$5".parse::<Cents>(),
            Err(CentsParseError::NotDigits(_))
        ));
        assert_eq!(
            "99999999999999999999999".parse::<Cents>(),
            Err(CentsParseError::Overflow)
        );
    }

    #[test]
    fn serde_uses_json_strings_only() {
        let c: Cents = serde_json::from_str("\"500\"").unwrap();
        assert_eq!(c.get(), 500);
        assert_eq!(serde_json::to_string(&c).unwrap(), "\"500\"");

        // A JSON number must never be accepted, even if it is a whole number.
        assert!(serde_json::from_str::<Cents>("500").is_err());
        assert!(serde_json::from_str::<Cents>("5.0").is_err());
        assert!(serde_json::from_str::<Cents>("\"5.00\"").is_err());
    }
}
