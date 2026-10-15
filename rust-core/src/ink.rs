//! Clean, transparent scan ink -> palette-labelled medial axes. No page/template
//! pixels enter this kernel. The host traces the axes into ordinary editor pens.

fn colour(r: u8, g: u8, b: u8, alpha: u8) -> u8 {
    let (r, g, b) = (r as f32, g as f32, b as f32);
    let hi = r.max(g).max(b);
    let lo = r.min(g).min(b);
    // Unpremultiplication amplifies tiny JPEG colour errors on pale paper.
    // Require chroma in the original, covered pixel as well as a clear hue.
    // Navy ink can have low absolute chroma in a shadow while still being
    // strongly saturated. Keep the relative and covered-pixel requirements.
    if hi - lo < 18.0 || hi - lo < hi * 0.2 || (hi - lo) * (alpha as f32) < 255.0 * 12.0 {
        return 1;
    }
    let scale = 225.0 / (hi - lo);
    let c = [(r - lo) * scale, (g - lo) * scale, (b - lo) * scale];
    let targets = [[0.0, 0.0, 0.0], [0.0, 49.0, 225.0], [225.0, 0.0, 0.0]];
    let mut best = (f32::INFINITY, 1);
    for (i, target) in targets.iter().enumerate() {
        let error: f32 = (0..3).map(|j| (c[j] - target[j]).powi(2)).sum();
        if error < best.0 {
            best = (error, i as u8 + 1);
        }
    }
    best.1
}

// A faint continuation uses its transverse contrast, not the darkest
// neighbouring letter. Stop at the first valley so the opposite side of a
// counter cannot be mistaken for background beyond a second stroke.
fn transverse_ink(
    x: usize, y: usize, width: usize, height: usize, reach: usize,
    alpha: f64, sample: &impl Fn(usize) -> u8,
) -> bool {
    let (x, y, w, h, r) = (x as isize, y as isize, width as isize, height as isize, reach as isize);
    let cross = (width as f64 / 794.0 * 0.75).round().max(1.0) as isize;
    // A diagonal must have the same physical support as a horizontal pen.
    // Normalized oblique probes keep faint cursive bends near dark crossings;
    // four integer directions skipped those bends or looked beyond the pen.
    for (vx, vy) in [(1.0f64, 0.0f64), (0.0, 1.0), (1.0, 1.0), (1.0, -1.0),
        (2.0, 1.0), (2.0, -1.0), (1.0, 2.0), (1.0, -2.0)] {
        let length = vx.hypot(vy);
        let (dx, dy) = (vx / length, vy / length);
        let (x, y, r, cross) = (x as f64, y as f64, r as f64, cross as f64);
        if x - r * dx.abs() - cross * dy.abs() < 1.0 || x + r * dx.abs() + cross * dy.abs() >= (w - 1) as f64
            || y - r * dy.abs() - cross * dx.abs() < 1.0 || y + r * dy.abs() + cross * dx.abs() >= (h - 1) as f64 {
            continue;
        }
        let at = |xx: f64, yy: f64| sample(yy.round() as usize * width + xx.round() as usize) as f64;
        let before = at(x + dy * cross, y - dx * cross);
        let after = at(x - dy * cross, y + dx * cross);
        if before.min(after) < alpha * 0.4 { continue; }
        let mut peak = alpha;
        let mut centre = 0;
        for d in -(cross as isize)..=cross as isize {
            let value = at(x + dx * d as f64, y + dy * d as f64);
            if value > peak { peak = value; centre = d; }
        }
        if alpha * 100.0 < peak * 65.0 { continue; }
        let (mut left, mut right) = (peak, peak);
        for d in (-(r as isize)..centre).rev() {
            let value = at(x + dx * d as f64, y + dy * d as f64);
            if value > left + 2.0 { break; }
            left = left.min(value);
        }
        for d in centre + 1..=r as isize {
            let value = at(x + dx * d as f64, y + dy * d as f64);
            if value > right + 2.0 { break; }
            right = right.min(value);
        }
        if left.max(right) < peak * 0.45 && peak - left.max(right) >= 6.0 { return true; }
    }
    false
}

/// All buffers belong to the private host adapter; dimensions are checked there
/// and here. Zero means invalid, one complete (including a blank page), two
/// means the ink complexity exceeded the bounded processing budget.
#[no_mangle]
pub unsafe extern "C" fn ink_axes(
    input: *const u8,
    width: usize,
    height: usize,
    output: *mut u8,
    radii: *mut u16,
    coverage: *const u8,
) -> u32 {
    if input.is_null()
        || output.is_null()
        || radii.is_null()
        || width < 2
        || height < 2
        || width > 4096
        || height > 4096
        || width * height > 12_000_000
    {
        return 0;
    }
    let n = width * height;
    let rgba = std::slice::from_raw_parts(input, n * 4);
    let geometry = if coverage.is_null() { None } else { Some(std::slice::from_raw_parts(coverage, n)) };
    let alpha_at = |i: usize| if rgba[i*4+3] < 16 { 0 } else { geometry.map_or(rgba[i*4+3], |values| values[i]) };
    let reach = ((width as f64 / 794.0 * 2.5).ceil() as usize).clamp(3, 8);
    let mask = std::slice::from_raw_parts_mut(output, n);
    let dist = std::slice::from_raw_parts_mut(radii, n);
    mask.fill(0);
    dist.fill(0);
    // Weak coverage belongs to ink only when connected to a real stroke core.
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if alpha_at(i) >= if geometry.is_some() { 8 } else { 16 } {
                mask[i] = colour(
                    rgba[i * 4],
                    rgba[i * 4 + 1],
                    rgba[i * 4 + 2],
                    rgba[i * 4 + 3],
                );
            }
        }
    }
    // Use local coverage rather than an absolute dark threshold: faint pencil
    // survives, while low-coverage JPEG fringes cannot become extra black pens.
    // Page-scaled support and a measured core preserve small letter counters.
    // A fixed 7px window was smaller than the blur in an enlarged photograph.
    let initial = mask.to_vec();
    let mut core = vec![false; n];
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if initial[i] == 0 {
                continue;
            }
            let alpha = alpha_at(i) as u32;
            let mut peak = alpha;
            for yy in y.saturating_sub(reach)..=(y + reach).min(height - 1) {
                for xx in x.saturating_sub(reach)..=(x + reach).min(width - 1) {
                    peak = peak.max(alpha_at(yy * width + xx) as u32);
                }
            }
            if alpha * 100 >= peak * if geometry.is_some() { 65 } else { 60 } {
                core[i] = true;
            } else if !transverse_ink(x, y, width, height, reach, alpha as f64, &alpha_at) {
                mask[i] = 0;
            }
        }
    }
    let offsets = [
        -(width as isize),
        1 - width as isize,
        1,
        1 + width as isize,
        width as isize,
        width as isize - 1,
        -1,
        -(width as isize) - 1,
    ];
    let mut seen = vec![false; n];
    let mut queue = Vec::new();
    let mut active = Vec::new();
    let mut anchors = Vec::new();
    for start in 0..n {
        if mask[start] == 0 || seen[start] {
            continue;
        }
        queue.clear();
        queue.push(start);
        seen[start] = true;
        let mut head = 0;
        let mut peak = 0;
        let mut anchored = false;
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (width, 0, height, 0);
        while head < queue.len() {
            let i = queue[head];
            head += 1;
            peak = peak.max(rgba[i * 4 + 3]);
            anchored |= core[i];
            min_x = min_x.min(i % width); max_x = max_x.max(i % width);
            min_y = min_y.min(i / width); max_y = max_y.max(i / width);
            for delta in offsets {
                let j = (i as isize + delta) as usize;
                if !seen[j] && mask[j] != 0 {
                    seen[j] = true;
                    queue.push(j);
                }
            }
        }
        // Retain punctuation and faint pencil; isolated JPEG dust has no core.
        if peak < 24 || (queue.len() == 1 && peak < 90)
            || (!anchored && ((max_x - min_x).max(max_y - min_y) as f64) < 4.0_f64.max(width as f64 / 794.0 * 3.0)) {
            for &i in &queue {
                mask[i] = 0;
            }
        } else {
            active.extend_from_slice(&queue);
            // Parallel thinning can otherwise erase a tiny symmetric 2x2
            // punctuation component in one pass. Retain its central pixel.
            if queue.len() <= 32 {
                let cx = queue.iter().map(|i| (i % width) as f64).sum::<f64>() / queue.len() as f64;
                let cy = queue.iter().map(|i| (i / width) as f64).sum::<f64>() / queue.len() as f64;
                let mut best = (f64::INFINITY, queue[0]);
                for &i in &queue {
                    let d = ((i % width) as f64 - cx).powi(2) + ((i / width) as f64 - cy).powi(2);
                    if d < best.0 {
                        best = (d, i);
                    }
                }
                anchors.push(best.1);
            }
        }
    }
    seen.fill(false);
    for i in anchors {
        seen[i] = true;
    }
    drop(queue);
    for &i in &active {
        dist[i] = 12000;
    }
    for i in 0..n {
        if mask[i] != 0 {
            for (delta, cost) in [
                (-1, 3),
                (-(width as isize), 3),
                (-(width as isize) - 1, 4),
                (1 - width as isize, 4),
            ] {
                let j = (i as isize + delta) as usize;
                dist[i] = dist[i].min(if mask[j] != 0 { dist[j] + cost } else { cost });
            }
        }
    }
    for i in (0..n).rev() {
        if mask[i] != 0 {
            for (delta, cost) in [
                (1, 3),
                (width as isize, 3),
                (width as isize + 1, 4),
                (width as isize - 1, 4),
            ] {
                let j = (i as isize + delta) as usize;
                dist[i] = dist[i].min(if mask[j] != 0 { dist[j] + cost } else { cost });
            }
        }
    }
    // All colours share one ink silhouette. Colour boundaries are not pen
    // edges: thinning each hue separately creates rainbow scan outlines.
    // Anchor supported tips in the measured silhouette before thinning can
    // consume diagonal caps. Do not anchor new corners created by thinning.
    for &i in &active {
        let (mut count, mut peak) = (0, 0);
        let (mut dx, mut dy) = (0isize, 0isize);
        for delta in offsets {
            let j = (i as isize + delta) as usize;
            if mask[j] != 0 {
                count += 1; peak = peak.max(alpha_at(j));
                dx += (j % width) as isize - (i % width) as isize;
                dy += (j / width) as isize - (i / width) as isize;
            }
        }
        if count != 2 || alpha_at(i) < peak { continue; }
        let length = (dx as f64).hypot(dy as f64);
        let distance = (width as f64 / 794.0 * 2.0).max(2.0);
        if length == 0.0 { continue; }
        let x = (i % width) as f64 + dx as f64 / length * distance;
        let y = (i / width) as f64 + dy as f64 / length * distance;
        let (x, y) = (x.round() as isize, y.round() as isize);
        if x > 0 && x < width as isize - 1 && y > 0 && y < height as isize - 1
            && alpha_at(y as usize * width + x as usize) as f64 >= alpha_at(i) as f64 * 0.5 {
            seen[i] = true;
        }
    }
    // Intensity-ordered homotopic thinning follows the measured ink ridge.
    // A single deletion must preserve connectivity and counters. Ordering
    // by the unenhanced coverage avoids colour-dependent letter shapes.
    // Keep the guide's endpoints. Ordered thinning must not peel a faint
    // two-pixel branch all the way back to its next darker crossing.
    {
        let silhouette = mask.to_vec();
        let mut guide_active = active.clone();
        let mut remove = Vec::new();
        let mut work = 0usize;
        loop {
            let mut changed = false;
            for phase in 0..2 {
                work += guide_active.len();
                if work > 40_000_000 {
                    return 2;
                }
                remove.clear();
                for &i in &guide_active {
                    let label = mask[i];
                    if label == 0 || seen[i] {
                        continue;
                    }
                    let p = offsets.map(|delta| (mask[(i as isize + delta) as usize] != 0) as u8);
                    let count: u8 = p.iter().sum();
                    if !(2..=6).contains(&count) {
                        continue;
                    }
                    let changes = (0..8).filter(|&k| p[k] == 0 && p[(k + 1) % 8] == 1).count();
                    if changes != 1 {
                        continue;
                    }
                    let safe = if phase == 0 {
                        p[0] * p[2] * p[4] == 0 && p[2] * p[4] * p[6] == 0
                    } else {
                        p[0] * p[2] * p[6] == 0 && p[0] * p[4] * p[6] == 0
                    };
                    if safe {
                        remove.push(i);
                    }
                }
                if !remove.is_empty() {
                    changed = true;
                    for &i in &remove {
                        mask[i] = 0;
                    }
                }
            }
            if !changed {
                break;
            }
            guide_active.retain(|&i| mask[i] != 0);
        }
        if geometry.is_some() {
            for &i in &guide_active {
                if mask[i] == 0 { continue; }
                let mut degree = 0;
                for k in 0..8 {
                    if mask[(i as isize + offsets[k]) as usize] == 0 { continue; }
                    if k % 2 != 0 && (mask[(i as isize + offsets[(k + 7) % 8]) as usize] != 0
                        || mask[(i as isize + offsets[(k + 1) % 8]) as usize] != 0) { continue; }
                    degree += 1;
                }
                if degree <= 1 { seen[i] = true; }
            }
            mask.copy_from_slice(&silhouette);
        }
    }
    if geometry.is_some() {
        use std::cmp::Reverse;
        use std::collections::BinaryHeap;
        let mut queued = vec![false; n];
        let mut heap = BinaryHeap::new();
        // The radius term centres flat opaque cores; measured coverage guides
        // ambiguous ridges. Use the same stable ordering in the JS fallback.
        for &i in &active {
            if !seen[i] && offsets.iter().any(|&d| mask[(i as isize + d) as usize] == 0) {
                heap.push(Reverse((dist[i] as u32 * 4 + alpha_at(i) as u32, i)));
                queued[i] = true;
            }
        }
        let mut work = 0usize;
        while let Some(Reverse((_, i))) = heap.pop() {
            queued[i] = false;
            work += 1;
            if work > 40_000_000 { return 2; }
            if mask[i] == 0 || seen[i] { continue; }
            let p = offsets.map(|delta| (mask[(i as isize + delta) as usize] != 0) as u8);
            let count: u8 = p.iter().sum();
            if !(2..=7).contains(&count) { continue; }
            let changes = (0..8).filter(|&k| p[k] == 0 && p[(k + 1) % 8] == 1).count();
            if changes != 1 { continue; }
            mask[i] = 0;
            for delta in offsets {
                let j = (i as isize + delta) as usize;
                if mask[j] != 0 && !seen[j] && !queued[j] {
                    heap.push(Reverse((dist[j] as u32 * 4 + alpha_at(j) as u32, j)));
                    queued[j] = true;
                }
            }
        }
    }
    // Classify only the surviving medial axis, using the covered ink across
    // its width. A dark core can belong to a coloured pen; weak chromatic JPEG
    // edges cannot overrule it, nor seed independent coloured outlines.
    for &i in &active {
        if mask[i] == 0 {
            continue;
        }
        let (x, y) = (i % width, i / width);
        let radius = ((dist[i] as usize + 2) / 3).clamp(2, 8);
        let mut votes = [0u64; 4];
        for yy in y.saturating_sub(radius)..=(y + radius).min(height - 1) {
            for xx in x.saturating_sub(radius)..=(x + radius).min(width - 1) {
                let j = yy * width + xx;
                let label = initial[j] as usize;
                if label == 0 {
                    continue;
                }
                let d2 = xx.abs_diff(x).pow(2) + yy.abs_diff(y).pow(2);
                if d2 > radius * radius {
                    continue;
                }
                let alpha = rgba[j * 4 + 3] as u64;
                votes[label] += alpha * alpha * 256 / (1 + d2 as u64);
            }
        }
        let total = votes[1] + votes[2] + votes[3];
        let colour = if votes[2] > votes[3] { 2 } else { 3 };
        mask[i] = if votes[colour] * 100 >= total * 22 && votes[colour] * 2 >= votes[5 - colour] * 3
        {
            colour as u8
        } else {
            1
        };
    }
    1
}
