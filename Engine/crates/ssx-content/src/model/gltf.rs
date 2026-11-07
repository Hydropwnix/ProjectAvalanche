use std::collections::HashMap;
use std::sync::Arc;

use gltf_json::Root;
use gltf_json::validation::Checked;

use super::collision::{self, CollisionTriangle};
use super::mesh::{Budget, Placing, Transform, primitive};
use super::uri::{self, Target, truncate};
use super::{AlphaMode, Extras, ImageRef, Limits, Lod, Model, ModelError, ModelImage, ModelMaterial, Part, glb, json};
use crate::hash::ContentHash;

mod nodes;
pub(super) use nodes::NodeMeshes;

pub trait FileReader {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String>;
    fn size(&self, path: &str) -> Result<u64, String>;
    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String>;
}

impl FileReader for () {
    fn read(&self, path: &str, _max: u64) -> Result<Vec<u8>, String> {
        Err(format!("{path}: no files are read beside this model"))
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        Err(format!("{path}: no files are read beside this model"))
    }

    fn read_range(&self, path: &str, _offset: u64, _len: u64) -> Result<Vec<u8>, String> {
        Err(format!("{path}: no files are read beside this model"))
    }
}

fn lod_of_name(name: &str) -> Option<usize> {
    let name = match name.rsplit_once('.') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => name,
    };
    let (_, k) = name.rsplit_once("_lod")?;
    if k.is_empty() || k.len() > 2 || k.starts_with('0') || !k.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    k.parse().ok()
}

fn is_collision_name(name: &str) -> bool {
    name == "collision"
        || name.strip_prefix("collision.").is_some_and(|t| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit()))
}

fn extras(raw: &gltf_json::extras::Extras, at: &str) -> Result<Extras, String> {
    let Some(raw) = raw else { return Ok(Extras::default()) };
    let m: AvaMembers = serde_json::from_str(raw.get())
        .map_err(|e| format!("{at}.extras: {}", ssx_core::text::message(&e.to_string())))?;
    let mut out = Extras::default();
    if let Some(v) = m.group {
        let g = serde_json::from_str::<u64>(v.get()).ok();
        let g = g.filter(|g| collision::GROUPS.contains(&(*g as u32)) && *g <= u64::from(u32::MAX));
        let g = g.ok_or_else(|| {
            format!("{at}.extras.ava_group: {} is not one of {:?}", truncate(v.get()), collision::GROUPS)
        })?;
        out.group = Some(g as u32);
    }
    if let Some(v) = m.slot {
        match serde_json::from_str::<String>(v.get()) {
            Ok(t)
                if (1..=32).contains(&t.len())
                    && t.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') =>
            {
                out.slot = Some(t)
            }
            _ => return Err(format!("{at}.extras.ava_slot: a slot name of 1 to 32 characters [a-z0-9_]")),
        }
    }
    if let Some(v) = m.surface {
        let s =
            serde_json::from_str::<String>(v.get()).map_err(|_| format!("{at}.extras.ava_surface: not a string"))?;
        collision::surface_id(&s).map_err(|e| format!("{at}.extras: {e}"))?;
        out.surface = Some(s);
    }
    Ok(out)
}

#[derive(Default)]
struct AvaMembers<'a> {
    group: Option<&'a serde_json::value::RawValue>,
    slot: Option<&'a serde_json::value::RawValue>,
    surface: Option<&'a serde_json::value::RawValue>,
}

impl<'de> serde::Deserialize<'de> for AvaMembers<'de> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(AvaVisitor)
    }
}

struct AvaVisitor;

impl<'de> serde::de::Visitor<'de> for AvaVisitor {
    type Value = AvaMembers<'de>;

    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("any JSON value")
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut out = AvaMembers::default();
        while let Some(key) = map.next_key::<AvaKey>()? {
            let member = match key {
                AvaKey::Group => &mut out.group,
                AvaKey::Slot => &mut out.slot,
                AvaKey::Surface => &mut out.surface,
                AvaKey::Other => {
                    map.next_value::<serde::de::IgnoredAny>()?;
                    continue;
                }
            };
            *member = Some(map.next_value()?);
        }
        Ok(out)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
        while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
        Ok(AvaMembers::default())
    }

    fn visit_bool<E>(self, _: bool) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }

    fn visit_i64<E>(self, _: i64) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }

    fn visit_u64<E>(self, _: u64) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }

    fn visit_f64<E>(self, _: f64) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }

    fn visit_str<E>(self, _: &str) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(AvaMembers::default())
    }
}

enum AvaKey {
    Group,
    Slot,
    Surface,
    Other,
}

impl<'de> serde::Deserialize<'de> for AvaKey {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Name;
        impl serde::de::Visitor<'_> for Name {
            type Value = AvaKey;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a member name")
            }
            fn visit_str<E>(self, s: &str) -> Result<AvaKey, E> {
                Ok(match s {
                    "ava_group" => AvaKey::Group,
                    "ava_slot" => AvaKey::Slot,
                    "ava_surface" => AvaKey::Surface,
                    _ => AvaKey::Other,
                })
            }
        }
        d.deserialize_str(Name)
    }
}

pub(super) struct Reader<'a> {
    pub(super) root: &'a Root,
    buffers: Vec<&'a [u8]>,
    limits: &'a Limits,
    budget: Budget,
    notes: Vec<String>,
}

pub(super) struct Held<'l> {
    limits: &'l Limits,
    total: u64,
    files: HashMap<String, Arc<Vec<u8>>>,
    ranges: HashMap<(usize, u64, u64), Arc<Vec<u8>>>,
}

impl<'l> Held<'l> {
    fn new(limits: &'l Limits) -> Self {
        Held { limits, total: 0, files: HashMap::new(), ranges: HashMap::new() }
    }

    fn remaining(&self) -> u64 {
        self.limits.max_held_bytes.saturating_sub(self.total)
    }

    fn total_note(&self) -> String {
        format!(
            "a model holds at most {} bytes of buffers and images in all, {} so far",
            self.limits.max_held_bytes, self.total
        )
    }

    fn count(&mut self, n: u64, at: &str) -> Result<(), String> {
        if n > self.remaining() {
            return Err(format!("{at}: {n} bytes more; {}", self.total_note()));
        }
        self.total += n;
        Ok(())
    }

    fn file(&mut self, files: &dyn FileReader, path: &str, max: u64, at: &str) -> Result<Arc<Vec<u8>>, String> {
        if let Some(bytes) = self.files.get(path) {
            return Ok(bytes.clone());
        }
        let cap = max.min(self.remaining());
        let bytes = files.read(path, cap).map_err(|e| match cap < max {
            true => format!("{at}: {e} ({})", self.total_note()),
            false => format!("{at}: {e}"),
        })?;
        self.count(bytes.len() as u64, at)?;
        let bytes = Arc::new(bytes);
        self.files.insert(path.to_string(), bytes.clone());
        Ok(bytes)
    }

    fn resolve(&mut self, uri: &str, dir: &str, at: &str) -> Result<Target, String> {
        let cap = self.limits.max_data_uri_bytes.min(self.remaining());
        let target = uri::resolve(uri, dir, cap).map_err(|e| match cap < self.limits.max_data_uri_bytes {
            true => format!("{at}: {e} ({})", self.total_note()),
            false => format!("{at}: {e}"),
        })?;
        if let Target::Data { bytes, .. } = &target {
            self.count(bytes.len() as u64, at)?;
        }
        Ok(target)
    }

    fn range(&mut self, buffer: usize, data: &[u8], start: u64, end: u64, at: &str) -> Result<Arc<Vec<u8>>, String> {
        if let Some(bytes) = self.ranges.get(&(buffer, start, end)) {
            return Ok(bytes.clone());
        }
        self.count(end - start, at)?;
        let bytes = Arc::new(data[start as usize..end as usize].to_vec());
        self.ranges.insert((buffer, start, end), bytes.clone());
        Ok(bytes)
    }
}

enum BufferData<'a> {
    Chunk(&'a [u8]),
    Held(Arc<Vec<u8>>),
}

impl BufferData<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            BufferData::Chunk(b) => b,
            BufferData::Held(b) => b,
        }
    }
}

fn load_buffers<'a>(
    root: &Root,
    bin: Option<&'a [u8]>,
    dir: &str,
    files: &dyn FileReader,
    held: &mut Held<'_>,
) -> Result<Vec<BufferData<'a>>, String> {
    let mut out = Vec::with_capacity(root.buffers.len());
    for (i, b) in root.buffers.iter().enumerate() {
        let len = b.byte_length.0;
        let at = format!("buffers[{i}]");
        let data = match (&b.uri, bin) {
            (None, Some(bin)) if i == 0 => {
                held.count(bin.len() as u64, &at)?;
                BufferData::Chunk(bin)
            }
            (None, _) => return Err(format!("{at} has no data (no uri, not the .glb's first buffer)")),
            (Some(u), _) => match held.resolve(u, dir, &at)? {
                Target::Data { bytes, .. } => BufferData::Held(Arc::new(bytes)),
                Target::File(path) => BufferData::Held(held.file(files, &path, len.saturating_add(3), &at)?),
            },
        };
        let n = data.bytes().len() as u64;
        if n < len || n - len > 3 {
            let what = if matches!(data, BufferData::Chunk(_)) { "the BIN chunk" } else { "its data" };
            return Err(format!("{at} states {len} bytes, {what} has {n} (at most 3 bytes of padding)"));
        }
        out.push(data);
    }
    Ok(out)
}

pub fn decode_file(rel: &str, bytes: &[u8], files: &dyn FileReader, limits: &Limits) -> Result<Model, ModelError> {
    let mut model = read_with(rel, bytes, files, limits, |r, dir, held| r.model(dir, files, held))?;
    model.source = Some(ContentHash::of(bytes));
    Ok(model)
}

pub(super) fn read_with<R>(
    rel: &str,
    bytes: &[u8],
    files: &dyn FileReader,
    limits: &Limits,
    f: impl FnOnce(&mut Reader<'_>, &str, &mut Held<'_>) -> Result<R, String>,
) -> Result<R, ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(fail(format!("{} bytes; at most {} are read", bytes.len(), limits.max_file_bytes)));
    }
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let is_glb = bytes.starts_with(b"glTF");
    let (json, bin) = match ext.as_str() {
        "glb" if is_glb => {
            let g = glb::split(bytes, limits.max_json_bytes).map_err(fail)?;
            (g.json, g.bin)
        }
        "glb" => return Err(fail("named .glb, but its content is not a binary glTF".into())),
        "gltf" if is_glb => return Err(fail("named .gltf, but its content is a binary glTF (.glb)".into())),
        "gltf" => (bytes, None),
        _ => return Err(fail(format!("a model is a .glb or .gltf file, not .{}", truncate(&ext)))),
    };
    let root = json::parse(json, limits).map_err(fail)?;
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let mut held = Held::new(limits);
    let buffers = load_buffers(&root, bin, dir, files, &mut held).map_err(fail)?;
    let mut r = Reader {
        root: &root,
        buffers: buffers.iter().map(BufferData::bytes).collect(),
        limits,
        budget: Budget::default(),
        notes: Vec::new(),
    };
    f(&mut r, dir, &mut held).map_err(fail)
}

pub(crate) fn with_buffers<R>(
    rel: &str,
    bytes: &[u8],
    files: &dyn FileReader,
    limits: &Limits,
    f: impl FnOnce(&Root, &[&[u8]]) -> Result<R, String>,
) -> Result<R, ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    if bytes.len() as u64 > limits.max_file_bytes {
        return Err(fail(format!("{} bytes; at most {} are read", bytes.len(), limits.max_file_bytes)));
    }
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let is_glb = bytes.starts_with(b"glTF");
    let (json, bin) = match ext.as_str() {
        "glb" if is_glb => {
            let g = glb::split(bytes, limits.max_json_bytes).map_err(fail)?;
            (g.json, g.bin)
        }
        "glb" => return Err(fail("named .glb, but its content is not a binary glTF".into())),
        "gltf" if is_glb => return Err(fail("named .gltf, but its content is a binary glTF (.glb)".into())),
        "gltf" => (bytes, None),
        _ => return Err(fail(format!("a scene is a .glb or .gltf file, not .{}", truncate(&ext)))),
    };
    let root = json::parse(json, limits).map_err(fail)?;
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let mut held = Held::new(limits);
    let buffers = load_buffers(&root, bin, dir, files, &mut held).map_err(fail)?;
    let views: Vec<&[u8]> = buffers.iter().map(BufferData::bytes).collect();
    f(&root, &views).map_err(fail)
}

impl Reader<'_> {
    pub(super) fn model(&mut self, dir: &str, files: &dyn FileReader, held: &mut Held<'_>) -> Result<Model, String> {
        let root = self.root;
        if !root.extensions_used.is_empty() {
            let names = ssx_core::text::quoted_list(&root.extensions_used);
            self.notes.push(format!("the optional extensions {names} are not read"));
        }
        let mut lods: Vec<Lod> = Vec::new();
        let mut collision: Vec<CollisionTriangle> = Vec::new();
        let mut skins = Vec::new();
        if self.limits.read_skins {
            if root.skins.len() > self.limits.max_skins {
                return Err(format!("{} skins (at most {} are read)", root.skins.len(), self.limits.max_skins));
            }
            for i in 0..root.skins.len() {
                skins.push(super::skin::read_skin(root, &self.buffers, i, self.limits)?);
            }
        }
        for n in mesh_nodes(root, self.limits, &mut self.notes)? {
            let place = NodeAt {
                placing: Placing { xf: n.xf, name: n.name },
                lod: n.lod,
                in_collision: n.in_collision,
                extras: extras(&root.nodes[n.node].extras, &format!("nodes[{}]", n.node))?,
                skin: n.skin.map(|s| (s, skins[s].joints.len())),
            };
            self.mesh(n.mesh, &place, &mut lods, &mut collision)?;
        }
        if lods.first().is_none_or(|l| l.parts.is_empty()) {
            return Err("no mesh in LOD 0 (a node whose name has no `_lod<k>` and is not below `collision`)".into());
        }
        if let Some(k) = lods.iter().position(|l| l.parts.is_empty()) {
            return Err(format!("LOD {k} has no mesh while LOD {} has", lods.len() - 1));
        }
        let collision = if collision.is_empty() {
            None
        } else {
            let (c, degenerate) = collision::build(&collision);
            if degenerate > 0 {
                self.notes.push(format!("collision: {degenerate} triangles without area left out"));
            }
            Some(Arc::new(c))
        };
        let (materials, images) = self.materials(dir, files, held)?;
        self.check_drawn_images(&materials, &images)?;
        let notes = std::mem::take(&mut self.notes);
        Ok(Model { lods, collision, materials, images, skins, source: None, notes })
    }

    fn check_drawn_images(&self, materials: &[ModelMaterial], images: &[ModelImage]) -> Result<u64, String> {
        let drawn: std::collections::BTreeSet<usize> =
            materials.iter().flat_map(ModelMaterial::drawn_images).map(|r| r.image).collect();
        let (side, cap) = (self.limits.max_image_side, self.limits.max_image_decoded_bytes);
        let mut total = 0u64;
        for i in drawn {
            let img = &images[i];
            let head = &img.bytes[..img.bytes.len().min(super::probe::JPEG_HEADER_BYTES as usize)];
            let (w, h) = crate::texture::image_size(&img.file_name, head).map_err(|e| e.to_string())?;
            if w == 0 || h == 0 || w > side || h > side {
                return Err(format!("{}: {w}×{h}; an image a material draws is 1 to {side} a side", img.file_name));
            }
            total += crate::texture::chain_bytes(w, h);
            if total > cap {
                return Err(format!(
                    "the images its materials draw decode to {total} bytes or more with their mips; at most {cap} \
                     for a model"
                ));
            }
        }
        Ok(total)
    }
}

pub(super) struct MeshNode<'r> {
    pub(super) node: usize,
    pub(super) mesh: usize,
    pub(super) xf: Transform,
    pub(super) name: &'r str,
    pub(super) lod: usize,
    pub(super) in_collision: bool,
    pub(super) skin: Option<usize>,
}

pub(super) fn mesh_nodes<'r>(
    root: &'r Root,
    limits: &Limits,
    notes: &mut Vec<String>,
) -> Result<Vec<MeshNode<'r>>, String> {
    let scene = match (root.scene, root.scenes.len()) {
        (Some(s), _) => s.value(),
        (None, 1) => 0,
        (None, 0) => return Err("no scene".into()),
        (None, n) => return Err(format!("{n} scenes and none chosen (`scene`)")),
    };
    let mut out = Vec::new();
    let mut visited = vec![false; root.nodes.len()];
    let mut stack: Vec<(usize, Transform, usize, Option<usize>, bool)> =
        root.scenes[scene].nodes.iter().rev().map(|n| (n.value(), Transform::IDENTITY, 1, None, false)).collect();
    while let Some((i, parent, depth, lod, in_collision)) = stack.pop() {
        if depth > limits.max_node_depth {
            return Err(format!("nodes nested deeper than {}", limits.max_node_depth));
        }
        if std::mem::replace(&mut visited[i], true) {
            return Err(format!("nodes[{i}] is reached twice (a shared node or a cycle; glTF nodes form trees)"));
        }
        let node = &root.nodes[i];
        let at = format!("nodes[{i}]");
        let name = node.name.as_deref().unwrap_or("");
        let xf = parent.then(&Transform::of_node(node).map_err(|e| format!("{at}: {e}"))?);
        let in_collision = in_collision || is_collision_name(name);
        let lod = lod_of_name(name).or(lod);
        if let Some(k) = lod
            && k >= limits.max_lods
        {
            return Err(format!("{at} {}: LOD {k}; at most {} LODs are read", uri::quoted(name), limits.max_lods));
        }
        let skin = match (node.skin, limits.read_skins) {
            (Some(_), false) => {
                notes.push(format!("{at}: its skin is not read (a static model; a rider's skinned models are)"));
                None
            }
            (Some(_), true) if node.mesh.is_none() => return Err(format!("{at}: a skin on a node without a mesh")),
            (Some(_), true) if in_collision => return Err(format!("{at}: a skinned mesh below `collision`")),
            (s, _) => s.map(|s| s.value()),
        };
        if let Some(m) = node.mesh {
            let placed = if skin.is_some() { Transform::IDENTITY } else { xf };
            out.push(MeshNode {
                node: i,
                mesh: m.value(),
                xf: placed,
                name,
                lod: lod.unwrap_or(0),
                in_collision,
                skin,
            });
        }
        for c in node.children.iter().flatten().rev() {
            stack.push((c.value(), xf, depth + 1, lod, in_collision));
        }
    }
    Ok(out)
}

struct NodeAt<'a> {
    placing: Placing<'a>,
    lod: usize,
    in_collision: bool,
    extras: Extras,
    skin: Option<(usize, usize)>,
}

impl Reader<'_> {
    fn mesh(
        &mut self,
        m: usize,
        at: &NodeAt<'_>,
        lods: &mut Vec<Lod>,
        collision: &mut Vec<CollisionTriangle>,
    ) -> Result<(), String> {
        let root = self.root;
        let mesh = &root.meshes[m];
        let mesh_extras = extras(&mesh.extras, &format!("meshes[{m}]"))?;
        for (pi, p) in mesh.primitives.iter().enumerate() {
            let joints = at.skin.map(|(_, n)| n);
            let built = primitive(
                root,
                &self.buffers,
                (m, pi),
                &at.placing,
                joints,
                self.limits,
                &mut self.budget,
                &mut self.notes,
            )?;
            let material = p.material.map(|x| x.value());
            if at.in_collision {
                let material_extras = match material {
                    Some(k) => extras(&root.materials[k].extras, &format!("materials[{k}]"))?,
                    None => Extras::default(),
                };
                let layers = [&at.extras, &mesh_extras, &material_extras];
                let surface = layers.iter().find_map(|e| e.surface.clone()).ok_or_else(|| {
                    format!(
                        "meshes[{m}].primitives[{pi}] ({}, collision): no ava_surface on its node, mesh or material",
                        uri::quoted(at.placing.name)
                    )
                })?;
                let surface_id = collision::surface_id(&surface)?;
                let group = layers.iter().find_map(|e| e.group).unwrap_or(0);
                for t in built.indices.as_chunks::<3>().0 {
                    let corners = [0, 1, 2].map(|k| built.positions[t[k] as usize]);
                    collision.push(CollisionTriangle { corners, surface_id, group });
                }
                continue;
            }
            if lods.len() <= at.lod {
                lods.resize_with(at.lod + 1, Lod::default);
            }
            let slot = at.extras.slot.clone().or_else(|| mesh_extras.slot.clone());
            lods[at.lod].parts.push(Part { mesh: Arc::new(built), material, skin: at.skin.map(|(s, _)| s), slot });
        }
        Ok(())
    }

    fn materials(
        &mut self,
        dir: &str,
        files: &dyn FileReader,
        held: &mut Held<'_>,
    ) -> Result<(Vec<ModelMaterial>, Vec<ModelImage>), String> {
        let root = self.root;
        let mut images: Vec<ModelImage> = Vec::new();
        let mut image_slot: Vec<Option<usize>> = vec![None; root.images.len()];
        let mut slot_of_source: HashMap<ImageSource, usize> = HashMap::new();
        let mut materials = Vec::with_capacity(root.materials.len());
        for (k, m) in root.materials.iter().enumerate() {
            let at = format!("materials[{k}]");
            let mut texture = |t: usize, tex_coord: u32| -> Result<ImageRef, String> {
                let index = root.textures[t].source.value();
                let slot = match image_slot[index] {
                    Some(s) => s,
                    None => {
                        let (source, image) = self.image(index, dir, files, held)?;
                        let s = match source.as_ref().and_then(|k| slot_of_source.get(k).copied()) {
                            Some(s) => s,
                            None => {
                                images.push(image);
                                if let Some(key) = source {
                                    slot_of_source.insert(key, images.len() - 1);
                                }
                                images.len() - 1
                            }
                        };
                        image_slot[index] = Some(s);
                        s
                    }
                };
                Ok(ImageRef { image: slot, tex_coord })
            };
            let pbr = &m.pbr_metallic_roughness;
            let info = |i: &Option<gltf_json::texture::Info>| i.as_ref().map(|i| (i.index.value(), i.tex_coord));
            let mut opt = |t: Option<(usize, u32)>| t.map(|(t, c)| texture(t, c)).transpose();
            let base_colour_texture = opt(info(&pbr.base_color_texture))?;
            let metallic_roughness_texture = opt(info(&pbr.metallic_roughness_texture))?;
            let normal_texture = opt(m.normal_texture.as_ref().map(|n| (n.index.value(), n.tex_coord)))?;
            let occlusion_texture = opt(m.occlusion_texture.as_ref().map(|o| (o.index.value(), o.tex_coord)))?;
            let emissive_texture = opt(info(&m.emissive_texture))?;
            let alpha = match m.alpha_mode {
                Checked::Valid(gltf_json::material::AlphaMode::Opaque) => AlphaMode::Opaque,
                Checked::Valid(gltf_json::material::AlphaMode::Mask) => AlphaMode::Mask,
                Checked::Valid(gltf_json::material::AlphaMode::Blend) => AlphaMode::Blend,
                Checked::Invalid => return Err(format!("{at}: an unknown alphaMode")),
            };
            let scalars = [
                pbr.metallic_factor.0,
                pbr.roughness_factor.0,
                m.normal_texture.as_ref().map_or(1.0, |n| n.scale),
                m.alpha_cutoff.map_or(0.5, |c| c.0),
            ];
            let mut numbers = pbr.base_color_factor.0.iter().chain(&m.emissive_factor.0).chain(&scalars);
            if numbers.any(|v| !v.is_finite()) {
                return Err(format!("{at}: a factor that is not a finite number"));
            }
            materials.push(ModelMaterial {
                name: m.name.clone(),
                base_colour: pbr.base_color_factor.0,
                base_colour_texture,
                metallic: pbr.metallic_factor.0,
                roughness: pbr.roughness_factor.0,
                metallic_roughness_texture,
                normal_texture,
                normal_scale: m.normal_texture.as_ref().map_or(1.0, |n| n.scale),
                occlusion_texture,
                emissive: m.emissive_factor.0,
                emissive_texture,
                alpha,
                alpha_cutoff: m.alpha_cutoff.map_or(0.5, |c| c.0),
                double_sided: m.double_sided,
                extras: extras(&m.extras, &at)?,
            });
        }
        Ok((materials, images))
    }

    fn image(
        &self,
        i: usize,
        dir: &str,
        files: &dyn FileReader,
        held: &mut Held<'_>,
    ) -> Result<(Option<ImageSource>, ModelImage), String> {
        let root = self.root;
        let img = &root.images[i];
        let at = format!("images[{i}]");
        let (source, bytes): (Option<ImageSource>, Arc<Vec<u8>>) = match (&img.buffer_view, &img.uri) {
            (Some(v), None) => {
                let view = &root.buffer_views[v.value()];
                let buffer = view.buffer.value();
                let data = self.buffers.get(buffer).copied().unwrap_or_default();
                let start = view.byte_offset.map_or(0, |o| o.0);
                let end = start.checked_add(view.byte_length.0).filter(|&e| e <= data.len() as u64);
                let end = end.ok_or_else(|| format!("{at}: its buffer view lies outside its buffer"))?;
                (Some(ImageSource::Range(buffer, start, end)), held.range(buffer, data, start, end, &at)?)
            }
            (None, Some(u)) => match held.resolve(u, dir, &at)? {
                Target::Data { bytes, .. } => (None, Arc::new(bytes)),
                Target::File(path) => {
                    let bytes = held.file(files, &path, self.limits.max_file_bytes, &at)?;
                    (Some(ImageSource::File(path)), bytes)
                }
            },
            _ => return Err(format!("{at}: neither a buffer view nor a uri (exactly one is required)")),
        };
        let ext = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "png"
        } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
            "jpg"
        } else {
            return Err(format!("{at}: neither PNG nor JPEG (glTF images are read in those two formats)"));
        };
        let name =
            img.name.as_deref().filter(|n| crate::path::check_package_path(n, false).is_ok() && !n.contains('/'));
        let file_name = format!("{}.{ext}", name.map_or_else(|| format!("image{i}"), str::to_string));
        Ok((source, ModelImage { file_name, bytes }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum ImageSource {
    File(String),
    Range(usize, u64, u64),
}

impl FileReader for crate::path::PackageTree {
    fn read(&self, path: &str, max: u64) -> Result<Vec<u8>, String> {
        crate::path::PackageTree::read(self, path, max).map_err(|e| e.to_string())
    }

    fn size(&self, path: &str) -> Result<u64, String> {
        crate::path::PackageTree::file_size(self, path).map_err(|e| e.to_string())
    }

    fn read_range(&self, path: &str, offset: u64, len: u64) -> Result<Vec<u8>, String> {
        crate::path::PackageTree::read_range(self, path, offset, len).map_err(|e| e.to_string())
    }
}
