use bevy::mesh::MeshTag;
use bevy::prelude::*;

use super::MaterialHandle;
use crate::render::objects::material::ObjectMaterial;
use crate::render::warm_up::draw;

pub(super) fn spawn(
    commands: &mut Commands,
    pairs: impl IntoIterator<Item = (Handle<Mesh>, MaterialHandle, [Option<Handle<ObjectMaterial>>; 2])>,
) -> usize {
    let mut n = 0;
    for (mesh, base, variants) in pairs {
        match base {
            MaterialHandle::Object(m) => {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(m), MeshTag::new(0), draw()));
            }
            MaterialHandle::Foreground(m) => {
                commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(m), draw()));
            }
        }
        n += 1;
        for m in variants.into_iter().flatten() {
            commands.spawn((Mesh3d(mesh.clone()), MeshMaterial3d(m), MeshTag::new(0), draw()));
            n += 1;
        }
    }
    n
}
