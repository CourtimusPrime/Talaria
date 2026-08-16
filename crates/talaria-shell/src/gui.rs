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
use talaria_protocol::CredentialEntry;
use winit::dpi::PhysicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use crate::app::Shared;
use crate::tabs::ViewMode;

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
    /// Open or close the credentials panel. Even this goes through the
    /// round trip: teaching a later reader that "some" mutations are legal
    /// inline is how the anti-pattern comes back.
    SetCredentialsPanel(bool),
    /// Clear the vault's one-shot notices, once the chrome has shown them.
    DismissVaultNotice,
}

pub struct Gui {
    context: EguiGlow,
    rendering_context: Rc<WindowRenderingContext>,
    /// Ctrl+L: select the whole URL on the frame the bar gains focus, so
    /// typing replaces it (browser behaviour) instead of appending.
    select_location: bool,
    /// Whether the credentials panel has replaced the page. View state, so it
    /// lives here rather than on `Shared`.
    credentials_open: bool,
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
            credentials_open: false,
            focus_credentials: false,
            credential_site: String::new(),
            credential_username: String::new(),
            credential_password: String::new(),
            revealed_entry: None,
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

    /// Whether the credentials panel is currently showing. Read by the
    /// Ctrl+K shortcut so the toggle it emits is an intent like any other.
    pub fn credentials_open(&self) -> bool {
        self.credentials_open
    }

    /// Show or hide the credentials panel. Only [`UiAction::SetCredentialsPanel`]
    /// calls this, from `apply_ui_actions` — never the egui closure.
    ///
    /// A revealed password never survives the panel closing, and opening puts
    /// the caret in the first field so the whole panel is keyboard-reachable.
    pub fn set_credentials_panel(&mut self, open: bool) {
        self.credentials_open = open;
        self.revealed_entry = None;
        self.focus_credentials = open;
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
        let credentials_open = self.credentials_open;
        let mut focus_credentials = std::mem::take(&mut self.focus_credentials);
        let mut revealed_entry = self.revealed_entry;
        let mut credential_site = std::mem::take(&mut self.credential_site);
        let mut credential_username = std::mem::take(&mut self.credential_username);
        let mut credential_password = std::mem::take(&mut self.credential_password);

        self.context.run(&shared.window, |ctx| {
            let mut tabs = shared.tabs.borrow_mut();
            let mode = tabs.mode;

            egui::Panel::top("toolbar").show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let me = mode == ViewMode::Me;
                    if ui.selectable_label(me, "Me").clicked() && !me {
                        actions.push(UiAction::SwitchMode(ViewMode::Me));
                    }
                    if ui.selectable_label(!me, "Agents").clicked() && me {
                        actions.push(UiAction::SwitchMode(ViewMode::Agents));
                    }
                    ui.separator();

                    let has_tab = tabs.displayed().is_some();
                    if ui.add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_LEFT)).on_hover_text("Back").clicked() {
                        actions.push(UiAction::Back);
                    }
                    if ui.add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_RIGHT)).on_hover_text("Forward").clicked() {
                        actions.push(UiAction::Forward);
                    }
                    if ui
                        .add_enabled(has_tab, egui::Button::new(egui_phosphor::regular::ARROW_CLOCKWISE))
                        .on_hover_text("Reload (Ctrl+R)")
                        .clicked()
                    {
                        actions.push(UiAction::Reload);
                    }

                    let new_tab = ui.button(egui_phosphor::regular::PLUS).on_hover_text("New tab (Ctrl+T)").clicked();
                    if new_tab {
                        actions.push(UiAction::NewTab);
                    }

                    // The credentials surface is human-only. It opens from
                    // this button and from Ctrl+K, and from nowhere else: no
                    // control-socket command reaches it, so an agent cannot
                    // drive the human's own takeover affordance as a tool.
                    if ui
                        .add(
                            egui::Button::new(egui_phosphor::regular::KEY)
                                .selected(credentials_open),
                        )
                        .on_hover_text("Credentials (Ctrl+K)")
                        .clicked()
                    {
                        actions.push(UiAction::SetCredentialsPanel(!credentials_open));
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
                            let mut label = tab
                                .webview
                                .page_title()
                                .filter(|t| !t.is_empty())
                                .unwrap_or_else(|| tab.location.clone());
                            label.truncate(28);
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
                                let mut title = tab
                                    .webview
                                    .page_title()
                                    .filter(|t| !t.is_empty())
                                    .unwrap_or_else(|| tab.location.clone());
                                title.truncate(24);
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
            // The credentials panel replaces the page, so the tab is neither
            // painted nor blitted while it is up.
            let displayed = tabs
                .displayed()
                .filter(|tab| !tab.crashed && !credentials_open)
                .map(|tab| (tab.webview.clone(), tab.rendering_context.clone()));
            drop(tabs);

            if credentials_open {
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
                                ui.label(&host);
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
                                if ui
                                    .selectable_label(revealed, glyph)
                                    .on_hover_text("Show password")
                                    .clicked()
                                {
                                    revealed_entry = match revealed {
                                        true => None,
                                        false => Some(index),
                                    };
                                }
                                match revealed {
                                    true => ui.label(&entry.password),
                                    false => ui.label("••••••••"),
                                };
                                if ui
                                    .button(egui_phosphor::regular::TRASH)
                                    .on_hover_text("Delete")
                                    .clicked()
                                {
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
        self.credential_site = credential_site;
        self.credential_username = credential_username;
        self.credential_password = credential_password;

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
