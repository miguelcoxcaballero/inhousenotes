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
    let mask = std::slice::from_raw_parts_mut(output, n);
    let dist = std::slice::from_raw_parts_mut(radii, n);
    mask.fill(0);
    dist.fill(0);
    // Weak coverage belongs to ink only when connected to a real stroke core.
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if rgba[i * 4 + 3] >= 16 {
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
    let initial = mask.to_vec();
    for y in 1..height - 1 {
        for x in 1..width - 1 {
            let i = y * width + x;
            if initial[i] == 0 {
                continue;
            }
            let alpha = rgba[i * 4 + 3] as u32;
            let mut peak = alpha;
            for yy in y.saturating_sub(3)..=(y + 3).min(height - 1) {
                for xx in x.saturating_sub(3)..=(x + 3).min(width - 1) {
                    peak = peak.max(rgba[(yy * width + xx) * 4 + 3] as u32);
                }
            }
            if alpha * 100 < peak * 35 {
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
        while head < queue.len() {
            let i = queue[head];
            head += 1;
            peak = peak.max(rgba[i * 4 + 3]);
            for delta in offsets {
                let j = (i as isize + delta) as usize;
                if !seen[j] && mask[j] != 0 {
                    seen[j] = true;
                    queue.push(j);
                }
            }
        }
        // Retain punctuation and faint pencil; isolated JPEG dust has no core.
        if peak < 24 || (queue.len() == 1 && peak < 90) {
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
    // Thin the whole ink silhouette once. Colour boundaries are not pen edges:
    // thinning each hue separately creates the rainbow outlines of a scan.
    let mut remove = Vec::new();
    let mut work = 0usize;
    loop {
        let mut changed = false;
        for phase in 0..2 {
            work += active.len();
            if work > 40_000_000 {
                return 2;
            }
            remove.clear();
            for &i in &active {
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
        active.retain(|&i| mask[i] != 0);
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
