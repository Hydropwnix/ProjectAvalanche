use std::collections::BTreeMap;

use super::{Bracket, Class};

const BRACKET_STACK: usize = 63;

fn dir(level: u8) -> Class {
    if level & 1 == 0 { Class::L } else { Class::R }
}

fn isolating_runs(classes: &[Class], pdis: &[Option<usize>], levels: &[u8]) -> Vec<Vec<usize>> {
    let mut runs: Vec<Vec<usize>> = Vec::new();
    let mut current: Option<u8> = None;
    for (i, &c) in classes.iter().enumerate() {
        if c.removed() {
            continue;
        }
        match runs.last_mut() {
            Some(run) if current == Some(levels[i]) => run.push(i),
            _ => {
                runs.push(vec![i]);
                current = Some(levels[i]);
            }
        }
    }
    let mut run_of_start: Vec<Option<usize>> = vec![None; classes.len()];
    for (r, run) in runs.iter().enumerate() {
        run_of_start[run[0]] = Some(r);
    }
    let mut matched_pdi = vec![false; classes.len()];
    for p in pdis.iter().flatten() {
        matched_pdi[*p] = true;
    }
    let mut out = Vec::new();
    let mut taken = vec![false; runs.len()];
    for r in 0..runs.len() {
        if taken[r] || matched_pdi[runs[r][0]] {
            continue;
        }
        let mut seq: Vec<usize> = Vec::new();
        let mut at = r;
        loop {
            taken[at] = true;
            seq.extend_from_slice(&runs[at]);
            let last = *runs[at].last().unwrap_or(&0);
            let next = classes[last].isolate_initiator().then(|| pdis[last]).flatten().and_then(|p| run_of_start[p]);
            match next {
                Some(n) if !taken[n] => at = n,
                _ => break,
            }
        }
        out.push(seq);
    }
    for r in 0..runs.len() {
        if !taken[r] {
            out.push(runs[r].clone());
        }
    }
    out
}

pub(super) fn resolve_implicit(
    classes: &[Class],
    brackets: &[Bracket],
    pdis: &[Option<usize>],
    base: u8,
    levels: &mut [u8],
    types: &mut [Class],
) {
    let kept_before = |i: usize| (0..i).rev().find(|&j| !classes[j].removed());
    let kept_after = |i: usize| (i + 1..classes.len()).find(|&j| !classes[j].removed());
    let explicit: Vec<u8> = levels.to_vec();
    for seq in isolating_runs(classes, pdis, levels) {
        let (Some(&first), Some(&last)) = (seq.first(), seq.last()) else { continue };
        let level = explicit[first];
        let before = kept_before(first).map_or(base, |j| explicit[j]);
        let after =
            if classes[last].isolate_initiator() { base } else { kept_after(last).map_or(base, |j| explicit[j]) };
        let sos = dir(level.max(before));
        let eos = dir(level.max(after));
        let mut t: Vec<Class> = seq.iter().map(|&i| types[i]).collect();
        let original = t.clone();
        weak(&mut t, sos);
        let b: Vec<Bracket> = seq.iter().map(|&i| brackets[i]).collect();
        paired_brackets(&mut t, &original, &b, sos, dir(level));
        neutrals(&mut t, sos, eos, dir(level));
        for (k, &i) in seq.iter().enumerate() {
            types[i] = t[k];
            levels[i] = implicit(level, t[k]);
        }
    }
}

fn implicit(level: u8, t: Class) -> u8 {
    if level & 1 == 0 {
        match t {
            Class::R => level + 1,
            Class::AN | Class::EN => level + 2,
            _ => level,
        }
    } else {
        match t {
            Class::L | Class::EN | Class::AN => level + 1,
            _ => level,
        }
    }
}

fn weak(t: &mut [Class], sos: Class) {
    let mut prev = sos;
    for c in t.iter_mut() {
        if *c == Class::NSM {
            *c = if prev.isolate_initiator() || prev == Class::PDI { Class::ON } else { prev };
        }
        prev = *c;
    }
    let mut strong = sos;
    for c in t.iter_mut() {
        match *c {
            Class::L | Class::R | Class::AL => strong = *c,
            Class::EN if strong == Class::AL => *c = Class::AN,
            _ => {}
        }
    }
    for c in t.iter_mut() {
        if *c == Class::AL {
            *c = Class::R;
        }
    }
    for k in 1..t.len().saturating_sub(1) {
        let (a, c) = (t[k - 1], t[k + 1]);
        t[k] = match t[k] {
            Class::ES if a == Class::EN && c == Class::EN => Class::EN,
            Class::CS if a == Class::EN && c == Class::EN => Class::EN,
            Class::CS if a == Class::AN && c == Class::AN => Class::AN,
            other => other,
        };
    }
    let mut k = 0;
    while k < t.len() {
        if t[k] != Class::ET {
            k += 1;
            continue;
        }
        let start = k;
        while k < t.len() && t[k] == Class::ET {
            k += 1;
        }
        let next_en = t.get(k) == Some(&Class::EN);
        let prev_en = start > 0 && t[start - 1] == Class::EN;
        if next_en || prev_en {
            t[start..k].fill(Class::EN);
        }
    }
    for c in t.iter_mut() {
        if matches!(*c, Class::ES | Class::ET | Class::CS) {
            *c = Class::ON;
        }
    }
    let mut strong = sos;
    for c in t.iter_mut() {
        match *c {
            Class::L | Class::R => strong = *c,
            Class::EN if strong == Class::L => *c = Class::L,
            _ => {}
        }
    }
}

fn strong_dir(c: Class) -> Option<Class> {
    match c {
        Class::L => Some(Class::L),
        Class::R | Class::EN | Class::AN => Some(Class::R),
        _ => None,
    }
}

fn paired_brackets(t: &mut [Class], original: &[Class], b: &[Bracket], sos: Class, e: Class) {
    let mut stack: Vec<(u32, usize)> = Vec::new();
    let mut pairs: Vec<(usize, usize)> = Vec::new();
    for k in 0..t.len() {
        if t[k] != Class::ON {
            continue;
        }
        match b[k] {
            Bracket::Open(id) => {
                if stack.len() == BRACKET_STACK {
                    break;
                }
                stack.push((id, k));
            }
            Bracket::Close(id) => {
                if let Some(at) = stack.iter().rposition(|&(o, _)| o == id) {
                    pairs.push((stack[at].1, k));
                    stack.truncate(at);
                }
            }
            Bracket::None => {}
        }
    }
    if pairs.is_empty() {
        return;
    }
    pairs.sort_unstable();
    let (mut ls, mut rs) = (vec![0u32; t.len() + 1], vec![0u32; t.len() + 1]);
    for k in 0..t.len() {
        ls[k + 1] = ls[k] + u32::from(strong_dir(t[k]) == Some(Class::L));
        rs[k + 1] = rs[k] + u32::from(strong_dir(t[k]) == Some(Class::R));
    }
    let mut last_strong: Vec<Option<(usize, Class)>> = vec![None; t.len() + 1];
    for k in 0..t.len() {
        last_strong[k + 1] = strong_dir(t[k]).map(|d| (k, d)).or(last_strong[k]);
    }
    let mut set: BTreeMap<usize, Class> = BTreeMap::new();
    for (o, c) in pairs {
        let (l_in, r_in) = (ls[c] - ls[o + 1], rs[c] - rs[o + 1]);
        let (same, opposite) = if e == Class::L { (l_in, r_in) } else { (r_in, l_in) };
        let to = if same > 0 {
            e
        } else if opposite > 0 {
            let before = match (last_strong[o], set.range(..o).next_back()) {
                (Some((p, d)), Some((&q, &s))) => Some(if q > p { s } else { d }),
                (Some((_, d)), None) => Some(d),
                (None, Some((_, &s))) => Some(s),
                (None, None) => None,
            };
            let ctx = before.unwrap_or(sos);
            if ctx != e { ctx } else { e }
        } else {
            continue;
        };
        for at in [o, c] {
            t[at] = to;
            set.insert(at, to);
            let mut k = at + 1;
            while k < t.len() && original[k] == Class::NSM {
                t[k] = to;
                set.insert(k, to);
                k += 1;
            }
        }
    }
}

fn neutrals(t: &mut [Class], sos: Class, eos: Class, e: Class) {
    let mut k = 0;
    while k < t.len() {
        if !t[k].ni() {
            k += 1;
            continue;
        }
        let start = k;
        while k < t.len() && t[k].ni() {
            k += 1;
        }
        let before = if start == 0 { Some(sos) } else { strong_dir(t[start - 1]) };
        let after = if k == t.len() { Some(eos) } else { strong_dir(t[k]) };
        let to = match (before, after) {
            (Some(a), Some(b)) if a == b => a,
            _ => e,
        };
        t[start..k].fill(to);
    }
}
