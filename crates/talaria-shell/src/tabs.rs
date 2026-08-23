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
    /// How many remote viewers are holding this tab shown right now.
    ///
    /// **What it prevents.** [`TabManager::sync_visibility`] runs on every
    /// tab-set change and hides every tab that is not the displayed one.
    /// Without this counter a tab a viewer is watching would be hidden the
    /// next time the local human opened a tab, switched view or closed one —
    /// and the viewer's frames would simply stop, with nothing erroring
    /// anywhere. Servo needs a shown webview to answer a hit test at all, so
    /// the same absence takes remote input down with the pixels.
    ///
    /// **A count and not a flag**, because two viewers may hold one tab and
    /// the first of them to detach must not release it out from under the
    /// second.
    ///
    /// **Held means shown and deliberately *not* focused.** Focus belongs to
    /// the tab the local human is driving; a viewer attaching must change
    /// nothing on this window, and taking focus would be the first half of
    /// letting a remote party steer what somebody sitting here is typing at.
    pub held_for_view: usize,
    /// Set when the load currently in flight on this tab was started by an
    /// agent rather than by the human — today, by a `Command::Evaluate`
    /// script that can navigate the tab by assigning `location.href`.
    ///
    /// Provenance, not permission. It exists because the human's browsing
    /// history has to filter on who *caused* a load, not on who *owns* the
    /// tab: an agent can act on one of the human's own tabs, and the page
    /// that lands is not somewhere the human went. Cleared by the history
    /// drain as it skips the row, so the human's next navigation on the same
    /// tab is recorded normally.
    pub load_started_by_agent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    Me,
    Agents,
}

/// What [`TabManager::sync_visibility`] does with one tab.
///
/// Split out from the loop that applies it because every `Tab` owns a live
/// `WebView` and a real tab table therefore cannot exist in a unit test, while
/// the *decision* — which of three states a tab is in, given whether it is
/// displayed and how many viewers hold it — needs no engine at all. The three
/// engine calls are then two lines each, and their `and` cannot drift from the
/// table this enum makes assertable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    /// The tab the local human is looking at: shown **and** focused.
    DisplayedAndFocused,
    /// Held shown for at least one remote viewer, and deliberately not
    /// focused — see [`Tab::held_for_view`].
    HeldForViewing,
    /// Neither displayed nor held: hidden and blurred, as before.
    Hidden,
}

/// The visibility one tab should be in.
///
/// Displayed wins over held, and it wins rather than merely coming first: a
/// viewer attached to the tab the human happens to be looking at must not
/// downgrade it out of focus, which is the one way this table could have let a
/// remote party change something local.
pub fn visibility_of(displayed: bool, held_for_view: usize) -> Visibility {
    match (displayed, held_for_view) {
        (true, _) => Visibility::DisplayedAndFocused,
        (false, 0) => Visibility::Hidden,
        (false, _) => Visibility::HeldForViewing,
    }
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

    /// Every argument past `size` is an engine handle owned by `Shared`, and
    /// there is exactly one caller (`Shared::open_tab`). Grouping them into a
    /// parameter struct would move the same list one line up at that one call
    /// site without making anything clearer, so the arity is accepted.
    #[allow(clippy::too_many_arguments)]
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
            // A tab is born held by nobody: a lease is taken by an attach and
            // never inherited from the tab that opened this one.
            held_for_view: 0,
            // A tab starts out carrying the human's own first load: `open`
            // is reached from the address bar and from `open_for_user`, and
            // an agent-owned tab is filtered out on its owner anyway.
            load_started_by_agent: false,
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

    /// Invariant: the displayed tab is shown and focused, a tab **held for a
    /// remote viewer** is shown and *not* focused, and every other webview is
    /// hidden. This matters beyond bookkeeping — a webview's hidden→shown
    /// transition is what makes servo produce a fresh frame, and `paint()`
    /// without a fresh frame is a no-op that leaves the shared framebuffer
    /// stale (the "view switched but page didn't" bug).
    ///
    /// The held arm is the one that is invisible in a diff, so it is written
    /// down: this function is what runs whenever the local human opens,
    /// closes, switches or cycles a tab, and before [`Tab::held_for_view`]
    /// existed every one of those actions hid a tab a viewer was watching.
    /// The interaction runs the other way too — holding a background tab shown
    /// disturbs nothing the human sees, because each tab renders into its own
    /// framebuffer (see [`Tab::rendering_context`]).
    pub fn sync_visibility(&self) {
        let displayed_id = self.displayed().map(|tab| tab.id);
        for tab in &self.tabs {
            match visibility_of(Some(tab.id) == displayed_id, tab.held_for_view) {
                Visibility::DisplayedAndFocused => {
                    tab.webview.show();
                    tab.webview.focus();
                },
                Visibility::HeldForViewing => {
                    tab.webview.show();
                    tab.webview.blur();
                },
                Visibility::Hidden => {
                    tab.webview.hide();
                    tab.webview.blur();
                },
            }
        }
    }

    /// Hold `id` shown for a remote viewer, if an agent owns it.
    ///
    /// Through the agent-only predicate for the same reason
    /// [`TabManager::agent_tab`] exists: a tab the human owns must be
    /// unrepresentable on the remote path rather than refused after the fact.
    /// Answers whether the hold was taken, so a caller cannot record a lease
    /// on a tab that never took one.
    pub fn hold_for_view(&mut self, id: u64) -> bool {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id && t.owner.is_agent()) else {
            return false;
        };
        tab.held_for_view += 1;
        // Re-established rather than assumed: the tab may have been hidden a
        // moment ago and this is what shows it.
        self.sync_visibility();
        true
    }

    /// Release one hold on `id`.
    ///
    /// Releasing a hold that was never taken, or one on a tab that has since
    /// closed, is not an error — a lease outliving its tab is the ordinary
    /// case, and the tab table is the one that decides a closed tab has no
    /// visibility left to restore. When the last hold goes the tab returns to
    /// whatever visibility the local view state says it should have, which is
    /// [`TabManager::sync_visibility`]'s answer and never a remembered one.
    pub fn release_view_hold(&mut self, id: u64) {
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id) else {
            return;
        };
        if tab.held_for_view == 0 {
            return;
        }
        tab.held_for_view -= 1;
        self.sync_visibility();
    }

    /// How many viewers hold `id` shown. `None` for a tab that is not there.
    pub fn view_holds(&self, id: u64) -> Option<usize> {
        self.get(id).map(|tab| tab.held_for_view)
    }

    pub fn get(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id)
    }

    /// The tab `id` names, **only if an agent owns it**.
    ///
    /// This is the whole of the agent-tabs-only filter, and it is a *lookup*
    /// rather than a check on purpose. `D-05-02` says a remote viewer may
    /// reach agent tabs and nothing else; a `get` followed by an owner test
    /// would satisfy that today and stop satisfying it the first time somebody
    /// adds a second call site and forgets the second half. Resolving through
    /// this instead makes a human-owned tab **unrepresentable** on the remote
    /// path — the same structural shape `D-04-04` gives the bind address,
    /// where the guarantee is in what the program can express rather than in a
    /// validator that could be relaxed.
    ///
    /// The Me tabs are where the human's history rows, bookmarks and
    /// autofilled credentials live, so the tab a viewer cannot name is exactly
    /// the tab worth not naming.
    pub fn agent_tab(&self, id: u64) -> Option<&Tab> {
        self.tabs.iter().find(|t| t.id == id && t.owner.is_agent())
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

#[cfg(test)]
mod tests {

    /// The regression for a bug that silently broke remote takeover.
    ///
    /// A background capture shows a tab to paint it, then re-hides it. That
    /// re-hide used to be unconditional, so an agent's routine `screenshot`
    /// hid a tab a remote viewer was watching. The failure was invisible: the
    /// viewer's frames kept arriving, because they are read from the offscreen
    /// buffer and do not need the webview shown, while its clicks stopped
    /// landing, because a hidden webview answers no hit test. The hold count
    /// still read 1; only `last_applied_input` stopped advancing.
    ///
    /// This pins the predicate the capture drain now consults. A held tab is
    /// not `Hidden` even when it is not the displayed one, so `hide_after` is
    /// false and the capture leaves it shown.
    #[test]
    fn a_capture_must_not_re_hide_a_tab_a_viewer_is_holding() {
        // Not displayed, nobody watching: the capture re-hides, as before.
        assert_eq!(visibility_of(false, 0), Visibility::Hidden);

        // Not displayed, one viewer attached: it must stay shown.
        assert_ne!(visibility_of(false, 1), Visibility::Hidden);
        assert_eq!(visibility_of(false, 1), Visibility::HeldForViewing);

        // Two viewers on one tab, and the first detaching, are the same case.
        assert_ne!(visibility_of(false, 2), Visibility::Hidden);

        // The displayed tab is never downgraded by a viewer attaching to it.
        assert_eq!(visibility_of(true, 0), Visibility::DisplayedAndFocused);
        assert_eq!(visibility_of(true, 3), Visibility::DisplayedAndFocused);
    }

    use super::*;

    /// `TabOwner::is_agent` is what the agent-only lookup and both filtered
    /// iterators are built on, so it is worth pinning on its own: everything
    /// `D-05-02` promises a remote viewer reduces to this predicate answering
    /// `false` for a tab the human owns.
    ///
    /// The table itself cannot be exercised without a live engine — every
    /// `Tab` owns a real `WebView` — so `crate::view` tests the filter's
    /// *consequences* against a fake tab source instead, and this pins the
    /// one part that stands alone.
    #[test]
    fn only_an_agent_owned_tab_reports_as_an_agents() {
        assert!(!TabOwner::Me.is_agent());
        assert!(TabOwner::Agent { session_id: 1, client: "client-a".into() }.is_agent());
        assert_eq!(TabOwner::Me.view(), ViewMode::Me);
        assert_eq!(
            TabOwner::Agent { session_id: 1, client: "client-a".into() }.view(),
            ViewMode::Agents
        );
    }

    /// The whole visibility decision table, across displayed and held — the
    /// three states and every combination that reaches them.
    #[test]
    fn the_visibility_table_shows_a_held_tab_and_hides_only_an_unheld_one() {
        assert_eq!(visibility_of(false, 0), Visibility::Hidden);
        assert_eq!(visibility_of(false, 1), Visibility::HeldForViewing);
        assert_eq!(visibility_of(false, 2), Visibility::HeldForViewing);
        assert_eq!(visibility_of(true, 0), Visibility::DisplayedAndFocused);
    }

    /// A viewer attached to the tab the human is *already* looking at does not
    /// downgrade it out of focus. Displayed wins, and it wins for every hold
    /// count rather than only for the one somebody happened to test.
    #[test]
    fn a_hold_never_takes_focus_from_the_displayed_tab() {
        for holds in 0..4 {
            assert_eq!(
                visibility_of(true, holds),
                Visibility::DisplayedAndFocused,
                "{holds} viewers changed what the local human is focused on"
            );
        }
    }

    /// A held tab is shown and **not** focused: the two shown states are
    /// distinct values, so "shown" can never silently mean "focused".
    #[test]
    fn a_held_tab_is_shown_without_being_focused() {
        assert_ne!(visibility_of(false, 1), Visibility::DisplayedAndFocused);
        assert_ne!(visibility_of(false, 1), Visibility::Hidden);
    }
}
