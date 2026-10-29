//! Pause menu (Esc): the game stops, the mouse is released and the world
//! is dimmed behind a menu with Reanudar, Opciones (graphics and
//! controls), Controles and Salir (with a confirmation). Esc again
//! resumes; nothing quits on Esc (closing the window or Alt+F4 still do).

use std::path::{Path, PathBuf};

use forja_input::ActionMap;
use serde::{Deserialize, Serialize};

use crate::perf::{self, PerfState};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Screen {
    #[default]
    Main,
    Options,
    Help,
    ConfirmQuit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OptionsTab {
    #[default]
    Graphics,
    Controls,
}

/// What the menu asks the game loop to do this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    Resume,
    Quit,
}

/// The player's control settings, saved in `data/controls.toml` (the key
/// bindings stay in `data/input.toml`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Controls {
    pub mouse_sensitivity: f32,
    pub invert_y: bool,
}

impl Default for Controls {
    fn default() -> Self {
        Self { mouse_sensitivity: 0.0025, invert_y: false }
    }
}

impl Controls {
    /// The saved settings, or those of the key bindings file.
    pub fn load(path: &Path, map: &ActionMap) -> Self {
        let base = Controls { mouse_sensitivity: map.mouse_sensitivity, invert_y: map.invert_y };
        match std::fs::read_to_string(path) {
            Ok(t) => toml::from_str(&t).unwrap_or(base),
            Err(_) => base,
        }
    }

    pub fn apply(&self, map: &mut ActionMap) {
        map.mouse_sensitivity = self.mouse_sensitivity.clamp(0.0002, 0.02);
        map.invert_y = self.invert_y;
    }
}

pub struct PauseMenu {
    pub open: bool,
    pub screen: Screen,
    pub tab: OptionsTab,
    pub controls: Controls,
    controls_path: PathBuf,
    message: Option<String>,
}

impl PauseMenu {
    pub fn new(controls_path: PathBuf, map: &ActionMap) -> Self {
        Self { open: false, screen: Screen::Main, tab: OptionsTab::Graphics, controls: Controls::load(&controls_path, map), controls_path, message: None }
    }

    pub fn open(&mut self) {
        self.open = true;
        self.screen = Screen::Main;
        self.message = None;
    }

    fn save(&mut self, perf: &mut PerfState) {
        perf.save();
        let text = toml::to_string_pretty(&self.controls).unwrap_or_default();
        let ctl = match std::fs::write(&self.controls_path, text) {
            Ok(()) => String::new(),
            Err(e) => format!(" · {}: {e}", self.controls_path.display()),
        };
        self.message = Some(format!("{}{ctl}", perf.message.take().unwrap_or_default()));
    }
}

fn big_button(u: &mut egui::Ui, text: &str) -> bool {
    u.add_sized([260.0, 40.0], egui::Button::new(egui::RichText::new(text).size(19.0))).clicked()
}

/// Draws the menu over a dimmed screen; `map` receives control changes.
pub fn draw(ctx: &egui::Context, menu: &mut PauseMenu, perf_state: &mut PerfState, map: &mut ActionMap) -> Action {
    if !menu.open {
        return Action::None;
    }
    let mut action = Action::None;
    let screen = ctx.content_rect();
    egui::Area::new(egui::Id::unique("pause_menu")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |u| {
        // Dim the game and swallow clicks meant for it.
        let (rect, _) = u.allocate_exact_size(screen.size(), egui::Sense::click_and_drag());
        u.painter().rect_filled(rect, 0.0, egui::Color32::from_black_alpha(150));
        let width = match menu.screen {
            Screen::Options => (screen.width() - 32.0).min(720.0),
            Screen::Help => (screen.width() - 32.0).min(560.0),
            _ => 340.0,
        };
        // The main and quit screens sit in the middle; options and help
        // take the full height.
        let full = (screen.height() - 48.0).max(200.0);
        let height = match menu.screen {
            Screen::Main => full.min(360.0),
            Screen::ConfirmQuit => full.min(200.0),
            _ => full,
        };
        let panel = egui::Rect::from_center_size(screen.center(), egui::vec2(width, height));
        u.scope_builder(egui::UiBuilder::new().max_rect(panel).layout(egui::Layout::top_down(egui::Align::Center)), |u| {
            egui::Frame::window(u.style()).inner_margin(18.0).show(u, |u| {
                u.set_max_width(width - 36.0);
                match menu.screen {
                    Screen::Main => {
                        u.vertical_centered(|u| {
                            u.label(egui::RichText::new("PAUSA").size(30.0).strong().color(egui::Color32::from_rgb(235, 200, 140)));
                            u.add_space(12.0);
                            if big_button(u, "Reanudar") {
                                action = Action::Resume;
                            }
                            u.add_space(6.0);
                            if big_button(u, "Opciones") {
                                menu.screen = Screen::Options;
                            }
                            u.add_space(6.0);
                            if big_button(u, "Controles / Ayuda") {
                                menu.screen = Screen::Help;
                            }
                            u.add_space(6.0);
                            if big_button(u, "Salir") {
                                menu.screen = Screen::ConfirmQuit;
                            }
                            u.add_space(8.0);
                            u.small("Esc: reanudar");
                        });
                    }
                    Screen::ConfirmQuit => {
                        u.vertical_centered(|u| {
                            u.label(egui::RichText::new("¿Salir del juego?").size(24.0).strong());
                            u.add_space(14.0);
                            if big_button(u, "Sí, salir") {
                                action = Action::Quit;
                            }
                            u.add_space(6.0);
                            if big_button(u, "No, volver") {
                                menu.screen = Screen::Main;
                            }
                        });
                    }
                    Screen::Help => {
                        u.label(egui::RichText::new("Controles").size(24.0).strong());
                        u.add_space(6.0);
                        egui::ScrollArea::vertical().max_height(height - 140.0).show(u, crate::ui::controls_grid);
                        u.add_space(8.0);
                        u.small("Las teclas se cambian en data\\input.toml.");
                        if u.button("Volver").clicked() {
                            menu.screen = Screen::Main;
                        }
                    }
                    Screen::Options => {
                        u.horizontal(|u| {
                            u.label(egui::RichText::new("Opciones").size(24.0).strong());
                            u.add_space(16.0);
                            u.selectable_value(&mut menu.tab, OptionsTab::Graphics, egui::RichText::new("Gráficos").size(17.0));
                            u.selectable_value(&mut menu.tab, OptionsTab::Controls, egui::RichText::new("Controles").size(17.0));
                        });
                        u.separator();
                        egui::ScrollArea::vertical().max_height(height - 150.0).show(u, |u| match menu.tab {
                            OptionsTab::Graphics => perf::graphics_options(u, perf_state),
                            OptionsTab::Controls => {
                                let before = menu.controls.clone();
                                egui::Grid::new("controls").num_columns(2).striped(true).show(u, |g| {
                                    g.label("Sensibilidad del ratón");
                                    let mut mult = menu.controls.mouse_sensitivity / 0.0025;
                                    if g.add(egui::Slider::new(&mut mult, 0.2..=4.0).logarithmic(true).suffix("x").fixed_decimals(2)).changed() {
                                        menu.controls.mouse_sensitivity = mult * 0.0025;
                                    }
                                    g.end_row();
                                    g.label("Invertir eje Y");
                                    g.checkbox(&mut menu.controls.invert_y, "");
                                    g.end_row();
                                });
                                if menu.controls != before {
                                    menu.controls.apply(map);
                                }
                                u.add_space(8.0);
                                u.label(egui::RichText::new("Teclas").strong());
                                crate::ui::controls_grid(u);
                            }
                        });
                        u.separator();
                        u.horizontal(|u| {
                            if u.button("Guardar").clicked() {
                                menu.save(perf_state);
                            }
                            if u.button("Volver").clicked() {
                                menu.screen = Screen::Main;
                            }
                            if let Some(m) = &menu.message {
                                u.small(m);
                            }
                        });
                    }
                }
            });
        });
    });
    action
}
