//! Tab bookkeeping: every tab is either the human's ("Me") or owned by a
//! connected agent session. One tab per view is "active" (rendered + receiving
//! input); the top-left toggle switches which view is shown.

use std::rc::Rc;
use std::time::{Duration, Instant};

use euclid::Scale;
use servo::{
    OffscreenRenderingContext, Servo, WebView, WebViewBuilder, WebViewDelegate,
    WindowRenderingContext,
};
use url::Url;

/// How long an adopted popup keeps reporting as loading while waiting for the
/// navigation its opener asked for. Long enough to cover the round trip from
/// `window.open` to the first `LoadStatus::Started`, short enough that a
/// `window.open()` with no URL — which never navigates — settles promptly.
const ADOPTED_BLANK_GRACE: Duration = Duration::from_secs(2);

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

    /// The view a tab with this owner lives in.
    pub fn view(&self) -> ViewMode {
        if self.is_agent() { ViewMode::Agents } else { ViewMode::Me }
    }
}

pub struct Tab {
    pub id: u64,
    pub webview: WebView,
    /// Every tab renders into its own offscreen framebuffer, so painting or
    /// capturing one tab never disturbs another's pixels.
    pub rendering_context: Rc<OffscreenRenderingContext>,
    pub owner: TabOwner,
    pub crashed: bool,
    /// URL bar contents for this tab.
    pub location: String,
    /// True while the user is editing the URL bar, so page-driven URL updates
    /// don't clobber their typing.
    pub location_dirty: bool,
    /// Popups only. A `window.open` webview is handed over already "loaded" —
    /// its blank starting document is Complete — and only *then* navigates to
    /// the requested URL, so an agent that lists tabs in that window is told
    /// the tab is ready just before the page it asked for replaces it. Until
    /// this instant passes (or the real navigation starts, whichever comes
    /// first) the tab reports as loading. A `window.open()` with no URL never
    /// navigates, so this has to time out rather than wait forever.
    pub initial_blank_until: Option<Instant>,
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
        parent_context: &Rc<WindowRenderingContext>,
        size: winit::dpi::PhysicalSize<u32>,
        delegate: Rc<dyn WebViewDelegate>,
        hidpi_scale: f32,
        url: Url,
        owner: TabOwner,
    ) -> u64 {
        let rendering_context = Rc::new(parent_context.offscreen_context(size));
        let webview = WebViewBuilder::new(servo, rendering_context.clone())
            .url(url.clone())
            .hidpi_scale_factor(Scale::new(hidpi_scale))
            .delegate(delegate)
            .build();
        // A new tab becomes active in its own view, but opening an agent tab
        // never yanks the human out of the Me view — and never steals the
        // agent view from a tab that agent is already working in.
        let activate = !owner.is_agent() || self.active_agent.is_none();
        // Built with its URL, so there is no blank document to wait past.
        self.register(webview, rendering_context, owner, url.to_string(), activate, false)
    }

    /// Adopt an already-built webview as a tab. `activate` makes it the active
    /// tab of its own view — true for a popup whose opener was active, the way
    /// a popup fronts its window in every browser; false leaves it behind the
    /// tab the user (or agent) is looking at. `adopted` marks a webview handed
    /// over on a blank document (see [`Tab::initial_blank_until`]).
    pub fn register(
        &mut self,
        webview: WebView,
        rendering_context: Rc<OffscreenRenderingContext>,
        owner: TabOwner,
        location: String,
        activate: bool,
        adopted: bool,
    ) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.tabs.push(Tab {
            id,
            webview,
            rendering_context,
            owner,
            crashed: false,
            location,
            location_dirty: false,
            initial_blank_until: adopted.then(|| Instant::now() + ADOPTED_BLANK_GRACE),
        });

        // Either way the show/hide invariant is re-established, so the new
        // webview starts hidden rather than merely un-shown (set_active syncs
        // on its own).
        if activate {
            self.set_active(id);
        } else {
            self.sync_visibility();
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
        self.sync_visibility();
        true
    }

    /// Make `id` the active tab of the view it belongs to (does not switch
    /// the current view mode).
    pub fn set_active(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter().find(|t| t.id == id) else {
            return;
        };
        if tab.owner.is_agent() {
            self.active_agent = Some(id);
        } else {
            self.active_me = Some(id);
        }
        self.sync_visibility();
    }

    /// Invariant: exactly the displayed tab is shown (and focused); every
    /// other webview is hidden. This matters beyond bookkeeping — a webview's
    /// hidden→shown transition is what makes servo produce a fresh frame, and
    /// `paint()` without a fresh frame is a no-op that leaves the shared
    /// framebuffer stale (the "view switched but page didn't" bug).
    pub fn sync_visibility(&self) {
        let displayed_id = self.displayed().map(|tab| tab.id);
        for tab in &self.tabs {
            if Some(tab.id) == displayed_id {
                tab.webview.show();
                tab.webview.focus();
            } else {
                tab.webview.hide();
                tab.webview.blur();
            }
        }
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

    /// Cycle the active tab within the current view (Ctrl+Tab / Ctrl+Shift+Tab).
    pub fn cycle(&mut self, forward: bool) {
        let ids: Vec<u64> = match self.mode {
            ViewMode::Me => self.me_tabs().map(|t| t.id).collect(),
            ViewMode::Agents => self.agent_tabs().map(|t| t.id).collect(),
        };
        if ids.len() < 2 {
            return;
        }
        let current = self.active_id(self.mode);
        let position = current
            .and_then(|id| ids.iter().position(|&i| i == id))
            .unwrap_or(0);
        let next = if forward {
            (position + 1) % ids.len()
        } else {
            (position + ids.len() - 1) % ids.len()
        };
        self.set_active(ids[next]);
    }

    pub fn me_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| !t.owner.is_agent())
    }

    pub fn agent_tabs(&self) -> impl Iterator<Item = &Tab> {
        self.tabs.iter().filter(|t| t.owner.is_agent())
    }

}
