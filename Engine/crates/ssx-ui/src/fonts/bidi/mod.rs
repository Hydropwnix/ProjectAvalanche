pub mod class;
mod reorder;
mod sequences;
#[cfg(test)]
mod tests;

pub use reorder::{line_levels, visual_order};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Class {
    L,
    R,
    AL,
    EN,
    ES,
    ET,
    AN,
    CS,
    NSM,
    BN,
    B,
    S,
    WS,
    ON,
    LRE,
    LRO,
    RLE,
    RLO,
    PDF,
    LRI,
    RLI,
    FSI,
    PDI,
}

impl Class {
    pub fn isolate_initiator(self) -> bool {
        matches!(self, Class::LRI | Class::RLI | Class::FSI)
    }

    pub fn removed(self) -> bool {
        matches!(self, Class::LRE | Class::LRO | Class::RLE | Class::RLO | Class::PDF | Class::BN)
    }

    fn ni(self) -> bool {
        matches!(self, Class::B | Class::S | Class::WS | Class::ON | Class::LRI | Class::RLI | Class::FSI | Class::PDI)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Bracket {
    #[default]
    None,
    Open(u32),
    Close(u32),
}

pub const MAX_DEPTH: u8 = 125;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub base: u8,
    pub levels: Vec<u8>,
}

fn matching_pdis(classes: &[Class]) -> Vec<Option<usize>> {
    let mut out = vec![None; classes.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, &c) in classes.iter().enumerate() {
        if c.isolate_initiator() {
            open.push(i);
        } else if c == Class::PDI
            && let Some(o) = open.pop()
        {
            out[o] = Some(i);
        } else if c == Class::B {
            open.clear();
        }
    }
    out
}

fn first_strong(classes: &[Class], pdis: &[Option<usize>]) -> Option<u8> {
    let mut i = 0;
    while i < classes.len() {
        match classes[i] {
            Class::L => return Some(0),
            Class::R | Class::AL => return Some(1),
            c if c.isolate_initiator() => i = pdis[i]?,
            Class::B => return None,
            _ => {}
        }
        i += 1;
    }
    None
}

fn fsi_directions(classes: &[Class]) -> Vec<bool> {
    let mut rtl = vec![false; classes.len()];
    let mut open: Vec<(usize, bool)> = Vec::new();
    for (i, &c) in classes.iter().enumerate() {
        match c {
            Class::LRI | Class::RLI | Class::FSI => open.push((i, true)),
            Class::PDI => {
                open.pop();
            }
            Class::L | Class::R | Class::AL => {
                if let Some(top) = open.last_mut().filter(|t| t.1) {
                    rtl[top.0] = c != Class::L;
                    top.1 = false;
                }
            }
            Class::B => open.clear(),
            _ => {}
        }
    }
    rtl
}

#[derive(Clone, Copy)]
struct Status {
    level: u8,
    over: Option<Class>,
    isolate: bool,
}

fn next_level(level: u8, odd: bool) -> u8 {
    if odd { (level + 1) | 1 } else { (level + 2) & !1 }
}

pub fn resolve(classes: &[Class], brackets: &[Bracket], base: Option<u8>) -> Resolved {
    let n = classes.len();
    let pdis = matching_pdis(classes);
    let base = base.map_or_else(|| first_strong(classes, &pdis).unwrap_or(0), |b| b.min(1));
    let mut levels = vec![base; n];
    let mut types: Vec<Class> = classes.to_vec();
    explicit(classes, base, &mut levels, &mut types);
    sequences::resolve_implicit(classes, brackets, &pdis, base, &mut levels, &mut types);
    for i in 0..n {
        if classes[i].removed() {
            levels[i] = if i == 0 { base } else { levels[i - 1] };
        }
    }
    Resolved { base, levels }
}

fn explicit(classes: &[Class], base: u8, levels: &mut [u8], types: &mut [Class]) {
    let fsi_rtl = fsi_directions(classes);
    let mut stack: Vec<Status> = Vec::with_capacity(usize::from(MAX_DEPTH) + 2);
    stack.push(Status { level: base, over: None, isolate: false });
    let (mut overflow_isolates, mut overflow_embeddings, mut valid_isolates) = (0usize, 0usize, 0usize);
    for i in 0..classes.len() {
        let top = stack.last().copied().unwrap_or(Status { level: base, over: None, isolate: false });
        let c = classes[i];
        match c {
            Class::RLE | Class::LRE | Class::RLO | Class::LRO => {
                levels[i] = top.level;
                let level = next_level(top.level, matches!(c, Class::RLE | Class::RLO));
                if level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0 {
                    let over = match c {
                        Class::RLO => Some(Class::R),
                        Class::LRO => Some(Class::L),
                        _ => None,
                    };
                    stack.push(Status { level, over, isolate: false });
                } else if overflow_isolates == 0 {
                    overflow_embeddings += 1;
                }
            }
            Class::RLI | Class::LRI | Class::FSI => {
                levels[i] = top.level;
                if let Some(o) = top.over {
                    types[i] = o;
                }
                let rtl = match c {
                    Class::RLI => true,
                    Class::LRI => false,
                    _ => fsi_rtl[i],
                };
                let level = next_level(top.level, rtl);
                if level <= MAX_DEPTH && overflow_isolates == 0 && overflow_embeddings == 0 {
                    valid_isolates += 1;
                    stack.push(Status { level, over: None, isolate: true });
                } else {
                    overflow_isolates += 1;
                }
            }
            Class::PDI => {
                if overflow_isolates > 0 {
                    overflow_isolates -= 1;
                } else if valid_isolates > 0 {
                    overflow_embeddings = 0;
                    while stack.last().is_some_and(|s| !s.isolate) && stack.len() > 1 {
                        stack.pop();
                    }
                    if stack.len() > 1 {
                        stack.pop();
                    }
                    valid_isolates -= 1;
                }
                let top = stack.last().copied().unwrap_or(top);
                levels[i] = top.level;
                if let Some(o) = top.over {
                    types[i] = o;
                }
            }
            Class::PDF => {
                levels[i] = top.level;
                if overflow_isolates > 0 {
                } else if overflow_embeddings > 0 {
                    overflow_embeddings -= 1;
                } else if !top.isolate && stack.len() >= 2 {
                    stack.pop();
                }
            }
            Class::B => levels[i] = base,
            Class::BN => levels[i] = top.level,
            _ => {
                levels[i] = top.level;
                if let Some(o) = top.over {
                    types[i] = o;
                }
            }
        }
    }
}
