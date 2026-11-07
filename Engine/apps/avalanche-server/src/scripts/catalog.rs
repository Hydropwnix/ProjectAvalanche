use ssx_assets::content::GameContent;
use ssx_content::marker::Hit;
use ssx_content::{Kind, Layer};
use ssx_script::host::game::{Catalog, ContentMeta, MarkerHitShape, MarkerMeta};

fn layer_name(l: Layer) -> &'static str {
    match l {
        Layer::Base => "L0 base",
        Layer::Port => "L1 port",
        Layer::ClientMods => "L2 client mods",
        Layer::Server => "L3 server",
    }
}

fn hit_shape(h: Hit) -> MarkerHitShape {
    match h {
        Hit::None => MarkerHitShape::None,
        Hit::Sphere => MarkerHitShape::Sphere,
        Hit::Cylinder => MarkerHitShape::Cylinder,
        Hit::Ring => MarkerHitShape::Ring,
        Hit::Gate => MarkerHitShape::Gate,
    }
}

pub fn catalog_of(content: &GameContent) -> (Catalog, Vec<String>) {
    let mut cat = Catalog::default();
    content.with_registry(|r| {
        for kind in Kind::ALL {
            for id in r.list(kind) {
                if let Some(w) = r.winner(&id) {
                    cat.ids.insert(id, ContentMeta { source: w.name.to_string(), layer: layer_name(w.layer) });
                }
            }
        }
    });
    let disc = || ContentMeta { source: "ssx".to_string(), layer: layer_name(Layer::Base) };
    if let Some(s) = content.disc_sounds() {
        for id in s.by_id.keys() {
            cat.ids.entry(id.clone()).or_insert_with(disc);
        }
    }
    if let Some(m) = content.disc_music() {
        for id in m.by_id.keys() {
            cat.ids.entry(id.clone()).or_insert_with(disc);
        }
    }
    let mut problems = Vec::new();
    for id in content.marker_ids() {
        match content.marker(&id) {
            Ok(look) => {
                let d = &look.def;
                let meta = MarkerMeta {
                    size: d.size,
                    colour: d.colour,
                    hit: hit_shape(d.hit),
                    sound: d.sound.clone(),
                    vertices: look.vertices(),
                    triangles: look.triangles(),
                };
                cat.markers.insert(id, meta);
            }
            Err(e) => problems.push(format!("the marker {id} cannot be placed: {e}")),
        }
    }
    (cat, problems)
}
