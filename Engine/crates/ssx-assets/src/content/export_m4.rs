use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use ssx_content::{ContentId, Kind, Source};

use super::models::DiscModels;
use super::objects::DiscRecords;

fn header(id: &ContentId, what: &str) -> String {
    format!(
        "# {id}: {what} as the disc defines it — an export for your own use (Docs/architecture/content.md §15).\n\
         # A package may not ship it: `mod check` and `mod pack` refuse exported bytes. Change the fields you want and\n\
         # name it as a patch (only the fields you keep) or an override (the whole record).\n\n"
    )
}

pub fn export(
    big_root: &Path,
    id: &ContentId,
    models: &Arc<DiscModels>,
    package: &str,
    name: &str,
) -> Result<Vec<(String, Vec<u8>)>> {
    let table = match id.kind() {
        Kind::Lighting | Kind::Postfx => {
            let records = DiscRecords::new(big_root, models.clone());
            records.load(id)?.record().cloned().context("not a record")?
        }
        _ => {
            let disc = super::particles::DiscParticles::index(big_root)?;
            disc.load(id)?.record().cloned().context("not a record")?
        }
    };
    let what = match id.kind() {
        Kind::Lighting => "the lighting preset (lighting.toml)",
        Kind::Postfx => "the post-FX preset (postfx.toml)",
        _ if ssx_content::particles::is_spray(id) => "the snow-spray table (particles.toml)",
        _ => "the particle system (particles.toml)",
    };
    let mut files = Vec::new();
    let mut text = header(id, what) + &toml::to_string(&table).context("the record as TOML")?;
    if id.kind() == Kind::Postfx {
        let record =
            ssx_content::postfx::PostFxRecord::from_table(&table).map_err(|e| anyhow::anyhow!("{}", e.join("; ")))?;
        let blends: Vec<crate::postfx::ColourCubeBlend> = record
            .colour_cubes
            .iter()
            .map(|b| crate::postfx::ColourCubeBlend {
                stack: b.stack,
                mode: b.mode,
                group: b.group.clone(),
                weight: b.weight,
            })
            .collect();
        let build = crate::postfx::build_cube_for(big_root, &blends)?;
        let strip = ssx_content::postfx::strip_of(&build.cube.texels);
        let mut png = Vec::new();
        {
            let mut e = png::Encoder::new(&mut png, 1024, 32);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()?.write_image_data(&strip)?;
        }
        files.push((format!("{name}_cube.png"), png));
        text += &format!(
            "\n# The cube as its PNG strip ({name}_cube.png: 32 slices of 32x32, slice b at x = 32 b, R along x, G along y).\n\
             # To draw your own cube in place of the generated one, add it to your package and name it here:\n\
             #   [add] \"postfx/{name}_cube\" = \"postfx/{name}_cube.png\"    (resource.toml)\n\
             #   cube = \"{package}:postfx/{name}_cube\"\n"
        );
    }
    if id.kind() == Kind::Lighting && table.is_empty() {
        bail!("{id}: an empty preset");
    }
    files.insert(0, (format!("{name}.toml"), text.into_bytes()));
    Ok(files)
}
