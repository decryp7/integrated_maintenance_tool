//! Pages that need the user: cookie consent, CAPTCHAs, "verify you are human", logins. Pure: the caller passes the
//! URL, the title and a bounded excerpt of the visible text.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Challenge {
    /// A cookie or consent wall (for example Google's consent page).
    Consent,
    /// A CAPTCHA or "verify you are human" check.
    HumanCheck,
}

const CONSENT_HOSTS: &[&str] = &["consent.google.", "consent.youtube."];
const HUMAN_URL: &[&str] = &["/sorry/", "captcha", "/challenge", "cdn-cgi/challenge"];
const HUMAN_TEXT: &[&str] = &[
    "verify you are human",
    "verify you're human",
    "are you a robot",
    "i'm not a robot",
    "unusual traffic",
    "not a robot",
    "complete the security check",
    "checking your browser",
    "press & hold",
    "captcha",
];
const CONSENT_TEXT: &[&str] = &["before you continue", "we use cookies", "accept all", "reject all"];

/// What kind of page this is, if it needs the user. Only the first 4000 characters of `text` are looked at.
pub fn detect(url: &str, title: &str, text: &str) -> Option<Challenge> {
    let url = url.to_ascii_lowercase();
    let head: String = format!("{title} {}", text.chars().take(4000).collect::<String>()).to_lowercase();
    if HUMAN_URL.iter().any(|u| url.contains(u)) || HUMAN_TEXT.iter().any(|t| head.contains(t)) {
        return Some(Challenge::HumanCheck);
    }
    let host = url.split("//").nth(1).unwrap_or("");
    if CONSENT_HOSTS.iter().any(|h| host.starts_with(h)) {
        return Some(Challenge::Consent);
    }
    // Consent text needs two signals so an article that mentions cookies is not a wall.
    if CONSENT_TEXT.iter().filter(|t| head.contains(*t)).count() >= 2 && text.chars().count() < 3000 {
        return Some(Challenge::Consent);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captcha_pages() {
        assert_eq!(detect("https://www.google.com/sorry/index?continue=x", "", ""), Some(Challenge::HumanCheck));
        assert_eq!(detect("https://x.test/", "Just a moment", "Verify you are human by completing the action below"), Some(Challenge::HumanCheck));
        assert_eq!(detect("https://x.test/", "", "Our systems have detected unusual traffic"), Some(Challenge::HumanCheck));
    }

    #[test]
    fn consent_pages() {
        assert_eq!(detect("https://consent.google.com/ml?x=1", "Before you continue", ""), Some(Challenge::Consent));
        assert_eq!(detect("https://a.test/", "", "We use cookies. Accept all  Reject all"), Some(Challenge::Consent));
    }

    #[test]
    fn ordinary_pages_are_not_challenges() {
        assert_eq!(detect("https://docs.test/a", "NullReferenceException", "How to fix a null reference. We use cookies."), None);
        let long = "word ".repeat(2000) + " accept all reject all we use cookies";
        assert_eq!(detect("https://docs.test/a", "t", &long), None);
    }
}
