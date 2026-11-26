//! The browser page: the address it reads, the links it goes on with, and
//! the pages it walks back and forward through.

use super::*;

impl App {
    /// The page the browser read, as the runtime tells it.
    pub fn browser_read(&mut self, page: skyward_net::page::Page) {
        self.state.browsing = Browsing::Read(page);
        self.refresh();
    }

    /// The page that could not be read, and why.
    pub fn browser_failed(&mut self, why: String) {
        self.state.browsing = Browsing::Failed(why);
        self.refresh();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::testing::*;
    use skyward_net::page::{Link, Page};

    fn page(url: &str, title: &str, links: &[(&str, &str)]) -> Page {
        Page {
            url: url.into(),
            title: title.into(),
            text: vec!["words".into()],
            links: links.iter().map(|(text, url)| Link { text: (*text).into(), url: (*url).into() }).collect(),
        }
    }

    #[test]
    fn the_browser_is_reached_from_the_tools_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Browser));
        assert_eq!(app.screen(), Screen::Browser);
    }

    #[test]
    fn an_address_typed_is_read_from_the_web() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Browser, W, H, PPD);
        for c in "example.com".chars() {
            app.act(Action::Type(Cap::Char(c)), 0);
        }
        assert_eq!(tap(&mut app, &f, &i, Action::BrowseGo), vec![Effect::Browse("https://example.com".into())]);
        assert!(app.browsing(), "the page says it is opening");
        assert_eq!(app.state.trail, ["https://example.com"], "the page read is the first of the pages seen");
    }

    #[test]
    fn an_address_with_nothing_in_it_is_not_read() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Browser, W, H, PPD);
        assert!(tap(&mut app, &f, &i, Action::BrowseGo).is_empty());
        assert!(!app.browsing(), "nothing is under way");
    }

    #[test]
    fn return_reads_the_page_and_then_stops_the_reading() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Browser, W, H, PPD);
        app.state.browse_url = "https://example.com/a".into();
        assert_eq!(app.act(Action::Type(Cap::Enter), 0), vec![Effect::Browse("https://example.com/a".into())]);
        assert_eq!(app.act(Action::Type(Cap::Enter), 0), vec![Effect::StopBrowse], "the same key stops the reading it started");
    }

    #[test]
    fn a_link_goes_on_and_back_comes_back_to_the_page_before() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Browser, W, H, PPD);
        app.state.browse_url = "https://example.com/".into();
        app.act(Action::BrowseGo, 0);
        app.browser_read(page("https://example.com/", "First", &[("Next", "https://example.com/next")]));
        assert_eq!(tap(&mut app, &f, &i, Action::BrowseLink(0)), vec![Effect::Browse("https://example.com/next".into())]);
        app.browser_read(page("https://example.com/next", "Second", &[]));
        assert_eq!(app.state.trail, ["https://example.com/", "https://example.com/next"]);
        assert_eq!(tap(&mut app, &f, &i, Action::BrowseBack), vec![Effect::Browse("https://example.com/".into())]);
        app.browser_read(page("https://example.com/", "First", &[("Next", "https://example.com/next")]));
        assert_eq!(tap(&mut app, &f, &i, Action::BrowseForward), vec![Effect::Browse("https://example.com/next".into())]);
        // A link taken from a page gone back to gives up what was ahead.
        tap(&mut app, &f, &i, Action::BrowseBack);
        app.browser_read(page("https://example.com/", "First", &[("Elsewhere", "https://example.com/other")]));
        tap(&mut app, &f, &i, Action::BrowseLink(0));
        assert_eq!(app.state.trail, ["https://example.com/", "https://example.com/other"]);
        assert_eq!(app.state.trail_at, 1);
    }

    #[test]
    fn the_page_shown_is_offered_to_the_download_page() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Browser, W, H, PPD);
        app.browser_read(page("https://example.com/fw.zip", "Firmware", &[]));
        tap(&mut app, &f, &i, Action::BrowseDownload);
        assert_eq!((app.screen(), app.state.fetch_url.as_str()), (Screen::Fetch, "https://example.com/fw.zip"));
    }

    #[test]
    fn a_page_that_could_not_be_read_says_why_and_the_page_can_be_left() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        tap(&mut app, &f, &i, Action::Open(Screen::Browser));
        app.state.browsing = Browsing::Loading;
        app.state.browse_url = "https://example.com/".into();
        app.browser_failed("the server answered 404".into());
        app.frame(&f, &i);
        let frame = format!("{:?}", app.frame(&f, &i).hits.iter().map(|h| h.action).collect::<Vec<_>>());
        assert!(frame.contains("BrowseGo"), "the page can be read again: {frame}");
        tap_back(&mut app, &f, &i, 10);
        assert_eq!(app.screen(), Screen::Advanced);
    }

    #[test]
    fn the_buttons_clear_empties_the_address_and_leaves_the_search_alone() {
        let mut app = App::new(Screen::Browser, W, H, PPD);
        app.state.browse_url = "https://example.com/a".into();
        app.state.search = "files".into();
        app.act(Action::ClearSearch, 0);
        assert_eq!((app.state.browse_url.as_str(), app.state.search.as_str()), ("", "files"));
        let mut app = App::new(Screen::Fetch, W, H, PPD);
        app.state.fetch_url = "https://example.com/fw.zip".into();
        app.state.search = "files".into();
        app.act(Action::ClearSearch, 0);
        assert_eq!((app.state.fetch_url.as_str(), app.state.search.as_str()), ("", "files"));
    }

    #[test]
    fn every_tool_of_the_tools_page_is_still_there_once_grouped() {
        let (f, i) = (fonts(), info());
        let mut app = App::new(Screen::Advanced, W, H, PPD);
        let actions: Vec<String> = app.frame(&f, &i).hits.iter().map(|h| format!("{:?}", h.action)).collect();
        for tool in [
            "Open(Decrypt)",
            "Open(Mount)",
            "Open(Filesystems)",
            "FilesOtg",
            "Open(Relabel)",
            "Open(Browser)",
            "Open(Serve)",
            "Open(Fetch)",
            "Open(Terminal)",
            "Open(Apps)",
            "Open(Avb)",
            "Open(Update)",
            "Open(Logs)",
        ] {
            assert!(actions.iter().any(|a| a == tool), "{tool} is offered on the grouped page: {actions:?}");
        }
        assert_eq!(actions.len(), 13, "no tool is offered twice");
    }
}