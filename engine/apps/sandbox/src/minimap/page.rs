//! The pause menu's map: the whole world north up, dragged with the mouse
//! and zoomed with the wheel; a click sets the waypoint (a click on it, or
//! a right click, removes it) and the GPS route is drawn to it, as on GTA
//! V's map. A legend explains the icons.

use egui::{Color32, Pos2, Rect, Stroke, pos2, vec2};
use forja_core::World;
use forja_math::Vec2;

use super::data::{PoiKind, road};
use super::{Blip, BlipKind, MISSION_COLOR, Minimap, RadarView, Scene, WAYPOINT_COLOR, draw_arrow, draw_blip, draw_blips, raster, route_points, tile_meshes};

/// Where the map page looks.
#[derive(Clone, Copy, Debug)]
pub struct PageState {
    pub centre: Vec2,
    /// Screen points per metre.
    pub scale: f32,
    ready: bool,
    /// Zoom a capture asked for.
    pub(super) preset: Option<f32>,
}

impl Default for PageState {
    fn default() -> Self {
        Self { centre: Vec2::ZERO, scale: 0.45, ready: false, preset: None }
    }
}

/// What the page asks the menu to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAction {
    None,
    Back,
}

/// "850 m" or "1,2 km".
pub fn distance_text(m: f32) -> String {
    if m < 950.0 { format!("{:.0} m", (m / 10.0).round() * 10.0) } else { format!("{:.1} km", m / 1000.0).replace('.', ",") }
}

/// Places shown at a zoom: everything close up, only the services the
/// radar shows farther out, only airports over the whole island.
fn shows(kind: PoiKind, scale: f32) -> bool {
    match kind {
        PoiKind::Airport => true,
        PoiKind::Hospital | PoiKind::Police | PoiKind::Station => scale >= 0.07,
        k if k.on_radar() => scale >= 0.2,
        _ => scale >= 0.45,
    }
}

/// A round length (metres) that takes about `points` on screen.
fn scale_bar(scale: f32, points: f32) -> f32 {
    let want = points / scale;
    let pow = 10f32.powf(want.log10().floor());
    [1.0, 2.0, 5.0, 10.0].iter().map(|k| k * pow).rfind(|l| *l <= want).unwrap_or(pow)
}

/// Draws the map over `rect`. `fresh`: the page was just opened (centre on
/// the player).
pub fn draw(ui: &mut egui::Ui, world: &mut World, rect: Rect, fresh: bool) -> PageAction {
    if world.resources.get::<Minimap>().is_none() {
        ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "Este mapa no tiene plano.", egui::FontId::proportional(20.0), Color32::WHITE);
        return PageAction::None;
    }
    let ctx = ui.ctx().clone();
    let (dt, time) = ctx.input(|i| (i.stable_dt.min(0.1), i.time));
    let mut action = PageAction::None;
    world.scope::<Minimap, _>(|world, mm| {
        let scene = Scene::gather(world, mm.last_camera);
        let player = scene.ground();
        mm.update_route(player, dt);
        mm.tiles.pump(&ctx);
        let (lo, hi) = (mm.map.min, mm.map.max);
        let extent = (hi - lo).max(Vec2::splat(50.0));
        let min_scale = (rect.width() / extent.x).min(rect.height() / extent.y) * 0.8;
        let max_scale = 4.0f32.max(min_scale * 2.0);
        if fresh || !mm.page.ready {
            mm.page = PageState { centre: player, scale: mm.page.preset.unwrap_or(0.45).clamp(min_scale, max_scale), ready: true, preset: mm.page.preset };
        }
        let legend_w = 250.0f32.min(rect.width() * 0.34);
        let legend = Rect::from_min_size(pos2(rect.right() - legend_w - 14.0, rect.top() + 14.0), vec2(legend_w, (rect.height() - 28.0).min(690.0)));
        let resp = ui.interact(rect, ui.scope_id().with("pause_map"), egui::Sense::click_and_drag());
        let pointer = resp.hover_pos().filter(|p| !legend.contains(*p));
        let mut st = mm.page;
        if resp.dragged() {
            let d = resp.drag_delta();
            st.centre -= Vec2::new(d.x, d.y) / st.scale;
        }
        if let Some(m) = pointer {
            let (wheel, pinch) = ctx.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let f = (wheel * 0.0045).exp() * pinch;
            if (f - 1.0).abs() > 1e-4 {
                // The ground under the pointer stays under it.
                let before = RadarView::north_up(rect, st.centre, st.scale).unproject(m);
                st.scale = (st.scale * f).clamp(min_scale, max_scale);
                let off = m - rect.center();
                st.centre = before - Vec2::new(off.x, off.y) / st.scale;
            }
        }
        st.scale = st.scale.clamp(min_scale, max_scale);
        st.centre = st.centre.clamp(lo, hi);
        mm.page = st;
        let view = RadarView::north_up(rect, st.centre, st.scale);
        if let Some(m) = pointer {
            let near_waypoint = mm.waypoint.is_some_and(|w| (view.project(w) - m).length() < 16.0);
            if resp.secondary_clicked() || (resp.clicked() && near_waypoint) {
                mm.waypoint = None;
            } else if resp.clicked() {
                mm.waypoint = Some(view.unproject(m));
            }
        }
        // The map.
        let p = ui.painter().with_clip_rect(rect);
        let base = if mm.map.sea { raster::SEA } else { raster::VOID };
        p.rect_filled(rect, 0.0, Color32::from_rgb(base[0], base[1], base[2]));
        let mut meshes = Vec::new();
        tile_meshes(&mut mm.tiles, &ctx, &view, 255, &mut meshes);
        for m in meshes {
            p.add(egui::Shape::mesh(m));
        }
        if mm.tiles.waiting() > 0 {
            ctx.request_repaint();
        }
        let mission = mm.goal.is_some_and(|g| g.1);
        if let Some(route) = &mm.route {
            let mut pts = Vec::new();
            route_points(route, &view, Some(player), f32::MAX, &mut pts);
            if pts.len() >= 2 {
                let color = if mission { MISSION_COLOR } else { WAYPOINT_COLOR };
                p.add(egui::Shape::line(pts.clone(), Stroke::new((9.0 * st.scale).clamp(3.5, 8.0) + 2.0, Color32::from_black_alpha(120))));
                p.add(egui::Shape::line(pts, Stroke::new((9.0 * st.scale).clamp(3.5, 8.0), color)));
            }
        }
        let mut blips: Vec<Blip> = Vec::new();
        mm.collect_blips(&scene, true, None, &mut blips);
        blips.retain(|b| !matches!(b.kind, BlipKind::Poi(k) if !shows(k, st.scale)));
        let r = 7.5;
        let hit = draw_blips(&p, &view, rect.shrink(r * 1.3), &blips, None, r, time, pointer);
        draw_arrow(&p, view.project(player), view.screen_dir(scene.heading), 11.0);
        let shadow_text = |p: &egui::Painter, at: Pos2, align: egui::Align2, text: &str, size: f32| {
            let galley = p.layout_no_wrap(text.to_string(), egui::FontId::proportional(size), Color32::WHITE);
            let r = align.anchor_size(at, galley.size()).expand2(vec2(6.0, 3.0));
            p.rect_filled(r, 4.0, Color32::from_black_alpha(200));
            p.text(at, align, text, egui::FontId::proportional(size), Color32::WHITE);
        };
        if let (Some(i), Some(m)) = (hit, pointer) {
            shadow_text(&p, m + vec2(14.0, -14.0), egui::Align2::LEFT_BOTTOM, &blips[i].name, 14.0);
        }
        // Scale bar.
        let len = scale_bar(st.scale, 130.0);
        let a = rect.left_bottom() + vec2(20.0, -22.0);
        let b = a + vec2(len * st.scale, 0.0);
        for (w, c) in [(5.0, Color32::from_black_alpha(200)), (2.0, Color32::WHITE)] {
            p.line_segment([a, b], Stroke::new(w, c));
            p.line_segment([a - vec2(0.0, 5.0), a + vec2(0.0, 1.0)], Stroke::new(w, c));
            p.line_segment([b - vec2(0.0, 5.0), b + vec2(0.0, 1.0)], Stroke::new(w, c));
        }
        shadow_text(&p, a + vec2(0.0, -10.0), egui::Align2::LEFT_BOTTOM, &distance_text(len), 13.0);
        // North.
        let n = rect.left_top() + vec2(34.0, 40.0);
        p.circle(n, 15.0, Color32::from_black_alpha(200), Stroke::new(1.0, Color32::from_white_alpha(180)));
        p.text(n, egui::Align2::CENTER_CENTER, "N", egui::FontId::proportional(18.0), Color32::WHITE);
        p.add(egui::Shape::convex_polygon(vec![n + vec2(0.0, -25.0), n + vec2(5.0, -16.0), n + vec2(-5.0, -16.0)], Color32::WHITE, Stroke::NONE));

        // Legend.
        let p = ui.painter().with_clip_rect(legend);
        p.rect(legend, 8.0, Color32::from_rgba_unmultiplied(18, 20, 22, 232), Stroke::new(1.0, Color32::from_white_alpha(40)), egui::StrokeKind::Inside);
        let x = legend.left() + 16.0;
        let mut y = legend.top() + 14.0;
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, "MAPA", egui::FontId::proportional(24.0), Color32::from_rgb(235, 200, 140));
        y += 30.0;
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, &mm.map.name, egui::FontId::proportional(14.0), Color32::from_gray(200));
        y += 24.0;
        let goal_line = match (mm.goal, mm.remaining) {
            (Some(_), Some(left)) => {
                let t = mm.route.as_ref().map_or(0.0, |r| r.time * left / r.length().max(1.0));
                format!("{}: {} · {:.0} min", if mission { "Misión" } else { "Destino" }, distance_text(left), (t / 60.0).ceil().max(1.0))
            }
            (Some((g, _)), None) if mm.pending.is_some() => format!("Calculando ruta… ({})", distance_text(g.distance(player))),
            (Some((g, _)), None) => format!("Sin ruta por carretera ({})", distance_text(g.distance(player))),
            (None, _) => "Sin destino marcado".into(),
        };
        p.text(pos2(x, y), egui::Align2::LEFT_TOP, goal_line, egui::FontId::proportional(14.0), Color32::WHITE);
        y += 26.0;
        let row = 21.0;
        let small = legend.height() < 520.0;
        let entry = |y: &mut f32, icon: &dyn Fn(&egui::Painter, Pos2), label: &str| {
            if *y + row > legend.bottom() - 118.0 {
                return;
            }
            icon(&p, pos2(x + 9.0, *y + row * 0.5));
            p.text(pos2(x + 30.0, *y + row * 0.5), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), Color32::from_gray(225));
            *y += row;
        };
        entry(&mut y, &|p, c| draw_arrow(p, c, vec2(0.0, -1.0), 7.0), "Tú");
        entry(&mut y, &|p, c| draw_blip(p, c, BlipKind::Waypoint, 6.0, time), "Destino (clic en el mapa)");
        entry(&mut y, &|p, c| {
            p.line_segment([c - vec2(9.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(4.0, WAYPOINT_COLOR));
        }, "Ruta GPS");
        entry(&mut y, &|p, c| {
            p.line_segment([c - vec2(9.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(4.0, MISSION_COLOR));
        }, "Ruta de misión");
        entry(&mut y, &|p, c| draw_blip(p, c, BlipKind::Vehicle { owned: true }, 6.0, time), "Tu vehículo");
        entry(&mut y, &|p, c| draw_blip(p, c, BlipKind::Police, 6.5, time), "Policía (si te buscan)");
        for kind in PoiKind::ALL {
            if mm.map.pois.iter().any(|q| q.kind == kind) && !(small && !kind.on_radar()) {
                entry(&mut y, &|p, c| draw_blip(p, c, BlipKind::Poi(kind), 6.0, time), kind.label());
            }
        }
        for (class, label) in [(road::MOTORWAY, "Autovía"), (road::PRIMARY, "Avenida"), (road::RESIDENTIAL, "Calle")] {
            entry(&mut y, &|p, c| {
                let col = raster::road_color(class);
                p.line_segment([c - vec2(9.0, 0.0), c + vec2(9.0, 0.0)], Stroke::new(if class == road::RESIDENTIAL { 3.0 } else { 5.0 }, Color32::from_rgb(col[0], col[1], col[2])));
            }, label);
        }
        // Help and buttons.
        let mut hy = legend.bottom() - 112.0;
        for line in ["Arrastrar: mover el mapa", "Rueda: acercar / alejar", "Clic: marcar destino · clic derecho: quitar"] {
            p.text(pos2(x, hy), egui::Align2::LEFT_TOP, line, egui::FontId::proportional(12.0), Color32::from_gray(170));
            hy += 16.0;
        }
        let bw = (legend.width() - 32.0 - 12.0) / 3.0;
        let by = legend.bottom() - 46.0;
        let mut button = |i: usize, text: &str| ui.put(Rect::from_min_size(pos2(x + i as f32 * (bw + 6.0), by), vec2(bw, 32.0)), egui::Button::new(egui::RichText::new(text).size(13.0))).clicked();
        if button(0, "Centrar") {
            mm.page.centre = player;
        }
        if button(1, "Quitar") {
            mm.waypoint = None;
        }
        if button(2, "Volver") {
            action = PageAction::Back;
        }
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_read_in_spanish() {
        assert_eq!(distance_text(84.0), "80 m");
        assert_eq!(distance_text(946.0), "950 m");
        assert_eq!(distance_text(1240.0), "1,2 km");
        assert_eq!(distance_text(23_100.0), "23,1 km");
    }

    #[test]
    fn scale_bar_is_a_round_length() {
        // 0.45 points per metre: 130 points are 289 m, the bar says 200 m.
        assert_eq!(scale_bar(0.45, 130.0), 200.0);
        assert_eq!(scale_bar(0.02, 130.0), 5000.0);
        assert_eq!(scale_bar(3.0, 130.0), 20.0);
    }

    #[test]
    fn far_zooms_hide_minor_places() {
        assert!(shows(PoiKind::Airport, 0.01));
        assert!(!shows(PoiKind::Hospital, 0.03) && shows(PoiKind::Hospital, 0.1));
        assert!(!shows(PoiKind::Shop, 0.1) && shows(PoiKind::Shop, 0.3));
        assert!(!shows(PoiKind::Church, 0.3) && shows(PoiKind::Church, 0.5));
    }
}
