pub const MAX_MANIFEST_BYTES: usize = 256 << 10;
pub const MAX_RECORD_BYTES: u64 = 4 << 20;
pub const MAX_FILE_BYTES: u64 = 512 << 20;
pub const MAX_TOML_DEPTH: usize = 32;
pub const MAX_ENTRIES: usize = 16384;
pub const MAX_LIST: usize = 1024;
pub const MAX_SHORT_TEXT: usize = 256;
pub const MAX_LONG_TEXT: usize = 4096;
pub const MAX_PACKAGE_BYTES: u64 = 2 << 30;
pub const MAX_ZIP_RATIO: u64 = 100;
pub const RATIO_FLOOR: u64 = 1 << 20;

pub fn depth(v: &toml::Value) -> usize {
    match v {
        toml::Value::Table(t) => table_depth(t),
        toml::Value::Array(a) => 1 + a.iter().map(depth).max().unwrap_or(0),
        _ => 0,
    }
}

pub fn table_depth(t: &toml::Table) -> usize {
    1 + t.values().map(depth).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depths() {
        let t: toml::Table = toml::from_str("a = 1\n[b]\nc = [[1], 2]\n[b.d]\ne = {f = 1}").unwrap();
        assert_eq!(table_depth(&t), 4);
        assert_eq!(table_depth(&toml::Table::new()), 1);
        assert_eq!(depth(&toml::Value::Integer(1)), 0);
    }
}
