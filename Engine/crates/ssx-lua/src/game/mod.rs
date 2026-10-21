pub mod bindings;
pub mod controllers;
pub mod events;
pub mod graph;
pub mod objectives;
pub mod record;
pub mod scene;

use crate::stdlib;
use crate::vm::Lua;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Binder {
    Objectives,
    StateSets,
    Schemas,
    Scene,
    Badges,
}

impl Binder {
    pub const ALL: [Binder; 5] =
        [Binder::Objectives, Binder::StateSets, Binder::Schemas, Binder::Scene, Binder::Badges];

    pub fn name(self) -> &'static str {
        match self {
            Binder::Objectives => "objectives",
            Binder::StateSets => "state sets",
            Binder::Schemas => "schemas",
            Binder::Scene => "scene",
            Binder::Badges => "badges",
        }
    }

    pub fn register_addr(self) -> u32 {
        match self {
            Binder::Objectives => 0x823E_F800,
            Binder::StateSets => 0x823D_E620,
            Binder::Schemas => 0x823E_5C60,
            Binder::Scene => 0x823E_8A70,
            Binder::Badges => 0x823E_3D50,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Lib {
    Base,
    Package,
    Table,
    String,
    Math,
    Os,
    Debug,
}

pub const LIBS_ALL: [Lib; 7] = [Lib::Base, Lib::Package, Lib::Table, Lib::String, Lib::Math, Lib::Os, Lib::Debug];
pub const LIBS_WORLD: [Lib; 5] = [Lib::Base, Lib::Table, Lib::String, Lib::Math, Lib::Os];
pub const LIBS_FRONTEND: [Lib; 5] = [Lib::Base, Lib::Package, Lib::Table, Lib::String, Lib::Math];
pub const LIBS_ANIM: [Lib; 5] = [Lib::Base, Lib::Table, Lib::String, Lib::Math, Lib::Debug];

pub fn open_libs(l: &mut Lua, libs: &[Lib]) {
    for lib in libs {
        match lib {
            Lib::Base => stdlib::base::open(l),
            Lib::Package => stdlib::package::open(l),
            Lib::Table => stdlib::tablib::open(l),
            Lib::String => stdlib::string::open(l),
            Lib::Math => stdlib::math::open(l),
            Lib::Os => stdlib::os::open(l),
            Lib::Debug => stdlib::debug::open(l),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Role {
    World(Binder),
    FrontEnd,
}

pub fn role(path: &str) -> Role {
    let p = path.replace('\\', "/").to_ascii_lowercase();
    let file = p.rsplit('/').next().unwrap_or(&p);
    if p.contains("/fe/lua/") {
        return Role::FrontEnd;
    }
    if p.contains("gameexplorer/scripts/") || file == "world_entity_controllers_default.lua" {
        return Role::World(Binder::StateSets);
    }
    if file == "world_entity_schemas_default.lua" {
        return Role::World(Binder::Schemas);
    }
    if file == "init_badges.lua" {
        return Role::World(Binder::Badges);
    }
    let avalanche_instances = p.contains("common/avalanche/") && !file.starts_with("avalanche_objective");
    if file == "default_scene.lua" || file.ends_with("instances.lua") || avalanche_instances {
        return Role::World(Binder::Scene);
    }
    Role::World(Binder::Objectives)
}
