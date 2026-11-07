use toml::{Table, Value};

pub fn merge(base: &mut Table, patch: &Table) {
    for (key, value) in patch {
        match (base.get_mut(key), value) {
            (Some(Value::Table(b)), Value::Table(p)) => merge(b, p),
            _ => {
                base.insert(key.clone(), value.clone());
            }
        }
    }
}

pub fn apply<'a>(base: &Table, patches: impl IntoIterator<Item = &'a Table>) -> Table {
    let mut out = base.clone();
    for p in patches {
        merge(&mut out, p);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(s: &str) -> Table {
        toml::from_str(s).unwrap()
    }

    #[test]
    fn tables_merge_scalars_and_arrays_replace() {
        let base = t(r#"
            name = "elise_default"
            stats = [1, 2, 3]
            [physics]
            speed = 10
            edge = 0.5
            [physics.air]
            spin = 1
            "#);
        let patch = t(r#"
            stats = [9]
            [physics]
            speed = 12
            [physics.air]
            flip = 2
            "#);
        let out = apply(&base, [&patch]);
        assert_eq!(out["name"].as_str(), Some("elise_default"));
        assert_eq!(out["stats"].as_array().unwrap().len(), 1);
        let physics = out["physics"].as_table().unwrap();
        assert_eq!(physics["speed"].as_integer(), Some(12));
        assert_eq!(physics["edge"].as_float(), Some(0.5));
        let air = physics["air"].as_table().unwrap();
        assert_eq!((air["spin"].as_integer(), air["flip"].as_integer()), (Some(1), Some(2)));
    }

    #[test]
    fn order_and_type_changes() {
        let base = t("a = 1\n[b]\nc = 1");
        let p1 = t("a = 2\nb = 5");
        let p2 = t("a = 3\n[d]\ne = true");
        let out = apply(&base, [&p1, &p2]);
        assert_eq!(out["a"].as_integer(), Some(3), "later patches win");
        assert_eq!(out["b"].as_integer(), Some(5), "a scalar replaces a table");
        assert_eq!(out["d"]["e"].as_bool(), Some(true), "new keys are added");
        let back = apply(&out, [&t("[b]\nc = 7")]);
        assert_eq!(back["b"]["c"].as_integer(), Some(7), "a table replaces a scalar");
        assert_eq!(apply(&base, []), base);
    }
}
