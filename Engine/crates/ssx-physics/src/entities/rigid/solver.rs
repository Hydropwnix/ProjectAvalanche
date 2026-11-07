use glam::{Mat3, Quat, Vec3};

use super::setup::{Side, Sym3, contact_mass, world_inv_inertia};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Contact {
    pub mu_s: f32,
    pub mu_d: f32,
    pub restitution: f32,
    pub normal: Vec3,
    pub r_a: Vec3,
    pub r_b: Vec3,
    pub static_frame: Quat,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Motion {
    pub position: Vec3,
    pub orientation: Quat,
    pub velocity: Vec3,
    pub angular_velocity: Vec3,
    pub inv_mass: f32,
    pub inv_inertia: Vec3,
    pub max_speed: f32,
    pub max_angular_speed: f32,
    pub linear_damping: f32,
    pub angular_damping: f32,
}

pub fn step(m: &mut Motion, contacts: &[Contact], h: f32, gravity: Vec3, iterations: u32) {
    let mut dx = m.velocity * h + gravity * (h * h);
    let mut dth = m.angular_velocity * h;
    let mut c_b = Vec3::ZERO;
    let b: Vec<f32> = contacts.iter().map(|c| (m.position + c.r_b - c.r_a).dot(c.normal)).collect();
    let mut lam = vec![0.0f32; contacts.len()];
    for _ in 0..iterations {
        for (i, c) in contacts.iter().enumerate() {
            let rel = (-c_b).dot(c.normal);
            let new = (lam[i] + (b[i] - rel)).max(0.0);
            let dl = new - lam[i];
            lam[i] = new;
            c_b -= c.normal * dl;
        }
    }
    let inv_b = world_inv_inertia(m.orientation, m.inv_inertia, m.inv_mass);
    let inv_i = inv_b.to_mat3();
    struct Row {
        mm: Mat3,
        bias: Vec3,
    }
    let rows: Vec<Row> = contacts
        .iter()
        .map(|c| {
            let n = c.normal;
            let b = Side { inv_mass: m.inv_mass, inv_inertia: inv_b, r: c.r_b };
            let mm = contact_mass(n, &static_side(c), &b).to_mat3();
            let d = m.position + c.r_b - c.r_a;
            let dn = d.dot(n);
            let vn = (m.velocity + m.angular_velocity.cross(c.r_b)).dot(n);
            let d_pred = d + dx + dth.cross(c.r_b);
            let x = c.restitution * vn;
            let corr = if x > 0.0 && d_pred.dot(n) > 0.0 { dn.min(0.0) - h * x } else { 0.0 };
            Row { mm, bias: d - n * corr }
        })
        .collect();
    let mut p = vec![Vec3::ZERO; contacts.len()];
    for _ in 0..iterations {
        for (i, c) in contacts.iter().enumerate() {
            let n = c.normal;
            let du = rows[i].bias + dx + dth.cross(c.r_b);
            let pn_vec = p[i] + rows[i].mm * du;
            let pn = pn_vec.dot(n);
            let new = if pn <= 0.0 {
                Vec3::ZERO
            } else {
                let pt = pn_vec - n * pn;
                let mus = c.mu_s;
                if pt.length_squared() > mus * mus / (1.0 + mus * mus) * pn_vec.length_squared() {
                    n * pn + pt * (c.mu_d * pn / pt.length())
                } else {
                    pn_vec
                }
            };
            let dp = new - p[i];
            p[i] = new;
            dx -= dp * m.inv_mass;
            dth -= inv_i * c.r_b.cross(dp);
        }
    }
    integrate(m, dx, dth, c_b, h);
}

fn static_side(c: &Contact) -> Side {
    Side { inv_mass: 0.0, inv_inertia: world_inv_inertia(c.static_frame, Vec3::ZERO, 0.0), r: c.r_a }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Link {
    pub a: Option<usize>,
    pub b: Option<usize>,
    pub contact: Contact,
}

pub fn lane_order(links: &[Link]) -> Vec<usize> {
    let mut groups: Vec<(Vec<usize>, Vec<usize>)> = Vec::new();
    for (i, l) in links.iter().enumerate() {
        let bodies: Vec<usize> = [l.a, l.b].into_iter().flatten().collect();
        let free = |g: &(Vec<usize>, Vec<usize>)| g.0.len() < 4 && !bodies.iter().any(|b| g.1.contains(b));
        let k = match groups.iter().position(free) {
            Some(k) => k,
            None => {
                groups.push((Vec::new(), Vec::new()));
                groups.len() - 1
            }
        };
        groups[k].0.push(i);
        groups[k].1.extend(bodies);
    }
    groups.into_iter().flat_map(|g| g.0).collect()
}

pub fn step_partition(ms: &mut [Motion], links: &[Link], h: f32, gravity: Vec3, iterations: u32) {
    let order = lane_order(links);
    let n_b = ms.len();
    let mut dx: Vec<Vec3> = ms.iter().map(|m| m.velocity * h + gravity * (h * h)).collect();
    let mut dth: Vec<Vec3> = ms.iter().map(|m| m.angular_velocity * h).collect();
    let inv_w: Vec<Sym3> = ms.iter().map(|m| world_inv_inertia(m.orientation, m.inv_inertia, m.inv_mass)).collect();
    let inv_i: Vec<Mat3> = inv_w.iter().map(|w| w.to_mat3()).collect();
    let im = |s: Option<usize>| s.map_or(0.0, |i| ms[i].inv_mass);
    let pos = |s: Option<usize>| s.map_or(Vec3::ZERO, |i| ms[i].position);
    let d: Vec<Vec3> = links.iter().map(|l| (pos(l.b) + l.contact.r_b) - (pos(l.a) + l.contact.r_a)).collect();
    let mut c = vec![Vec3::ZERO; n_b];
    let mut lam = vec![0.0f32; links.len()];
    let shares: Vec<(f32, f32)> = links
        .iter()
        .map(|l| {
            let (ia, ib) = (im(l.a), im(l.b));
            let sum = ia + ib;
            if sum > 0.0 { (ia / sum, ib / sum) } else { (0.0, 0.0) }
        })
        .collect();
    let at = |c: &[Vec3], s: Option<usize>| s.map_or(Vec3::ZERO, |i| c[i]);
    for _ in 0..iterations {
        for &k in &order {
            let l = &links[k];
            let n = l.contact.normal;
            let rel = (at(&c, l.a) - at(&c, l.b)).dot(n);
            let new = (lam[k] + (d[k].dot(n) - rel)).max(0.0);
            let dl = new - lam[k];
            lam[k] = new;
            if let Some(a) = l.a {
                c[a] += n * (shares[k].0 * dl);
            }
            if let Some(b) = l.b {
                c[b] -= n * (shares[k].1 * dl);
            }
        }
    }
    let delta =
        |dx: &[Vec3], dth: &[Vec3], s: Option<usize>, r: Vec3| s.map_or(Vec3::ZERO, |i| dx[i] + dth[i].cross(r));
    let rows: Vec<(Mat3, Vec3)> = links
        .iter()
        .enumerate()
        .map(|(k, l)| {
            let ct = &l.contact;
            let side = |s: Option<usize>, r: Vec3| match s {
                Some(i) => Side { inv_mass: ms[i].inv_mass, inv_inertia: inv_w[i], r },
                None => Side { r, ..static_side(ct) },
            };
            let n = ct.normal;
            let mm = contact_mass(n, &side(l.a, ct.r_a), &side(l.b, ct.r_b)).to_mat3();
            let vel =
                |s: Option<usize>, r: Vec3| s.map_or(Vec3::ZERO, |i| ms[i].velocity + ms[i].angular_velocity.cross(r));
            let vn = (vel(l.b, ct.r_b) - vel(l.a, ct.r_a)).dot(n);
            let d_pred = d[k] + delta(&dx, &dth, l.b, ct.r_b) - delta(&dx, &dth, l.a, ct.r_a);
            let x = ct.restitution * vn;
            let corr = if x > 0.0 && d_pred.dot(n) > 0.0 { d[k].dot(n).min(0.0) - h * x } else { 0.0 };
            (mm, d[k] - n * corr)
        })
        .collect();
    let mut p = vec![Vec3::ZERO; links.len()];
    for _ in 0..iterations {
        for &k in &order {
            let l = &links[k];
            let ct = &l.contact;
            let n = ct.normal;
            let du = rows[k].1 + delta(&dx, &dth, l.b, ct.r_b) - delta(&dx, &dth, l.a, ct.r_a);
            let pv = p[k] + rows[k].0 * du;
            let pn = pv.dot(n);
            let new = if pn <= 0.0 {
                Vec3::ZERO
            } else {
                let pt = pv - n * pn;
                let mus = ct.mu_s;
                if pt.length_squared() > mus * mus / (1.0 + mus * mus) * pv.length_squared() {
                    n * pn + pt * (ct.mu_d * pn / pt.length())
                } else {
                    pv
                }
            };
            let dp = new - p[k];
            p[k] = new;
            if let Some(a) = l.a {
                dx[a] += dp * ms[a].inv_mass;
                dth[a] += inv_i[a] * ct.r_a.cross(dp);
            }
            if let Some(b) = l.b {
                dx[b] -= dp * ms[b].inv_mass;
                dth[b] -= inv_i[b] * ct.r_b.cross(dp);
            }
        }
    }
    for (i, m) in ms.iter_mut().enumerate() {
        integrate(m, dx[i], dth[i], c[i], h);
    }
}

fn integrate(m: &mut Motion, dx: Vec3, dth: Vec3, c: Vec3, h: f32) {
    m.position += dx;
    let rel = dx - c;
    let mut f = 1.0 / h - m.linear_damping;
    let l = rel.length();
    if l > 0.0 {
        f = f.min(m.max_speed / l);
    }
    m.velocity = rel * f;
    let mut fw = 1.0 / h - m.angular_damping;
    let lw = dth.length();
    if lw > 0.0 {
        fw = fw.min(m.max_angular_speed / lw);
    }
    m.angular_velocity = dth * fw;
    let a = dth * 0.5;
    let q = m.orientation;
    let v = Vec3::new(q.x, q.y, q.z);
    let nv = v + a * q.w + a.cross(v);
    let nw = q.w - a.dot(v);
    m.orientation = Quat::from_xyzw(nv.x, nv.y, nv.z, nw).normalize();
}
