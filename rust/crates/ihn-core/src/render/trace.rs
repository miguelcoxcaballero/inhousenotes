//! Test support: a fake canvas host that replays a command batch and writes it down exactly like
//! `tests/v6/parity/trace-init.js` + `normalize.mjs` (`stringifyTrace`), and the reader of the recorded 5.18.0
//! traces in `tests/v6/fixtures/traces`.

use super::cmd::{op, Cmds, CAPS, COMPOSITES, JOINS};
use crate::codec::Reader;
use crate::op::{Color, Tool};
use std::collections::HashMap;

pub fn num(v: f64) -> String { format!("\"~{:016x}\"", v.to_bits()) }

struct Meta {
    label: Option<String>,
    kind: &'static str,
    w: u32,
    h: u32,
}

/// The replayer. `dom` and `images` are what the page would hold: canvases of the document by DOM handle, and the
/// trace form of each image by URL (or `blob:<object handle>`).
#[derive(Default)]
pub struct Host {
    pub dom: HashMap<u32, (String, u32, u32)>,
    pub images: HashMap<String, String>,
    /// `(request id, surface, mime)` of every `toDataURL` / `toBlob`, and `(request id, url)` of every image load.
    pub encodes: Vec<(u32, u32, String)>,
    pub loads: Vec<(u32, String)>,
    /// Everything replayed, one byte string per recording window, and the document's canvases: what
    /// `tests/v6/e2e/internal-canvas.spec.mjs` feeds to the real `host/canvas.js`.
    log: Vec<Vec<u8>>,
    bound: Vec<(u32, String, u32, u32)>,
    detached: Vec<u32>,
    surfs: HashMap<u32, Meta>,
    imgs: HashMap<u32, String>,
    paths: HashMap<u32, Vec<String>>,
    /// Surfaces in order of first use in the recording window (as a target or as an argument).
    used: Vec<u32>,
    streams: Vec<(u32, Vec<String>)>,
    cur: u32,
    path: Option<u32>,
}

impl Host {
    /// Notes the use of a surface; its name is decided when the window is written (`normalize.mjs` `nameOf`).
    fn name(&mut self, id: u32) -> String {
        if !self.used.contains(&id) {
            self.used.push(id);
        }
        format!("\u{1}{id}\u{1}")
    }
    fn push(&mut self, id: u32, line: String) {
        self.name(id);
        match self.streams.iter_mut().find(|s| s.0 == id) {
            Some(s) => s.1.push(line),
            None => self.streams.push((id, vec![line])),
        }
    }
    fn emit(&mut self, name: &str, args: &[String]) {
        let mut line = format!("[\"{name}\"");
        for a in args {
            line.push(',');
            line.push_str(a);
        }
        line.push(']');
        match self.path {
            Some(p) => self.paths.entry(p).or_default().push(line.replacen("[\"", "[\"path:", 1)),
            None => self.push(self.cur, line),
        }
    }
    /// `__ihnTrace.reset()`: a new recording window (surface names start again, canvases and caches stay).
    pub fn reset(&mut self) {
        self.streams.clear();
        self.used.clear();
        if self.log.is_empty() {
            self.log.push(Vec::new());
        }
        self.log.push(Vec::new());
    }
    /// A canvas left the document (its label is gone, like a detached element's).
    pub fn detach(&mut self, node: u32) {
        self.detached.push(node);
        let label = self.dom.remove(&node).map(|d| d.0);
        for m in self.surfs.values_mut().filter(|m| m.label.is_some() && m.label == label) {
            m.label = None;
        }
    }

    pub fn run(&mut self, c: &mut Cmds) {
        let bytes = c.take();
        if self.log.is_empty() {
            self.log.push(Vec::new());
        }
        self.log.last_mut().unwrap().extend_from_slice(&bytes);
        let mut r = Reader::new(&bytes);
        let s = |v: String| format!("\"{v}\"");
        while let Some(code) = r.u8() {
            let mut f = |n: usize| -> Vec<String> { (0..n).map(|_| num(r.f64().expect("f64"))).collect() };
            match code {
                op::SAVE => self.emit("save", &[]),
                op::RESTORE => self.emit("restore", &[]),
                op::SET_TRANSFORM => self.emit("setTransform", &f(6)),
                op::SCALE => self.emit("scale", &f(2)),
                op::TRANSLATE => self.emit("translate", &f(2)),
                op::ROTATE => self.emit("rotate", &f(1)),
                op::LINE_WIDTH => self.emit("set:lineWidth", &f(1)),
                op::GLOBAL_ALPHA => self.emit("set:globalAlpha", &f(1)),
                op::SHADOW_BLUR => self.emit("set:shadowBlur", &f(1)),
                op::BEGIN_PATH => self.emit("beginPath", &[]),
                op::MOVE_TO => self.emit("moveTo", &f(2)),
                op::LINE_TO => self.emit("lineTo", &f(2)),
                op::BEZIER_TO => self.emit("bezierCurveTo", &f(6)),
                op::QUAD_TO => self.emit("quadraticCurveTo", &f(4)),
                op::ARC => self.emit("arc", &f(5)),
                op::ROUND_RECT => self.emit("roundRect", &f(5)),
                op::RECT => self.emit("rect", &f(4)),
                op::CLIP => self.emit("clip", &[]),
                op::STROKE => self.emit("stroke", &[]),
                op::FILL => self.emit("fill", &[]),
                op::CLEAR_RECT => self.emit("clearRect", &f(4)),
                op::FILL_RECT => self.emit("fillRect", &f(4)),
                op::STROKE_RECT => self.emit("strokeRect", &f(4)),
                op::READ_PIXELS => drop(f(4)),
                op::PATH_END => self.path = None,
                _ => {
                    drop(f);
                    self.other(code, &mut r, s);
                }
            }
        }
    }

    fn other(&mut self, code: u8, r: &mut Reader, s: impl Fn(String) -> String) {
        let u = |r: &mut Reader| r.u32().expect("u32");
        match code {
            op::CREATE => {
                let id = u(r);
                let meta = match r.u8() {
                    Some(0) => Meta { label: None, kind: "canvas", w: 300, h: 150 },
                    _ => Meta { label: None, kind: "offscreen", w: u(r), h: u(r) },
                };
                self.surfs.insert(id, meta);
            }
            op::BIND => {
                let (id, node) = (u(r), u(r));
                let (label, w, h) = self.dom.get(&node).cloned().expect("a canvas of the document");
                self.bound.push((node, label.clone(), w, h));
                self.surfs.insert(id, Meta { label: Some(label), kind: "canvas", w, h });
            }
            op::DROP => drop(u(r)),
            op::WIDTH | op::HEIGHT => {
                let (id, px) = (u(r), u(r));
                let meta = self.surfs.get_mut(&id).expect("surface");
                if code == op::WIDTH { meta.w = px } else { meta.h = px }
                self.push(id, format!("[\"set:{}\",{}]", if code == op::WIDTH { "width" } else { "height" }, num(px as f64)));
            }
            op::CTX => {
                self.cur = u(r);
                assert!(self.surfs.contains_key(&self.cur), "getContext of an unknown surface");
                let attrs = if r.u8() == Some(1) { ",{\"willReadFrequently\":true}" } else { "" };
                self.push(self.cur, format!("[\"getContext\",\"2d\"{attrs}]"));
            }
            op::SELECT => self.cur = u(r),
            op::FILL_STYLE => self.emit("set:fillStyle", &[s(r.str().unwrap().into())]),
            op::STROKE_STYLE => self.emit("set:strokeStyle", &[s(r.str().unwrap().into())]),
            op::FILTER => self.emit("set:filter", &[s(r.str().unwrap().into())]),
            op::SHADOW_COLOR => self.emit("set:shadowColor", &[s(r.str().unwrap().into())]),
            op::FONT => self.emit("set:font", &[s(r.str().unwrap().into())]),
            op::TEXT_ALIGN => self.emit("set:textAlign", &[s(r.str().unwrap().into())]),
            op::TEXT_BASELINE => self.emit("set:textBaseline", &[s(r.str().unwrap().into())]),
            op::FILL_TEXT => {
                let text = s(r.str().unwrap().into());
                self.emit("fillText", &[text, num(r.f64().unwrap()), num(r.f64().unwrap())]);
            }
            op::LINE_CAP => self.emit("set:lineCap", &[s(CAPS[r.u8().unwrap() as usize].into())]),
            op::LINE_JOIN => self.emit("set:lineJoin", &[s(JOINS[r.u8().unwrap() as usize].into())]),
            op::COMPOSITE => self.emit("set:globalCompositeOperation", &[s(COMPOSITES[r.u8().unwrap() as usize].into())]),
            op::LINE_DASH => {
                let n = r.u8().unwrap();
                let list: Vec<String> = (0..n).map(|_| num(r.f64().unwrap())).collect();
                self.emit("setLineDash", &[format!("[{}]", list.join(","))]);
            }
            op::RADIAL_FILL => {
                let circles: Vec<String> = (0..6).map(|_| num(r.f64().unwrap())).collect();
                let mut text = format!("{{\"gradient\":[[\"createRadialGradient\",{}]", circles.join(","));
                for _ in 0..r.u8().unwrap() {
                    text.push_str(&format!(",[\"addColorStop\",{},\"{}\"]", num(r.f64().unwrap()), r.str().unwrap()));
                }
                self.emit("set:fillStyle", &[text + "]}"]);
            }
            op::DRAW_IMAGE => {
                let (kind, handle, argc) = (r.u8().unwrap(), u(r), r.u8().unwrap());
                // normalize.mjs names the target stream before the surfaces among the arguments.
                let _ = self.name(self.cur);
                let mut args = vec![if kind == 0 { format!("{{\"surface\":\"{}\"}}", self.name(handle)) } else { self.imgs[&handle].clone() }];
                args.extend((0..argc).map(|_| num(r.f64().unwrap())));
                self.emit("drawImage", &args);
            }
            op::PATH_BEGIN => {
                let p = u(r);
                self.paths.insert(p, Vec::new());
                self.path = Some(p);
            }
            op::STROKE_PATH => {
                let p = u(r);
                self.push(self.cur, "[\"path:begin\"]".into());
                for line in self.paths[&p].clone() {
                    self.push(self.cur, line);
                }
                self.push(self.cur, "[\"stroke(path)\"]".into());
            }
            op::PATH_DROP => drop(self.paths.remove(&u(r))),
            op::TO_DATA_URL | op::TO_BLOB => {
                let (id, req, mime) = (u(r), u(r), r.str().unwrap().to_string());
                self.push(id, format!("[\"{}\",\"{mime}\",{}]", if code == op::TO_BLOB { "toBlob" } else { "toDataURL" }, num(r.f64().unwrap())));
                self.encodes.push((req, id, mime));
            }
            op::IMAGE_URL | op::IMAGE_BLOB => {
                let (img, req) = (u(r), u(r));
                let url = if code == op::IMAGE_URL { r.str().unwrap().to_string() } else { format!("blob:{}", u(r)) };
                self.imgs.insert(img, self.images.get(&url).cloned().unwrap_or_else(|| panic!("no image {url}")));
                self.loads.push((req, url));
            }
            op::IMAGE_DROP => drop(self.imgs.remove(&u(r))),
            other => panic!("unknown canvas command {other}"),
        }
    }

    /// With `IHN_CMD_DUMP=<dir>`: writes `<dir>/<scene>.json` (`{dom, detach, segments}`, segments in base64)
    /// for scenes the browser can replay as they are (no images: their trace identity is their `src`).
    pub fn dump(&self, scene: &str) {
        let Ok(dir) = std::env::var("IHN_CMD_DUMP") else { return };
        if !self.imgs.is_empty() || !self.loads.is_empty() {
            return;
        }
        let dom: Vec<String> = self.bound.iter().map(|(node, label, w, h)| format!("[{node},\"{label}\",{w},{h}]")).collect();
        let detach: Vec<String> = self.detached.iter().map(|n| n.to_string()).collect();
        let segments: Vec<String> = self.log.iter().map(|s| format!("\"{}\"", crate::codec::b64_encode(s, false))).collect();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(std::path::Path::new(&dir).join(format!("{scene}.json")), format!("{{\"dom\":[{}],\"detach\":[{}],\"segments\":[{}]}}\n", dom.join(","), detach.join(","), segments.join(","))).unwrap();
    }

    /// The recording window as `stringifyTrace` prints it: `(surface header, op lines)` sorted by label.
    pub fn surfaces(&self) -> Vec<(String, Vec<String>)> {
        let mut names: HashMap<u32, String> = HashMap::new();
        for id in &self.used {
            let n = names.len() - names.values().filter(|n| !n.starts_with("offscreen#")).count();
            names.insert(*id, self.surfs[id].label.clone().unwrap_or_else(|| format!("offscreen#{n}")));
        }
        let named = |line: &String| -> String {
            let mut parts = line.split('\u{1}');
            let mut text = parts.next().unwrap_or("").to_string();
            while let (Some(id), Some(rest)) = (parts.next(), parts.next()) {
                text.push_str(&names[&id.parse::<u32>().unwrap()]);
                text.push_str(rest);
            }
            text
        };
        let mut out: Vec<(String, String, Vec<String>)> = self
            .streams
            .iter()
            .map(|(id, ops)| {
                let (m, name) = (&self.surfs[id], &names[id]);
                (name.clone(), format!("{{\"label\":\"{name}\",\"kind\":\"{}\",\"width\":{},\"height\":{},\"ops\":[", m.kind, m.w, m.h), ops.iter().map(named).collect())
            })
            .collect();
        crate::sort::by(&mut out, |a, b| a.0.cmp(&b.0));
        out.into_iter().map(|(_, header, ops)| (header, ops)).collect()
    }
}

/// A recorded trace: the `input` line and the surfaces in file order.
pub struct Golden {
    pub input: String,
    pub surfaces: Vec<(String, Vec<String>)>,
}

pub fn fixture(rel: &str) -> std::path::PathBuf { std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../tests/v6/fixtures").join(rel) }

impl Golden {
    pub fn load(scene: &str) -> Golden {
        let path = fixture(&format!("traces/{scene}.json"));
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut g = Golden { input: String::new(), surfaces: Vec::new() };
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("  \"input\": ") {
                g.input = rest.to_string();
            } else if let Some(header) = line.strip_prefix("    {") {
                g.surfaces.push((format!("{{{header}"), Vec::new()));
            } else if let Some(entry) = line.strip_prefix("      ") {
                g.surfaces.last_mut().expect("surface").1.push(entry.trim_end_matches(',').to_string());
            }
        }
        assert!(!g.surfaces.is_empty(), "{scene}: no surfaces");
        g
    }
    pub fn surface(&self, label: &str) -> &[String] {
        let key = format!("{{\"label\":\"{label}\",");
        &self.surfaces.iter().find(|s| s.0.starts_with(&key)).unwrap_or_else(|| panic!("no surface {label}")).1
    }
    /// The whole file equals what `host` recorded: same surfaces, same sizes, same calls, bit for bit.
    pub fn assert_eq(&self, host: &Host) {
        let ours = host.surfaces();
        let headers = |list: &[(String, Vec<String>)]| list.iter().map(|s| format!("{} {} ops", s.0, s.1.len())).collect::<Vec<_>>().join("\n");
        for (mine, theirs) in ours.iter().zip(&self.surfaces) {
            assert_eq!(mine.0, theirs.0, "surface list differs.\nours:\n{}\n5.18.0:\n{}", headers(&ours), headers(&self.surfaces));
            if let Some(i) = (0..mine.1.len().max(theirs.1.len())).find(|i| mine.1.get(*i) != theirs.1.get(*i)) {
                let around = |list: &[String]| list[i.saturating_sub(6)..list.len().min(i + 3)].join("\n    ");
                panic!("{} op {i} differs ({} ours, {} recorded)\n  ours:\n    {}\n  5.18.0:\n    {}", mine.0, mine.1.len(), theirs.1.len(), around(&mine.1), around(&theirs.1));
            }
        }
        assert_eq!(ours.len(), self.surfaces.len(), "surface count differs.\nours:\n{}\n5.18.0:\n{}", headers(&ours), headers(&self.surfaces));
    }
}

/// The numbers of a trace line.
pub fn nums(line: &str) -> Vec<f64> {
    line.match_indices("\"~").map(|(i, _)| f64::from_bits(u64::from_str_radix(&line[i + 2..i + 18], 16).expect("hex"))).collect()
}
/// The op name of a trace line.
pub fn name(line: &str) -> &str { line.split('"').nth(1).unwrap_or("") }

/// A stroke of a fixture document.
#[derive(Clone, Debug)]
pub struct Stroke {
    /// Position in the page when it was read (stays with the stroke: the `Path2D` cache key).
    pub id: u32,
    pub tool: Tool,
    pub color: Color,
    pub width: f64,
    pub xy: Vec<f64>,
}

/// The strokes of a fixture page, read back from a recorded full repaint of it: the points are the end points of
/// each path segment (pen) or the control points (highlighter), tool/colour/width the state set before it. The
/// 5.x PDFs hold them compressed and `expect.json` only their counts; the v5 reader (W2.2) is not in this crate
/// yet. What the renderer computes — control points, line-or-curve choice, state calls, order, culling — is not
/// taken from here.
pub fn strokes_of(ops: &[String]) -> Vec<Stroke> {
    let (mut out, mut colour, mut width) = (Vec::new(), String::new(), 0.0);
    let mut i = 0;
    while i < ops.len() {
        let line = &ops[i];
        match name(line) {
            "set:strokeStyle" => colour = line.split('"').nth(3).unwrap().to_string(),
            "set:lineWidth" => width = nums(line)[0],
            "path:moveTo" => {
                let mut xy = nums(line);
                while matches!(name(&ops[i + 1]), "path:lineTo" | "path:bezierCurveTo") {
                    i += 1;
                    let v = nums(&ops[i]);
                    xy.extend_from_slice(&v[v.len() - 2..]);
                }
                out.push(Stroke { id: out.len() as u32, tool: Tool::Pen, color: Color::Css(colour.as_str().into()), width, xy });
            }
            "set:globalCompositeOperation" if line.contains("multiply") => {
                let mut xy = Vec::new();
                while name(&ops[i]) != "stroke" {
                    let v = nums(&ops[i]);
                    match name(&ops[i]) {
                        "set:strokeStyle" => colour = ops[i].split('"').nth(3).unwrap().to_string(),
                        "set:lineWidth" => width = v[0],
                        "moveTo" | "lineTo" => xy.extend_from_slice(&v),
                        "quadraticCurveTo" => xy.extend_from_slice(&v[..2]),
                        _ => {}
                    }
                    i += 1;
                }
                while name(&ops[i]) != "restore" {
                    i += 1;
                }
                out.push(Stroke { id: out.len() as u32, tool: Tool::Highlighter, color: Color::Css(colour.as_str().into()), width, xy });
            }
            _ => {}
        }
        i += 1;
    }
    out
}
