//! Tab bookkeeping: every tab is either the human's ("Me") or owned by a
//! connected agent session. One tab per view is "active" (rendered + receiving
//! input); the top-left toggle switches which view is shown.

use std::rc::Rc;

use euclid::Scale;
use servo::{Servo, WebView, WebViewBuilder, WebViewDelegate};
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TabOwner {
    Me,
    Agent { session_id: u64, client: String },
}

impl TabOwner {
    pub fn label(&self) -> String {
        match self {
            TabOwner::Me => "me".to_owned(),
            TabOwner::Agent { client, .. } => client.clone(),
        }
    }

    pub fn is_agent(&self) -> bool {
        matches!(self, TabOwner::Agent { .. })
    }
}

pub struct Tab {
    pub id: u64,
    pub webview: WebView,
    pub owner: TabOwner,
    pub crashed: bool,
    /// URL bar contents for this tab.
    pub location: String,
    /// True while the user is editing the URL bar, so page-driven URL updates
    /// don't clobber their typing.
    pub location_dirty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Me,
    Agents,
}

pub struct TabManager {
    tabs: Vec<Tab>,
    next_id: u64,
    pub mode: ViewMode,
    active_me: Option<u64>,
    active_agent: Option<u64>,
}

impl TabManager {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            next_id: 1,
            mode: ViewMode::Me,
            active_me: None,
            active_agent: None,
        }
    }

    pub fn open(
        &mut self,
        servo: &Servo,
        rendering_context: Rc<servo::OffscreenRenderingContext>,
        delegate: Rc<dyn WebViewDelegate>,
        hidpi_scale: f32,
        url: Url,
        owner: TabOwner,
    ) -> u64 {
        let webview = WebViewBuilder::new(servo, rendering_context)
            .url(url.clone())
            .hidpi_scale_factor(Scale::new(hidpi_scale))
            .delegate(delegate)
            .build();

        let id = self.next_id;
        self.next_id += 1;
        let is_agent = owner.is_agent();
        self.tabs.push(Tab {
            id,
            webview,
            owner,
            crashed: false,
            location: url.to_string(),
            location_dirty: false,
        });

        // Newly opened tabs become active within their own view, but opening
        // an agent tab never yanks the human out of the Me view.
        if is_agent {
            if self.active_agent.is_none() {
                self.set_active(id);
            }
        } else {
            self.set_active(id);
        }
        id
    }

    pub fn close(&mut self, id: u64) -> bool {
        let Some(index) = self.tabs.iter().position(|t| t.id == id) else {
            return false;
        };
        let tab = self.tabs.remove(index);
        drop(tab.webview);
        if self.active_me == Some(id) {
            self.active_me = self.tabs.iter().rev().find(|t| !t.owner.is_agent()).map(|t| t.id);
        }
        if self.active_agent == Some(id) {
            self.active_agent = self.tabs.iter().rev().find(|t| t.owner.is_agent()).map(|t| t.id);
        }
        true
    }

    /// Make `id` the active tab of the view it belongs to (does not switch
    /// the current view mode).
    pub fn set_active(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        let previous = if tab.owner.is_agent() {
            self.active_agent.replace(id)
        } else {
            self.active_me.replace(id)
        };
        if previous != Some(id) {
            if let Some(prev) = previous.and_then(|p| self.get(p)) {
                prev.webview.hide();
                prev.webview.blur();
            }
        }
        let tab = self.get(id).expect("checked above");
        tab.webview.show();
        tab.webview.focus();
    }

    pub fn get(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.id == id)
    }

    pub fn find_by_webview(&self, webview: &WebView) -> Option<u64> {
        self.tabs.iter().find(|t| t.webview == *webview).map(|t| t.id)
    }

    pub fn find_by_webview_mut(&mut self, webview: &WebView) -> Option<&mut Tab> {
        self.tabs.iter_mut().find(|t| t.webview == *webview)
    }

    /// The tab currently displayed and receiving input, given the view mode.
    pub fn displayed(&self) -> Option<&Tab> {
        let id = match self.mode {
            ViewMode::Me => self.active_me,
            ViewMode::Agents => self.active_agent,
        };
        id.and_then(|id| self.get(id))
    }

    pub fn displayed_mut(&mut self) -> Option<&mut Tab> {
        let id = match self.mode {
            ViewMode::Me => self.active_me,
            ViewMode::Agents => self.active_agent,
        };
        id.and_then(move |id| self.get_mut(id))
    }

    pub fn active_id(&self, mode: ViewMode) -> Option<u64> {
        match mode {
            ViewMode::Me => self.active_me,
            ViewMode::Agents => self.active_agent,
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter()
    }

    pub fn me_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| !t.owner.is_agent())
    }

    pub fn agent_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| t.owner.is_agent())
    }

}
