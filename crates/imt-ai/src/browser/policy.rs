//! What agent mode may do without asking. Pure: an action, the page it acts on, the sites this chat may
//! visit and what the leak guard found go in; a verdict comes out.
//!
//! - Going to a new site needs approval; "allow this site for this chat" covers later visits.
//! - Typing always needs approval with the exact text; submitting a form or pressing a button too.
//! - Ordinary link clicks within allowed sites need none, and neither do reading, scrolling and going back.
//! - Data from the loaded logs headed into a URL or a field always needs approval.
//! - The AI never goes to this machine or a private network, never types into a password field, and stops
//!   on a CAPTCHA, a login or a payment page, handing control back to the user.

use super::leak::LeakReport;
use super::snapshot::{DomSnapshot, Element, ElementKind, HandOff};
use super::url::{Site, WebUrl};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ScrollDirection {
    Up,
    Down,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAction {
    Navigate(WebUrl),
    /// A search with the user's engine: `url` is the engine's search page for `query`.
    Search {
        query: String,
        url: WebUrl,
    },
    Click(Element),
    Type {
        element: Element,
        text: String,
        submit: bool,
    },
    Scroll(ScrollDirection),
    Back,
    Read,
}

impl AgentAction {
    /// A short, stable name for logs and the UI.
    pub fn kind(&self) -> &'static str {
        match self {
            AgentAction::Navigate(_) => "navigate",
            AgentAction::Search { .. } => "search",
            AgentAction::Click(_) => "click",
            AgentAction::Type { .. } => "type",
            AgentAction::Scroll(_) => "scroll",
            AgentAction::Back => "back",
            AgentAction::Read => "read",
        }
    }

    /// The address the action would load, if it loads one.
    pub fn target(&self) -> Option<&str> {
        match self {
            AgentAction::Navigate(u) | AgentAction::Search { url: u, .. } => Some(u.as_str()),
            AgentAction::Click(e) => e.href.as_deref(),
            _ => None,
        }
    }
}

/// Why the user is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AskKind {
    /// Load a site this chat has not been allowed to visit.
    NewSite,
    /// Data from the loaded logs would leave in the address.
    LeavingData,
    TypeText,
    SubmitForm,
    PressButton,
}

/// What an approval card shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub kind: AskKind,
    /// The site the action loads or acts on.
    pub site: Option<Site>,
    /// The full address it would load.
    pub url: Option<String>,
    /// The element it acts on ("search box \"Search\"").
    pub element: Option<String>,
    /// The exact text it would type.
    pub text: Option<String>,
    /// Typing ends with pressing Enter / submitting the form.
    pub submit: bool,
    /// "Allow this site for this chat" is offered (never while data from the logs is in the address).
    pub can_allow_site: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Proceed,
    Ask(Ask),
    /// Not done, and the model is told why.
    Refuse(String),
    /// The agent stops and the user takes over.
    HandBack(HandOff),
}

/// What the chat has allowed agent mode to do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Grants {
    sites: BTreeSet<Site>,
}

impl Grants {
    pub fn allows(&self, site: &Site) -> bool {
        self.sites.contains(site)
    }

    pub fn allow(&mut self, site: Site) {
        self.sites.insert(site);
    }

    pub fn sites(&self) -> impl Iterator<Item = &Site> {
        self.sites.iter()
    }
}

/// The leak guard's findings for the parts of an action that leave the machine.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Leaks {
    pub url: LeakReport,
    pub text: LeakReport,
}

fn element_label(e: &Element) -> String {
    format!("{} \"{}\"", e.kind.describe(), e.name)
}

fn site_of(page: Option<&DomSnapshot>) -> Option<Site> {
    page.and_then(|p| WebUrl::parse(&p.url).ok()).map(|u| u.site())
}

fn load(url: &WebUrl, grants: &Grants, leaks: &Leaks) -> Verdict {
    if url.is_private() {
        return Verdict::Refuse(format!(
            "{} is on this computer or a private network; agent mode never goes there",
            url.host()
        ));
    }
    let site = url.site();
    let leaving = leaks.url.from_logs();
    if grants.allows(&site) && !leaving {
        return Verdict::Proceed;
    }
    Verdict::Ask(Ask {
        kind: if leaving { AskKind::LeavingData } else { AskKind::NewSite },
        site: Some(site),
        url: Some(url.as_str().to_string()),
        element: None,
        text: None,
        submit: false,
        can_allow_site: !leaving,
    })
}

/// Decides whether `action` on `page` (the latest snapshot) runs, needs the user's approval or stops.
pub fn judge(action: &AgentAction, page: Option<&DomSnapshot>, grants: &Grants, leaks: &Leaks) -> Verdict {
    if !matches!(action, AgentAction::Read | AgentAction::Scroll(_) | AgentAction::Back) {
        if let Some(h) = page.and_then(|p| p.hand_off) {
            return Verdict::HandBack(h);
        }
    }
    match action {
        AgentAction::Read | AgentAction::Scroll(_) | AgentAction::Back => Verdict::Proceed,
        AgentAction::Navigate(url) | AgentAction::Search { url, .. } => load(url, grants, leaks),
        AgentAction::Click(e) => {
            let here = site_of(page);
            match (&e.kind, e.href.as_deref().and_then(|h| WebUrl::parse(h).ok())) {
                (ElementKind::Link, Some(target)) => load(&target, grants, leaks),
                (ElementKind::Password, _) => Verdict::HandBack(HandOff::Login),
                (kind, _) => Verdict::Ask(Ask {
                    kind: if *kind == ElementKind::Submit || (e.in_form && *kind == ElementKind::Button) {
                        AskKind::SubmitForm
                    } else {
                        AskKind::PressButton
                    },
                    site: here,
                    url: None,
                    element: Some(element_label(e)),
                    text: None,
                    submit: false,
                    can_allow_site: false,
                }),
            }
        }
        AgentAction::Type { element, text, submit } => {
            if element.kind == ElementKind::Password {
                return Verdict::HandBack(HandOff::Login);
            }
            if !element.kind.is_text_input() {
                return Verdict::Refuse(format!("[{}] is not a text field; pick a text box from the snapshot", element.r));
            }
            Verdict::Ask(Ask {
                kind: AskKind::TypeText,
                site: site_of(page),
                url: None,
                element: Some(element_label(element)),
                text: Some(text.clone()),
                submit: *submit,
                can_allow_site: false,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::leak::{LeakKind, Segment};
    use super::*;

    fn url(s: &str) -> WebUrl {
        WebUrl::parse(s).unwrap()
    }

    fn el(r: u32, kind: ElementKind, href: Option<&str>, in_form: bool) -> Element {
        Element {
            r,
            kind,
            name: format!("e{r}"),
            href: href.map(str::to_string),
            in_form,
            in_view: true,
            tag: "x".into(),
        }
    }

    fn page(u: &str, hand_off: Option<HandOff>) -> DomSnapshot {
        DomSnapshot {
            url: u.into(),
            title: "t".into(),
            elements: vec![],
            text: String::new(),
            hand_off,
        }
    }

    fn leaking() -> Leaks {
        Leaks {
            url: LeakReport {
                segments: vec![Segment {
                    text: "WEB01".into(),
                    leak: Some(LeakKind::LogValue),
                }],
            },
            text: LeakReport::default(),
        }
    }

    fn ask_kind(v: Verdict) -> AskKind {
        match v {
            Verdict::Ask(a) => a.kind,
            other => panic!("expected a question, got {other:?}"),
        }
    }

    #[test]
    fn new_sites_need_approval_and_allowed_ones_do_not() {
        let mut g = Grants::default();
        let docs = AgentAction::Navigate(url("https://learn.microsoft.com/a"));
        let v = judge(&docs, None, &g, &Leaks::default());
        let Verdict::Ask(a) = v else { panic!() };
        assert_eq!((a.kind, a.can_allow_site), (AskKind::NewSite, true));
        assert_eq!(a.site.as_ref().unwrap().as_str(), "learn.microsoft.com");
        assert_eq!(a.url.as_deref(), Some("https://learn.microsoft.com/a"));
        g.allow(a.site.unwrap());
        assert_eq!(judge(&docs, None, &g, &Leaks::default()), Verdict::Proceed);
        assert_eq!(g.sites().count(), 1);
        // Data from the logs in the address: always asked, and not for the whole site.
        let v = judge(&docs, None, &g, &leaking());
        let Verdict::Ask(a) = v else { panic!() };
        assert_eq!((a.kind, a.can_allow_site), (AskKind::LeavingData, false));
        let search = AgentAction::Search {
            query: "x".into(),
            url: url("https://duckduckgo.com/?q=x"),
        };
        assert_eq!(ask_kind(judge(&search, None, &g, &Leaks::default())), AskKind::NewSite);
        assert!(matches!(
            judge(
                &AgentAction::Navigate(url("http://192.168.1.10/admin")),
                None,
                &g,
                &Leaks::default()
            ),
            Verdict::Refuse(_)
        ));
    }

    #[test]
    fn links_within_allowed_sites_are_followed_and_buttons_ask() {
        let mut g = Grants::default();
        g.allow(url("https://e.org/").site());
        let here = page("https://e.org/a", None);
        let same = AgentAction::Click(el(1, ElementKind::Link, Some("https://www.e.org/b"), false));
        assert_eq!(judge(&same, Some(&here), &g, &Leaks::default()), Verdict::Proceed);
        let away = AgentAction::Click(el(2, ElementKind::Link, Some("https://other.example/"), false));
        assert_eq!(ask_kind(judge(&away, Some(&here), &g, &Leaks::default())), AskKind::NewSite);
        let js_link = AgentAction::Click(el(3, ElementKind::Link, None, false));
        assert_eq!(ask_kind(judge(&js_link, Some(&here), &g, &Leaks::default())), AskKind::PressButton);
        let submit = AgentAction::Click(el(4, ElementKind::Submit, None, true));
        assert_eq!(ask_kind(judge(&submit, Some(&here), &g, &Leaks::default())), AskKind::SubmitForm);
        let form_button = AgentAction::Click(el(5, ElementKind::Button, None, true));
        assert_eq!(
            ask_kind(judge(&form_button, Some(&here), &g, &Leaks::default())),
            AskKind::SubmitForm
        );
        let Verdict::Ask(a) = judge(
            &AgentAction::Click(el(6, ElementKind::Button, None, false)),
            Some(&here),
            &g,
            &Leaks::default(),
        ) else {
            panic!()
        };
        assert_eq!((a.kind, a.element.as_deref()), (AskKind::PressButton, Some("button \"e6\"")));
        assert_eq!(a.site.unwrap().as_str(), "e.org");
        for ok in [AgentAction::Read, AgentAction::Back, AgentAction::Scroll(ScrollDirection::Down)] {
            assert_eq!(judge(&ok, Some(&here), &g, &Leaks::default()), Verdict::Proceed);
        }
    }

    #[test]
    fn typing_always_asks_with_the_exact_text_and_never_into_passwords() {
        let g = Grants::default();
        let here = page("https://e.org/", None);
        let t = AgentAction::Type {
            element: el(1, ElementKind::SearchBox, None, true),
            text: "TimeoutException".into(),
            submit: true,
        };
        let Verdict::Ask(a) = judge(&t, Some(&here), &g, &Leaks::default()) else {
            panic!()
        };
        assert_eq!(
            (a.kind, a.text.as_deref(), a.submit),
            (AskKind::TypeText, Some("TimeoutException"), true)
        );
        let pw = AgentAction::Type {
            element: el(2, ElementKind::Password, None, true),
            text: "x".into(),
            submit: false,
        };
        assert_eq!(judge(&pw, Some(&here), &g, &Leaks::default()), Verdict::HandBack(HandOff::Login));
        assert_eq!(
            judge(
                &AgentAction::Click(el(2, ElementKind::Password, None, true)),
                Some(&here),
                &g,
                &Leaks::default()
            ),
            Verdict::HandBack(HandOff::Login)
        );
        let not_text = AgentAction::Type {
            element: el(3, ElementKind::Button, None, false),
            text: "x".into(),
            submit: false,
        };
        assert!(matches!(judge(&not_text, Some(&here), &g, &Leaks::default()), Verdict::Refuse(_)));
    }

    #[test]
    fn captchas_logins_and_payments_stop_everything_but_reading() {
        let mut g = Grants::default();
        g.allow(url("https://e.org/").site());
        for h in [HandOff::Captcha, HandOff::Login, HandOff::Payment] {
            let p = page("https://e.org/", Some(h));
            let click = AgentAction::Click(el(1, ElementKind::Link, Some("https://e.org/b"), false));
            assert_eq!(judge(&click, Some(&p), &g, &Leaks::default()), Verdict::HandBack(h));
            assert_eq!(judge(&AgentAction::Read, Some(&p), &g, &Leaks::default()), Verdict::Proceed);
        }
    }

    #[test]
    fn actions_name_their_kind_and_target() {
        let u = url("https://e.org/");
        assert_eq!(AgentAction::Navigate(u.clone()).target(), Some("https://e.org/"));
        assert_eq!(
            AgentAction::Click(el(1, ElementKind::Link, Some("https://e.org/x"), false)).target(),
            Some("https://e.org/x")
        );
        assert_eq!(AgentAction::Back.target(), None);
        let kinds: Vec<&str> = [
            AgentAction::Navigate(u.clone()),
            AgentAction::Search { query: "q".into(), url: u },
            AgentAction::Click(el(1, ElementKind::Button, None, false)),
            AgentAction::Type {
                element: el(1, ElementKind::TextBox, None, false),
                text: String::new(),
                submit: false,
            },
            AgentAction::Scroll(ScrollDirection::Up),
            AgentAction::Back,
            AgentAction::Read,
        ]
        .iter()
        .map(AgentAction::kind)
        .collect();
        assert_eq!(kinds, ["navigate", "search", "click", "type", "scroll", "back", "read"]);
    }
}
