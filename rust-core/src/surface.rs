//! Developable pinhole least squares. The host retains independent holdout,
//! contour, orientation and photographed-raster validation of every result.
const STEPS: usize = 128;

struct Model {
    rotation: [f64; 9],
    xs: [f64; STEPS + 1],
    zs: [f64; STEPS + 1],
    cb: f64,
    sb: f64,
    length: f64,
    depth: f64,
    focal: f64,
    tx: f64,
    ty: f64,
    width: f64,
    height: f64,
}
impl Model {
    fn new(p: &[f64], width: f64, height: f64) -> Self {
        let theta = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt();
        let rotation = if theta < 1e-10 {
            [1., 0., 0., 0., 1., 0., 0., 0., 1.]
        } else {
            let x = p[0] / theta;
            let y = p[1] / theta;
            let z = p[2] / theta;
            let c = theta.cos();
            let s = theta.sin();
            let d = 1. - c;
            [
                c + x * x * d,
                x * y * d - z * s,
                x * z * d + y * s,
                y * x * d + z * s,
                c + y * y * d,
                y * z * d - x * s,
                z * x * d - y * s,
                z * y * d + x * s,
                c + z * z * d,
            ]
        };
        let cb = p[7].cos();
        let sb = p[7].sin();
        let length = (cb.abs() * 210. + sb.abs() * 297.) / 2.;
        let middle = (p.len() - 8) / 2;
        let mut angles = p[8..8 + middle].to_vec();
        angles.push(0.);
        angles.extend_from_slice(&p[8 + middle..]);
        let mut xs = [0.; STEPS + 1];
        let mut zs = [0.; STEPS + 1];
        let mut previous = angles[0];
        for i in 1..=STEPS {
            let t = i as f64 / STEPS as f64 * (angles.len() - 1) as f64;
            let index = (t.floor() as usize).min(angles.len() - 2);
            let angle = angles[index] + (angles[index + 1] - angles[index]) * (t - index as f64);
            xs[i] = xs[i - 1] + (previous.cos() + angle.cos()) * length / STEPS as f64;
            zs[i] = zs[i - 1] + (previous.sin() + angle.sin()) * length / STEPS as f64;
            previous = angle;
        }
        let center_x = xs[STEPS / 2];
        let center_z = zs[STEPS / 2];
        for i in 0..=STEPS {
            xs[i] -= center_x;
            zs[i] -= center_z;
        }
        Self {
            rotation,
            xs,
            zs,
            cb,
            sb,
            length,
            depth: p[5].exp(),
            focal: p[6].exp(),
            tx: p[3] * 100.,
            ty: p[4] * 100.,
            width,
            height,
        }
    }
    fn project(&self, u: f64, v: f64) -> [f64; 2] {
        let x = (u - 0.5) * 210.;
        let y = (v - 0.5) * 297.;
        let s = x * self.cb + y * self.sb;
        let t = -x * self.sb + y * self.cb;
        let pos = ((s / self.length + 1.) * STEPS as f64 / 2.).clamp(0., STEPS as f64);
        let i = (pos.floor() as usize).min(STEPS - 1);
        let mix = pos - i as f64;
        let curve = self.xs[i] + (self.xs[i + 1] - self.xs[i]) * mix;
        let xx = curve * self.cb - t * self.sb;
        let yy = curve * self.sb + t * self.cb;
        let z = self.zs[i] + (self.zs[i + 1] - self.zs[i]) * mix;
        let r = &self.rotation;
        let x = r[0] * xx + r[1] * yy + r[2] * z + self.tx;
        let y = r[3] * xx + r[4] * yy + r[5] * z + self.ty;
        let depth = r[6] * xx + r[7] * yy + r[8] * z + self.depth;
        [
            self.width / 2. + self.focal * x / depth,
            self.height / 2. + self.focal * y / depth,
        ]
    }
}

pub struct SurfaceFit {
    p: Vec<f64>,
    points: Vec<f64>,
    paths: Vec<f64>,
    offsets: [usize; 5],
    guides: Vec<f64>,
    width: f64,
    height: f64,
    preserve_ends: bool,
    r: Vec<f64>,
    cost: f64,
    lambda: f64,
    iterations: usize,
    done: bool,
}
impl SurfaceFit {
    fn residual(&self, p: &[f64]) -> Vec<f64> {
        let model = Model::new(p, self.width, self.height);
        let middle = (p.len() - 8) / 2;
        let mut angles = p[8..8 + middle].to_vec();
        angles.push(0.);
        angles.extend_from_slice(&p[8 + middle..]);
        let mut out = Vec::with_capacity(
            self.points.len() / 2 + angles.len() - 2 + 200 + self.guides.len() / 2,
        );
        for point in self.points.chunks_exact(4) {
            let predicted = model.project(point[0], point[1]);
            out.push(predicted[0] - point[2]);
            out.push(predicted[1] - point[3]);
        }
        for a in angles.windows(3) {
            out.push((a[0] - 2. * a[1] + a[2]) * 0.5);
        }
        if !self.paths.is_empty() {
            for side in 0..4 {
                for i in 0..=24 {
                    let t = i as f64 / 24.;
                    let u = if side == 3 {
                        15. / 210.
                    } else if side == 1 {
                        195. / 210.
                    } else {
                        (15. + 180. * t) / 210.
                    };
                    let v = if side == 0 {
                        14.3 / 297.
                    } else if side == 2 {
                        284.3 / 297.
                    } else {
                        (14.3 + 270. * t) / 297.
                    };
                    let point = model.project(u, v);
                    let path = &self.paths[self.offsets[side] * 2..self.offsets[side + 1] * 2];
                    let mut best = f64::INFINITY;
                    let mut nearest = [path[0], path[1]];
                    for j in (0..path.len() - 2).step_by(2) {
                        let dx = path[j + 2] - path[j];
                        let dy = path[j + 3] - path[j + 1];
                        let divisor = dx * dx + dy * dy;
                        let f = (((point[0] - path[j]) * dx + (point[1] - path[j + 1]) * dy)
                            / if divisor == 0. { 1. } else { divisor })
                        .clamp(0., 1.);
                        let q = [path[j] + f * dx, path[j + 1] + f * dy];
                        let distance = (q[0] - point[0]).powi(2) + (q[1] - point[1]).powi(2);
                        if distance < best {
                            best = distance;
                            nearest = q;
                        }
                    }
                    out.push((point[0] - nearest[0]) * 0.35);
                    out.push((point[1] - nearest[1]) * 0.35);
                }
            }
        }
        for g in self.guides.chunks_exact(8) {
            let center = model.project(g[0], g[1]);
            let before = model.project(g[0] - g[4] * 0.5, g[1] - g[5] * 0.5);
            let after = model.project(g[0] + g[4] * 0.5, g[1] + g[5] * 0.5);
            out.push((center[0] - g[2]) * 0.65);
            out.push((center[1] - g[3]) * 0.65);
            out.push((after[0] - before[0] - g[6]) * 1.7);
            out.push((after[1] - before[1] - g[7]) * 1.7);
        }
        out
    }
    fn step(&mut self) {
        if self.done {
            return;
        }
        let n = self.p.len();
        let m = self.r.len();
        let mut jac = vec![0.; n * m];
        for j in 0..n {
            if self.preserve_ends && (j == 8 || j == n - 1) {
                continue;
            }
            let mut perturbed = self.p.clone();
            perturbed[j] += 1e-4;
            let next = self.residual(&perturbed);
            for i in 0..m {
                jac[j * m + i] = (next[i] - self.r[i]) / 1e-4;
            }
        }
        let mut a = vec![0.; n * n];
        let mut b = vec![0.; n];
        for i in 0..m {
            let weight = 1. / (1. + self.r[i] * self.r[i]).sqrt();
            for j in 0..n {
                let v = jac[j * m + i] * weight;
                b[j] -= v * self.r[i];
                for k in 0..=j {
                    a[j * n + k] += v * jac[k * m + i];
                }
            }
        }
        for j in 0..n {
            for k in 0..j {
                a[k * n + j] = a[j * n + k];
            }
            a[j * n + j] += self.lambda * a[j * n + j].max(1e-6);
        }
        let delta = match solve(&a, &b, n) {
            Some(value) => value,
            None => {
                self.done = true;
                return;
            }
        };
        let mut trial = self
            .p
            .iter()
            .zip(delta)
            .map(|(p, d)| p + d.clamp(-0.4, 0.4))
            .collect::<Vec<_>>();
        trial[3] = trial[3].clamp(-5., 5.);
        trial[4] = trial[4].clamp(-5., 5.);
        trial[5] = trial[5].clamp(120_f64.ln(), 4000_f64.ln());
        let dimension = self.width.max(self.height);
        trial[6] = trial[6].clamp((dimension * 0.5).ln(), (dimension * 5.).ln());
        for value in &mut trial[8..] {
            *value = value.clamp(-1.5, 1.5);
        }
        let next = self.residual(&trial);
        let cost = loss(&next);
        self.iterations += 1;
        if cost < self.cost {
            let improvement = self.cost - cost;
            self.p = trial;
            self.r = next;
            self.cost = cost;
            self.lambda = (self.lambda * 0.35).max(1e-7);
            if improvement < 1e-5 {
                self.done = true;
            }
        } else {
            self.lambda = (self.lambda * 5.).min(1e8);
        }
    }
}
fn loss(values: &[f64]) -> f64 {
    values
        .iter()
        .fold(0., |sum, v| sum + 2. * ((1. + v * v).sqrt() - 1.))
}
fn solve(matrix: &[f64], values: &[f64], n: usize) -> Option<Vec<f64>> {
    let stride = n + 1;
    let mut rows = vec![0.; n * stride];
    for i in 0..n {
        rows[i * stride..i * stride + n].copy_from_slice(&matrix[i * n..(i + 1) * n]);
        rows[i * stride + n] = values[i];
    }
    for i in 0..n {
        let mut pivot = i;
        for j in i + 1..n {
            if rows[j * stride + i].abs() > rows[pivot * stride + i].abs() {
                pivot = j;
            }
        }
        if rows[pivot * stride + i].abs() < 1e-12 {
            return None;
        }
        if pivot != i {
            for k in 0..stride {
                rows.swap(i * stride + k, pivot * stride + k);
            }
        }
        let divisor = rows[i * stride + i];
        for k in i..=n {
            rows[i * stride + k] /= divisor;
        }
        for j in 0..n {
            if i != j {
                let factor = rows[j * stride + i];
                for k in i..=n {
                    rows[j * stride + k] -= factor * rows[i * stride + k];
                }
            }
        }
    }
    Some((0..n).map(|i| rows[i * stride + n]).collect())
}

/// All pointer/count pairs are validated and owned by the private host adapter.
#[no_mangle]
pub unsafe extern "C" fn surface_fit_new(
    parameters: *const f64,
    count: usize,
    points: *const f64,
    point_count: usize,
    paths: *const f64,
    path_count: usize,
    offsets: *const u32,
    guides: *const f64,
    guide_count: usize,
    width: f64,
    height: f64,
    preserve_ends: u32,
) -> *mut SurfaceFit {
    if parameters.is_null()
        || points.is_null()
        || paths.is_null()
        || offsets.is_null()
        || guides.is_null()
        || count < 10
        || count > 32
        || (count - 8) % 2 != 0
        || point_count > 4500
        || path_count > 4096
        || guide_count > 160
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.
        || height <= 0.
    {
        return std::ptr::null_mut();
    }
    let p = std::slice::from_raw_parts(parameters, count).to_vec();
    let points = std::slice::from_raw_parts(points, point_count * 4).to_vec();
    let paths = std::slice::from_raw_parts(paths, path_count * 2).to_vec();
    let guides = std::slice::from_raw_parts(guides, guide_count * 8).to_vec();
    let offsets: [usize; 5] = std::array::from_fn(|i| *offsets.add(i) as usize);
    if !p
        .iter()
        .chain(&points)
        .chain(&paths)
        .chain(&guides)
        .all(|x| x.is_finite())
        || offsets[0] != 0
        || offsets[4] != path_count
        || offsets
            .windows(2)
            .any(|pair| pair[0] > pair[1] || (path_count > 0 && pair[1] - pair[0] < 2))
    {
        return std::ptr::null_mut();
    }
    let mut fit = SurfaceFit {
        p,
        points,
        paths,
        offsets,
        guides,
        width,
        height,
        preserve_ends: preserve_ends != 0,
        r: Vec::new(),
        cost: 0.,
        lambda: 0.01,
        iterations: 0,
        done: false,
    };
    fit.r = fit.residual(&fit.p);
    fit.cost = loss(&fit.r);
    Box::into_raw(Box::new(fit))
}
#[no_mangle]
pub unsafe extern "C" fn surface_fit_step(context: *mut SurfaceFit, steps: usize) -> u32 {
    if context.is_null() {
        return 0;
    }
    let fit = &mut *context;
    for _ in 0..steps.min(8) {
        if fit.done {
            break;
        }
        fit.step();
    }
    if fit.done {
        0
    } else {
        1
    }
}
#[no_mangle]
pub unsafe extern "C" fn surface_fit_result(
    context: *const SurfaceFit,
    parameters: *mut f64,
    info: *mut f64,
) {
    if context.is_null() || parameters.is_null() || info.is_null() {
        return;
    }
    let fit = &*context;
    std::ptr::copy_nonoverlapping(fit.p.as_ptr(), parameters, fit.p.len());
    *info = fit.cost;
    *info.add(1) = fit.iterations as f64;
}
#[no_mangle]
pub unsafe extern "C" fn surface_fit_drop(context: *mut SurfaceFit) {
    if !context.is_null() {
        drop(Box::from_raw(context));
    }
}
