//! BVH motion capture loader (e.g. the CMU database). Joints become
//! scene nodes (End Sites included, so end bones have a direction) and the
//! motion becomes one clip with a rotation track per joint and a
//! translation track on the root.

use std::path::Path;

use forja_math::{Quat, Vec3};

use crate::{AssetError, ChannelData, ChannelValues, ClipData, Interpolation, NodeData, SceneData, Trs};

/// Meters per BVH unit of the CMU conversions (inches scaled by 1/0.45).
pub const CMU_SCALE: f32 = 0.056444;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ch {
    Xpos,
    Ypos,
    Zpos,
    Xrot,
    Yrot,
    Zrot,
}

pub fn load_bvh(path: &Path, scale: f32) -> Result<SceneData, AssetError> {
    let text = std::fs::read_to_string(path).map_err(|e| AssetError::Io(format!("{}: {e}", path.display())))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "bvh".into());
    parse_bvh(&text, &name, scale)
}

pub fn parse_bvh(text: &str, clip_name: &str, scale: f32) -> Result<SceneData, AssetError> {
    let err = |m: &str| AssetError::Format(format!("bvh: {m}"));
    let mut tok = text.split_whitespace().peekable();
    let mut scene = SceneData { name: clip_name.into(), ..Default::default() };
    let mut channels: Vec<(usize, Vec<Ch>)> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut pending: Option<usize> = None;
    if tok.next() != Some("HIERARCHY") {
        return Err(err("missing HIERARCHY"));
    }
    loop {
        let Some(t) = tok.next() else { return Err(err("unexpected end of hierarchy")) };
        match t {
            "ROOT" | "JOINT" | "End" => {
                let jname = if t == "End" {
                    tok.next();
                    let parent = stack.last().map(|p| scene.nodes[*p].name.clone()).unwrap_or_default();
                    format!("{parent}_end")
                } else {
                    tok.next().ok_or_else(|| err("joint name"))?.to_string()
                };
                let idx = scene.nodes.len();
                let parent = stack.last().copied();
                scene.nodes.push(NodeData { name: jname, parent, children: Vec::new(), local: Trs::default(), mesh: None, skin: None });
                match parent {
                    Some(p) => scene.nodes[p].children.push(idx),
                    None => scene.roots.push(idx),
                }
                pending = Some(idx);
            }
            "{" => stack.push(pending.take().ok_or_else(|| err("brace without joint"))?),
            "}" => {
                stack.pop();
            }
            "OFFSET" => {
                let mut v = [0.0f32; 3];
                for x in &mut v {
                    *x = tok.next().and_then(|s| s.parse().ok()).ok_or_else(|| err("offset"))?;
                }
                let j = *stack.last().ok_or_else(|| err("offset outside joint"))?;
                scene.nodes[j].local.t = Vec3::from(v) * scale;
            }
            "CHANNELS" => {
                let n: usize = tok.next().and_then(|s| s.parse().ok()).ok_or_else(|| err("channel count"))?;
                let mut list = Vec::with_capacity(n);
                for _ in 0..n {
                    list.push(match tok.next().unwrap_or("") {
                        "Xposition" => Ch::Xpos,
                        "Yposition" => Ch::Ypos,
                        "Zposition" => Ch::Zpos,
                        "Xrotation" => Ch::Xrot,
                        "Yrotation" => Ch::Yrot,
                        "Zrotation" => Ch::Zrot,
                        other => return Err(err(&format!("channel {other}"))),
                    });
                }
                channels.push((*stack.last().ok_or_else(|| err("channels outside joint"))?, list));
            }
            "MOTION" => break,
            _ => {}
        }
    }
    // MOTION header.
    let mut frames = 0usize;
    let mut dt = 1.0 / 30.0;
    while let Some(t) = tok.peek().copied() {
        match t {
            "Frames:" => {
                tok.next();
                frames = tok.next().and_then(|s| s.parse().ok()).ok_or_else(|| err("frames"))?;
            }
            "Frame" => {
                tok.next();
                tok.next();
                dt = tok.next().and_then(|s| s.parse().ok()).ok_or_else(|| err("frame time"))?;
                break;
            }
            _ => {
                tok.next();
            }
        }
    }
    let per_frame: usize = channels.iter().map(|c| c.1.len()).sum();
    let values: Vec<f32> = tok.take(frames * per_frame).map(|s| s.parse::<f32>().unwrap_or(0.0)).collect();
    if values.len() < frames * per_frame || frames < 2 {
        return Err(err("truncated motion"));
    }
    // The cgspeed conversion adds a T-pose as frame 0; skip it.
    let first = 1;
    let n = frames - first;
    let times: Vec<f32> = (0..n).map(|i| i as f32 * dt).collect();
    let mut rot: Vec<Vec<Quat>> = vec![Vec::with_capacity(n); channels.len()];
    let mut pos: Vec<Vec<Vec3>> = vec![Vec::with_capacity(n); channels.len()];
    for f in first..frames {
        let mut k = f * per_frame;
        for (ci, (_, list)) in channels.iter().enumerate() {
            let mut q = Quat::IDENTITY;
            let mut p = Vec3::ZERO;
            for c in list {
                let v = values[k];
                k += 1;
                match c {
                    Ch::Xpos => p.x = v * scale,
                    Ch::Ypos => p.y = v * scale,
                    Ch::Zpos => p.z = v * scale,
                    // Rotations compose in the listed order (intrinsic).
                    Ch::Xrot => q *= Quat::from_rotation_x(v.to_radians()),
                    Ch::Yrot => q *= Quat::from_rotation_y(v.to_radians()),
                    Ch::Zrot => q *= Quat::from_rotation_z(v.to_radians()),
                }
            }
            rot[ci].push(q.normalize());
            pos[ci].push(p);
        }
    }
    let mut out = Vec::new();
    for (ci, (node, list)) in channels.iter().enumerate() {
        if list.iter().any(|c| matches!(c, Ch::Xpos | Ch::Ypos | Ch::Zpos)) {
            // Position channels replace the offset.
            out.push(ChannelData { node: *node, interpolation: Interpolation::Linear, times: times.clone(), values: ChannelValues::Translation(pos[ci].clone()) });
            scene.nodes[*node].local.t = pos[ci][0];
        }
        out.push(ChannelData { node: *node, interpolation: Interpolation::Linear, times: times.clone(), values: ChannelValues::Rotation(std::mem::take(&mut rot[ci])) });
    }
    scene.clips.push(ClipData { name: clip_name.into(), duration: times.last().copied().unwrap_or(0.0), channels: out });
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "HIERARCHY
ROOT Hips
{
  OFFSET 0 0 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
  JOINT Spine
  {
    OFFSET 0 10 0
    CHANNELS 3 Zrotation Yrotation Xrotation
    End Site
    {
      OFFSET 0 5 0
    }
  }
}
MOTION
Frames: 3
Frame Time: 0.5
0 0 0 0 0 0 0 0 0
1 20 3 0 90 0 0 0 0
2 20 6 0 90 0 90 0 0
";

    #[test]
    fn parses_hierarchy_and_motion() {
        let s = parse_bvh(SAMPLE, "t", 0.1).unwrap();
        assert_eq!(s.nodes.len(), 3);
        assert_eq!(s.nodes[2].name, "Spine_end");
        assert!((s.nodes[1].local.t.y - 1.0).abs() < 1e-6);
        let c = &s.clips[0];
        assert_eq!(c.channels.len(), 3);
        assert!((c.duration - 0.5).abs() < 1e-6);
        // Root moved to (0.1, 2.0, 0.3) in frame 1.
        let ChannelValues::Translation(t) = &c.channels[0].values else { panic!() };
        assert!((t[0] - Vec3::new(0.1, 2.0, 0.3)).length() < 1e-5);
        // 90 degrees about Y on the root.
        let ChannelValues::Rotation(r) = &c.channels[1].values else { panic!() };
        assert!((r[0] * Vec3::X - Vec3::NEG_Z).length() < 1e-5);
    }
}
