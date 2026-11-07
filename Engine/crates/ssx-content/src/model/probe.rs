use std::collections::BTreeSet;

use gltf_json::Root;
use gltf_json::mesh::{Mode, Semantic};
use gltf_json::validation::Checked;

use super::gltf::{FileReader, mesh_nodes};
use super::uri::{self, Target, truncate};
use super::{Limits, ModelError, glb, json};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModelCost {
    pub vertices: u64,
    pub triangles: u64,
    pub collision_triangles: u64,
    pub held_bytes: u64,
    pub drawn_image_bytes: u64,
    pub joints: u64,
    pub parts: u64,
}

const HEADER_BYTES: u64 = 64 << 10;
pub(super) const JPEG_HEADER_BYTES: u64 = 1 << 20;

enum BufferAt {
    Chunk(u64),
    File(String),
    Data(Vec<u8>),
}

pub fn read_json(
    rel: &str,
    files: &dyn FileReader,
    limits: &Limits,
) -> Result<(Vec<u8>, Option<std::ops::Range<u64>>), ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    let size = files.size(rel).map_err(fail)?;
    if size > limits.max_file_bytes {
        return Err(fail(format!("{size} bytes; at most {} are read", limits.max_file_bytes)));
    }
    let ext = rel.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let magic = if size >= 4 { files.read_range(rel, 0, 4).map_err(fail)? } else { Vec::new() };
    let is_glb = magic == b"glTF";
    match ext.as_str() {
        "glb" if is_glb => {
            let mut read = |at: u64, len: usize| files.read_range(rel, at, len as u64);
            let l = glb::layout(size, &mut read, limits.max_json_bytes).map_err(fail)?;
            let json = files.read_range(rel, l.json.start, l.json.end - l.json.start).map_err(fail)?;
            Ok((json, l.bin))
        }
        "glb" => Err(fail("named .glb, but its content is not a binary glTF".into())),
        "gltf" if is_glb => Err(fail("named .gltf, but its content is a binary glTF (.glb)".into())),
        "gltf" if size > limits.max_json_bytes => {
            Err(fail(format!("the JSON has {size} bytes; at most {} are read", limits.max_json_bytes)))
        }
        "gltf" => Ok((files.read(rel, limits.max_json_bytes).map_err(fail)?, None)),
        _ => Err(fail(format!("a model is a .glb or .gltf file, not .{}", truncate(&ext)))),
    }
}

pub fn files_of(rel: &str, bytes: &[u8], limits: &Limits) -> Result<Vec<String>, ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    let json =
        if bytes.starts_with(b"glTF") { glb::split(bytes, limits.max_json_bytes).map_err(fail)?.json } else { bytes };
    files_of_json(rel, json, limits)
}

pub fn files_of_json(rel: &str, json: &[u8], limits: &Limits) -> Result<Vec<String>, ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    let root = json::parse(json, limits).map_err(fail)?;
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    let uris =
        root.buffers.iter().filter_map(|b| b.uri.as_deref()).chain(root.images.iter().filter_map(|i| i.uri.as_deref()));
    let mut out = Vec::new();
    for u in uris.filter(|u| !u.starts_with("data:")) {
        if let Target::File(p) = uri::resolve(u, dir, 0).map_err(fail)? {
            out.push(p);
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

pub fn files_named(rel: &str, files: &dyn FileReader, limits: &Limits) -> Result<Vec<String>, ModelError> {
    let (json, _) = read_json(rel, files, limits)?;
    files_of_json(rel, &json, limits)
}

pub fn probe_file(rel: &str, files: &dyn FileReader, limits: &Limits) -> Result<ModelCost, ModelError> {
    let fail = |m: String| ModelError::new(rel, m);
    let (json, bin) = read_json(rel, files, limits)?;
    let root = json::parse(&json, limits).map_err(fail)?;
    let dir = rel.rsplit_once('/').map_or("", |(d, _)| d);
    Probe { root: &root, rel, dir, files, limits, held: 0, counted_files: BTreeSet::new() }
        .cost(bin.map(|b| (b.start, b.end - b.start)))
        .map_err(fail)
}

struct Probe<'a> {
    root: &'a Root,
    rel: &'a str,
    dir: &'a str,
    files: &'a dyn FileReader,
    limits: &'a Limits,
    held: u64,
    counted_files: BTreeSet<String>,
}

impl Probe<'_> {
    fn count(&mut self, n: u64, at: &str) -> Result<(), String> {
        self.held = self.held.saturating_add(n);
        if self.held > self.limits.max_held_bytes {
            return Err(format!(
                "{at}: a model holds at most {} bytes of buffers and images in all, {} with this",
                self.limits.max_held_bytes, self.held
            ));
        }
        Ok(())
    }

    fn file(&mut self, path: &str, at: &str) -> Result<u64, String> {
        let size = self.files.size(path).map_err(|e| format!("{at}: {e}"))?;
        if self.counted_files.insert(path.to_string()) {
            self.count(size, at)?;
        }
        Ok(size)
    }

    fn cost(mut self, bin: Option<(u64, u64)>) -> Result<ModelCost, String> {
        let root = self.root;
        let buffers = self.buffers(bin)?;
        let mut cost = ModelCost::default();
        let mut notes = Vec::new();
        let skin_joints = self.skins()?;
        for n in mesh_nodes(root, self.limits, &mut notes)? {
            for (pi, p) in root.meshes[n.mesh].primitives.iter().enumerate() {
                let (vertices, triangles) = primitive_counts(root, (n.mesh, pi), p, self.limits)?;
                if n.in_collision {
                    cost.collision_triangles = cost.collision_triangles.saturating_add(triangles);
                } else {
                    cost.vertices = cost.vertices.saturating_add(vertices);
                    cost.triangles = cost.triangles.saturating_add(triangles);
                    cost.parts += 1;
                    if let Some(s) = n.skin {
                        cost.joints = cost.joints.saturating_add(skin_joints[s]);
                    }
                }
            }
        }
        cost.drawn_image_bytes = self.images(&buffers)?;
        cost.held_bytes = self.held;
        Ok(cost)
    }

    fn skins(&self) -> Result<Vec<u64>, String> {
        let root = self.root;
        if !self.limits.read_skins {
            return Ok(Vec::new());
        }
        if root.skins.len() > self.limits.max_skins {
            return Err(format!("{} skins (at most {} are read)", root.skins.len(), self.limits.max_skins));
        }
        let mut out = Vec::with_capacity(root.skins.len());
        for (i, s) in root.skins.iter().enumerate() {
            let n = s.joints.len();
            if n == 0 || n > self.limits.max_skin_joints {
                return Err(format!("skins[{i}]: {n} joints (1 to {} are read)", self.limits.max_skin_joints));
            }
            if let Some(a) = s.inverse_bind_matrices
                && root.accessors[a.value()].count.0 != n as u64
            {
                return Err(format!(
                    "skins[{i}]: {} inverse bind matrices for {n} joints",
                    root.accessors[a.value()].count.0
                ));
            }
            out.push(n as u64);
        }
        Ok(out)
    }

    fn buffers(&mut self, bin: Option<(u64, u64)>) -> Result<Vec<BufferAt>, String> {
        let mut out = Vec::with_capacity(self.root.buffers.len());
        for (i, b) in self.root.buffers.iter().enumerate() {
            let len = b.byte_length.0;
            let at = format!("buffers[{i}]");
            let (place, n) = match (&b.uri, bin) {
                (None, Some((start, n))) if i == 0 => {
                    self.count(n, &at)?;
                    (BufferAt::Chunk(start), n)
                }
                (None, _) => return Err(format!("{at} has no data (no uri, not the .glb's first buffer)")),
                (Some(u), _) => {
                    match uri::resolve(u, self.dir, self.limits.max_data_uri_bytes).map_err(|e| format!("{at}: {e}"))? {
                        Target::Data { bytes, .. } => {
                            self.count(bytes.len() as u64, &at)?;
                            let n = bytes.len() as u64;
                            (BufferAt::Data(bytes), n)
                        }
                        Target::File(path) => {
                            let n = self.file(&path, &at)?;
                            (BufferAt::File(path), n)
                        }
                    }
                }
            };
            if n < len || n - len > 3 {
                let what = if matches!(place, BufferAt::Chunk(_)) { "the BIN chunk" } else { "its data" };
                return Err(format!("{at} states {len} bytes, {what} has {n} (at most 3 bytes of padding)"));
            }
            out.push(place);
        }
        Ok(out)
    }

    fn images(&mut self, buffers: &[BufferAt]) -> Result<u64, String> {
        let root = self.root;
        let mut drawn = BTreeSet::new();
        let mut named = BTreeSet::new();
        for m in &root.materials {
            let pbr = &m.pbr_metallic_roughness;
            let base = pbr.base_color_texture.as_ref().map(|t| (t.index.value(), t.tex_coord));
            let normal = m.normal_texture.as_ref().map(|t| (t.index.value(), t.tex_coord));
            for (t, uv) in base.into_iter().chain(normal) {
                let image = root.textures[t].source.value();
                named.insert(image);
                if uv == 0 {
                    drawn.insert(image);
                }
            }
            let others = [
                pbr.metallic_roughness_texture.as_ref().map(|t| t.index.value()),
                m.occlusion_texture.as_ref().map(|t| t.index.value()),
                m.emissive_texture.as_ref().map(|t| t.index.value()),
            ];
            named.extend(others.into_iter().flatten().map(|t| root.textures[t].source.value()));
        }
        let mut ranges = BTreeSet::new();
        for &i in &named {
            let img = &root.images[i];
            let at = format!("images[{i}]");
            match (&img.buffer_view, &img.uri) {
                (Some(v), None) => {
                    let (buffer, start, end) = self.view(v.value(), buffers, &at)?;
                    if ranges.insert((buffer, start, end)) {
                        self.count(end - start, &at)?;
                    }
                }
                (None, Some(u)) => {
                    match uri::resolve(u, self.dir, self.limits.max_data_uri_bytes).map_err(|e| format!("{at}: {e}"))? {
                        Target::Data { bytes, .. } => self.count(bytes.len() as u64, &at)?,
                        Target::File(path) => {
                            let size = self.file(&path, &at)?;
                            if size > self.limits.max_file_bytes {
                                return Err(format!(
                                    "{at}: {size} bytes; at most {} are read",
                                    self.limits.max_file_bytes
                                ));
                            }
                        }
                    }
                }
                _ => return Err(format!("{at}: neither a buffer view nor a uri (exactly one is required)")),
            }
        }
        let (side, cap) = (self.limits.max_image_side, self.limits.max_image_decoded_bytes);
        let mut total = 0u64;
        let mut sources = BTreeSet::new();
        for &i in &drawn {
            let img = &root.images[i];
            let source = match (&img.buffer_view, &img.uri) {
                (Some(v), None) => format!("view {:?}", self.view(v.value(), buffers, "")?),
                (None, Some(u)) if !u.starts_with("data:") => match uri::resolve(u, self.dir, 0) {
                    Ok(Target::File(path)) => format!("file {path}"),
                    _ => format!("image {i}"),
                },
                _ => format!("image {i}"),
            };
            if !sources.insert(source) {
                continue;
            }
            let at = format!("images[{i}]");
            let (w, h) = self.image_size(i, buffers)?;
            if w == 0 || h == 0 || w > side || h > side {
                return Err(format!("{at}: {w}×{h}; an image a material draws is 1 to {side} a side"));
            }
            total += crate::texture::chain_bytes(w, h);
            if total > cap {
                return Err(format!(
                    "the images its materials draw decode to {total} bytes or more with their mips; at most {cap} for a \
                     model"
                ));
            }
        }
        Ok(total)
    }

    fn view(&self, v: usize, buffers: &[BufferAt], at: &str) -> Result<(usize, u64, u64), String> {
        let view = &self.root.buffer_views[v];
        let buffer = view.buffer.value();
        let len = self.root.buffers[buffer].byte_length.0;
        let start = view.byte_offset.map_or(0, |o| o.0);
        let end = start.checked_add(view.byte_length.0).filter(|&e| e <= len);
        let end = end.ok_or_else(|| format!("{at}: its buffer view lies outside its buffer"))?;
        buffers.get(buffer).ok_or_else(|| format!("{at}: its buffer has no data"))?;
        Ok((buffer, start, end))
    }

    fn image_size(&self, i: usize, buffers: &[BufferAt]) -> Result<(u32, u32), String> {
        let img = &self.root.images[i];
        let at = format!("images[{i}]");
        let name = format!("{at} of {}", self.rel);
        let head = |n: u64| -> Result<Vec<u8>, String> {
            match (&img.buffer_view, &img.uri) {
                (Some(v), None) => {
                    let (buffer, start, end) = self.view(v.value(), buffers, &at)?;
                    let n = n.min(end - start);
                    match &buffers[buffer] {
                        BufferAt::Chunk(c) => self.files.read_range(self.rel, c + start, n),
                        BufferAt::File(path) => self.files.read_range(path, start, n),
                        BufferAt::Data(bytes) => Ok(bytes[start as usize..(start + n) as usize].to_vec()),
                    }
                }
                (None, Some(u)) => match uri::resolve(u, self.dir, self.limits.max_data_uri_bytes)? {
                    Target::Data { bytes, .. } => Ok(bytes[..(n as usize).min(bytes.len())].to_vec()),
                    Target::File(path) => {
                        let size = self.files.size(&path)?;
                        self.files.read_range(&path, 0, n.min(size))
                    }
                },
                _ => Err("neither a buffer view nor a uri".into()),
            }
            .map_err(|e| format!("{at}: {e}"))
        };
        let first = head(HEADER_BYTES)?;
        match crate::texture::image_size(&name, &first) {
            Ok(wh) => Ok(wh),
            Err(_) if first.starts_with(&[0xFF, 0xD8, 0xFF]) && first.len() as u64 == HEADER_BYTES => {
                crate::texture::image_size(&name, &head(JPEG_HEADER_BYTES)?)
                    .map_err(|e| format!("{e} (its frame header is looked for in the first {JPEG_HEADER_BYTES} bytes)"))
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

fn primitive_counts(
    root: &Root,
    (mi, pi): (usize, usize),
    p: &gltf_json::mesh::Primitive,
    limits: &Limits,
) -> Result<(u64, u64), String> {
    let what = format!("meshes[{mi}].primitives[{pi}]");
    let Checked::Valid(mode) = p.mode else { return Err(format!("{what}: an unknown mode")) };
    let attr = |s: Semantic| p.attributes.get(&Checked::Valid(s)).map(|a| a.value());
    let positions = attr(Semantic::Positions).ok_or_else(|| format!("{what}: no POSITION"))?;
    let n = root.accessors[positions].count.0;
    if n == 0 || n > limits.max_primitive_vertices {
        return Err(format!("{what}: {n} positions (1 to {} are read)", limits.max_primitive_vertices));
    }
    let corners = match p.indices {
        Some(i) => root.accessors[i.value()].count.0,
        None => n,
    };
    let triangles = match mode {
        Mode::Triangles if corners.is_multiple_of(3) => corners / 3,
        Mode::Triangles => return Err(format!("{what}: {corners} indices, not a multiple of 3")),
        Mode::TriangleStrip | Mode::TriangleFan => corners.saturating_sub(2),
        other => return Err(format!("{what}: mode {other:?}: only triangles are drawn")),
    };
    let targets = p.targets.as_ref().map_or(0, Vec::len);
    if targets > limits.max_morph_targets {
        return Err(format!("{what}: {targets} morph targets (at most {}; none is read)", limits.max_morph_targets));
    }
    let has = |s: Semantic| attr(s).is_some();
    let vertices = if !has(Semantic::Normals) || (!has(Semantic::Tangents) && has(Semantic::TexCoords(0))) {
        triangles.saturating_mul(3)
    } else {
        n
    };
    Ok((vertices, triangles))
}
