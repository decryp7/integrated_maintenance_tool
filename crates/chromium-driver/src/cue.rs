//! The text that marks a browser as AI-driven, shown outside the page (the status chip and, where the OS allows it,
//! the window title). Nothing is injected into a page: that could disturb page scripts and layout and leak into
//! extracted text or screenshots.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Reading,
    Searching,
    WaitingForYou,
}

impl Activity {
    pub fn word(self) -> &'static str {
        match self {
            Activity::Reading => "reading",
            Activity::Searching => "searching",
            Activity::WaitingForYou => "waiting for you",
        }
    }
}

/// `"<product> AI — <activity>: <host>"`; `product` is the application's product name.
pub fn window_title(product: &str, activity: Activity, host: &str) -> String {
    format!("{product} AI — {}: {host}", activity.word())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn title_format() {
        assert_eq!(
            window_title("IMT", Activity::Reading, "example.com"),
            "IMT AI — reading: example.com"
        );
        assert_eq!(
            window_title("IMT", Activity::WaitingForYou, "g.test"),
            "IMT AI — waiting for you: g.test"
        );
        assert_eq!(window_title("X", Activity::Searching, "h"), "X AI — searching: h");
    }
}
