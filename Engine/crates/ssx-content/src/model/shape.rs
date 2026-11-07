use super::Limits;

const TOP: [&str; 15] = [
    "accessors",
    "animations",
    "buffers",
    "bufferViews",
    "cameras",
    "images",
    "materials",
    "meshes",
    "nodes",
    "samplers",
    "scenes",
    "skins",
    "textures",
    "extensionsUsed",
    "extensionsRequired",
];
const NODES: usize = 8;
const NAMES: [&str; 9] =
    ["min", "max", "primitives", "attributes", "targets", "children", "nodes", "extras", "extensions"];
const MIN_MAX_ENTRIES: usize = 16;
const MAX_NAME_BYTES: usize = 6 * 18;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Class {
    Root,
    Top(usize),
    Accessor(usize),
    MinMax(usize, &'static str),
    Mesh(usize),
    Primitives(usize),
    Primitive(usize, usize),
    Attributes(usize, usize),
    Targets,
    Node(usize),
    Children(usize),
    Scene(usize),
    SceneNodes(usize),
    Extensions,
    Skipped,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Open,
    Key,
    Value,
    After,
}

struct Frame {
    class: Class,
    object: bool,
    count: usize,
    state: State,
    key: Option<&'static str>,
}

struct Scan<'l> {
    limits: &'l Limits,
    stack: Vec<Frame>,
    done: bool,
    top: [usize; TOP.len()],
    primitives: usize,
    targets: usize,
    children: usize,
    extension_values: usize,
    longest_children: Option<(usize, usize)>,
    longest_scene: Option<(usize, usize)>,
    first: Option<String>,
}

pub fn check(json: &[u8], limits: &Limits) -> Result<(), String> {
    let mut s = Scan {
        limits,
        stack: Vec::with_capacity(limits.max_json_depth.min(256)),
        done: false,
        top: [0; TOP.len()],
        primitives: 0,
        targets: 0,
        children: 0,
        extension_values: 0,
        longest_children: None,
        longest_scene: None,
        first: None,
    };
    s.run(json);
    while let Some(f) = s.stack.pop() {
        s.closed(&f);
    }
    s.verdict()
}

impl Scan<'_> {
    fn run(&mut self, json: &[u8]) {
        let mut i = 0;
        while i < json.len() {
            let ok = match json[i] {
                b' ' | b'\t' | b'\n' | b'\r' => true,
                b'"' => {
                    let Some(end) = string_end(json, i) else { return };
                    let ok = self.string(&json[i + 1..end - 1]);
                    i = end;
                    if !ok {
                        return;
                    }
                    continue;
                }
                b'{' => self.open(true),
                b'[' => self.open(false),
                b'}' => self.close(true),
                b']' => self.close(false),
                b':' => self.colon(),
                b',' => self.comma(),
                _ => {
                    if !self.value() {
                        return;
                    }
                    while i < json.len() && !b",]}:\"{[ \t\n\r".contains(&json[i]) {
                        i += 1;
                    }
                    continue;
                }
            };
            if !ok {
                return;
            }
            i += 1;
        }
    }

    fn value(&mut self) -> bool {
        let Some(f) = self.stack.last_mut() else {
            return !std::mem::replace(&mut self.done, true);
        };
        let first = !f.object && f.state == State::Open;
        if !(f.state == State::Value || first) {
            return false;
        }
        f.state = State::After;
        if !f.object {
            f.count += 1;
            let class = f.class;
            self.entry(class);
        }
        true
    }

    fn entry(&mut self, class: Class) {
        match class {
            Class::Primitives(_) => self.primitives += 1,
            Class::Targets => self.targets += 1,
            Class::Children(_) => self.children += 1,
            Class::Extensions => self.extension_values += 1,
            _ => {}
        }
    }

    fn string(&mut self, raw: &[u8]) -> bool {
        match self.stack.last_mut() {
            Some(f) if f.object && f.state == State::Open => {
                f.count += 1;
                f.state = State::Key;
                f.key = name(raw);
                let class = f.class;
                self.entry(class);
                true
            }
            _ => self.value(),
        }
    }

    fn colon(&mut self) -> bool {
        match self.stack.last_mut() {
            Some(f) if f.object && f.state == State::Key => {
                f.state = State::Value;
                true
            }
            _ => false,
        }
    }

    fn comma(&mut self) -> bool {
        match self.stack.last_mut() {
            Some(f) if f.state == State::After => {
                f.state = if f.object { State::Open } else { State::Value };
                true
            }
            _ => false,
        }
    }

    fn open(&mut self, object: bool) -> bool {
        if self.stack.len() >= self.limits.max_json_depth || !self.value() {
            return false;
        }
        let class = self.class_of(object);
        self.stack.push(Frame { class, object, count: 0, state: State::Open, key: None });
        true
    }

    fn close(&mut self, object: bool) -> bool {
        let Some(f) = self.stack.last() else { return false };
        let empty = f.count == 0 && f.state == State::Open;
        if f.object != object || !(f.state == State::After || empty) {
            return false;
        }
        if let Some(f) = self.stack.pop() {
            self.closed(&f);
        }
        true
    }

    fn class_of(&mut self, object: bool) -> Class {
        let Some(p) = self.stack.last() else {
            return if object { Class::Root } else { Class::Other };
        };
        let (index, key) = (p.count.saturating_sub(1), if p.object { p.key } else { None });
        match (p.class, key) {
            (Class::Skipped, _) => Class::Skipped,
            (Class::Extensions, _) => Class::Extensions,
            (Class::MinMax(i, which), _) => {
                self.wrong(format!("accessors[{i}].{which}: not an array of numbers"));
                Class::Other
            }
            (_, Some("extras")) => Class::Skipped,
            (_, Some("extensions")) => Class::Extensions,
            (Class::Root, Some(k)) if !object => TOP.iter().position(|t| *t == k).map_or(Class::Other, Class::Top),
            (Class::Top(t), None) if object => match TOP[t] {
                "accessors" => Class::Accessor(index),
                "meshes" => Class::Mesh(index),
                "nodes" => Class::Node(index),
                "scenes" => Class::Scene(index),
                _ => Class::Other,
            },
            (Class::Accessor(i), Some(k @ ("min" | "max"))) if object => {
                self.wrong(format!("accessors[{i}].{k}: not an array of numbers"));
                Class::Other
            }
            (Class::Accessor(i), Some(k @ ("min" | "max"))) => Class::MinMax(i, k),
            (Class::Mesh(m), Some("primitives")) if !object => Class::Primitives(m),
            (Class::Primitives(m), None) if object => Class::Primitive(m, index),
            (Class::Primitive(m, i), Some("attributes")) if object => Class::Attributes(m, i),
            (Class::Primitive(..), Some("targets")) if !object => Class::Targets,
            (Class::Node(i), Some("children")) if !object => Class::Children(i),
            (Class::Scene(i), Some("nodes")) if !object => Class::SceneNodes(i),
            _ => Class::Other,
        }
    }

    fn closed(&mut self, f: &Frame) {
        let l = self.limits;
        match f.class {
            Class::Top(t) => self.top[t] = self.top[t].max(f.count),
            Class::Attributes(m, p) if f.count > l.max_attributes => self.wrong(format!(
                "meshes[{m}].primitives[{p}].attributes: {} attributes; at most {} are read",
                f.count, l.max_attributes
            )),
            Class::MinMax(i, which) if f.count > MIN_MAX_ENTRIES => self.wrong(format!(
                "accessors[{i}].{which}: {} entries; at most {MIN_MAX_ENTRIES} (one per component)",
                f.count
            )),
            Class::Children(i) if self.longest_children.is_none_or(|(_, n)| f.count > n) => {
                self.longest_children = Some((i, f.count))
            }
            Class::SceneNodes(i) if self.longest_scene.is_none_or(|(_, n)| f.count > n) => {
                self.longest_scene = Some((i, f.count))
            }
            _ => {}
        }
    }

    fn wrong(&mut self, message: String) {
        self.first.get_or_insert(message);
    }

    fn verdict(&self) -> Result<(), String> {
        let l = self.limits;
        if let Some((name, n)) = TOP.iter().zip(self.top).find(|&(_, n)| n > l.max_items) {
            return Err(format!("{n} {name}; at most {} are read", l.max_items));
        }
        if self.primitives > l.max_primitives {
            return Err(format!("{} mesh primitives; at most {} are read", self.primitives, l.max_primitives));
        }
        if let Some(e) = &self.first {
            return Err(e.clone());
        }
        if self.targets > l.max_items {
            return Err(format!(
                "{} morph targets in all primitives together; at most {} are read",
                self.targets, l.max_items
            ));
        }
        let nodes = self.top[NODES];
        if self.children > nodes {
            let (i, n) = self.longest_children.unwrap_or_default();
            return Err(format!(
                "the nodes' children lists hold {} entries together, more than there are nodes ({nodes}): glTF nodes \
                 form trees, a node is the child of one node at most; the longest is nodes[{i}].children, {n} entries",
                self.children
            ));
        }
        if let Some((i, n)) = self.longest_scene
            && n > nodes
        {
            return Err(format!(
                "scenes[{i}].nodes: {n} entries, more than there are nodes ({nodes}): a scene names a root once"
            ));
        }
        if self.extension_values > l.max_extension_values {
            return Err(format!(
                "{} values in extensions; at most {} are read (no extension is read)",
                self.extension_values, l.max_extension_values
            ));
        }
        Ok(())
    }
}

fn name(raw: &[u8]) -> Option<&'static str> {
    let known = |s: &[u8]| TOP.iter().chain(&NAMES).copied().find(|n| n.as_bytes() == s);
    if !raw.contains(&b'\\') {
        return known(raw);
    }
    if raw.len() > MAX_NAME_BYTES {
        return None;
    }
    let mut quoted = Vec::with_capacity(raw.len() + 2);
    quoted.push(b'"');
    quoted.extend_from_slice(raw);
    quoted.push(b'"');
    let decoded: String = serde_json::from_slice(&quoted).ok()?;
    known(decoded.as_bytes())
}

fn string_end(json: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    while i < json.len() {
        match json[i] {
            b'\\' => i += 2,
            b'"' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(json: &str, limits: &Limits, want: &str) {
        let e = check(json.as_bytes(), limits).expect_err(json);
        assert!(e.contains(want), "{e}");
    }

    fn passes(json: &str, limits: &Limits) {
        check(json.as_bytes(), limits).unwrap_or_else(|e| panic!("{json}: {e}"));
    }

    #[test]
    fn children_and_scene_lists_name_each_node_once_at_most() {
        let l = Limits::default();
        passes(r#"{"scenes":[{"nodes":[0]}],"nodes":[{"children":[1,2]},{},{}]}"#, &l);
        let wide = r#"{"scenes":[{"nodes":[0]}],"nodes":[{"children":[1]},{"children":[1,1,1]}]}"#;
        refused(wide, &l, "hold 4 entries together, more than there are nodes (2)");
        refused(wide, &l, "the longest is nodes[1].children, 3 entries");
        refused(
            r#"{"scenes":[{},{"nodes":[0,0,0]}],"nodes":[{},{}]}"#,
            &l,
            "scenes[1].nodes: 3 entries, more than there are nodes (2)",
        );
        refused(r#"{"scenes":[{"n\u006fdes":[0,0,0]}],"nodes":[{},{}]}"#, &l, "scenes[0].nodes: 3 entries");
        refused(r#"{"n\u006fdes":[{"childr\u0065n":[1,1,1]},{}]}"#, &l, "more than there are nodes (2)");
        passes(r#"{"scenes":[{"nodes":[0]}],"nodes":[{"extras":{"children":[1,1,1,1]}},{}]}"#, &l);
        passes(r#"{"nodes":[{"extensions":{"X":{"children":[1,1,1,1]}}}]}"#, &l);
    }

    #[test]
    fn lists_are_counted_before_parsing() {
        let l = Limits::default();
        let few = Limits { max_items: 4, max_primitives: 3, max_attributes: 2, max_extension_values: 5, ..l };
        passes(r#"{"nodes":[{},{},{},{}],"extensionsUsed":["a","b"]}"#, &few);
        refused(r#"{"nodes":[{},{},{},{},{}]}"#, &few, "5 nodes; at most 4 are read");
        refused(r#"{"extensionsUsed":["a","b","c","d","e"]}"#, &few, "5 extensionsUsed; at most 4");
        let prims = r#"{"attributes":{}},{"attributes":{}}"#;
        refused(
            &format!(r#"{{"meshes":[{{"primitives":[{prims}]}},{{"primitives":[{prims}]}}]}}"#),
            &few,
            "4 mesh primitives",
        );
        let attributes = r#"{"meshes":[{},{"primitives":[{"attributes":{"POSITION":0,"NORMAL":1,"_X":2}}]}]}"#;
        refused(attributes, &few, "meshes[1].primitives[0].attributes: 3 attributes; at most 2 are read");
        let targets = r#"{"attributes":{},"targets":[{},{},{}]}"#;
        refused(&format!(r#"{{"meshes":[{{"primitives":[{targets},{targets}]}}]}}"#), &few, "6 morph targets in all");
        let seventeen = format!("[{}]", ["0"; 17].join(","));
        refused(&format!(r#"{{"accessors":[{{"min":{seventeen}}}]}}"#), &l, "accessors[0].min: 17 entries; at most 16");
        passes(&format!(r#"{{"accessors":[{{"max":[{}]}}]}}"#, ["1.5"; 16].join(",")), &l);
        refused(r#"{"accessors":[{},{"max":[[0]]}]}"#, &l, "accessors[1].max: not an array of numbers");
        refused(r#"{"accessors":[{"min":{"a":[0,0]}}]}"#, &l, "accessors[0].min: not an array of numbers");
        refused(r#"{"nodes":[{"extensions":{"X":[1,2,3,4,5]}}]}"#, &few, "6 values in extensions; at most 5");
        passes(r#"{"nodes":[{"extras":[1,2,3,4,5,6,7,8],"extensions":{"X":[1,2,3]}}]}"#, &few);
        refused(r#"{"nodes":[{},{},{},{},{}"#, &few, "5 nodes");
    }

    #[test]
    fn the_scan_follows_the_json_and_stops_where_it_ends() {
        let few = Limits { max_items: 2, ..Limits::default() };
        passes(r#"{"nodes":[{"name":"a\"],[{\\"},{"name":"}}]]"}]}"#, &few);
        passes(r#"{"nodes" : [ {"matrix":[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1.0e0]} , { } ]}"#, &few);
        for text in
            ["", "]]]]", "\"open", "{\"nodes\":[{},{}]}trailing", "{\"nodes\":[{} {}]}", "{\"nodes\":[{},]}", "[1]"]
        {
            passes(text, &few);
        }
        refused(r#"{"nodes":[{},{},{}],]"#, &few, "3 nodes");
        let deep = format!("{}{}", "[".repeat(200), "]".repeat(200));
        passes(&deep, &Limits::default());
        assert_eq!(name(br"n\u006fdes"), Some("nodes"));
        assert_eq!(name(&[b"\\u0020".as_slice(); 40].concat()), None);
        assert_eq!(name(b"children"), Some("children"));
        assert_eq!(name(br"bad\escape"), None);
    }
}
