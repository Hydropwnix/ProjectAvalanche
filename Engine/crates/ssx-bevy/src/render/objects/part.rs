use std::path::Path;

use anyhow::{Context, Result};
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use ssx_assets::object_mesh::{self, ObjectMesh};
use ssx_assets::vault_db::Material as VaultMaterial;

use super::MaterialHandle;
use super::load::to_bevy_mesh;
use super::specs::{MaterialSpec, prepare_materials};
use crate::render::lighting::game::GameLightAssets;
use crate::render::objects::foreground::ForegroundMaterial;
use crate::render::objects::material::ObjectMaterial;
use crate::rider::material::{
    CharacterExtension, CharacterLightUniform, CharacterMaterial, CharacterParams, CharacterShading,
};

pub struct LoadedPart {
    mesh: Mesh,
    pub bounds: (Vec3, Vec3),
    images: Vec<Image>,
    material: MaterialSpec,
    pub problems: Vec<String>,
}

pub fn load_part(big_root: &Path, geom: &Path, material: &VaultMaterial) -> Result<LoadedPart> {
    let mesh = object_mesh::load(geom)?;
    let stem = geom.file_stem().unwrap_or_default().to_string_lossy().into_owned();
    part_from_mesh(big_root, mesh, &stem, material)
}

pub fn part_from_mesh(big_root: &Path, mesh: ObjectMesh, label: &str, material: &VaultMaterial) -> Result<LoadedPart> {
    let (lo, hi) = mesh.bounds().with_context(|| format!("{label} has no vertices"))?;
    let uv1 = CharacterShading::of(material.effect.as_deref().unwrap_or("")).and(mesh.uv1.clone());
    let mut mesh = to_bevy_mesh(mesh, label);
    if let Some(uv1) = uv1 {
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, uv1);
    }
    let mut problems = Vec::new();
    let (images, mut specs) = prepare_materials(big_root, &[material], &mut problems)?;
    let material = specs.pop().context("no material prepared")?;
    Ok(LoadedPart { mesh, bounds: (Vec3::from_array(lo), Vec3::from_array(hi)), images, material, problems })
}

impl LoadedPart {
    pub fn with_skin(mut self, joints: Vec<[u16; 4]>, weights: Vec<[f32; 4]>) -> Result<Self> {
        self.mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(joints));
        self.mesh.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, weights);
        self.mesh.generate_skinned_mesh_bounds()?;
        Ok(self)
    }

    pub fn add(
        self,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        light: &GameLightAssets,
        objects: &mut Assets<ObjectMaterial>,
        foreground: &mut Assets<ForegroundMaterial>,
    ) -> (Handle<Mesh>, MaterialHandle) {
        let handles: Vec<Handle<Image>> = self.images.into_iter().map(|i| images.add(i)).collect();
        (meshes.add(self.mesh), self.material.add(&handles, light, objects, foreground))
    }

    pub fn is_character(&self) -> bool {
        self.material.character.is_some()
    }

    pub fn add_character(
        self,
        meshes: &mut Assets<Mesh>,
        images: &mut Assets<Image>,
        light: &GameLightAssets,
        rider: &CharacterLightUniform,
        characters: &mut Assets<CharacterMaterial>,
    ) -> (Handle<Mesh>, Handle<CharacterMaterial>) {
        let handles: Vec<Handle<Image>> = self.images.into_iter().map(|i| images.add(i)).collect();
        let spec = &self.material;
        let image_or =
            |i: Option<usize>, fallback: &Handle<Image>| i.map_or_else(|| fallback.clone(), |i| handles[i].clone());
        let params = spec.character.unwrap_or_else(|| {
            let p = spec.params.shading;
            CharacterParams::new(CharacterShading::Body, p.y, p.z, Vec3::ZERO)
        });
        let material = CharacterMaterial {
            base: spec.standard_material(&handles),
            extension: CharacterExtension {
                light: light.uniform,
                rider: *rider,
                params,
                normal: image_or(spec.normal, &light.flat_normal),
                specular: image_or(spec.specular, &light.white),
                ambient: image_or(spec.ambient, &light.white),
                shadow_alpha: spec.shadow_alpha,
                local_lights: crate::render::lighting::local::LOCAL_LIGHT_BUFFER,
                local_light_textures: crate::render::lighting::local::LOCAL_LIGHT_TEXTURES,
                frame: crate::rider::material::RIDER_FRAME,
                shadow_pass: spec.shadow_pass,
            },
        };
        (meshes.add(self.mesh), characters.add(material))
    }
}
