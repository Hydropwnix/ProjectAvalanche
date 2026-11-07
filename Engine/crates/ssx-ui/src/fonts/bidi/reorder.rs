use std::ops::Range;

use super::Class;

pub fn line_levels(classes: &[Class], levels: &[u8], base: u8, range: Range<usize>) -> Vec<u8> {
    let mut out: Vec<u8> = levels[range.clone()].to_vec();
    let cls = &classes[range];
    let resettable =
        |c: Class| matches!(c, Class::WS | Class::FSI | Class::LRI | Class::RLI | Class::PDI) || c.removed();
    let mut trailing = true;
    for k in (0..cls.len()).rev() {
        match cls[k] {
            Class::S | Class::B => {
                out[k] = base;
                trailing = true;
            }
            c if resettable(c) => {
                if trailing {
                    out[k] = base;
                }
            }
            _ => trailing = false,
        }
    }
    out
}

pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..levels.len()).collect();
    let (Some(&max), Some(&min)) = (levels.iter().max(), levels.iter().min()) else { return order };
    let min_odd = min | 1;
    let mut lv: Vec<u8> = levels.to_vec();
    let mut level = max;
    while level >= min_odd {
        let mut k = 0;
        while k < lv.len() {
            if lv[k] < level {
                k += 1;
                continue;
            }
            let start = k;
            while k < lv.len() && lv[k] >= level {
                k += 1;
            }
            order[start..k].reverse();
            lv[start..k].reverse();
        }
        if level == 0 {
            break;
        }
        level -= 1;
    }
    order
}
