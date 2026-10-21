use super::*;

#[derive(Clone, Debug)]
pub struct Voice {
    pub unk_0: u8,
    pub unk_level: u8,
    pub name: String,
    pub priority: Ref,
    pub mute: Ref,
    pub solo: Ref,
    pub solo_safe: Ref,
    pub expelled: Ref,
    pub plugins: Vec<Plugin>,
    pub ctor_params: Vec<PluginParam>,
    pub params: Vec<PluginParam>,
    pub readbacks: Vec<PluginParam>,
    pub inputs: Vec<Ref>,
    pub sends: Vec<(Ref, u8)>,
    pub events: Vec<PluginEvent>,
}

#[derive(Clone, Debug)]
pub struct Plugin {
    pub fourcc: [u8; 4],
    pub bypass: Ref,
    pub channels: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct PluginParam {
    pub value: Ref,
    pub plugin: u8,
    pub index: u8,
    pub kind: u8,
    pub unk_7: u8,
}

#[derive(Clone, Debug)]
pub struct PluginEvent {
    pub event: Ref,
    pub plugin: u8,
    pub index: u8,
    pub args: Vec<PluginParam>,
}

fn plugin_param(r: &mut Reader) -> Result<PluginParam> {
    Ok(PluginParam { value: Ref(r.u32()?), plugin: r.u8()?, index: r.u8()?, kind: r.u8()?, unk_7: r.u8()? })
}

impl Voice {
    pub(super) fn parse(b: &[u8]) -> Result<Self> {
        let mut r = Reader::be(b);
        let (unk_0, unk_level, n_pl, z3, z4) = (r.u8()?, r.u8()?, r.u8()? as usize, r.u8()?, r.u16()?);
        let mut n = [0usize; 7];
        for v in &mut n {
            *v = r.u16()? as usize;
        }
        let [n_ctor, n_par, n_rb, n_in, n_send, n_ev, n_args] = n;
        let mut o = [0usize; 8];
        for v in &mut o {
            *v = r.u32()? as usize;
        }
        let [priority, mute, solo, solo_safe, expelled] = refs::<5>(&mut r)?;
        if z3 != 0 || z4 != 0 || o[0] <= 0x48 {
            return invalid(format!("voice header bytes {z3} {z4}, plug-ins at {:#x}", o[0]));
        }
        let mut cov = Coverage::new(b.len());
        let name = cstr_at(b, 0x48)?;
        cov.mark(0, 0x48 + name.len() + 1, "voice header")?;
        let mut plugins = Vec::with_capacity(n_pl);
        let mut r = at(b, o[0])?;
        for _ in 0..n_pl {
            let (z0, z1, fourcc, bypass) = (r.u32()?, r.u32()?, r.array::<4>()?, Ref(r.u32()?));
            let (channels, z11, z12, z14) = (r.u8()?, r.u8()?, r.u16()?, r.u32()?);
            if z0 | z1 | z14 != 0 || z11 != 0 || z12 != 0 || !(1..=8).contains(&channels) {
                return invalid("plug-in record padding not zero / channel count out of range");
            }
            plugins.push(Plugin { fourcc, bypass, channels });
        }
        cov.mark(o[0], 0x18 * n_pl, "plug-ins")?;
        let list = |off: usize, n: usize, cov: &mut Coverage| -> Result<Vec<PluginParam>> {
            let mut r = at(b, off)?;
            let v = (0..n).map(|_| plugin_param(&mut r)).collect::<Result<Vec<_>>>()?;
            cov.mark(off, 8 * n, "plug-in bindings")?;
            Ok(v)
        };
        let ctor_params = list(o[1], n_ctor, &mut cov)?;
        let mut params = Vec::with_capacity(n_par);
        let mut r = at(b, o[2])?;
        for _ in 0..n_par {
            let p = plugin_param(&mut r)?;
            let cache = match p.kind {
                2 | 4 => 8,
                3 => r.clone().u16()? as usize + 2,
                _ => 4,
            };
            if r.bytes(cache)?.iter().any(|&x| x != 0) {
                return invalid("parameter cache not zero");
            }
            params.push(p);
        }
        cov.mark(o[2], r.pos().saturating_sub(o[2]), "parameters")?;
        let readbacks = list(o[3], n_rb, &mut cov)?;
        let inputs = list(o[4], n_in, &mut cov)?;
        if inputs.iter().any(|p| p.plugin | p.index | p.kind | p.unk_7 != 0) {
            return invalid("voice input record padding not zero");
        }
        let inputs = inputs.into_iter().map(|p| p.value).collect();
        let mut sends = Vec::with_capacity(n_send);
        let mut r = at(b, o[5])?;
        for _ in 0..n_send {
            let p = plugin_param(&mut r)?;
            if r.u64()? != 0 || p.plugin as usize >= n_pl {
                return invalid("send record");
            }
            sends.push((p.value, p.plugin));
        }
        cov.mark(o[5], 16 * n_send, "sends")?;
        let mut events = Vec::with_capacity(n_ev);
        let mut r = at(b, o[6])?;
        let mut arg_pos = o[7];
        for _ in 0..n_ev {
            let (event, last, plugin, index, na, z) =
                (Ref(r.u32()?), r.u32()?, r.u8()?, r.u8()?, r.u8()? as usize, r.u8()?);
            if last != 0 || z != 0 {
                return invalid("voice event record padding not zero");
            }
            let args = list(arg_pos, na, &mut cov)?;
            arg_pos += 8 * na;
            events.push(PluginEvent { event, plugin, index, args });
        }
        cov.mark(o[6], 12 * n_ev, "voice events")?;
        if arg_pos != o[7] + 8 * n_args {
            return invalid("event argument count mismatch");
        }
        cov.finish(b)?;
        Ok(Self {
            unk_0,
            unk_level,
            name,
            priority,
            mute,
            solo,
            solo_safe,
            expelled,
            plugins,
            ctor_params,
            params,
            readbacks,
            inputs,
            sends,
            events,
        })
    }
}
