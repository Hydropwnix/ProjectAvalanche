mod cube;
mod xml;

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use ssx_formats::attrib::collection::{self, Collection, DecodedCollection, Field, FieldData};
use ssx_formats::attrib::{NameDict, Schema, Value, Vault};

pub use cube::{BlendMode, ColourCube, LayerKind, N as CUBE_SIZE, Stage, apply_stage, quantise};
pub use xml::{XmlGroup, XmlLayer, XmlStack, parse_stack};

use crate::vault_db::fnv1;

pub const CLASS_POSTFX: u32 = fnv1("postfx");
const CLASS_DROP: u32 = fnv1("ge_drop");
const FIELD_DROP_POSTFX: u32 = 0x5595_A25C;
const FIELD_LAYER: u32 = 0xE8B4_B707;
pub const FIELD_BLOOM_ACTIVE: u32 = 0x8E58_1409;
const FIELD_COLOUR_CUBE_BLEND: u32 = 0x8E8B_3DA4;
const FIELD_COLOUR_CUBE_STACK: u32 = 0x92F1_952A;

pub const FIELD_NAMES: [&str; 16] = [
    "bloom_blur_x",
    "bloom_blur_y",
    "colourcube_depth_near",
    "colourcube_depth_far",
    "motion_blur_intensity",
    "motion_blur_origin",
    "topogrid_colour_grid",
    "topogrid_pulse_falloff",
    "topogrid_dist_start",
    "topogrid_dist_end",
    "topogrid_dist_fadein",
    "topogrid_dist_fadeout",
    "topogrid_min_intensities",
    "topogrid_max_intensities",
    "vignette_colour",
    "vignette_fadeoffbounds",
];

#[derive(Clone, Debug, PartialEq)]
pub struct ColourCubeBlend {
    pub stack: u32,
    pub mode: u32,
    pub group: String,
    pub weight: f32,
}

#[derive(Clone, Debug, Default)]
pub struct PostFxPreset {
    pub name: String,
    pub chain: Vec<String>,
    pub layer: Option<String>,
    pub floats: BTreeMap<String, f32>,
    pub vectors: BTreeMap<String, Vec<f32>>,
    pub colour_cubes: Vec<ColourCubeBlend>,
}

impl PostFxPreset {
    pub fn float(&self, name: &str) -> Option<f32> {
        self.floats.get(name).copied()
    }

    pub fn bloom_active(&self) -> bool {
        self.float(&format!("{FIELD_BLOOM_ACTIVE:#010x}")).is_some_and(|v| v > 0.001)
    }

    pub fn bloom_blur(&self) -> Option<[f32; 2]> {
        Some([self.float("bloom_blur_x")?, self.float("bloom_blur_y")?])
    }
}

#[derive(Clone, Debug)]
pub struct ColourStack {
    pub index: u32,
    pub file: PathBuf,
    pub xml: XmlStack,
}

#[derive(Clone, Debug)]
pub struct CubeBuild {
    pub cube: ColourCube,
    pub stages: Vec<String>,
    pub needs_second_lut: bool,
}

struct VaultFiles {
    vlt: Vec<u8>,
    bin: Vec<u8>,
}

fn read_pair(vlt: &Path) -> Result<VaultFiles> {
    let bin = vlt.with_extension("bin");
    Ok(VaultFiles {
        vlt: std::fs::read(vlt).with_context(|| format!("reading {}", vlt.display()))?,
        bin: std::fs::read(&bin).with_context(|| format!("reading {}", bin.display()))?,
    })
}

struct Decoded<'a> {
    vault: Vault<'a>,
    cols: Vec<Collection>,
    decoded: Vec<DecodedCollection>,
    index: HashMap<(u32, u32), usize>,
    dict: NameDict,
}

fn with_vault<R>(big_root: &Path, name: &str, f: impl FnOnce(&Decoded<'_>) -> Result<R>) -> Result<R> {
    let dir = big_root.join("bootcache/data/xenon/attribdb");
    let files = read_pair(&dir.join(name))?;
    let schema_files = read_pair(&dir.join("attribdb_schema.vlt"))?;
    let schema_vault = Vault::parse(&schema_files.vlt, &schema_files.bin)?;
    let schema = Schema::from_vault(&schema_vault).context("reading the schema")?;
    let vault = Vault::parse(&files.vlt, &files.bin).with_context(|| format!("parsing {name}"))?;
    let cols = collection::collections(&vault)?;
    let decoded = cols.iter().map(|c| c.decode(&vault, &schema)).collect::<ssx_formats::Result<Vec<_>>>()?;
    let index = collection::index(&cols);
    let mut dict = NameDict::new();
    dict.add_strings_from(&files.vlt);
    dict.add_strings_from(&files.bin);
    f(&Decoded { vault, cols, decoded, index, dict })
}

fn values(d: &FieldData) -> Vec<&Value> {
    match d {
        FieldData::Single(v) => vec![v],
        FieldData::Array(vs) => vs.iter().collect(),
    }
}

fn be_u32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl Decoded<'_> {
    fn find(&self, class: u32, key: u32) -> Option<usize> {
        self.index.get(&(class, key)).copied()
    }

    fn fields(&self, i: usize) -> Result<Vec<&Field>> {
        Ok(collection::with_inherited(&self.cols, &self.decoded, &self.index, i)?.into_iter().map(|(_, f)| f).collect())
    }

    fn collection_name(&self, class_name: &str, i: usize) -> String {
        let c = &self.cols[i];
        match self.dict.get(c.key) {
            Some(n) if fnv1(&format!("{class_name}/{n}")) == c.export_key => n.to_string(),
            _ => format!("{:08x}", c.key),
        }
    }

    fn raw_text(&self, v: &Value, offset: u32) -> Result<Option<String>> {
        let Value::Raw { pointers, .. } = v else { bail!("not a raw value") };
        match pointers.iter().find(|p| p.offset == offset) {
            Some(p) => Ok(Some(self.vault.cstr(p.target)?)),
            None => Ok(None),
        }
    }

    fn colour_cube_blend(&self, v: &Value) -> Result<ColourCubeBlend> {
        let Value::Raw { bytes, .. } = v else { bail!("ColourCubeBlend is not a raw value") };
        if bytes.len() != 16 || be_u32(bytes, 8) != 0 {
            bail!("ColourCubeBlend: unexpected layout {bytes:02x?}");
        }
        Ok(ColourCubeBlend {
            stack: be_u32(bytes, 0),
            mode: be_u32(bytes, 4),
            group: self.raw_text(v, 8)?.context("ColourCubeBlend without a group name")?,
            weight: f32::from_bits(be_u32(bytes, 12)),
        })
    }

    fn preset(&self, name: &str) -> Result<PostFxPreset> {
        let i = self.find(CLASS_POSTFX, fnv1(name)).with_context(|| format!("no postfx collection {name:?}"))?;
        let mut p = PostFxPreset { name: name.to_string(), ..Default::default() };
        let mut cur = Some(i);
        while let Some(c) = cur {
            p.chain.push(self.collection_name("postfx", c));
            let parent = self.cols[c].parent;
            cur = if parent == 0 { None } else { self.find(CLASS_POSTFX, parent) };
            if p.chain.len() > self.cols.len() {
                bail!("{name}: parent chain loops");
            }
        }
        for f in self.fields(i)? {
            let field = FIELD_NAMES
                .iter()
                .find(|n| fnv1(n) == f.key)
                .map_or_else(|| format!("{:#010x}", f.key), |n| (*n).to_string());
            let vals = values(&f.data);
            match vals.first() {
                Some(Value::Float(x)) => {
                    p.floats.insert(field, *x);
                }
                Some(Value::Vector2(v)) => {
                    p.vectors.insert(field, v.to_vec());
                }
                Some(Value::Vector3(v)) => {
                    p.vectors.insert(field, v.to_vec());
                }
                Some(Value::Vector4(v)) => {
                    p.vectors.insert(field, v.to_vec());
                }
                Some(Value::Ref(r)) if f.key == FIELD_LAYER && r.collection != 0 => {
                    p.layer = Some(match self.find(r.class, r.collection) {
                        Some(li) => self.collection_name("postfx_layers", li),
                        None => format!("{:08x}", r.collection),
                    });
                }
                Some(_) if f.key == FIELD_COLOUR_CUBE_BLEND => {
                    p.colour_cubes = vals.into_iter().map(|v| self.colour_cube_blend(v)).collect::<Result<_>>()?;
                }
                _ => {}
            }
        }
        Ok(p)
    }
}

impl Decoded<'_> {
    fn stacks(&self, big_root: &Path) -> Result<Vec<ColourStack>> {
        let mut found = None;
        for (i, c) in self.cols.iter().enumerate().filter(|(_, c)| c.class == CLASS_POSTFX) {
            if let Some(f) = self.decoded[i].field(FIELD_COLOUR_CUBE_STACK) {
                if found.is_some() {
                    bail!("more than one postfx collection has a ColourCubeStack ({:08x})", c.key);
                }
                found = Some(f);
            }
        }
        let field = found.context("no postfx collection has a ColourCubeStack")?;
        let dir = big_root.join("memresident/data/common/postfx/colourcube");
        let mut out = Vec::new();
        for v in values(&field.data) {
            let Value::Raw { bytes, .. } = v else { bail!("ColourCubeStack is not a raw value") };
            if bytes.len() != 8 {
                bail!("ColourCubeStack entry of {} bytes", bytes.len());
            }
            let name = self.raw_text(v, 4)?.context("ColourCubeStack entry without a name")?;
            let file = dir.join(format!("{name}.xml"));
            let text = std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
            let xml = parse_stack(&text).with_context(|| format!("parsing {}", file.display()))?;
            if xml.name != name {
                bail!("{} declares stack {:?}", file.display(), xml.name);
            }
            out.push(ColourStack { index: be_u32(bytes, 0), file, xml });
        }
        out.sort_by_key(|s| s.index);
        Ok(out)
    }
}

pub fn build_cube(stacks: &[ColourStack], blends: &[ColourCubeBlend]) -> Result<CubeBuild> {
    for b in blends {
        let stack = stacks.iter().find(|s| s.index == b.stack).with_context(|| format!("no stack {}", b.stack))?;
        if !stack.xml.groups.iter().any(|g| g.name == b.group) {
            bail!("stack {} has no group {:?}", stack.xml.name, b.group);
        }
        if b.mode > 2 {
            bail!("ColourCubeBlend mode {} (expected 0..2)", b.mode);
        }
    }
    let mut cube = ColourCube::identity();
    let mut stages = Vec::new();
    for stack in stacks {
        for group in &stack.xml.groups {
            let blend = blends.iter().find(|b| b.stack == stack.index && b.group == group.name && b.mode != 2);
            let (enabled, opacity) = match blend {
                Some(b) => {
                    let o = ((b.weight * 255.0) as i32).clamp(0, 255) as u32;
                    (o > 0, o)
                }
                None => (group.enabled, group.opacity),
            };
            for layer in group.layers.iter().filter(|l| enabled && l.enabled) {
                let opacity = layer.opacity * opacity / 255;
                if opacity == 0 {
                    continue;
                }
                let kind = LayerKind::parse(layer).with_context(|| format!("{}/{}", stack.xml.name, group.name))?;
                let blend = BlendMode::parse(&layer.blend_mode).with_context(|| {
                    format!(
                        "{}/{}/{}: blend mode {} is not decoded",
                        stack.xml.name, group.name, layer.name, layer.blend_mode
                    )
                })?;
                let label = format!(
                    "{}/{}/{} ({}, {}, {opacity})",
                    stack.xml.name, group.name, layer.name, layer.type_name, layer.blend_mode
                );
                if kind != LayerKind::LoadColourCube {
                    apply_stage(&mut cube, &Stage { label: label.clone(), kind, blend, opacity });
                    stages.push(label);
                }
            }
        }
    }
    let needs_second_lut = blends.iter().any(|b| b.mode != 0 && b.weight > 0.0);
    Ok(CubeBuild { cube, stages, needs_second_lut })
}

pub fn load_presets(big_root: &Path, names: &[&str]) -> Result<Vec<(PostFxPreset, CubeBuild)>> {
    with_vault(big_root, "attribdb.vlt", |db| {
        let stacks = db.stacks(big_root)?;
        names
            .iter()
            .map(|name| {
                let preset = db.preset(name)?;
                let build =
                    build_cube(&stacks, &preset.colour_cubes).with_context(|| format!("colour cube of {name}"))?;
                Ok((preset, build))
            })
            .collect()
    })
}

pub fn drop_postfx_names(big_root: &Path) -> Result<Vec<String>> {
    with_vault(big_root, "ge_attribdb.vlt", |db| {
        let mut out = Vec::new();
        for (i, _) in db.cols.iter().enumerate().filter(|(_, c)| c.class == CLASS_DROP) {
            let text = db.fields(i)?.into_iter().find(|f| f.key == FIELD_DROP_POSTFX).and_then(|f| match &f.data {
                FieldData::Single(Value::Text(Some(t))) => Some(t.clone()),
                _ => None,
            });
            out.extend(text);
        }
        Ok(out)
    })
}

pub fn check_base_cube(big_root: &Path) -> Result<(usize, usize)> {
    use ssx_formats::xpr::XprFile;
    let path = big_root.join("bootcache/data/xenon/gameexplorer/postfx/textures/postfx_colourcube_base.xpr");
    let data = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let xpr = XprFile::parse(&data)?;
    let res = xpr.resources.first().context("no resource")?;
    let levels = crate::texture::decode_resource(&data, &xpr, res, false, crate::texture::DxnExport::Raw)?;
    let img = &levels.first().context("no level")?.image;
    if (img.width as usize, img.height as usize) != (CUBE_SIZE * CUBE_SIZE, CUBE_SIZE) {
        bail!("base cube decodes to {}x{}, expected a 1024x32 strip", img.width, img.height);
    }
    let id = ColourCube::identity();
    let mut matching = 0;
    for b in 0..CUBE_SIZE {
        for g in 0..CUBE_SIZE {
            for r in 0..CUBE_SIZE {
                let at = (g * CUBE_SIZE * CUBE_SIZE + b * CUBE_SIZE + r) * 4;
                let want = id.texels[r + CUBE_SIZE * g + CUBE_SIZE * CUBE_SIZE * b];
                matching += usize::from(img.rgba[at..at + 3] == want);
            }
        }
    }
    Ok((matching, CUBE_SIZE.pow(3)))
}

pub fn write_cube_png(path: &Path, cube: &ColourCube) -> Result<()> {
    let (w, h) = (CUBE_SIZE * CUBE_SIZE, CUBE_SIZE);
    let mut rgba = vec![0u8; w * h * 4];
    for (i, t) in cube.texels.iter().enumerate() {
        let (r, g, b) = (i % CUBE_SIZE, i / CUBE_SIZE % CUBE_SIZE, i / (CUBE_SIZE * CUBE_SIZE));
        let at = (g * w + b * CUBE_SIZE + r) * 4;
        rgba[at..at + 4].copy_from_slice(&[t[0], t[1], t[2], 255]);
    }
    crate::texture::write_png(path, &crate::texture::Image { width: w as u32, height: h as u32, rgba })
}

pub fn run(big_root: &Path, names: &[String], png_dir: Option<&Path>) -> Result<()> {
    let (matching, total) = check_base_cube(big_root)?;
    println!("base cube postfx_colourcube_base.xpr: {matching}/{total} texels = floor(i*255/31)");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let drops = drop_postfx_names(big_root)?;
    for n in &drops {
        *counts.entry(n.clone()).or_default() += 1;
    }
    let wanted: Vec<&str> = if names.is_empty() {
        counts.keys().map(String::as_str).collect()
    } else {
        names.iter().map(String::as_str).collect()
    };
    if names.is_empty() {
        println!("{} ge_drops name a post-FX preset; {} distinct", drops.len(), counts.len());
    }
    let built = load_presets(big_root, &wanted)?;
    let mut non_identity = 0;
    for (p, b) in &built {
        let dev = b.cube.max_deviation();
        non_identity += usize::from(dev > 0);
        println!(
            "\n{} ({} drops): chain {} | layer {} | bloom {} blur {:?}",
            p.name,
            counts.get(&p.name).copied().unwrap_or(0),
            p.chain.join(" -> "),
            p.layer.as_deref().unwrap_or("-"),
            if p.bloom_active() { "on" } else { "off" },
            p.bloom_blur()
        );
        for c in &p.colour_cubes {
            println!("  colour cube: stack {} mode {} group {} weight {}", c.stack, c.mode, c.group, c.weight);
        }
        for s in &b.stages {
            println!("  stage {s}");
        }
        let ramp: Vec<[u8; 3]> = [0, 8, 16, 24, 31].iter().map(|&i| b.cube.texels[i * (1 + 32 + 1024)]).collect();
        println!(
            "  LUT: max |delta| to identity {dev}, grey ramp (i=0,8,16,24,31) {ramp:?}{}",
            if b.needs_second_lut { " | NEEDS the depth-blended second LUT (not built)" } else { "" }
        );
        if let Some(dir) = png_dir {
            std::fs::create_dir_all(dir)?;
            write_cube_png(&dir.join(format!("{}_lut.png", p.name)), &b.cube)?;
        }
    }
    println!("\n{} presets resolved, {} LUTs built ({non_identity} differ from identity)", built.len(), built.len());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_field_names_hash_to_their_keys() {
        let known = [
            ("bloom_blur_x", 0xCA95_B073),
            ("bloom_blur_y", 0xCA95_B072),
            ("colourcube_depth_near", 0xFA03_885D),
            ("colourcube_depth_far", 0xF408_EB22),
            ("vignette_colour", 0xAF70_D8F2),
            ("vignette_fadeoffbounds", 0x18EC_F294),
        ];
        for (name, key) in known {
            assert_eq!(fnv1(name), key, "{name}");
            assert!(FIELD_NAMES.contains(&name));
        }
        assert_eq!(fnv1("day_sunny_postfx"), 0xF6F1_7684);
        assert_eq!(CLASS_POSTFX, 0x3D70_D4ED);
    }

    #[test]
    fn blend_replaces_group_opacity() {
        let text = r#"<?xml version="1.0"?>
<stack name="s">
  <group name="DO_NOT_EDIT" enabled="true" opacity="255">
    <layer name="LOAD_PREVIOUS" enabled="true" type="LoadColourCube" blendmode="Normal" opacity="255"/>
  </group>
  <group name="g" enabled="false" opacity="130">
    <layer name="c" enabled="true" type="BrightnessContrast" blendmode="Normal" opacity="255">
      <parameter name="brightness" value="0"/>
      <parameter name="contrast" value="15"/>
    </layer>
  </group>
</stack>"#;
        let xml = parse_stack(text).unwrap();
        assert_eq!(xml.groups[1].layers[0].parameters.len(), 2);
        let stacks = [ColourStack { index: 0, file: PathBuf::new(), xml }];
        let none = build_cube(&stacks, &[]).unwrap();
        assert_eq!(none.cube, ColourCube::identity());
        let blend = ColourCubeBlend { stack: 0, mode: 0, group: "g".into(), weight: 1.0 };
        let full = build_cube(&stacks, &[blend]).unwrap();
        assert!(full.stages[0].ends_with("255)"), "{:?}", full.stages);
        assert!(!full.needs_second_lut);
        let bad = ColourCubeBlend { stack: 0, mode: 0, group: "missing".into(), weight: 1.0 };
        assert!(build_cube(&stacks, &[bad]).is_err());
    }
}
