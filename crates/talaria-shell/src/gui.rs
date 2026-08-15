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
}

pub struct Gui {
    context: EguiGlow,
    rendering_context: Rc<WindowRenderingContext>,
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
        Self { context, rendering_context }
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
    pub fn focus_location_bar(&self) {
        self.context.egui_ctx.memory_mut(|memory| {
            memory.request_focus(egui::Id::new("location-bar"));
        });
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

                    if let Some(tab) = tabs.displayed() {
                        if tab.webview.load_status() != servo::LoadStatus::Complete
                            && !tab.crashed
                        {
                            ui.spinner();
                        }
                    }
                    if let Some(tab) = tabs.displayed_mut() {
                        let response = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::singleline(&mut tab.location)
                                .id(egui::Id::new("location-bar"))
                                .hint_text("Search or enter address"),
                        );
                        if response.changed() {
                            tab.location_dirty = true;
                        }
                        if response.lost_focus()
                            && ui.input(|input| input.key_pressed(egui::Key::Enter))
                        {
                            actions.push(UiAction::Go(tab.location.clone()));
                        }
                    } else {
                        ui.label("No tab open");
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
                            ui.label(format!("{} connected", sessions.len()));
                            ui.separator();
                            let active = tabs.active_id(ViewMode::Agents);
                            for tab in tabs.agent_tabs() {
                                let mut title = tab
                                    .webview
                                    .page_title()
                                    .filter(|t| !t.is_empty())
                                    .unwrap_or_else(|| tab.location.clone());
                                title.truncate(24);
                                let mut label = format!("{} {} · {title}", egui_phosphor::regular::ROBOT, tab.owner.label());
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
                        }
                    },
                });
            });

            let available = ctx.available_rect();
            shared.toolbar_height.set(available.min.y);
            let scale = ctx.pixels_per_point();

            let crashed_tab = tabs
                .displayed()
                .filter(|tab| tab.crashed)
                .map(|tab| tab.id);
            let displayed = tabs
                .displayed()
                .filter(|tab| !tab.crashed)
                .map(|tab| (tab.webview.clone(), tab.rendering_context.clone()));
            drop(tabs);

            if let Some(tab_id) = crashed_tab {
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
