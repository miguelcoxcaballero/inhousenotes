//! Bounded reconstruction of measured ink omitted by the editable centreline.
//! The renderer remains in JS; this kernel chooses small native pen discs only
//! inside the measured silhouette and takes colour from connected native ink.
const PALETTE: [[u8; 3]; 3] = [[77, 77, 77], [0, 47, 217], [232, 16, 16]];
const DIRECTIONS: [(i32, i32, u16); 8] = [
    (0, -1, 3), (1, -1, 4), (1, 0, 3), (1, 1, 4),
    (0, 1, 3), (-1, 1, 4), (-1, 0, 3), (-1, -1, 4),
];
const MAX_CANDIDATES: usize = 250_000;

fn native_label(rgba: &[u8], index: usize) -> u8 {
    let at = index * 4;
    if rgba[at + 3] <= 200 { return 0; }
    for (label, rgb) in PALETTE.iter().enumerate() {
        let distance: i32 = (0..3).map(|c| {
            let delta = rgba[at + c] as i32 - rgb[c] as i32;
            delta * delta
        }).sum();
        if distance < 32 * 32 { return label as u8 + 1; }
    }
    0
}

fn connected_label(rgba: &[u8], radii: &[u16], width: usize, height: usize,
    index: usize, reach: usize, visited: &mut [u8], queue: &mut Vec<usize>) -> u8 {
    let x = index % width;
    let y = index / width;
    let side = reach * 2 + 1;
    visited[..side * side].fill(0);
    queue.clear();
    queue.push(index);
    visited[reach * side + reach] = 1;
    let mut head = 0;
    while head < queue.len() {
        let at = queue[head]; head += 1;
        let label = native_label(rgba, at);
        if label != 0 { return label; }
        let ax = (at % width) as i32;
        let ay = (at / width) as i32;
        for &(dx, dy, _) in &DIRECTIONS {
            let nx = ax + dx;
            let ny = ay + dy;
            let sx = nx - x as i32 + reach as i32;
            let sy = ny - y as i32 + reach as i32;
            if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32
                || sx < 0 || sy < 0 || sx >= side as i32 || sy >= side as i32 { continue; }
            let local = sy as usize * side + sx as usize;
            let next = ny as usize * width + nx as usize;
            if visited[local] == 0 && radii[next] != 0 {
                visited[local] = 1;
                queue.push(next);
            }
        }
    }
    0
}

/// Returns the number of output [pixel index, radius, palette label] triplets.
/// Invalid dimensions return -1; pathological candidate density returns no
/// repairs, preserving the existing editable strokes without unbounded work.
#[no_mangle]
pub unsafe extern "C" fn ink_repair_discs(rendered: *mut u8, radii: *const u16,
    coverage: *const u8, width: usize, height: usize, output: *mut f64, capacity: usize) -> i32 {
    if rendered.is_null() || radii.is_null() || coverage.is_null() || output.is_null()
        || width < 2 || height < 2 || width > 4096 || height > 4096
        || width * height > 12_000_000 || capacity > 12_000 { return -1; }
    if capacity == 0 { return 0; }
    let size = width * height;
    let rgba = std::slice::from_raw_parts_mut(rendered, size * 4);
    let radii = std::slice::from_raw_parts(radii, size);
    let coverage = std::slice::from_raw_parts(coverage, size);
    let output = std::slice::from_raw_parts_mut(output, capacity * 3);
    let scale = width as f64 / 794.0;
    let mut candidates = Vec::with_capacity(size.min(MAX_CANDIDATES + 1));
    for index in width + 1..size - width - 1 {
        let radius = radii[index];
        if radius < 6 { continue; }
        let contained = DIRECTIONS.iter().any(|&(dx, dy, cost)| {
            let neighbour = (index as i32 + dy * width as i32 + dx) as usize;
            radii[neighbour] as u32 >= radius as u32 + cost as u32
        });
        if !contained {
            candidates.push(index as u32);
            if candidates.len() > MAX_CANDIDATES { return 0; }
        }
    }
    candidates.sort_unstable_by(|&a, &b| radii[b as usize].cmp(&radii[a as usize])
        .then_with(|| coverage[b as usize].cmp(&coverage[a as usize]))
        .then_with(|| a.cmp(&b)));
    let max_vote = (scale * 4.25).ceil() as usize;
    let max_side = max_vote * 2 + 1;
    let mut visited = vec![0; max_side * max_side];
    let mut queue = Vec::with_capacity(visited.len());
    let minimum_radius = 1.25_f64.max(scale * 0.6);
    let minimum_missing = 2.0_f64.max(scale * scale * 0.4);
    let mut count = 0;
    for index in candidates {
        let index = index as usize;
        let x = index % width; let y = index / width;
        let mut radius = (scale * 1.25).min(0.6_f64.max(radii[index] as f64 / 3.0 - 0.5));
        let reach = (radius + 1.0).ceil() as usize;
        let left = x.saturating_sub(reach); let right = (x + reach).min(width - 1);
        let top = y.saturating_sub(reach); let bottom = (y + reach).min(height - 1);
        for yy in top..=bottom { for xx in left..=right {
            if radii[yy * width + xx] == 0 {
                let dx = xx as f64 - x as f64; let dy = yy as f64 - y as f64;
                radius = radius.min(0.6_f64.max(dx.hypot(dy) - 0.5));
            }
        }}
        if radius < minimum_radius { continue; }
        radius = (radius * 1024.0).floor() / 1024.0;
        let inner_squared = (radius - 0.1) * (radius - 0.1);
        let mut missing = 0;
        for yy in top..=bottom { for xx in left..=right {
            let dx = xx as f64 - x as f64; let dy = yy as f64 - y as f64;
            if dx * dx + dy * dy <= inner_squared && rgba[(yy * width + xx) * 4 + 3] < 100 { missing += 1; }
        }}
        if (missing as f64) < minimum_missing { continue; }
        let vote_reach = (radius + scale * 3.0).ceil() as usize;
        let label = connected_label(rgba, radii, width, height, index, vote_reach, &mut visited, &mut queue);
        if label == 0 { continue; }
        let outer_squared = (radius + 0.25) * (radius + 0.25);
        let mut conflict = false;
        for yy in top..=bottom { for xx in left..=right {
            let dx = xx as f64 - x as f64; let dy = yy as f64 - y as f64;
            if dx * dx + dy * dy > outer_squared { continue; }
            let other = native_label(rgba, yy * width + xx);
            if other != 0 && other != label { conflict = true; break; }
        } if conflict { break; }}
        if conflict { continue; }
        output[count * 3] = index as f64;
        output[count * 3 + 1] = radius;
        output[count * 3 + 2] = label as f64;
        count += 1;
        let rgb = PALETTE[label as usize - 1];
        for yy in top..=bottom { for xx in left..=right {
            let dx = xx as f64 - x as f64; let dy = yy as f64 - y as f64;
            if dx * dx + dy * dy <= outer_squared {
                let at = (yy * width + xx) * 4;
                rgba[at..at + 3].copy_from_slice(&rgb); rgba[at + 3] = 255;
            }
        }}
        if count == capacity { break; }
    }
    count as i32
}
