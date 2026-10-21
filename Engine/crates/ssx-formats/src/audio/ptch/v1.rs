use super::*;

#[derive(Clone, Debug)]
pub struct PatchV1 {
    pub name: String,
    pub variables: Vec<VariableV1>,
    pub data: Vec<u8>,
    pub modules: Vec<ModuleV1>,
}

#[derive(Clone, Debug)]
pub struct VariableV1 {
    pub name: String,
    pub offset: u16,
    pub size: u8,
    pub kind: u8,
}

#[derive(Clone, Debug)]
pub struct ModuleV1 {
    pub offset: usize,
    pub fourcc: [u8; 4],
    pub kind: ModuleV1Kind,
}

#[derive(Clone, Debug)]
pub enum ModuleV1Kind {
    Voice(VoiceV1),
    Snapshot(SnapshotV1),
}

fn name32(r: &mut Reader) -> Result<String> {
    r.fixed_str(0x20)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingV1 {
    pub variable: String,
    pub kind: u8,
    pub plugin: u8,
    pub index: u8,
}

fn binding_v1(r: &mut Reader) -> Result<BindingV1> {
    let variable = name32(r)?;
    let (resolved, kind, plugin, index, z) = (r.u32()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
    if resolved != 0 || z != 0 {
        return invalid(format!("binding {variable}: run-time words {resolved:#x} {z}"));
    }
    Ok(BindingV1 { variable, kind, plugin, index })
}

#[derive(Clone, Debug)]
pub struct VoiceV1 {
    pub unk_0: u8,
    pub level: u8,
    pub mute: String,
    pub gain: String,
    pub mute_output: String,
    pub name: String,
    pub plugins: Vec<PluginV1>,
    pub links: Vec<LinkV1>,
    pub priority_mode: u8,
    pub priority: Option<BindingV1>,
    pub channel_vars: Vec<BindingV1>,
    pub params: Vec<BindingV1>,
    pub readbacks: Vec<BindingV1>,
    pub ctor_params: Vec<BindingV1>,
}

#[derive(Clone, Debug)]
pub struct PluginV1 {
    pub fourcc: [u8; 4],
    pub channels: u8,
    pub n_ctor: u8,
}

#[derive(Clone, Debug)]
pub struct LinkV1 {
    pub target: String,
    pub namespace: u8,
    pub plugin: u8,
    pub source_channel: u8,
    pub target_channel: u8,
    pub channels: u8,
}

impl VoiceV1 {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (unk_0, level, n_pl, n_links) = (r.u8()?, r.u8()?, r.u8()? as usize, r.u8()? as usize);
        let (n_par, n_rb, n_ctor) = (r.u16()? as usize, r.u16()? as usize, r.u16()? as usize);
        let (priority_mode, n_ch) = (r.u8()?, r.u8()? as usize);
        if (0..3).map(|_| r.u32()).collect::<Result<Vec<_>>>()?.iter().any(|&w| w != 0) {
            return invalid("voice reference words not zero");
        }
        let (mute, gain, mute_output, name) = (name32(&mut r)?, name32(&mut r)?, name32(&mut r)?, name32(&mut r)?);
        let mut plugins = Vec::with_capacity(n_pl);
        for _ in 0..n_pl {
            let (fourcc, z4, channels, n_ctor, pad) = (r.array::<4>()?, r.u32()?, r.u8()?, r.u8()?, r.bytes(6)?);
            if z4 != 0 || pad.iter().any(|&x| x != 0) {
                return invalid(format!("plug-in {} record padding", fourcc_str(fourcc)));
            }
            plugins.push(PluginV1 { fourcc, channels, n_ctor });
        }
        let mut links = Vec::with_capacity(n_links);
        for _ in 0..n_links {
            let target = name32(&mut r)?;
            let (namespace, plugin, source_channel, target_channel, channels) =
                (r.u8()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            if r.bytes(3)?.iter().any(|&x| x != 0) || plugin as usize >= n_pl || namespace > 2 {
                return invalid(format!("link {target}: namespace {namespace}, plug-in {plugin}"));
            }
            links.push(LinkV1 { target, namespace, plugin, source_channel, target_channel, channels });
        }
        let mut list = |n: usize| (0..n).map(|_| binding_v1(&mut r)).collect::<Result<Vec<_>>>();
        let priority = if priority_mode != 0 { list(1)?.pop() } else { None };
        let (channel_vars, params, readbacks, ctor_params) = (list(n_ch)?, list(n_par)?, list(n_rb)?, list(n_ctor)?);
        let fe = plugins.iter().filter(|p| p.channels == 0xFE).count();
        let ctor: usize = plugins.iter().map(|p| usize::from(p.n_ctor)).sum();
        if r.remaining() != 0 || fe != n_ch || ctor != n_ctor {
            return invalid(format!(
                "voice {name}: {} bytes left, {fe}/{n_ch} channel vars, {ctor}/{n_ctor} ctor",
                r.remaining()
            ));
        }
        let bad = params.iter().chain(&readbacks).chain(&ctor_params).find(|b| b.plugin as usize >= n_pl);
        if let Some(b) = bad {
            return invalid(format!("voice {name}: binding {} on plug-in {}", b.variable, b.plugin));
        }
        Ok(Self {
            unk_0,
            level,
            mute,
            gain,
            mute_output,
            name,
            plugins,
            links,
            priority_mode,
            priority,
            channel_vars,
            params,
            readbacks,
            ctor_params,
        })
    }
}

#[derive(Clone, Debug)]
pub struct SnapshotV1 {
    pub unk_0: u8,
    pub per_snapshot: u16,
    pub selector: String,
    pub snapshots: Vec<SnapshotEntry>,
    pub values: Vec<SnapshotValue>,
}

#[derive(Clone, Debug)]
pub struct SnapshotEntry {
    pub name: String,
    pub first: u16,
    pub count: u16,
    pub fade_time: String,
}

#[derive(Clone, Debug)]
pub struct SnapshotValue {
    pub source: String,
    pub target: String,
    pub kind: u8,
    pub curve: u8,
    pub unk_4a: u8,
}

impl SnapshotV1 {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (unk_0, z) = (r.u8()?, r.bytes(3)?);
        let (n, per_snapshot) = (r.u16()? as usize, r.u16()?);
        let selector = name32(&mut r)?;
        if z.iter().any(|&x| x != 0) || r.u32()? != 0 {
            return invalid("snapshot header padding");
        }
        let mut snapshots = Vec::with_capacity(n);
        for _ in 0..n {
            let name = name32(&mut r)?;
            let (count, first) = (r.u16()?, r.u16()?);
            let fade_time = name32(&mut r)?;
            if r.u32()? != 0 {
                return invalid(format!("snapshot {name}: reference word"));
            }
            snapshots.push(SnapshotEntry { name, first, count, fade_time });
        }
        let total: usize = snapshots.iter().map(|s| usize::from(s.count)).sum();
        let mut values = Vec::with_capacity(total);
        for _ in 0..total {
            let (source, target) = (name32(&mut r)?, name32(&mut r)?);
            let (r0, r1, kind, curve, unk_4a, z) = (r.u32()?, r.u32()?, r.u8()?, r.u8()?, r.u8()?, r.u8()?);
            if r0 != 0 || r1 != 0 || z != 0 || kind > 4 || curve > 4 {
                return invalid(format!("snapshot value {source} → {target}: {kind} {curve} {z}"));
            }
            values.push(SnapshotValue { source, target, kind, curve, unk_4a });
        }
        let in_range = snapshots.iter().all(|s| usize::from(s.first) + usize::from(s.count) <= total);
        if r.remaining() != 0 || !in_range {
            return invalid(format!("snapshot module {selector}: {} bytes left", r.remaining()));
        }
        Ok(Self { unk_0, per_snapshot, selector, snapshots, values })
    }
}

impl PatchV1 {
    pub fn variable(&self, name: &str) -> Option<&VariableV1> {
        self.variables.iter().find(|v| v.name == name)
    }

    pub fn parse(data: &[u8]) -> Result<Self> {
        let mut r = Reader::be(data);
        r.expect(MAGIC)?;
        let (version, loaded) = (r.u8()?, r.u8()?);
        let (n_vars, data_size, unk_0a, table_size) =
            (r.u16()? as usize, r.u16()? as usize, r.u16()?, r.u32()? as usize);
        let links = r.bytes(0x14)?;
        if version != 1
            || loaded != 0
            || unk_0a != 0
            || table_size != n_vars * 0x24 + data_size
            || links.iter().any(|&b| b != 0)
        {
            return invalid(format!("PTCH v1 header {version} {loaded} {n_vars} {data_size} {unk_0a} {table_size:#x}"));
        }
        let name = r.fixed_str(0x20)?;
        let mut variables = Vec::with_capacity(n_vars);
        for i in 0..n_vars {
            let name = r.fixed_str(0x20)?;
            let (offset, size, kind) = (r.u16()?, r.u8()?, r.u8()?);
            if offset as usize + size as usize > data_size {
                return invalid(format!("variable {i} ({name}) outside the {data_size:#x}-byte data"));
            }
            variables.push(VariableV1 { name, offset, size, kind });
        }
        let data_block = r.bytes(data_size)?.to_vec();
        let mut modules = Vec::new();
        loop {
            let offset = r.pos();
            let fourcc = r.array::<4>()?;
            if fourcc == [0; 4] {
                break;
            }
            let size = r.u32()? as usize;
            if size < 8 {
                return invalid(format!("v1 module at {offset:#x} with size {size}"));
            }
            let body = r.bytes(size - 8)?;
            let kind = match &fourcc {
                b"ACV0" => ModuleV1Kind::Voice(VoiceV1::parse(body)?),
                b"Sna0" => ModuleV1Kind::Snapshot(SnapshotV1::parse(body)?),
                _ => return invalid(format!("v1 module {} not registered", fourcc_str(fourcc))),
            };
            modules.push(ModuleV1 { offset, fourcc, kind });
        }
        if r.remaining() != 0 {
            return invalid(format!("{} bytes after the v1 module terminator", r.remaining()));
        }
        Ok(Self { name, variables, data: data_block, modules })
    }
}
