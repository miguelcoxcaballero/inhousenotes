//! egui HUD: status, speedometer, weapon and ammo, crosshair, inventory,
//! controls help and debug stats.

use forja_core::World;
use forja_gameplay::ItemCategory;

use crate::game::combat::Combat;
use crate::game::env::TimeOfDay;
use crate::game::{Hud, Intent};

#[derive(Default)]
pub struct UiState {
    pub inventory_open: bool,
    pub help: bool,
    pub debug: bool,
    pub fps: f32,
    pub render_stats: forja_render::RenderStats,
    pub adapter: String,
}

/// Loading screen: title, stage, progress bar and elapsed time.
pub fn draw_loading(ctx: &egui::Context, title: &str, stage: &str, fraction: f32, elapsed: Option<f32>) {
    let screen = ctx.content_rect();
    let p = ctx.layer_painter(egui::LayerId::background());
    let c = screen.center();
    let w = (screen.width() * 0.5).clamp(240.0, 720.0);
    p.text(egui::pos2(c.x, c.y - 70.0), egui::Align2::CENTER_BOTTOM, "FORJA", egui::FontId::proportional(42.0), egui::Color32::from_rgb(235, 200, 140));
    p.text(egui::pos2(c.x, c.y - 30.0), egui::Align2::CENTER_BOTTOM, title, egui::FontId::proportional(20.0), egui::Color32::WHITE);
    let bar = egui::Rect::from_center_size(egui::pos2(c.x, c.y), egui::vec2(w, 10.0));
    p.rect_filled(bar, 5.0, egui::Color32::from_gray(45));
    let f = fraction.clamp(0.0, 1.0);
    if f > 0.0 {
        let fill = egui::Rect::from_min_size(bar.min, egui::vec2((w * f).max(10.0), bar.height()));
        p.rect_filled(fill, 5.0, egui::Color32::from_rgb(222, 150, 60));
    }
    let detail = match elapsed {
        Some(t) => format!("{stage}  ·  {:.0} %  ·  {t:.0} s", f * 100.0),
        None => format!("{stage}  ·  {:.0} %", f * 100.0),
    };
    p.text(egui::pos2(c.x, c.y + 22.0), egui::Align2::CENTER_TOP, detail, egui::FontId::proportional(15.0), egui::Color32::from_gray(190));
    // Keep animating while the worker runs.
    ctx.request_repaint();
}

pub fn draw(ctx: &egui::Context, world: &mut World, ui: &mut UiState) {
    let hud = world.res::<Hud>().clone();
    let screen = ctx.content_rect();
    let painter = ctx.layer_painter(egui::LayerId::background());
    let shadow = |p: &egui::Painter, pos: egui::Pos2, align: egui::Align2, text: &str, size: f32, color: egui::Color32| {
        p.text(pos + egui::vec2(1.5, 1.5), align, text, egui::FontId::proportional(size), egui::Color32::from_black_alpha(180));
        p.text(pos, align, text, egui::FontId::proportional(size), color);
    };
    // Top-left: map and hints.
    shadow(&painter, egui::pos2(18.0, 14.0), egui::Align2::LEFT_TOP, &hud.map, 18.0, egui::Color32::WHITE);
    shadow(&painter, egui::pos2(18.0, 38.0), egui::Align2::LEFT_TOP, &format!("{:.0} fps · F10 help", ui.fps), 13.0, egui::Color32::from_gray(200));
    for (i, (m, t)) in hud.messages.iter().rev().take(5).enumerate() {
        let a = (t.min(1.0) * 255.0) as u8;
        shadow(&painter, egui::pos2(18.0, 70.0 + i as f32 * 22.0), egui::Align2::LEFT_TOP, m, 15.0, egui::Color32::from_rgba_unmultiplied(255, 236, 170, a));
    }
    if let Some(h) = &hud.interact_hint {
        shadow(&painter, screen.center_bottom() - egui::vec2(0.0, 120.0), egui::Align2::CENTER_CENTER, h, 18.0, egui::Color32::WHITE);
    }
    // Bottom-left: health and armor.
    let bar = |p: &egui::Painter, y: f32, frac: f32, color: egui::Color32| {
        let r = egui::Rect::from_min_size(egui::pos2(20.0, screen.bottom() - y), egui::vec2(220.0, 9.0));
        p.rect_filled(r, 3.0, egui::Color32::from_black_alpha(150));
        p.rect_filled(egui::Rect::from_min_size(r.min, egui::vec2(220.0 * frac.clamp(0.0, 1.0), 9.0)), 3.0, color);
    };
    bar(&painter, 44.0, hud.health / 100.0, egui::Color32::from_rgb(95, 200, 110));
    bar(&painter, 30.0, hud.armor / 100.0, egui::Color32::from_rgb(90, 160, 230));
    if hud.health <= 0.0 {
        shadow(&painter, screen.center(), egui::Align2::CENTER_CENTER, "WASTED", 64.0, egui::Color32::from_rgb(220, 40, 40));
    }
    // Bottom-right: weapon, ammo, speedometer.
    let br = screen.right_bottom() - egui::vec2(24.0, 24.0);
    if let Some(wpn) = &hud.weapon {
        shadow(&painter, br - egui::vec2(0.0, 30.0), egui::Align2::RIGHT_BOTTOM, wpn, 16.0, egui::Color32::WHITE);
        let ammo = match (hud.ammo, hud.reloading) {
            (_, Some(p)) => format!("reloading {:.0}%", p * 100.0),
            (Some((m, r)), None) => format!("{m} / {r}"),
            (None, None) => "∞".into(),
        };
        shadow(&painter, br, egui::Align2::RIGHT_BOTTOM, &ammo, 26.0, egui::Color32::WHITE);
    }
    if let Some(kmh) = hud.speed_kmh {
        let c = br - egui::vec2(0.0, 90.0);
        shadow(&painter, c, egui::Align2::RIGHT_BOTTOM, &format!("{kmh:.0} km/h"), 34.0, egui::Color32::WHITE);
        let gear = match hud.gear {
            g if g < 0 => "R".to_string(),
            0 => "N".to_string(),
            g => g.to_string(),
        };
        shadow(&painter, c + egui::vec2(0.0, 22.0), egui::Align2::RIGHT_BOTTOM, &format!("gear {gear} · {:.0} rpm", hud.rpm), 14.0, egui::Color32::from_gray(210));
    }
    // Crosshair.
    if hud.weapon.is_some() && hud.speed_kmh.is_none() {
        let c = screen.center();
        let gap = if hud.aiming { 4.0 } else { 9.0 };
        let stroke = egui::Stroke::new(2.0, egui::Color32::from_white_alpha(230));
        for d in [egui::vec2(1.0, 0.0), egui::vec2(-1.0, 0.0), egui::vec2(0.0, 1.0), egui::vec2(0.0, -1.0)] {
            painter.line_segment([c + d * gap, c + d * (gap + 7.0)], stroke);
        }
        if hud.hit_marker > 0.0 {
            let s = egui::Stroke::new(2.0, egui::Color32::from_rgba_unmultiplied(255, 80, 60, (hud.hit_marker * 255.0) as u8));
            for d in [egui::vec2(1.0, 1.0), egui::vec2(-1.0, 1.0), egui::vec2(1.0, -1.0), egui::vec2(-1.0, -1.0)] {
                painter.line_segment([c + d * 6.0, c + d * 12.0], s);
            }
        }
    }
    if ui.inventory_open {
        inventory_window(ctx, world);
    }
    if ui.help {
        egui::Window::new("Controls").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).collapsible(false).resizable(false).show(ctx, |u| {
            egui::Grid::new("help").striped(true).show(u, |g| {
                for (k, v) in [
                    ("WASD / mouse", "move / look"),
                    ("Shift", "sprint"),
                    ("Space", "jump · handbrake"),
                    ("F", "enter / exit vehicle"),
                    ("Left / right mouse", "fire / aim"),
                    ("R", "reload"),
                    ("1-6", "weapon wheel categories"),
                    ("H", "holster"),
                    ("Tab", "inventory"),
                    ("F1", "physics debug"),
                    ("F2", "next map"),
                    ("F3", "time of day +3 h (night at 21-6 h)"),
                    ("F9", "graphics options (presets)"),
                    ("F11", "frame profiler"),
                    ("F12", "screenshot"),
                    ("Esc", "release mouse / quit"),
                ] {
                    g.label(egui::RichText::new(k).strong());
                    g.label(v);
                    g.end_row();
                }
            });
        });
    }
    if ui.debug {
        let tod = world.res::<TimeOfDay>().hours;
        egui::Window::new("Debug").anchor(egui::Align2::RIGHT_TOP, [-12.0, 12.0]).resizable(false).show(ctx, |u| {
            let s = &ui.render_stats;
            u.label(format!("GPU: {}", ui.adapter));
            u.label(format!("draws {} · shadow draws {} · instances {} (culled {})", s.draw_calls, s.shadow_draws, s.instances_drawn, s.instances_culled));
            u.label(format!("triangles {:.2} M · render cpu {:.2} ms", s.triangles as f64 / 1e6, s.cpu_ms));
            u.label(format!("physics {:.2} ms · bodies {}", hud.physics_ms, hud.bodies));
            u.label(format!("time {:02}:{:02}", tod as u32, ((tod.fract()) * 60.0) as u32));
            for a in &hud.attribution {
                u.small(a);
            }
        });
    }
}

fn inventory_window(ctx: &egui::Context, world: &mut World) {
    let mut action: Option<(u8, String)> = None;
    {
        let c = world.res::<Combat>();
        let weight = c.inventory.weight(&c.items);
        egui::Window::new("Inventory").anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).collapsible(false).resizable(false).show(ctx, |u| {
            u.add(egui::ProgressBar::new(weight / c.inventory.max_weight).text(format!("{weight:.1} / {:.0} kg", c.inventory.max_weight)));
            u.separator();
            egui::Grid::new("slots").num_columns(6).spacing([6.0, 6.0]).show(u, |g| {
                for (i, slot) in c.inventory.slots.iter().enumerate() {
                    let (text, item) = match slot {
                        Some(s) => {
                            let def = c.items.get(&s.item);
                            let name = def.map(|d| d.name.as_str()).unwrap_or(&s.item);
                            (if s.count > 1 { format!("{name}\n×{}", s.count) } else { name.to_string() }, Some((s.item.clone(), def.map(|d| d.category))))
                        }
                        None => (String::new(), None),
                    };
                    let resp = g.add_sized([104.0, 46.0], egui::Button::new(egui::RichText::new(text).size(12.0)));
                    if let Some((id, cat)) = item {
                        let resp = resp.on_hover_text("click: use / equip · right click: drop");
                        if resp.clicked() {
                            action = Some((if cat == Some(ItemCategory::Weapon) { 1 } else { 0 }, id.clone()));
                        }
                        if resp.secondary_clicked() {
                            action = Some((2, id));
                        }
                    }
                    if i % 6 == 5 {
                        g.end_row();
                    }
                }
            });
            u.separator();
            u.horizontal(|u| {
                for (cat, list, sel) in &c.wheel.slots {
                    if !list.is_empty() {
                        u.label(format!("{cat:?}: {}", list.get(*sel).cloned().unwrap_or_default()));
                    }
                }
            });
        });
    }
    if let Some((kind, id)) = action {
        let intent = world.res_mut::<Intent>();
        match kind {
            0 => intent.use_item = Some(id),
            1 => intent.equip_item = Some(id),
            _ => intent.drop_item = Some(id),
        }
    }
}
