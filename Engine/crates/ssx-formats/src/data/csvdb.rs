use crate::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geotag {
    pub id: u32,
    pub quality: u32,
    pub level: u32,
    pub position: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GhostRow {
    pub ui_drop: u32,
    pub starting_idx: i32,
    pub ghost_id: i32,
    pub score: u32,
    pub tuned_score: u32,
    pub character: u32,
    pub catchup: u32,
    pub splits: [u32; 9],
}

impl GhostRow {
    pub fn value(&self) -> u32 {
        if self.tuned_score == u32::MAX { self.score } else { self.tuned_score }
    }
}

fn data_lines(text: &str) -> impl Iterator<Item = (usize, &str)> {
    text.split('\n').map(|l| l.trim_end_matches('\r')).enumerate().skip(1).filter(|(_, l)| !l.trim().is_empty())
}

fn int(s: &str, line: usize, what: &str) -> Result<i64> {
    s.trim()
        .parse::<i64>()
        .map_err(|_| Error::invalid(format!("csv line {}: {what} {s:?} is not an integer", line + 1)))
}

pub fn parse_geotags(text: &str) -> Result<Vec<Geotag>> {
    let mut out = Vec::new();
    for (n, line) in data_lines(text) {
        let f: Vec<&str> = line.split(',').collect();
        if f.len() != 6 {
            return Err(Error::invalid(format!("geotagdb line {}: {} columns, expected 6", n + 1, f.len())));
        }
        let float = |i: usize| -> Result<f32> {
            f[i].trim().parse().map_err(|_| Error::invalid(format!("geotagdb line {}: column {i} not a number", n + 1)))
        };
        out.push(Geotag {
            id: int(f[0], n, "id")? as u32,
            quality: int(f[1], n, "quality")? as u32,
            level: int(f[2], n, "level")? as u32,
            position: [float(3)?, float(4)?, float(5)?],
        });
    }
    Ok(out)
}

pub fn parse_ghosts(text: &str) -> Result<Vec<GhostRow>> {
    let mut out = Vec::new();
    for (n, line) in data_lines(text) {
        let f: Vec<&str> = line.trim_end_matches(',').split(',').collect();
        if f.len() != 16 {
            return Err(Error::invalid(format!("ghostdb line {}: {} columns, expected 16", n + 1, f.len())));
        }
        let mut splits = [0u32; 9];
        for (k, s) in splits.iter_mut().enumerate() {
            *s = int(f[7 + k], n, "split")? as u32;
        }
        out.push(GhostRow {
            ui_drop: int(f[0], n, "ui_drop_id")? as u32,
            starting_idx: int(f[1], n, "startingidx")? as i32,
            ghost_id: int(f[2], n, "ghostid")? as i32,
            score: int(f[3], n, "score")? as u32,
            tuned_score: int(f[4], n, "tuned_score")? as u32,
            character: int(f[5], n, "characterid")? as u32,
            catchup: int(f[6], n, "catchup")? as u32,
            splits,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geotags() {
        let g = parse_geotags("Id,Quality,Level,XPos,YPos,ZPos\r\n1,2,4,-1.5,2,3.25\r\n\r\n").unwrap();
        assert_eq!(g, [Geotag { id: 1, quality: 2, level: 4, position: [-1.5, 2.0, 3.25] }]);
        assert!(parse_geotags("h\n1,2,3\n").is_err());
    }

    #[test]
    fn ghosts() {
        let t = "h\r\n7,1,9,100,-1,3660148671,5,0,1,2,3,4,5,6,7,8,\r\n";
        let r = parse_ghosts(t).unwrap();
        assert_eq!((r[0].ui_drop, r[0].tuned_score, r[0].value(), r[0].character), (7, u32::MAX, 100, 3_660_148_671));
        assert_eq!(r[0].splits[8], 8);
        assert!(parse_ghosts("h\n1,2\n").is_err());
    }
}
