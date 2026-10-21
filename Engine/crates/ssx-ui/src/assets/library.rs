use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use ssx_formats::gfx::avm1::Action;
use ssx_formats::gfx::place::PlaceObject;
use ssx_formats::gfx::shape::{DefineShape, MorphShape};
use ssx_formats::gfx::text::{EditText, Font, StaticText};
use ssx_formats::gfx::types::Rgba;
use ssx_formats::gfx::{Movie, Tag, TagRecord};

use crate::fs::{FE_ROOT, FileSource, normalise};
use crate::geom::RectF;
use crate::image::{self, ImageData};

pub type MovieId = usize;
pub type ImageId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharRef {
    pub movie: MovieId,
    pub id: u16,
}

#[derive(Clone, Debug)]
pub enum FrameTag {
    Place(Arc<PlaceObject>),
    Remove(u16),
}

#[derive(Clone, Debug, Default)]
pub struct FrameDef {
    pub tags: Vec<FrameTag>,
    pub actions: Vec<Arc<[Action]>>,
    pub init_actions: Vec<(u16, Arc<[Action]>)>,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TimelineDef {
    pub movie: MovieId,
    pub id: u16,
    pub frames: Vec<FrameDef>,
}

impl TimelineDef {
    pub fn frame_count(&self) -> u16 {
        self.frames.len() as u16
    }

    pub fn label_frame(&self, label: &str) -> Option<u16> {
        self.frames.iter().position(|f| f.labels.iter().any(|l| l.eq_ignore_ascii_case(label))).map(|i| i as u16 + 1)
    }

    fn build(movie: MovieId, id: u16, frame_count: u16, tags: &[TagRecord]) -> TimelineDef {
        let mut frames = Vec::with_capacity(usize::from(frame_count));
        let mut cur = FrameDef::default();
        for t in tags {
            match &t.tag {
                Tag::ShowFrame => frames.push(std::mem::take(&mut cur)),
                Tag::PlaceObject(p) => cur.tags.push(FrameTag::Place(Arc::new((**p).clone()))),
                Tag::RemoveObject2 { depth } => cur.tags.push(FrameTag::Remove(*depth)),
                Tag::DoAction(a) => cur.actions.push(Arc::from(a.as_slice())),
                Tag::DoInitAction { sprite_id, actions } => {
                    cur.init_actions.push((*sprite_id, Arc::from(actions.as_slice())))
                }
                Tag::FrameLabel { name, .. } => cur.labels.push(name.clone()),
                _ => {}
            }
        }
        let n = usize::from(frame_count.max(1));
        frames.resize_with(n, FrameDef::default);
        frames.truncate(n);
        TimelineDef { movie, id, frames }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BitmapDef {
    pub image: ImageId,
    pub rect: [u32; 4],
    pub size: [u32; 2],
}

impl BitmapDef {
    pub fn of_rect(image: ImageId, rect: [u32; 4]) -> Self {
        BitmapDef { image, rect, size: [rect[2] - rect[0], rect[3] - rect[1]] }
    }

    pub fn width(&self) -> u32 {
        self.rect[2] - self.rect[0]
    }

    pub fn height(&self) -> u32 {
        self.rect[3] - self.rect[1]
    }

    pub fn texel_to_pixel(&self) -> crate::geom::Mat2d {
        let k = |size: u32, texels: u32| if texels == 0 { 1.0 } else { size as f32 / texels as f32 };
        crate::geom::Mat2d::scale(k(self.size[0], self.width()), k(self.size[1], self.height()))
    }
}

#[derive(Clone, Debug)]
pub enum Character {
    Shape(Arc<DefineShape>),
    Morph(Arc<MorphShape>),
    Sprite(Arc<TimelineDef>),
    EditText(Arc<EditText>),
    Text(Arc<StaticText>),
    Bitmap(BitmapDef),
    Font(Arc<Font>),
}

impl Character {
    pub fn kind(&self) -> &'static str {
        match self {
            Character::Shape(_) => "shape",
            Character::Morph(_) => "morph shape",
            Character::Sprite(_) => "sprite",
            Character::EditText(_) => "edit text",
            Character::Text(_) => "static text",
            Character::Bitmap(_) => "bitmap",
            Character::Font(_) => "font",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Import {
    pub url: String,
    pub name: String,
    pub target: Option<CharRef>,
}

#[derive(Clone, Debug)]
pub struct MovieDef {
    pub path: String,
    pub dir: String,
    pub version: u8,
    pub frame_rate: f32,
    pub stage: RectF,
    pub background: Option<Rgba>,
    pub swf_name: String,
    pub root: Arc<TimelineDef>,
    pub chars: HashMap<u16, Character>,
    pub exports: HashMap<String, u16>,
    pub imports: HashMap<u16, Import>,
    pub scaling_grids: HashMap<u16, RectF>,
}

pub struct Library {
    fs: Box<dyn FileSource>,
    movies: Vec<MovieDef>,
    by_path: HashMap<String, MovieId>,
    images: Vec<Arc<ImageData>>,
    image_by_path: HashMap<String, ImageId>,
    font_lib: HashMap<String, Arc<Font>>,
    pub log: Vec<String>,
}

pub fn movie_key(rel: &str) -> String {
    let mut k = normalise(rel);
    if let Some(stem) = k.strip_suffix(".swf") {
        k = format!("{stem}.gfx");
    }
    if let Some(rest) = k.strip_prefix(FE_ROOT) {
        k = rest.to_string();
    }
    k
}

impl Library {
    pub fn new(fs: Box<dyn FileSource>) -> Self {
        Library {
            fs,
            movies: Vec::new(),
            by_path: HashMap::new(),
            images: Vec::new(),
            image_by_path: HashMap::new(),
            font_lib: HashMap::new(),
            log: Vec::new(),
        }
    }

    pub fn files(&self) -> &dyn FileSource {
        self.fs.as_ref()
    }

    pub fn movie(&self, id: MovieId) -> &MovieDef {
        &self.movies[id]
    }

    pub fn movies(&self) -> &[MovieDef] {
        &self.movies
    }

    pub fn image(&self, id: ImageId) -> &Arc<ImageData> {
        &self.images[id]
    }

    pub fn images(&self) -> &[Arc<ImageData>] {
        &self.images
    }

    pub fn movie_by_path(&self, rel: &str) -> Option<MovieId> {
        self.by_path.get(&movie_key(rel)).copied()
    }

    pub fn insert_movie(&mut self, m: MovieDef) -> MovieId {
        let id = self.movies.len();
        self.by_path.insert(m.path.clone(), id);
        self.movies.push(m);
        id
    }

    pub fn insert_image(&mut self, img: ImageData) -> ImageId {
        self.images.push(Arc::new(img));
        self.images.len() - 1
    }

    pub fn load_font_library(&mut self, region: &str) -> Result<MovieId> {
        let id = self.load_movie(&format!("gfxfontlib/{region}/gfxfontlib.gfx"))?;
        let fonts: Vec<Arc<Font>> = self.movies[id]
            .chars
            .values()
            .filter_map(|c| match c {
                Character::Font(f) if !f.glyphs.is_empty() => Some(f.clone()),
                _ => None,
            })
            .collect();
        for f in fonts {
            self.font_lib.insert(f.name.clone(), f);
        }
        Ok(id)
    }

    pub fn font_library(&self) -> &HashMap<String, Arc<Font>> {
        &self.font_lib
    }

    // TODO(guess): a zero-glyph font resolves by name to the font library's export (the disc's placeholders match by name); settle: Scaleform's FontLib / FontMap lookup (font library set up at 0x82594A68).
    pub fn font_for(&self, owner: MovieId, font_id: Option<u16>, face: Option<&str>) -> Option<Arc<Font>> {
        let own = font_id.and_then(|i| match self.resolve(CharRef { movie: owner, id: i }) {
            Some((_, Character::Font(f))) => Some(f.clone()),
            _ => None,
        });
        if let Some(f) = own.as_ref().filter(|f| !f.glyphs.is_empty() && face.is_none_or(|n| n == f.name)) {
            return Some(f.clone());
        }
        let name = face.map(str::to_string).or_else(|| own.as_ref().map(|f| f.name.clone()))?;
        self.font_lib.get(&name).cloned()
    }

    pub fn resolve(&self, r: CharRef) -> Option<(CharRef, &Character)> {
        let mut r = r;
        for _ in 0..16 {
            let m = self.movies.get(r.movie)?;
            if let Some(c) = m.chars.get(&r.id) {
                return Some((r, c));
            }
            r = m.imports.get(&r.id)?.target?;
        }
        None
    }

    pub fn export(&self, movie: MovieId, name: &str) -> Option<CharRef> {
        let m = self.movies.get(movie)?;
        let id = m
            .exports
            .get(name)
            .or_else(|| m.exports.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|e| e.1))?;
        self.resolve(CharRef { movie, id: *id }).map(|(r, _)| r)
    }

    // TODO(guess): the root's definition list (+0x3C) is walked newest first (only that order lets the main menu's `ListMenu` find the `CellWrapper`, as the console does); settle: the list's order.
    pub fn export_from_importers(
        &self,
        movie: MovieId,
        name: &str,
        loaded: impl Fn(MovieId) -> bool,
    ) -> Option<CharRef> {
        let mut current = movie;
        for (i, m) in self.movies.iter().enumerate().rev() {
            if i == movie || !loaded(i) {
                continue;
            }
            if m.imports.values().any(|imp| imp.target.is_some_and(|t| t.movie == current)) {
                if let Some(r) = self.export(i, name) {
                    return Some(r);
                }
                current = i;
            }
        }
        None
    }

    pub fn scaling_grid(&self, r: CharRef) -> Option<RectF> {
        self.movies.get(r.movie)?.scaling_grids.get(&r.id).copied()
    }

    fn image_file(&mut self, path: &str) -> Option<ImageId> {
        if let Some(&id) = self.image_by_path.get(path) {
            return Some(id);
        }
        match image::load(self.fs.as_ref(), path) {
            Ok(img) => {
                let id = self.images.len();
                self.images.push(Arc::new(img));
                self.image_by_path.insert(path.to_string(), id);
                Some(id)
            }
            Err(e) => {
                self.log.push(format!("{e:#}"));
                None
            }
        }
    }

    // TODO(guess): the image movie's layout (one frame, the bitmap at depth 1 at 0, 0, the stage the image's size, 30 fps) follows Flash's loadMovie of a bitmap; settle: Scaleform's load-queue image branch.
    fn load_image_movie(&mut self, key: &str, file: &str) -> Result<MovieId> {
        let img = image::load(self.fs.as_ref(), file).with_context(|| format!("loading {file} as an image"))?;
        let (w, h) = (img.width, img.height);
        let image = self.insert_image(img);
        let id = self.movies.len();
        let dir = file.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
        let place = PlaceObject {
            version: 2,
            flags: PlaceObject::HAS_CHARACTER | 0x04,
            flags3: 0,
            depth: 1,
            class_name: None,
            character_id: Some(1),
            matrix: Some(ssx_formats::gfx::types::Matrix::IDENTITY),
            color_transform: None,
            ratio: None,
            name: None,
            clip_depth: None,
            filters: Vec::new(),
            blend_mode: None,
            cache_as_bitmap: None,
            visible: None,
            background: None,
            clip_actions: None,
        };
        let frame = FrameDef { tags: vec![FrameTag::Place(Arc::new(place))], ..Default::default() };
        let mut chars = HashMap::new();
        chars.insert(1, Character::Bitmap(BitmapDef::of_rect(image, [0, 0, w, h])));
        self.movies.push(MovieDef {
            path: key.to_string(),
            dir,
            version: 8,
            frame_rate: 30.0,
            stage: RectF::new(0.0, 0.0, w as f32 * 20.0, h as f32 * 20.0),
            background: None,
            swf_name: String::new(),
            root: Arc::new(TimelineDef { movie: id, id: 0, frames: vec![frame] }),
            chars,
            exports: HashMap::new(),
            imports: HashMap::new(),
            scaling_grids: HashMap::new(),
        });
        self.by_path.insert(key.to_string(), id);
        Ok(id)
    }

    pub fn load_movie(&mut self, rel: &str) -> Result<MovieId> {
        let key = movie_key(rel);
        if let Some(&id) = self.by_path.get(&key) {
            return Ok(id);
        }
        let file = format!("{FE_ROOT}{key}");
        let data = self.fs.read(&file).with_context(|| format!("movie {file} not found"))?;
        if !matches!(data.get(..3), Some(b"CFX" | b"GFX" | b"FWS" | b"CWS")) {
            return self.load_image_movie(&key, &file);
        }
        let parsed = Movie::parse(&data).with_context(|| format!("parsing {file}"))?;
        let id = self.movies.len();
        let dir = file.rsplit_once('/').map_or(String::new(), |(d, _)| format!("{d}/"));
        let mut m = MovieDef {
            path: key.clone(),
            dir: dir.clone(),
            version: parsed.version,
            frame_rate: f32::from(parsed.frame_rate) / 256.0,
            stage: RectF::from_swf(&parsed.frame_size),
            background: None,
            swf_name: parsed.exporter_info().map(|e| e.swf_name.clone()).unwrap_or_default(),
            root: Arc::new(TimelineDef::build(id, 0, parsed.frame_count, &parsed.tags)),
            chars: HashMap::new(),
            exports: HashMap::new(),
            imports: HashMap::new(),
            scaling_grids: HashMap::new(),
        };
        let mut atlases: HashMap<u32, ImageId> = HashMap::new();
        for t in &parsed.tags {
            match &t.tag {
                Tag::Shape(s) => drop(m.chars.insert(s.id, Character::Shape(Arc::new(s.clone())))),
                Tag::MorphShape(s) => drop(m.chars.insert(s.id, Character::Morph(Arc::new(s.clone())))),
                Tag::Sprite(s) => {
                    let tl = TimelineDef::build(id, s.id, s.frame_count, &s.tags);
                    m.chars.insert(s.id, Character::Sprite(Arc::new(tl)));
                }
                Tag::EditText(e) => drop(m.chars.insert(e.id, Character::EditText(Arc::new(e.clone())))),
                Tag::Text(s) => drop(m.chars.insert(s.id, Character::Text(Arc::new(s.clone())))),
                Tag::Font(f) => drop(m.chars.insert(f.id, Character::Font(Arc::new(f.clone())))),
                Tag::SetBackgroundColor(c) => m.background = Some(*c),
                Tag::ExportAssets(a) => {
                    for (cid, name) in a {
                        m.exports.entry(name.clone()).or_insert(*cid);
                    }
                }
                Tag::ImportAssets { url, assets, .. } => {
                    for (cid, name) in assets {
                        m.imports.insert(*cid, Import { url: url.clone(), name: name.clone(), target: None });
                    }
                }
                Tag::ScalingGrid { id: cid, splitter } => drop(m.scaling_grids.insert(*cid, RectF::from_swf(splitter))),
                Tag::ExternalImage(e) => {
                    let Some(path) = image::resolve_external(self.fs.as_ref(), &dir, &e.file_name) else {
                        self.log.push(format!("{key}: image {} not found", e.file_name));
                        continue;
                    };
                    let Some(img) = self.image_file(&path) else { continue };
                    let (w, h) = (self.images[img].width, self.images[img].height);
                    if (w, h) != (u32::from(e.target_width), u32::from(e.target_height)) {
                        self.log.push(format!(
                            "{key}: image {} is {w}×{h}, the tag says {}×{}",
                            e.file_name, e.target_width, e.target_height
                        ));
                    }
                    if e.id & ssx_formats::gfx::scaleform::ExternalImage::EXPORTER_IMAGE != 0 {
                        atlases.insert(e.id, img);
                    } else {
                        let size = [u32::from(e.target_width), u32::from(e.target_height)];
                        m.chars
                            .insert(e.id as u16, Character::Bitmap(BitmapDef { image: img, rect: [0, 0, w, h], size }));
                    }
                }
                Tag::SubImage(s) => {
                    let atlas = ssx_formats::gfx::scaleform::ExternalImage::EXPORTER_IMAGE | u32::from(s.image);
                    match atlases.get(&atlas) {
                        Some(&img) => {
                            let rect = [s.x1, s.y1, s.x2, s.y2].map(u32::from);
                            m.chars.insert(s.id, Character::Bitmap(BitmapDef::of_rect(img, rect)));
                        }
                        None => self.log.push(format!("{key}: sub image {} of a missing atlas", s.id)),
                    }
                }
                _ => {}
            }
        }
        self.movies.push(m);
        self.by_path.insert(key.clone(), id);
        let mut imports: Vec<(u16, String, String)> =
            self.movies[id].imports.iter().map(|(cid, i)| (*cid, i.url.clone(), i.name.clone())).collect();
        imports.sort();
        for (cid, url, name) in imports {
            let target = match self.load_movie(&url) {
                Ok(other) => match self.movies[other].exports.get(&name) {
                    Some(&oid) => Some(CharRef { movie: other, id: oid }),
                    None => {
                        self.log.push(format!("{key}: {url} does not export {name}"));
                        None
                    }
                },
                Err(e) => {
                    self.log.push(format!("{key}: import {name} from {url}: {e:#}"));
                    None
                }
            };
            if let Some(i) = self.movies[id].imports.get_mut(&cid) {
                i.target = target;
            }
        }
        Ok(id)
    }
}
