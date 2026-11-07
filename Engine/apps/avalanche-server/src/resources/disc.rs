use std::collections::HashMap;
use std::sync::OnceLock;

use ssx_content::ContentHash;

const TSV: &str = include_str!("../../../../assets/server/disc-hashes.tsv");
const HEADER: &str = "blake3\tsize\twhat";

type List = HashMap<ContentHash, (u64, &'static str)>;

fn parse(text: &'static str) -> Result<List, String> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err("not its header".into());
    }
    let mut out = HashMap::new();
    for (i, l) in lines.enumerate() {
        let mut c = l.splitn(3, '\t');
        let (Some(h), Some(n), Some(what)) = (c.next(), c.next(), c.next()) else {
            return Err(format!("line {}: three columns", i + 2));
        };
        let hash = h.parse().map_err(|e| format!("line {}: {e}", i + 2))?;
        let size = n.parse().map_err(|_| format!("line {}: the size", i + 2))?;
        out.insert(hash, (size, what));
    }
    Ok(out)
}

fn list() -> Result<&'static List, String> {
    static LIST: OnceLock<Result<List, String>> = OnceLock::new();
    LIST.get_or_init(|| parse(TSV)).as_ref().map_err(|e| format!("the disc's hashes: {e}"))
}

pub fn disc_file(hash: &ContentHash, size: u64) -> Result<Option<&'static str>, String> {
    Ok(list()?.get(hash).filter(|(n, _)| *n == size).map(|(_, what)| *what))
}

pub fn count() -> Result<usize, String> {
    Ok(list()?.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_list_reads_and_names_disc_files() {
        let list = list().expect("reads");
        assert_eq!(list.len(), 21_336);
        let (h, (n, _)) = list.iter().find(|(_, (_, w))| *w == "disc/default.xex").expect("the executable");
        assert_eq!(disc_file(h, *n), Ok(Some("disc/default.xex")));
        assert_eq!(disc_file(h, n + 1), Ok(None), "a size of its own is another file");
        assert_eq!(disc_file(&ContentHash::of(b"a resource's own bytes"), 22), Ok(None));
    }
}
