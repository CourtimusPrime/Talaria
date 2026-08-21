/* This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 *
 * The egui/servo compositing approach is derived from Servo's servoshell
 * (servo v0.4.0, ports/servoshell/desktop/gui.rs); this file keeps the MPL
 * 2.0 header per its file-level copyleft. */

use std::rc::Rc;
use std::sync::Arc;

use egui::{LayerId, PaintCallback};
use egui_glow::CallbackFn;
use egui_glow::winit::EguiGlow;
use servo::{RenderingContext, WindowRenderingContext};
use talaria_protocol::{ChromeRect, CredentialEntry};
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::app::Shared;
use crate::settings::SearchEngine;
use crate::tabs::ViewMode;

/// Which chrome panel, if any, has replaced the page.
///
/// This generalises what was a single `credentials_open: bool`. Later phases
/// add their own variants (bookmarks, downloads, settings); the panels are
/// mutually exclusive by construction rather than by a set of booleans that
/// could all be true at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromePanel {
    None,
    Credentials,
    History,
    Bookmarks,
    Downloads,
    Settings,
}

pub enum UiAction {
    SwitchMode(ViewMode),
    Go(String),
    NewTab,
    CloseTab(u64),
    SelectTab(u64),
    Back,
    Forward,
    Reload,
    /// Reload a crashed tab: clears the crashed flag and reloads the page.
    ReloadCrashed(u64),
    /// Store a credential the user typed into the credentials panel. The
    /// panel cannot write the vault itself — a `borrow_mut` inside the egui
    /// closure is the named anti-pattern, so the write leaves as an intent
    /// and lands in `apply_ui_actions` once egui's borrows are released.
    SaveCredential(CredentialEntry),
    /// Remove the stored credential keyed on a **bare host** plus a username.
    /// `Vault::delete` normalises a domain, not an address, so the panel
    /// passes the host parsed out of the row's URL rather than the URL.
    DeleteCredential { host: String, username: String },
    /// Show a chrome panel, or [`ChromePanel::None`] to go back to the page.
    /// Even this goes through the round trip: teaching a later reader that
    /// "some" mutations are legal inline is how the anti-pattern comes back.
    SetPanel(ChromePanel),
    /// Forget every recorded visit. Emitted only by the History panel's
    /// second, confirming click — never by the first.
    ClearHistory,
    /// Bookmark the page the star was pressed on, or un-bookmark it — the
    /// star emits this same intent in either state, and `apply_ui_actions`
    /// resolves which one it is against the store. Deciding here would mean
    /// reading and then writing the store from inside the egui closure, which
    /// is the named anti-pattern, and would also race the frame: the state
    /// the button drew with is one frame old.
    ///
    /// Carries `(url, title)`: the title is captured at press time so a
    /// bookmark remembers the page as the human saw it.
    ToggleBookmark(String, String),
    /// Remove the bookmark for this URL. Emitted by a bookmark row's Trash
    /// button — immediate, with no confirming click, matching
    /// [`UiAction::DeleteCredential`]'s existing precedent.
    RemoveBookmark(String),
    /// Persist the search engine the human typed into the Settings panel.
    ///
    /// The panel has already refused to emit this for an invalid template —
    /// its Save button is disabled until [`crate::settings::is_valid_template`]
    /// passes — so `apply_ui_actions` writes what arrives without re-checking.
    SaveSearchEngine(SearchEngine),
    /// Hand a completed download's file to the OS's default handler.
    ///
    /// Carries the **stored** [`crate::downloads::DownloadEntry::path`]
    /// verbatim — the path `create_unique` actually wrote, cloned straight out
    /// of the store — never a path rebuilt from the requested filename, which
    /// a uniquifying rename may have made name a different file entirely.
    ///
    /// This is the one action in this enum that starts an external process, so
    /// where it can be raised from is the security property (T-03-04-01): a
    /// Downloads panel button, applied in `apply_ui_actions`, and nowhere
    /// else. No control-socket command reaches it, no `talaria_protocol`
    /// variant carries it, and no MCP tool exists for it — an agent-reachable
    /// opener would hand back exactly the local-execution surface Phase 2's
    /// SEC-01/SEC-02 spent their whole scope removing.
    OpenDownload(String),
    /// Drop a download's row from the list, keyed on the same stored path.
    ///
    /// Removes a row, never a file: `Downloads::remove` touches nothing on
    /// disk, and the row's hover text says so.
    RemoveDownload(String),
    /// Clear the one-shot "couldn't open that download" notice.
    DismissDownloadError,
    /// Clear the vault's one-shot notices, once the chrome has shown them.
    DismissVaultNotice,
}

/// The first `limit` characters of `text`, with an ellipsis when there was
/// more.
///
/// Counted in characters rather than bytes on purpose: `String::truncate`
/// takes a byte index and panics when it lands inside a multibyte character,
/// and a page title is exactly where one turns up.
fn truncate_chars(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((index, _)) => format!("{}…", &text[..index]),
        None => text.to_owned(),
    }
}

/// Characters that let a name lie about itself once it is a label.
///
/// Two classes, one problem. Control characters — newline and tab most
/// obviously — take text out of the row it was given: a filename of
/// `"invoice.pdf\n\nSafe — from your bank"` renders as two lines, the second
/// of which the human has no reason to read as a filename. The Unicode
/// bidirectional overrides and isolates reorder what follows them, so
/// `report.fdp.exe` renders as `report.exe.pdf` and the extension the human
/// checks is not the extension the OS will dispatch on.
///
/// Neither is a path problem, which is why neither was caught by the `/` and
/// `..` checks this predicate now sits beside at the download boundary — see
/// `crate::app`'s `validate_download_filename`, its other caller.
pub fn is_display_unsafe(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// `text` with every [`is_display_unsafe`] character replaced by U+FFFD.
///
/// Defence in depth rather than the fix. `Command::Download` refuses these at
/// the boundary, so a row this build wrote cannot carry one; a
/// `downloads.json` written by a build without that check can, and this is
/// what stops it rendering. The replacement character is deliberate: a
/// dropped character would let the name shorten silently into something else
/// plausible, where U+FFFD shows the human that something was there.
pub fn sanitize_for_display(text: &str) -> String {
    text.chars()
        .map(|character| if is_display_unsafe(character) { '\u{FFFD}' } else { character })
        .collect()
}

/// The action the Downloads panel's `Open` button emits for one row.
///
/// A named function rather than an inline `UiAction::OpenDownload(...)` so
/// that "Open launches the path this shell actually wrote, never the name the
/// request asked for" is a claim a test can hold. The two differ whenever a
/// collision was uniquified, and the requested name is the string an agent
/// chose; `03-VALIDATION.md`'s T-03-04-01/T-03-04-02 assert exactly this and
/// nothing pinned it until now.
///
/// The path is cloned verbatim — not sanitized, not re-derived. Sanitization
/// is for what the panel *renders*; this is what the OS is handed, and it has
/// to name the same file `create_unique` created.
fn open_action(entry: &crate::downloads::DownloadEntry) -> UiAction {
    UiAction::OpenDownload(entry.path.clone())
}

/// Note down where one named chrome control was laid out this frame.
///
/// `rects` is `Some` only when the shell was started with
/// `TALARIA_TEST_HOOKS=1` — see [`Gui::chrome_rects`]. The absence of the
/// collection *is* the off switch, so a production frame does not build a
/// name, does not push a rect, and has nothing to hand out.
///
/// `index` names one row of a list panel; `None` is a control there is only
/// ever one of. The formatting happens inside the `Some` arm on purpose:
/// a `format!` at the call site would allocate on every row of every frame in
/// a build that then throws the string away.
fn record_rect(
    rects: Option<&mut Vec<ChromeRect>>,
    name: &str,
    index: Option<usize>,
    rect: egui::Rect,
) {
    let Some(rects) = rects else { return };
    rects.push(ChromeRect {
        name: match index {
            Some(index) => format!("{name}.{index}"),
            None => name.to_owned(),
        },
        x: rect.left(),
        y: rect.top(),
        width: rect.width(),
        height: rect.height(),
    });
}

/// A visit's age in words, at the resolution a history list actually needs.
///
/// No date crate is in this workspace and this phase adds none, so the
/// arithmetic is done here. A timestamp in the future (a clock that moved
/// backwards between the visit and now) reads as "just now" rather than
/// underflowing.
fn relative_time(visited_at_ms: u64, now_ms: u64) -> String {
    let seconds = now_ms.saturating_sub(visited_at_ms) / 1_000;
    match seconds {
        0..=59 => "just now".to_owned(),
        60..=3_599 => format!("{}m ago", seconds / 60),
        3_600..=86_399 => format!("{}h ago", seconds / 3_600),
        _ => format!("{}d ago", seconds / 86_400),
    }
}

/// A byte count in words, at the resolution a downloads list actually needs.
///
/// No formatting crate is in this workspace and this phase adds none, so the
/// arithmetic is done here — the same call [`relative_time`] makes. Powers of
/// 1024 with the familiar KB/MB/GB labels, matching what every browser's own
/// downloads list shows; one decimal place above a kilobyte, because "1.4 MB"
/// is informative where "1 MB" and "1048576 bytes" are respectively vague and
/// unreadable.
fn human_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let bytes = bytes as f64;
    if bytes < KB {
        return format!("{bytes:.0} bytes");
    }
    if bytes < MB {
        return format!("{:.1} KB", bytes / KB);
    }
    if bytes < GB {
        return format!("{:.1} MB", bytes / MB);
    }
    format!("{:.1} GB", bytes / GB)
}

pub struct Gui {
    context: EguiGlow,
    rendering_context: Rc<WindowRenderingContext>,
    /// Ctrl+L: select the whole URL on the frame the bar gains focus, so
    /// typing replaces it (browser behaviour) instead of appending.
    select_location: bool,
    /// Which panel, if any, has replaced the page. View state, so it lives
    /// here rather than on `Shared`.
    panel: ChromePanel,
    /// Put the caret in the panel's first field on the frame it opens, so the
    /// panel is usable from the keyboard alone (Ctrl+K, type, Tab, Enter).
    focus_credentials: bool,
    /// Draft inputs for the add form, cleared once saved.
    credential_site: String,
    credential_username: String,
    credential_password: String,
    /// Which stored row, if any, currently has its password on screen. One at
    /// a time and never across a close: a list that shows every password at a
    /// glance is the shoulder-surfing surface the encryption exists to avoid.
    revealed_entry: Option<usize>,
    /// Whether the History panel's Clear control is in its confirming state.
    /// Reset on every panel change, exactly like `revealed_entry`, so a
    /// half-pressed clear cannot survive the panel closing and be completed
    /// by a click the user meant for something else.
    confirm_clear_history: bool,
    /// Draft inputs for the Settings form. Unlike the credentials draft,
    /// these are *pre-filled* rather than cleared on save: a settings form
    /// shows what is currently configured, and after a save that is what the
    /// human just typed.
    settings_name: String,
    settings_template: String,
    /// Whether the Settings draft still needs filling from the store.
    ///
    /// `set_panel` has no `&Shared` to read the engine from, so the prefill
    /// happens on the first frame the panel is actually drawn and this flag
    /// is what makes it happen once — the same "do this on the frame it
    /// opens" shape as `focus_credentials`. Without it, every frame would
    /// overwrite the characters being typed.
    settings_draft_stale: bool,
    /// The filename and OS error from the last failed Open, or `None`.
    ///
    /// Lives on `Gui` rather than on `Shared`, unlike the vault's equivalent
    /// one-shot notices: this is a chrome-level notice about a chrome-level
    /// action, it is never persisted, and nothing outside this panel has any
    /// reason to know it happened.
    download_open_error: Option<(String, String)>,
    /// Where each named chrome element was on the last frame that was drawn,
    /// or `None` in a build that is not running under `TALARIA_TEST_HOOKS=1`.
    ///
    /// View state, like every other field here: the chrome reports its own
    /// geometry and nothing on `Shared` is touched to do it. Read back out
    /// through [`Gui::chrome_rects`] by the one command that may see it.
    chrome_rects: Option<Vec<ChromeRect>>,
}

/// The bare host of a stored entry's URL — what [`UiAction::DeleteCredential`]
/// and `Vault::delete` key on. An entry whose URL has no parseable host (a
/// hand-written `vault.json` can hold one) falls back to the raw string, so
/// the row still shows something and still names itself.
fn entry_host(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| url.trim().to_ascii_lowercase())
}

/// The URL a typed site becomes. A user types `example.com`; stored as typed,
/// that entry has no parseable host, so it would never match a domain and
/// could never be deleted again. Completing it to https keeps every entry the
/// panel creates addressable by the same key the vault matches on.
fn credential_url(site: &str) -> String {
    let site = site.trim();
    match url::Url::parse(site) {
        Ok(url) if url.host_str().is_some() => url.to_string(),
        _ => format!("https://{site}"),
    }
}

impl Gui {
    pub fn new(
        event_loop: &ActiveEventLoop,
        rendering_context: Rc<WindowRenderingContext>,
    ) -> Self {
        let _ = rendering_context.make_current();
        let context = EguiGlow::new(
            event_loop,
            rendering_context.glow_gl_api(),
            None,
            None,
            false,
        );
        let mut fonts = egui::FontDefinitions::default();
        egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
        context.egui_ctx.set_fonts(fonts);
        Self {
            context,
            rendering_context,
            select_location: false,
            panel: ChromePanel::None,
            focus_credentials: false,
            credential_site: String::new(),
            credential_username: String::new(),
            credential_password: String::new(),
            revealed_entry: None,
            confirm_clear_history: false,
            settings_name: String::new(),
            settings_template: String::new(),
            settings_draft_stale: true,
            download_open_error: None,
            // Read once, here, rather than per frame: the variable cannot
            // change under a running process, and `std::env::var` inside the
            // render loop would be a lock and an allocation per element. The
            // command that reads these back checks the same variable at its
            // own point of use, exactly as the `evaluate` crash hook does.
            chrome_rects: match std::env::var("TALARIA_TEST_HOOKS").as_deref() == Ok("1") {
                true => Some(Vec::new()),
                false => None,
            },
        }
    }

    pub fn on_window_event(
        &mut self,
        window: &Window,
        event: &WindowEvent,
    ) -> egui_winit::EventResponse {
        self.context.on_window_event(window, event)
    }

    pub fn has_keyboard_focus(&self) -> bool {
        self.context.egui_ctx.memory(|memory| memory.focused().is_some())
    }

    /// Focus the URL bar (Ctrl+L) and select its contents.
    pub fn focus_location_bar(&mut self) {
        self.context.egui_ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::new("location-bar"));
        });
        self.select_location = true;
    }

    /// Which panel is currently showing. Read by the Ctrl+K / Ctrl+H
    /// shortcuts so the toggle each emits is an intent like any other.
    pub fn panel(&self) -> ChromePanel {
        self.panel
    }

    /// Whether any panel has replaced the page — what decides that the area
    /// below the toolbar belongs to egui rather than to a webview.
    pub fn panel_open(&self) -> bool {
        self.panel != ChromePanel::None
    }

    /// Show `panel`, or hide the current one with [`ChromePanel::None`]. Only
    /// [`UiAction::SetPanel`] calls this, from `apply_ui_actions` — never the
    /// egui closure.
    ///
    /// Every panel-local view state resets here, on every switch: a revealed
    /// password never survives the panel closing, and neither does a
    /// half-pressed Clear history. Only the credentials panel wants its first
    /// field focused on open, so the whole panel is keyboard-reachable
    /// (Ctrl+K, type, Tab, Enter); the History panel has no field to fill.
    pub fn set_panel(&mut self, panel: ChromePanel) {
        self.panel = panel;
        self.revealed_entry = None;
        self.confirm_clear_history = false;
        self.focus_credentials = panel == ChromePanel::Credentials;
        // A half-typed engine does not survive the panel closing either. The
        // draft is refilled from the store on the next frame the panel is
        // drawn, so what a reopened panel shows is what is actually saved,
        // never what someone abandoned mid-edit.
        self.settings_draft_stale = true;
        // A failed Open is a notice about one press in one panel session. It
        // does not outlive that session: reopening Downloads after closing it
        // must not re-accuse the user of something they already walked away
        // from. `DismissDownloadError` exists for clearing it without leaving.
        self.download_open_error = None;
    }

    /// Where the named chrome elements were on the last frame this `Gui`
    /// drew, or nothing at all outside `TALARIA_TEST_HOOKS=1`.
    ///
    /// Last frame, not this instant: layout only exists while a frame is
    /// being built, so there is nothing newer to report. A caller that has
    /// just changed what is on screen should read this until it sees the
    /// element it is waiting for — every control-socket command asks for a
    /// redraw when it finishes, so the next read is against a fresh frame.
    pub fn chrome_rects(&self) -> &[ChromeRect] {
        self.chrome_rects.as_deref().unwrap_or_default()
    }

    /// Record that an Open failed, so the Downloads panel can say so inline.
    ///
    /// Called from `apply_ui_actions` — the panel itself cannot set this, for
    /// the same reason it cannot write any other shell state from inside an
    /// egui closure.
    pub fn set_download_open_error(&mut self, filename: String, error: String) {
        self.download_open_error = Some((filename, error));
    }

    /// Forget the last failed Open. The `Dismiss` button's intent lands here.
    pub fn clear_download_open_error(&mut self) {
        self.download_open_error = None;
    }

    pub fn surrender_focus(&self) {
        self.context.egui_ctx.memory_mut(|memory| {
            if let Some(id) = memory.focused() {
                memory.surrender_focus(id);
            }
        });
    }

    /// Build the chrome UI, resize + paint the displayed webview, and record
    /// the blit of servo's offscreen buffer into egui's background layer.
    /// Returns UI actions for the caller to apply once borrows are released.
    // Top-level Panel::show is deprecated in egui 0.34 but is still what
    // servoshell itself uses; revisit when migrating to the show_inside idiom.
    #[allow(deprecated)]
    pub fn update(&mut self, shared: &Rc<Shared>) -> Vec<UiAction> {
        let _ = self.rendering_context.make_current();
        let mut actions: Vec<UiAction> = Vec::new();
        let mut select_location = std::mem::take(&mut self.select_location);
        // The closure cannot see `self` (its `context` field is borrowed for
        // the whole frame), so the panel's view state travels in and out as
        // locals — the same shape `select_location` already uses.
        let panel = self.panel;
        let mut focus_credentials = std::mem::take(&mut self.focus_credentials);
        let mut revealed_entry = self.revealed_entry;
        let mut confirm_clear_history = self.confirm_clear_history;
        let mut credential_site = std::mem::take(&mut self.credential_site);
        let mut credential_username = std::mem::take(&mut self.credential_username);
        let mut credential_password = std::mem::take(&mut self.credential_password);
        let mut settings_name = std::mem::take(&mut self.settings_name);
        let mut settings_template = std::mem::take(&mut self.settings_template);
        let mut settings_draft_stale = self.settings_draft_stale;
        let download_open_error = self.download_open_error.clone();
        // Rebuilt from scratch every frame, so an element that stopped being
        // drawn stops being reported — a stale rect for a control that is no
        // longer there would be worse than no rect at all. Present only when
        // the hook is on, which is the whole of the gate on this side; travels
        // in and out as a local for the same reason the fields above do.
        let mut chrome_rects = self.chrome_rects.is_some().then(Vec::new);

        self.context.run(&shared.window, |ctx| {
            let mut tabs = shared.tabs.borrow_mut();
            let mode = tabs.mode;

            egui::Panel::top("toolbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let me = mode == ViewMode::Me;
                    // Bound rather than used inline, here and below, so the
                    // rect the test hook reports is the one this control was
                    // actually laid out at — see `record_rect`. In a build
                    // without the hook every one of these calls is a branch
                    // on a `None` and nothing else.
                    let me_toggle = ui.selectable_label(me, "Me");
                    record_rect(chrome_rects.as_mut(), "toolbar.me", None, me_toggle.rect);
                    if me_toggle.clicked() && !me {
                        actions.push(UiAction::SwitchMode(ViewMode::Me));
                    }
                    let agents_toggle = ui.selectable_label(!me, "Agents");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.agents",
                        None,
                        agents_toggle.rect,
                    );
                    if agents_toggle.clicked() && me {
                        actions.push(UiAction::SwitchMode(ViewMode::Agents));
                    }
                    ui.separator();

                    let has_tab = tabs.displayed().is_some();
                    // What the bookmark star acts on: the URL and title of
                    // the page the human is looking at, read here while the
                    // tab table is still borrowed. The title is captured now
                    // rather than at apply time so the bookmark remembers the
                    // page as it was on screen when the star was pressed.
                    let current_page = tabs.displayed().map(|tab| {
                        let url = tab
                            .webview
                            .url()
                            .map(|url| url.to_string())
                            .filter(|url| !url.is_empty())
                            .unwrap_or_else(|| tab.location.clone());
                        let title = tab
                            .webview
                            .page_title()
                            .filter(|title| !title.is_empty())
                            .unwrap_or_else(|| url.clone());
                        (url, title)
                    });
                    let back = ui
                        .add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_LEFT))
                        .on_hover_text("Back");
                    record_rect(chrome_rects.as_mut(), "toolbar.back", None, back.rect);
                    if back.clicked() {
                        actions.push(UiAction::Back);
                    }
                    let forward = ui
                        .add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_RIGHT))
                        .on_hover_text("Forward");
                    record_rect(chrome_rects.as_mut(), "toolbar.forward", None, forward.rect);
                    if forward.clicked() {
                        actions.push(UiAction::Forward);
                    }
                    let reload = ui
                        .add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_CLOCKWISE))
                        .on_hover_text("Reload (Ctrl+R)");
                    record_rect(chrome_rects.as_mut(), "toolbar.reload", None, reload.rect);
                    if reload.clicked() {
                        actions.push(UiAction::Reload);
                    }

                    let new_tab = ui
                        .button(egui_phosphor::regular::PLUS)
                        .on_hover_text("New tab (Ctrl+T)");
                    record_rect(chrome_rects.as_mut(), "toolbar.new-tab", None, new_tab.rect);
                    if new_tab.clicked() {
                        actions.push(UiAction::NewTab);
                    }

                    ui.separator();

                    // Same human-only posture as the credentials button
                    // below: history opens from here and from Ctrl+H, and
                    // from nowhere else. No control-socket command reaches
                    // it, and no MCP tool reads the store behind it — where
                    // the human has been is not an agent's to enumerate.
                    let history_open = panel == ChromePanel::History;
                    let history_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE)
                                .selected(history_open),
                        )
                        .on_hover_text("Open history (Ctrl+H)");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.history",
                        None,
                        history_button.rect,
                    );
                    if history_button.clicked() {
                        actions.push(UiAction::SetPanel(match history_open {
                            true => ChromePanel::None,
                            false => ChromePanel::History,
                        }));
                    }

                    // The star acts on the page the human is looking at, and
                    // it is the one control in this chrome that is a toggle
                    // rather than a panel. Its `.selected()` highlight is the
                    // reserved accent colour's only non-panel use, per
                    // 03-UI-SPEC.md — it says "this page is kept", which is
                    // exactly what the accent is reserved for.
                    let bookmarked = current_page
                        .as_ref()
                        .is_some_and(|(url, _)| shared.bookmarks.borrow().is_bookmarked(url));
                    let star_hover = match bookmarked {
                        true => "Remove bookmark (Ctrl+D)",
                        false => "Bookmark this page (Ctrl+D)",
                    };
                    let star = ui
                        .add_enabled(
                            has_tab,
                            egui::Button::new(egui_phosphor::regular::STAR)
                                .selected(bookmarked),
                        )
                        .on_hover_text(star_hover);
                    record_rect(chrome_rects.as_mut(), "toolbar.bookmark-star", None, star.rect);
                    if star.clicked() {
                        if let Some((url, title)) = current_page.clone() {
                            actions.push(UiAction::ToggleBookmark(url, title));
                        }
                    }

                    // Same human-only posture as History and Credentials: the
                    // list opens from here and from Ctrl+B, and from nowhere
                    // else. No control-socket command reaches it, and no MCP
                    // tool reads the store behind it.
                    let bookmarks_open = panel == ChromePanel::Bookmarks;
                    let bookmarks_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::BOOKMARKS)
                                .selected(bookmarks_open),
                        )
                        .on_hover_text("Open bookmarks (Ctrl+B)");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.bookmarks",
                        None,
                        bookmarks_button.rect,
                    );
                    if bookmarks_button.clicked() {
                        actions.push(UiAction::SetPanel(match bookmarks_open {
                            true => ChromePanel::None,
                            false => ChromePanel::Bookmarks,
                        }));
                    }

                    // Downloads is the one new panel whose store an agent can
                    // *write* — a `download` command is how a row gets here.
                    // Reading it, opening from it and removing from it are
                    // still human-only: this button and Ctrl+J, and nothing
                    // else. That asymmetry is the whole design. See
                    // `UiAction::OpenDownload` for why the Open button in
                    // particular must stay unreachable from the tool surface.
                    let downloads_open = panel == ChromePanel::Downloads;
                    let downloads_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::DOWNLOAD)
                                .selected(downloads_open),
                        )
                        .on_hover_text("Open downloads (Ctrl+J)");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.downloads",
                        None,
                        downloads_button.rect,
                    );
                    if downloads_button.clicked() {
                        actions.push(UiAction::SetPanel(match downloads_open {
                            true => ChromePanel::None,
                            false => ChromePanel::Downloads,
                        }));
                    }

                    // Toolbar-only, with no keyboard shortcut: every other
                    // panel here maps to a convention a real browser already
                    // taught the user (Ctrl+H, Ctrl+B), and there is no such
                    // convention for settings — Chrome, Firefox and Safari
                    // each bind something different or nothing at all. Per
                    // 03-UI-SPEC's Open Question 4, this phase declines to
                    // invent one rather than picking arbitrarily.
                    let settings_open = panel == ChromePanel::Settings;
                    let settings_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::GEAR)
                                .selected(settings_open),
                        )
                        .on_hover_text("Settings");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.settings",
                        None,
                        settings_button.rect,
                    );
                    if settings_button.clicked() {
                        actions.push(UiAction::SetPanel(match settings_open {
                            true => ChromePanel::None,
                            false => ChromePanel::Settings,
                        }));
                    }

                    // The credentials surface is human-only. It opens from
                    // this button and from Ctrl+K, and from nowhere else: no
                    // control-socket command reaches it, so an agent cannot
                    // drive the human's own takeover affordance as a tool.
                    let credentials_open = panel == ChromePanel::Credentials;
                    let credentials_button = ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::KEY)
                                .selected(credentials_open),
                        )
                        .on_hover_text("Credentials (Ctrl+K)");
                    record_rect(
                        chrome_rects.as_mut(),
                        "toolbar.credentials",
                        None,
                        credentials_button.rect,
                    );
                    if credentials_button.clicked() {
                        actions.push(UiAction::SetPanel(match credentials_open {
                            true => ChromePanel::None,
                            false => ChromePanel::Credentials,
                        }));
                    }

                    // Autofill (D-23) is a *suggestion in the chrome*, matched
                    // on the host the human is actually looking at. It never
                    // writes into the page: no script is evaluated, no form
                    // field is located, no DOM node is touched. Injection
                    // would collide with an agent's own `evaluate` on the same
                    // form, and it would put the user's password inside a
                    // document every script on that page can read. Filling the
                    // form directly looks friendlier, and is the change most
                    // likely to be made by someone who has not read this.
                    //
                    // It is also driven entirely by what the human is looking
                    // at: an agent has no way to make it appear and no way to
                    // read what it shows.
                    let suggestion_host = tabs.displayed().and_then(|tab| {
                        let address = tab
                            .webview
                            .url()
                            .map(|url| url.to_string())
                            .filter(|url| !url.is_empty())
                            .unwrap_or_else(|| tab.location.clone());
                        url::Url::parse(&address)
                            .ok()
                            .and_then(|parsed| parsed.host_str().map(str::to_ascii_lowercase))
                    });
                    // Exact-or-subdomain matching, borrowed from the vault
                    // rather than reimplemented here, so a lookalike host does
                    // not match on a substring.
                    let suggestions = suggestion_host
                        .as_deref()
                        .map(|host| shared.vault.borrow().matching(host))
                        .unwrap_or_default();
                    // Reserve room before the location bar claims the rest of
                    // the row, so the suggestion has somewhere to sit.
                    let suggestion_width = match suggestions.is_empty() {
                        true => 0.0,
                        false => 168.0,
                    };

                    if let Some(tab) = tabs.displayed() {
                        if tab.webview.load_status() != servo::LoadStatus::Complete
                            && !tab.crashed
                        {
                            ui.spinner();
                        }
                    }
                    if let Some(tab) = tabs.displayed_mut() {
                        let output = egui::TextEdit::singleline(&mut tab.location)
                            .id(egui::Id::new("location-bar"))
                            .hint_text("Search or enter address")
                            .desired_width((ui.available_width() - suggestion_width).max(80.0))
                            .show(ui);
                        let response = output.response;
                        record_rect(chrome_rects.as_mut(), "toolbar.location", None, response.rect);
                        if select_location && response.has_focus() {
                            use egui::text::{CCursor, CCursorRange};
                            let mut state = output.state;
                            let end = CCursor::new(tab.location.chars().count());
                            state.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(0), end)));
                            state.store(ctx, response.id);
                            select_location = false;
                        }
                        if response.changed() {
                            tab.location_dirty = true;
                        }
                        if response.lost_focus() {
                            let (enter, escape) = ui.input(|input| {
                                (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Escape))
                            });
                            if enter {
                                actions.push(UiAction::Go(tab.location.clone()));
                            } else if escape {
                                // Escape abandons the edit: back to the page's URL,
                                // keyboard focus returns to the page.
                                tab.location = tab
                                    .webview
                                    .url()
                                    .map(|url| url.to_string())
                                    .unwrap_or_default();
                                tab.location_dirty = false;
                            }
                        }
                    } else {
                        ui.label("No tab open");
                    }

                    if let Some(host) = suggestion_host.filter(|_| !suggestions.is_empty()) {
                        // Named, so the user sees which login they are about
                        // to copy before a lookalike host can borrow one.
                        let label = match suggestions.len() {
                            1 => format!(
                                "{} {}",
                                egui_phosphor::regular::KEY,
                                suggestions[0].username
                            ),
                            count => {
                                format!("{} {count} logins", egui_phosphor::regular::KEY)
                            },
                        };
                        ui.menu_button(label, |ui| {
                            ui.label(format!("Saved for {host}"));
                            for entry in &suggestions {
                                ui.horizontal(|ui| {
                                    ui.label(&entry.username);
                                    // A clipboard write is not shell state, so
                                    // it needs no intent — and no other inline
                                    // mutation belongs here.
                                    if ui.button("Copy username").clicked() {
                                        ui.ctx().copy_text(entry.username.clone());
                                        ui.close();
                                    }
                                    // T-02-10-04, accepted: this puts the
                                    // password on the system clipboard, where
                                    // other applications can read it. It is an
                                    // explicit act on an obviously-labelled
                                    // control, and it is exactly what the user
                                    // would otherwise do by hand out of a
                                    // password manager.
                                    if ui.button("Copy password").clicked() {
                                        ui.ctx().copy_text(entry.password.clone());
                                        ui.close();
                                    }
                                });
                            }
                        })
                        .response
                        .on_hover_text(
                            "Saved logins for this site — copied, never typed into the page",
                        );
                    }
                });
            });

            egui::Panel::top("strip").show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| match mode {
                    ViewMode::Me => {
                        let active = tabs.active_id(ViewMode::Me);
                        for tab in tabs.me_tabs() {
                            // truncate_chars, not String::truncate: the title is
                            // page-supplied, and a byte index inside a multibyte
                            // character panics the render pass and with it the
                            // whole browser.
                            let mut label = truncate_chars(
                                &tab.webview
                                    .page_title()
                                    .filter(|t| !t.is_empty())
                                    .unwrap_or_else(|| tab.location.clone()),
                                28,
                            );
                            if tab.crashed {
                                label = format!("💥 {label}");
                            }
                            if ui
                                .selectable_label(active == Some(tab.id), label)
                                .on_hover_text(&tab.location)
                                .clicked()
                            {
                                actions.push(UiAction::SelectTab(tab.id));
                            }
                            if ui.small_button(egui_phosphor::regular::X).on_hover_text("Close tab").clicked() {
                                actions.push(UiAction::CloseTab(tab.id));
                            }
                            ui.separator();
                        }
                    },
                    ViewMode::Agents => {
                        let sessions = shared.sessions.borrow();
                        if sessions.is_empty() && tabs.agent_tabs().next().is_none() {
                            ui.label(
                                "No agents connected — point an MCP client at talaria-mcp",
                            );
                        } else {
                            // Tabs grouped by owning session; sessions that
                            // disconnected leave their tabs under a
                            // "disconnected" group (tabs outlive sessions by
                            // design).
                            let active = tabs.active_id(ViewMode::Agents);
                            let render_tab = |ui: &mut egui::Ui,
                                                  tab: &crate::tabs::Tab,
                                                  actions: &mut Vec<UiAction>| {
                                // truncate_chars, not String::truncate — see the
                                // Me strip above; the hazard is identical here.
                                let title = truncate_chars(
                                    &tab.webview
                                        .page_title()
                                        .filter(|t| !t.is_empty())
                                        .unwrap_or_else(|| tab.location.clone()),
                                    24,
                                );
                                let label = if tab.crashed {
                                    format!("💥 {title}")
                                } else {
                                    title
                                };
                                if ui
                                    .selectable_label(active == Some(tab.id), label)
                                    .on_hover_text(&tab.location)
                                    .clicked()
                                {
                                    actions.push(UiAction::SelectTab(tab.id));
                                }
                                if ui
                                    .small_button(egui_phosphor::regular::X)
                                    .on_hover_text("Close tab")
                                    .clicked()
                                {
                                    actions.push(UiAction::CloseTab(tab.id));
                                }
                            };
                            for (session_id, session) in sessions.iter() {
                                ui.label(format!(
                                    "{} {}",
                                    egui_phosphor::regular::ROBOT,
                                    session.client
                                ));
                                for tab in tabs.agent_tabs().filter(|t| {
                                    matches!(&t.owner,
                                        crate::tabs::TabOwner::Agent { session_id: sid, .. }
                                            if sid == session_id)
                                }) {
                                    render_tab(ui, tab, &mut actions);
                                }
                                ui.separator();
                            }
                            let orphaned: Vec<_> = tabs
                                .agent_tabs()
                                .filter(|t| {
                                    matches!(&t.owner,
                                        crate::tabs::TabOwner::Agent { session_id: sid, .. }
                                            if !sessions.contains_key(sid))
                                })
                                .collect();
                            if !orphaned.is_empty() {
                                ui.label(format!(
                                    "{} disconnected",
                                    egui_phosphor::regular::PLUGS
                                ));
                                for tab in orphaned {
                                    render_tab(ui, tab, &mut actions);
                                }
                                ui.separator();
                            }
                        }
                    },
                });
            });

            let available = ctx.available_rect();
            shared.toolbar_height.set(available.min.y);
            shared.refresh_window_title(
                tabs.displayed().map(|tab| match tab.crashed {
                    true => "Tab crashed".to_owned(),
                    false => tab.webview.page_title().unwrap_or_default(),
                }),
            );
            let scale = ctx.pixels_per_point();

            let crashed_tab = tabs
                .displayed()
                .filter(|tab| tab.crashed)
                .map(|tab| tab.id);
            // A panel replaces the page, so the tab is neither painted nor
            // blitted while one is up.
            let displayed = tabs
                .displayed()
                .filter(|tab| !tab.crashed && panel == ChromePanel::None)
                .map(|tab| (tab.webview.clone(), tab.rendering_context.clone()));
            drop(tabs);

            if panel == ChromePanel::Credentials {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let vault = shared.vault.borrow();

                        // The one-shot notices: things a `log::warn!` cannot
                        // tell a user who has no terminal open. Shown here
                        // once, then cleared through the dismiss intent.
                        let mut notice_shown = false;
                        if let Some(notice) = vault.import_notice() {
                            let path = notice.path.display();
                            let text = match notice.source_removed {
                                true => format!(
                                    "Imported credentials from {path} and removed that \
                                     plaintext file — they are encrypted now."
                                ),
                                false => format!(
                                    "Imported credentials from {path}, but that file could \
                                     not be removed: a readable plaintext copy of these \
                                     passwords is still on disk there (now owner-only). \
                                     Delete it yourself."
                                ),
                            };
                            ui.label(text);
                            notice_shown = true;
                        }
                        if vault.key_downgraded() {
                            ui.label(
                                "The OS keychain was unavailable, so the vault key now sits \
                                 in an owner-only file beside the encrypted vault. Anyone \
                                 who can read your home directory can decrypt it.",
                            );
                            notice_shown = true;
                        }
                        if notice_shown {
                            if ui.button("Dismiss").clicked() {
                                actions.push(UiAction::DismissVaultNotice);
                            }
                            ui.separator();
                        }

                        ui.heading(format!("{} Credentials", egui_phosphor::regular::KEY));
                        ui.label(
                            "Saved logins are encrypted at rest and offered back in the \
                             toolbar when you visit a matching site. They are never typed \
                             into a page for you.",
                        );
                        ui.add_space(8.0);

                        let field = |ui: &mut egui::Ui,
                                     label: &str,
                                     id: &'static str,
                                     text: &mut String| {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.add(
                                    egui::TextEdit::singleline(text)
                                        .id(egui::Id::new(id))
                                        .desired_width(320.0),
                                )
                            })
                            .inner
                        };
                        let site = field(ui, "Site", "credential-site", &mut credential_site);
                        if focus_credentials {
                            site.request_focus();
                            focus_credentials = false;
                        }
                        field(
                            ui,
                            "Username",
                            "credential-username",
                            &mut credential_username,
                        );
                        // Masked as it is typed, like every other password
                        // field the user has ever met — and so a screen share
                        // or a shoulder does not capture it on the way in.
                        let secret = ui
                            .horizontal(|ui| {
                                ui.label("Password");
                                ui.add(
                                    egui::TextEdit::singleline(&mut credential_password)
                                        .id(egui::Id::new("credential-password"))
                                        .password(true)
                                        .desired_width(320.0),
                                )
                            })
                            .inner;

                        let complete = !credential_site.trim().is_empty()
                            && !credential_username.trim().is_empty()
                            && !credential_password.is_empty();
                        // Enter in the password field is the save, following
                        // the location bar's lost-focus-then-Enter idiom.
                        let entered = secret.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter));
                        let saved = ui
                            .add_enabled(complete, egui::Button::new("Save"))
                            .clicked();
                        if complete && (saved || entered) {
                            actions.push(UiAction::SaveCredential(CredentialEntry {
                                url: credential_url(&credential_site),
                                username: credential_username.trim().to_owned(),
                                password: std::mem::take(&mut credential_password),
                                cookies: Vec::new(),
                            }));
                            credential_site.clear();
                            credential_username.clear();
                        }

                        ui.separator();
                        ui.heading("Stored");
                        let entries = vault.entries();
                        if entries.is_empty() {
                            ui.label("Nothing saved yet.");
                        }
                        for (index, entry) in entries.iter().enumerate() {
                            let host = entry_host(&entry.url);
                            ui.horizontal(|ui| {
                                let row_label = ui.label(&host);
                                record_rect(
                                    chrome_rects.as_mut(),
                                    "credentials.row",
                                    Some(index),
                                    row_label.rect,
                                );
                                ui.label(&entry.username);
                                // Masked by default: a credentials list that
                                // shows every password at a glance is the
                                // shoulder-surfing surface the vault's
                                // encryption was meant to avoid, so revealing
                                // one is a deliberate, one-at-a-time act.
                                let revealed = revealed_entry == Some(index);
                                let glyph = match revealed {
                                    true => egui_phosphor::regular::EYE_SLASH,
                                    false => egui_phosphor::regular::EYE,
                                };
                                let reveal = ui
                                    .selectable_label(revealed, glyph)
                                    .on_hover_text("Show password");
                                record_rect(
                                    chrome_rects.as_mut(),
                                    "credentials.reveal",
                                    Some(index),
                                    reveal.rect,
                                );
                                if reveal.clicked() {
                                    revealed_entry = match revealed {
                                        true => None,
                                        false => Some(index),
                                    };
                                }
                                match revealed {
                                    true => ui.label(&entry.password),
                                    false => ui.label("••••••••"),
                                };
                                let delete = ui
                                    .button(egui_phosphor::regular::TRASH)
                                    .on_hover_text("Delete");
                                record_rect(
                                    chrome_rects.as_mut(),
                                    "credentials.delete",
                                    Some(index),
                                    delete.rect,
                                );
                                if delete.clicked() {
                                    // A removal renumbers the rows below it,
                                    // so a reveal must not outlive it and
                                    // uncover a different entry.
                                    revealed_entry = None;
                                    actions.push(UiAction::DeleteCredential {
                                        host: host.clone(),
                                        username: entry.username.clone(),
                                    });
                                }
                            });
                        }
                    });
                });
            } else if panel == ChromePanel::History {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let history = shared.history.borrow();
                        let entries = history.entries();

                        ui.heading(format!(
                            "{} History",
                            egui_phosphor::regular::CLOCK_COUNTER_CLOCKWISE
                        ));
                        ui.label(
                            "Pages you visited in your own tabs. A page an agent loaded is \
                             not recorded here, whichever tab it loaded it in.",
                        );
                        ui.add_space(16.0);

                        if entries.is_empty() {
                            ui.label("Nothing visited yet.");
                        }

                        let now = crate::app::now_ms();
                        // The store stays in append order — reversing is a
                        // display choice, so newest-first costs nothing on
                        // the write side and cannot destabilise the order of
                        // two visits that share a millisecond.
                        for (row, entry) in entries.iter().rev().enumerate() {
                            let full = match entry.title.trim().is_empty() {
                                true => entry.url.clone(),
                                false => entry.title.clone(),
                            };
                            let label = truncate_chars(&full, 60);
                            let fill = match row % 2 {
                                1 => ui.visuals().faint_bg_color,
                                _ => egui::Color32::TRANSPARENT,
                            };
                            egui::Frame::new().fill(fill).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // The row itself is the control — there
                                    // is no separate "open" button to hunt
                                    // for, matching how the tab strip already
                                    // behaves.
                                    let row_label = ui
                                        .selectable_label(false, &label)
                                        .on_hover_text(&entry.url);
                                    record_rect(
                                        chrome_rects.as_mut(),
                                        "history.row",
                                        Some(row),
                                        row_label.rect,
                                    );
                                    if row_label.clicked() {
                                        actions.push(UiAction::Go(entry.url.clone()));
                                        actions.push(UiAction::SetPanel(ChromePanel::None));
                                    }
                                    ui.label(
                                        egui::RichText::new(relative_time(
                                            entry.visited_at_ms,
                                            now,
                                        ))
                                        .small(),
                                    );
                                });
                            });
                        }

                        if !entries.is_empty() {
                            ui.add_space(16.0);
                            ui.separator();
                            // Two clicks, in place: no modal exists anywhere
                            // in this chrome and this is not the surface to
                            // introduce the first one. The confirming state
                            // dies with the panel (see `Gui::set_panel`), so
                            // it cannot be completed by a later, unrelated
                            // click.
                            let clear = match confirm_clear_history {
                                true => ui.button(
                                    egui::RichText::new("Confirm clear?")
                                        .color(ui.visuals().error_fg_color),
                                ),
                                false => ui.button("Clear history"),
                            };
                            // Named for which of the two states it is in, so
                            // "the confirm armed" and "the confirm reset" are
                            // observable rather than inferred from what the
                            // store did or did not lose. The two names are
                            // never both present: this is one control.
                            record_rect(
                                chrome_rects.as_mut(),
                                match confirm_clear_history {
                                    true => "history.confirm-clear",
                                    false => "history.clear",
                                },
                                None,
                                clear.rect,
                            );
                            if clear.clicked() {
                                match confirm_clear_history {
                                    true => actions.push(UiAction::ClearHistory),
                                    false => confirm_clear_history = true,
                                }
                            }
                        }
                    });
                });
            } else if panel == ChromePanel::Bookmarks {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let bookmarks = shared.bookmarks.borrow();
                        let entries = bookmarks.entries();

                        ui.heading(format!(
                            "{} Bookmarks",
                            egui_phosphor::regular::BOOKMARKS
                        ));
                        ui.label(
                            "Pages you chose to keep. Only you can add one — no agent \
                             can read this list or write to it.",
                        );
                        ui.add_space(16.0);

                        if entries.is_empty() {
                            ui.label("Nothing bookmarked yet.");
                        }

                        // Newest first, reversed at display time exactly like
                        // the History panel: the store keeps insertion order,
                        // so two bookmarks made in the same millisecond are
                        // not reordered by a sort that cannot tell them apart.
                        for (row, entry) in entries.iter().rev().enumerate() {
                            let full = match entry.title.trim().is_empty() {
                                true => entry.url.clone(),
                                false => entry.title.clone(),
                            };
                            let label = truncate_chars(&full, 60);
                            let fill = match row % 2 {
                                1 => ui.visuals().faint_bg_color,
                                _ => egui::Color32::TRANSPARENT,
                            };
                            egui::Frame::new().fill(fill).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    // The row is the control, same as in the
                                    // History panel — no separate "open".
                                    let row_label = ui
                                        .selectable_label(false, &label)
                                        .on_hover_text(&entry.url);
                                    record_rect(
                                        chrome_rects.as_mut(),
                                        "bookmarks.row",
                                        Some(row),
                                        row_label.rect,
                                    );
                                    if row_label.clicked() {
                                        actions.push(UiAction::Go(entry.url.clone()));
                                        actions.push(UiAction::SetPanel(ChromePanel::None));
                                    }
                                    // Immediate, with no confirming click:
                                    // the credentials panel's Trash button
                                    // already behaves this way, and a
                                    // bookmark is one press away from being
                                    // made again — unlike Clear history,
                                    // which cannot be undone at all.
                                    let remove = ui
                                        .button(egui_phosphor::regular::TRASH)
                                        .on_hover_text("Remove bookmark");
                                    record_rect(
                                        chrome_rects.as_mut(),
                                        "bookmarks.remove",
                                        Some(row),
                                        remove.rect,
                                    );
                                    if remove.clicked() {
                                        actions.push(UiAction::RemoveBookmark(
                                            entry.url.clone(),
                                        ));
                                    }
                                });
                            });
                        }
                    });
                });
            } else if panel == ChromePanel::Downloads {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let downloads = shared.downloads.borrow();
                        let entries = downloads.entries();
                        // Read once per frame rather than per row, so every
                        // row on screen is described against the same instant.
                        let now = crate::app::now_ms();

                        // The one-shot Open failure, above the list and before
                        // the heading's own copy — the same shape the vault's
                        // notices use in the credentials panel, and the only
                        // use of `error_fg_color` in this phase.
                        if let Some((filename, error)) = &download_open_error {
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "Couldn't open {} — {error}",
                                        sanitize_for_display(filename),
                                    ))
                                    .color(ui.visuals().error_fg_color),
                                );
                                let dismiss = ui.button("Dismiss");
                                // Drawn only while a notice exists, so its
                                // presence in the reported rects is itself
                                // the evidence that the failed-Open notice is
                                // on screen — and its absence, that Dismiss
                                // cleared it.
                                record_rect(
                                    chrome_rects.as_mut(),
                                    "downloads.dismiss-error",
                                    None,
                                    dismiss.rect,
                                );
                                if dismiss.clicked() {
                                    actions.push(UiAction::DismissDownloadError);
                                }
                            });
                            ui.separator();
                        }

                        ui.heading(format!(
                            "{} Downloads",
                            egui_phosphor::regular::DOWNLOAD
                        ));
                        ui.label(
                            "Files that finished downloading — including ones an agent \
                             asked for, because they landed on your disk either way. \
                             Opening one hands it to your system's default application.",
                        );
                        ui.add_space(16.0);

                        if entries.is_empty() {
                            ui.label("Nothing downloaded yet.");
                        }

                        // Most-recently-completed first, reversed at display
                        // time exactly like History and Bookmarks: the store
                        // keeps completion order, so two downloads that
                        // finished in the same millisecond are not reordered
                        // by a sort that cannot tell them apart.
                        for (row, entry) in entries.iter().rev().enumerate() {
                            // Sanitized before it is cut, not after: a name
                            // that runs past 60 characters must not be able
                            // to hide an override inside the part that
                            // survives. The row's whole label is a string an
                            // agent chose — see `is_display_unsafe`.
                            let label =
                                truncate_chars(&sanitize_for_display(&entry.filename), 60);
                            // Provenance in the line the row already has,
                            // rather than a badge or a colour: `03-UI-SPEC.md`
                            // reserves accent for a closed list of controls,
                            // and the human deciding whether to press Open
                            // should read this in the same glance as the size
                            // and the age.
                            let mut detail = format!(
                                "{} · {}",
                                human_bytes(entry.bytes),
                                relative_time(entry.completed_at_ms, now),
                            );
                            if entry.requested_by_agent {
                                detail.push_str(" · an agent asked for this");
                            }
                            let fill = match row % 2 {
                                1 => ui.visuals().faint_bg_color,
                                _ => egui::Color32::TRANSPARENT,
                            };
                            egui::Frame::new().fill(fill).show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        // The tooltip shows the path that was
                                        // actually written, not the requested
                                        // name: when a collision was resolved
                                        // those differ, and the written one is
                                        // what Open will hand to the OS.
                                        // Sanitized for the same reason the
                                        // label is — this is a *rendering* of
                                        // the path, and the value handed to
                                        // the opener below is untouched.
                                        let row_label = ui
                                            .label(&label)
                                            .on_hover_text(sanitize_for_display(&entry.path));
                                        record_rect(
                                            chrome_rects.as_mut(),
                                            "downloads.row",
                                            Some(row),
                                            row_label.rect,
                                        );
                                        ui.label(
                                            egui::RichText::new(&detail)
                                                .text_style(egui::TextStyle::Small),
                                        );
                                    });
                                    // The stored path, cloned verbatim. Not
                                    // rebuilt from `entry.filename`, which a
                                    // uniquifying rename may have made name a
                                    // different file, and not the sanitized
                                    // rendering either — see the security note
                                    // on `UiAction::OpenDownload`, and
                                    // `open_action`, which is where the choice
                                    // of field is pinned by a test.
                                    let open = ui.button("Open");
                                    record_rect(
                                        chrome_rects.as_mut(),
                                        "downloads.open",
                                        Some(row),
                                        open.rect,
                                    );
                                    if open.clicked() {
                                        actions.push(open_action(entry));
                                    }
                                    // Immediate, with no confirming click, the
                                    // same as the Bookmarks and Credentials
                                    // Trash buttons. The hover text has to say
                                    // "from list": the file itself is left
                                    // exactly where it is, and copy that
                                    // implied otherwise would be a lie the
                                    // store could not tell.
                                    let remove = ui
                                        .button(egui_phosphor::regular::TRASH)
                                        .on_hover_text("Remove from list");
                                    record_rect(
                                        chrome_rects.as_mut(),
                                        "downloads.remove",
                                        Some(row),
                                        remove.rect,
                                    );
                                    if remove.clicked() {
                                        actions.push(UiAction::RemoveDownload(
                                            entry.path.clone(),
                                        ));
                                    }
                                });
                            });
                        }
                    });
                });
            } else if panel == ChromePanel::Settings {
                // The one panel that is not a scrolling list: a short,
                // fixed-height form, so no `ScrollArea` wraps it.
                egui::CentralPanel::default().show(ctx, |ui| {
                    // Filled from the store on the first frame this panel is
                    // drawn, and not again until it closes and reopens —
                    // otherwise every frame would overwrite what is being
                    // typed. There is no empty state to design for: the form
                    // always shows either the saved engine or the default.
                    if settings_draft_stale {
                        let engine = &shared.settings.borrow().search_engine;
                        settings_name = engine.name.clone();
                        settings_template = engine.url_template.clone();
                        settings_draft_stale = false;
                    }

                    ui.heading(format!("{} Settings", egui_phosphor::regular::GEAR));
                    ui.label(
                        "What the address bar searches when what you typed is not a \
                         web address. Stored in plain text in config.json — no agent \
                         can read it or change it.",
                    );
                    ui.add_space(16.0);

                    let field = |ui: &mut egui::Ui,
                                 label: &str,
                                 id: &'static str,
                                 text: &mut String| {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.add(
                                egui::TextEdit::singleline(text)
                                    .id(egui::Id::new(id))
                                    .desired_width(320.0),
                            )
                        })
                        .inner
                    };
                    field(ui, "Name", "settings-engine-name", &mut settings_name);
                    // Same 320pt width as the credentials fields. A
                    // pathologically long template scrolls inside the field
                    // rather than widening it — `TextEdit::singleline`'s own
                    // behaviour, pinned by
                    // `a_long_template_scrolls_inside_the_field_rather_than_widening_it`
                    // so nothing has to take it on faith.
                    field(ui, "URL template", "settings-engine-template", &mut settings_template);
                    // Persistent, not error-triggered: the rule is stated up
                    // front, so a disabled Save button is explained before it
                    // is met rather than apologised for afterwards.
                    ui.label(
                        egui::RichText::new(
                            "Must be an http:// or https:// URL containing {query} exactly once",
                        )
                        .text_style(egui::TextStyle::Small),
                    );

                    ui.add_space(8.0);
                    let valid = crate::settings::is_valid_template(&settings_template)
                        && !settings_name.trim().is_empty();
                    if ui.add_enabled(valid, egui::Button::new("Save")).clicked() && valid {
                        actions.push(UiAction::SaveSearchEngine(SearchEngine {
                            name: settings_name.trim().to_owned(),
                            // Not trimmed: a template's leading or trailing
                            // space would be a typo the `Url::parse` check
                            // catches, and silently editing what someone
                            // typed into a URL is worse than refusing it.
                            url_template: settings_template.clone(),
                        }));
                    }
                });
            } else if let Some(tab_id) = crashed_tab {
                // Crashed state replaces the page (same shape as "Aw, Snap").
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() * 0.35);
                        ui.heading("💥 This tab crashed");
                        ui.label("The page's rendering process went away.");
                        ui.add_space(8.0);
                        if ui.button("Reload").clicked() {
                            actions.push(UiAction::ReloadCrashed(tab_id));
                        }
                    });
                });
            }

            if let Some((webview, tab_context)) = displayed {
                let width = (available.width() * scale).round().max(1.0) as u32;
                let height = (available.height() * scale).round().max(1.0) as u32;
                let current = webview.size();
                if (current.width as u32, current.height as u32) != (width, height) {
                    webview.resize(PhysicalSize::new(width, height));
                }
                webview.paint();

                if let Some(render_to_parent) = tab_context.render_to_parent_callback() {
                    ctx.layer_painter(LayerId::background()).add(PaintCallback {
                        rect: available,
                        callback: Arc::new(CallbackFn::new(move |info, painter| {
                            let clip = info.viewport_in_pixels();
                            let rect = euclid::Rect::new(
                                euclid::Point2D::new(clip.left_px, clip.from_bottom_px),
                                euclid::Size2D::new(clip.width_px, clip.height_px),
                            );
                            render_to_parent(painter.gl(), rect);
                        })),
                    });
                }
            }
        });

        self.focus_credentials = focus_credentials;
        self.revealed_entry = revealed_entry;
        self.confirm_clear_history = confirm_clear_history;
        self.credential_site = credential_site;
        self.credential_username = credential_username;
        self.credential_password = credential_password;
        self.settings_name = settings_name;
        self.settings_template = settings_template;
        self.settings_draft_stale = settings_draft_stale;
        self.chrome_rects = chrome_rects;

        if self.context.egui_ctx.has_requested_repaint() {
            shared.window.request_redraw();
        }
        actions
    }

    pub fn paint(&mut self, window: &Window) {
        let _ = self.rendering_context.make_current();
        self.rendering_context.prepare_for_rendering();
        self.context.paint(window);
        self.rendering_context.present();
    }
}

impl Drop for Gui {
    fn drop(&mut self) {
        let _ = self.rendering_context.make_current();
        self.context.destroy();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: u64 = 60 * 1_000;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    #[test]
    fn a_short_label_is_left_alone() {
        assert_eq!(truncate_chars("Example", 60), "Example");
    }

    #[test]
    fn a_long_label_is_cut_to_the_limit_with_an_ellipsis() {
        let long = "a".repeat(100);
        let cut = truncate_chars(&long, 60);
        assert_eq!(cut.chars().count(), 61, "60 characters plus the ellipsis");
        assert!(cut.ends_with('…'));
    }

    /// CR-04(c). A `downloads.json` written by a build without the boundary
    /// check can still hold a name that renders as two rows; the panel is
    /// what stops it, so this is asserted on the panel's own path — sanitize
    /// first, then cut.
    #[test]
    fn a_stored_name_that_renders_as_two_rows_displays_sanitized() {
        let stored = "invoice.pdf\n\nSafe \u{2014} from your bank";
        let shown = truncate_chars(&sanitize_for_display(stored), 60);

        assert!(!shown.contains('\n'), "a newline reached the label: {shown:?}");
        assert!(shown.starts_with("invoice.pdf"), "{shown:?}");
        assert!(shown.contains('\u{FFFD}'), "the removal was silent: {shown:?}");
        // The rest of the name is still visible — the point is that it can no
        // longer leave the row it was given, not that it disappears.
        assert!(shown.contains("from your bank"), "{shown:?}");
    }

    /// The other half, and the one a control-character check alone misses:
    /// every character in this name is printable, and it renders as
    /// `report.exe.pdf` without the substitution.
    #[test]
    fn a_stored_name_carrying_a_bidi_override_displays_sanitized() {
        let shown = sanitize_for_display("report\u{202E}fdp.exe");
        assert_eq!(shown, "report\u{FFFD}fdp.exe");
        for character in ['\u{202A}', '\u{202B}', '\u{202C}', '\u{202D}', '\u{202E}',
                          '\u{2066}', '\u{2067}', '\u{2068}', '\u{2069}'] {
            assert!(is_display_unsafe(character), "U+{:04X} was let through", character as u32);
        }
    }

    /// Sanitization is for labels and must not touch an ordinary name — a
    /// filter that mangled real filenames would be traded for the one it
    /// fixed.
    #[test]
    fn an_ordinary_name_is_displayed_unchanged() {
        for name in ["report (1).pdf", "\u{65E5}\u{672C}\u{8A9E}.txt", "a b c.bin", "naïve.csv"] {
            assert_eq!(sanitize_for_display(name), name);
        }
    }

    /// `03-VALIDATION.md`'s T-03-04-01/T-03-04-02 claim a unit test for this
    /// and there was not one. `Open` launches the path this shell wrote, not
    /// the name the request asked for and not the sanitized rendering of
    /// either — the three differ here on purpose.
    #[test]
    fn the_open_button_launches_the_stored_path_verbatim() {
        let entry = crate::downloads::DownloadEntry {
            path: "/home/someone/Downloads/report (1).pdf".to_owned(),
            filename: "report\u{202E}fdp.exe".to_owned(),
            url: "https://example.com/report.pdf".to_owned(),
            bytes: 1024,
            completed_at_ms: 42,
            requested_by_agent: true,
        };

        match open_action(&entry) {
            UiAction::OpenDownload(path) => assert_eq!(
                path, entry.path,
                "the Open button stopped carrying the path that was written",
            ),
            _ => panic!("the Downloads Open button stopped emitting OpenDownload"),
        }
    }

    /// The case `String::truncate` would panic on: a byte limit landing
    /// inside a multibyte character. Every one of these is 3 bytes, so a
    /// 60-*byte* cut of 40 of them would split one in half.
    #[test]
    fn truncation_counts_characters_not_bytes() {
        let title = "日".repeat(40);
        let cut = truncate_chars(&title, 20);
        assert_eq!(cut.chars().count(), 21);
        assert!(cut.starts_with('日'));
    }

    /// The tab strip's own limits, pinned separately from the panels'.
    ///
    /// A page picks its own `<title>`, and both strips render it on the main
    /// thread inside egui's render pass — a panic there takes the whole
    /// browser down, every tab and every agent session with it. The two
    /// limits below are the Me strip's 28 and the Agents strip's 24; each
    /// input is chosen so the corresponding *byte* index falls inside a
    /// character, which is precisely what `String::truncate` panics on.
    #[test]
    fn tab_strip_limits_survive_a_multibyte_title() {
        for limit in [28_usize, 24] {
            // A wholly CJK title: 3 bytes a character, so the byte index the
            // old code used walks off into the middle of the string.
            let cjk = "日".repeat(40);
            let cut = truncate_chars(&cjk, limit);
            assert_eq!(cut.chars().count(), limit + 1);
            assert!(cut.starts_with('日'));
            assert!(cut.ends_with('…'));

            // A character sitting exactly astride the byte index, ASCII either
            // side. This is the shape a repeat-only input can miss: with a
            // uniform width the index sometimes lands on a boundary by luck,
            // and here it never can.
            for astride in ['日', '🦀'] {
                let title = format!("{}{astride}{}", "a".repeat(limit - 1), "b".repeat(40));
                assert!(
                    !title.is_char_boundary(limit),
                    "the byte index must split a character for this case to mean anything",
                );
                let cut = truncate_chars(&title, limit);
                assert_eq!(cut.chars().count(), limit + 1);
                assert!(cut.ends_with('…'));
            }
        }
    }

    #[test]
    fn a_visit_this_minute_reads_as_just_now() {
        assert_eq!(relative_time(1_000, 1_000), "just now");
        assert_eq!(relative_time(0, 59 * 1_000), "just now");
    }

    #[test]
    fn older_visits_read_in_the_largest_unit_that_fits() {
        assert_eq!(relative_time(0, MINUTE), "1m ago");
        assert_eq!(relative_time(0, 59 * MINUTE), "59m ago");
        assert_eq!(relative_time(0, HOUR), "1h ago");
        assert_eq!(relative_time(0, 23 * HOUR), "23h ago");
        assert_eq!(relative_time(0, DAY), "1d ago");
        assert_eq!(relative_time(0, 400 * DAY), "400d ago");
    }

    /// A clock that moved backwards between the visit and now must not
    /// underflow into a nonsense age.
    #[test]
    fn a_visit_in_the_future_reads_as_just_now() {
        assert_eq!(relative_time(10 * DAY, 0), "just now");
    }

    /// Each unit's own boundary, both sides, so an off-by-one in the
    /// comparison chain shows up as a wrong label rather than as a rounding
    /// quibble nobody notices.
    #[test]
    fn a_byte_count_reads_in_the_largest_unit_that_fits() {
        assert_eq!(human_bytes(0), "0 bytes");
        assert_eq!(human_bytes(1), "1 bytes");
        assert_eq!(human_bytes(1_023), "1023 bytes");
        assert_eq!(human_bytes(1_024), "1.0 KB");
        assert_eq!(human_bytes(1_048_575), "1024.0 KB");
        assert_eq!(human_bytes(1_048_576), "1.0 MB");
        assert_eq!(human_bytes(1_073_741_824), "1.0 GB");
    }

    /// The largest count `download`'s own cap could ever produce, and the
    /// largest a hand-edited `downloads.json` could hold. Neither may panic
    /// or overflow into a negative-looking size.
    #[test]
    fn an_absurd_byte_count_still_formats() {
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
        assert!(human_bytes(u64::MAX).ends_with(" GB"));
    }

    /// `03-UI-SPEC.md`'s one `backstop` item, made an assertion rather than
    /// an assumption.
    ///
    /// The Settings panel's URL-template field is a
    /// `TextEdit::singleline(..).desired_width(320.0)`, the same shape and
    /// the same width as the credentials panel's fields. The spec assumed a
    /// pathologically long template would scroll inside that field rather
    /// than widening it and pushing the panel out of the window, but nothing
    /// had ever rendered one. This lays out both the real field and a
    /// 4,000-character template through egui's own layout pass and measures
    /// the result: the field must not grow, and it must not exceed the
    /// available width.
    ///
    /// `__run_test_ui` is egui's own headless harness — the one its
    /// documentation examples run under — so this needs no window, no GL
    /// context and no display.
    #[test]
    fn a_long_template_scrolls_inside_the_field_rather_than_widening_it() {
        // Longer than any real search template, and with no spaces, so no
        // word-wrapping opportunity exists to rescue the layout.
        let mut pathological = format!(
            "https://example.com/search?q={{query}}&filter={}",
            "a".repeat(4_000)
        );
        let mut ordinary = "https://duckduckgo.com/?q={query}".to_owned();

        let (long_width, short_width, available) = {
            let mut measured = (0.0_f32, 0.0_f32, 0.0_f32);
            egui::__run_test_ui(|ui| {
                let field = |ui: &mut egui::Ui, id: &'static str, text: &mut String| {
                    ui.horizontal(|ui| {
                        ui.label("URL template");
                        ui.add(
                            egui::TextEdit::singleline(text)
                                .id(egui::Id::new(id))
                                .desired_width(320.0),
                        )
                    })
                    .inner
                };
                let available_width = ui.available_width();
                let long = field(ui, "settings-engine-template", &mut pathological);
                let short = field(ui, "settings-engine-template-control", &mut ordinary);
                measured = (long.rect.width(), short.rect.width(), available_width);
            });
            measured
        };

        assert_eq!(
            long_width, short_width,
            "a 4,000-character template widened the field ({long_width} vs {short_width})"
        );
        // Measured when this test was written: both fields lay out at exactly
        // 320.0 points on a 10,000-point canvas, so the content length does
        // not reach the layout at all. Asserting the literal rather than
        // `<= available` because the harness's canvas is deliberately huge,
        // which would make an available-width bound almost vacuous.
        assert_eq!(long_width, 320.0, "the field is no longer the requested 320 points");
        assert!(available > long_width, "the harness gave the field no room to grow into");
    }
}
