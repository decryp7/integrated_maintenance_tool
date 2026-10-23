//! The built-in detectors of secrets and personal data.
//!
//! Every pattern is a constant covered by tests. A pattern with a group
//! named `value` replaces only that group (`password=<password>` keeps the
//! key); otherwise the whole match is replaced.

use serde::{Deserialize, Serialize};

/// A built-in detector. The serde names are the profile's `[redact] detectors`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Detector {
    BearerToken,
    Jwt,
    ApiKey,
    ConnectionString,
    Password,
    UserPath,
    Email,
    CreditCard,
    Ipv4,
    Ipv6,
    Guid,
    Phone,
    Hostname,
}

impl Detector {
    /// Every detector, in the order they are applied: secrets first (a token
    /// may contain anything), then the more specific personal data before the
    /// looser patterns (an e-mail address contains a host name).
    pub const ALL: [Detector; 13] = [
        Detector::BearerToken,
        Detector::Jwt,
        Detector::ApiKey,
        Detector::ConnectionString,
        Detector::Password,
        Detector::UserPath,
        Detector::Email,
        Detector::CreditCard,
        Detector::Ipv4,
        Detector::Ipv6,
        Detector::Guid,
        Detector::Phone,
        Detector::Hostname,
    ];

    /// The profile name (and the rule name in hit counts).
    pub fn name(self) -> &'static str {
        match self {
            Detector::BearerToken => "bearer-token",
            Detector::Jwt => "jwt",
            Detector::ApiKey => "api-key",
            Detector::ConnectionString => "connection-string",
            Detector::Password => "password",
            Detector::UserPath => "user-path",
            Detector::Email => "email",
            Detector::CreditCard => "credit-card",
            Detector::Ipv4 => "ipv4",
            Detector::Ipv6 => "ipv6",
            Detector::Guid => "guid",
            Detector::Phone => "phone",
            Detector::Hostname => "hostname",
        }
    }

    /// The word in a mask (`<email>`) or hash (`<email:1a2b3c4d5e>`).
    pub fn label(self) -> &'static str {
        match self {
            Detector::BearerToken | Detector::Jwt => "token",
            Detector::ApiKey | Detector::ConnectionString => "secret",
            Detector::Password => "password",
            Detector::UserPath => "user",
            Detector::Email => "email",
            Detector::CreditCard => "card",
            Detector::Ipv4 => "ipv4",
            Detector::Ipv6 => "ipv6",
            Detector::Guid => "guid",
            Detector::Phone => "phone",
            Detector::Hostname => "host",
        }
    }

    /// One line for the settings UI.
    pub fn description(self) -> &'static str {
        match self {
            Detector::BearerToken => "Authorization: Bearer tokens",
            Detector::Jwt => "JSON Web Tokens (eyJ…)",
            Detector::ApiKey => "api_key / token / secret = value",
            Detector::ConnectionString => "AccountKey, SharedAccessKey and similar connection-string secrets",
            Detector::Password => "password / pwd = value",
            Detector::UserPath => "user names in Windows and Unix home folders",
            Detector::Email => "e-mail addresses",
            Detector::CreditCard => "card numbers (Luhn checked)",
            Detector::Ipv4 => "IPv4 addresses",
            Detector::Ipv6 => "IPv6 addresses",
            Detector::Guid => "GUIDs / UUIDs",
            Detector::Phone => "phone numbers (+country code, (area) or 555-123-4567)",
            Detector::Hostname => "fully qualified lower-case host names (a.b.example)",
        }
    }

    pub(crate) fn pattern(self) -> &'static str {
        match self {
            Detector::BearerToken => r"(?i)\bBearer\s+(?P<value>[A-Za-z0-9._~+/=-]{8,})",
            Detector::Jwt => r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]*",
            Detector::ApiKey => {
                r#"(?i)(?:api[_-]?key|access[_-]?token|auth[_-]?token|client[_-]?secret|secret|token)["']?\s*[=:]\s*["']?(?P<value>[A-Za-z0-9._~+/=-]{8,})"#
            }
            Detector::ConnectionString => {
                r#"(?i)(?:AccountKey|SharedAccessKey|SharedAccessSignature|User\s*Password)\s*=\s*(?P<value>[^;\s"']+)"#
            }
            Detector::Password => r#"(?i)(?:password|passwd|pwd)["']?\s*[=:]\s*["']?(?P<value>[^\s;,&"']+)"#,
            Detector::UserPath => {
                r#"(?i)(?:\b[A-Z]:\\(?:Users|Documents and Settings)\\|(?:^|[^\w/.])/(?:home|Users)/)(?P<value>[^\\/\s:*?"<>|]+)"#
            }
            Detector::Email => r"\b[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}\b",
            Detector::CreditCard => r"\b\d(?:[ -]?\d){12,18}\b",
            Detector::Ipv4 => r"\b(?:(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\.){3}(?:25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)\b",
            Detector::Ipv6 => r"(?i)\b(?:[0-9a-f]{1,4}:){7}[0-9a-f]{1,4}\b|\b(?:[0-9a-f]{1,4}:){1,6}(?::[0-9a-f]{1,4}){1,6}\b|\B::1\b",
            Detector::Guid => r"\b[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}\b",
            Detector::Phone => {
                r"\+\d{1,3}[ .-]?(?:\(\d{1,4}\)[ .-]?)?\d{1,4}(?:[ .-]?\d{2,4}){1,4}\b|\(\d{3}\) ?\d{3}[ .-]\d{4}\b|\b\d{3}[.-]\d{3}[.-]\d{4}\b"
            }
            Detector::Hostname => {
                r"\b(?:[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?\.){2,}(?:com|net|org|edu|gov|mil|int|io|biz|info|dev|app|cloud|local|lan|corp|internal|intra|home|[a-z]{2})\b"
            }
        }
    }

    /// Extra check on a match the pattern alone cannot make.
    pub(crate) fn accepts(self, matched: &str) -> bool {
        match self {
            Detector::CreditCard => luhn_ok(matched),
            Detector::Phone => {
                let digits = matched.chars().filter(char::is_ascii_digit).count();
                (8..=15).contains(&digits)
            }
            _ => true,
        }
    }
}

/// Luhn checksum over the digits of `s`; 13 to 19 digits.
pub fn luhn_ok(s: &str) -> bool {
    let digits: Vec<u32> = s.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, d)| {
            if i % 2 == 1 {
                let x = d * 2;
                if x > 9 {
                    x - 9
                } else {
                    x
                }
            } else {
                *d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}
