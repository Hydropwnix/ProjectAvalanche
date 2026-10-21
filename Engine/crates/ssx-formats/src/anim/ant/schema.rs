use super::*;

pub const FIELD_DYNAMIC_ARRAY: u16 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct FieldDesc {
    pub index: i32,
    pub name: String,
    pub type_id: u32,
    pub type_index: usize,
    pub size: u32,
    pub offset: u32,
    pub count: u16,
    pub flags: u16,
    pub align: u16,
    pub run: u16,
}

impl FieldDesc {
    pub fn is_dynamic_array(&self) -> bool {
        self.flags & FIELD_DYNAMIC_ARRAY != 0
    }

    pub fn effective_align(&self) -> u32 {
        let a = self.align as u32;
        if self.flags & 2 != 0 {
            a.max(4)
        } else if self.flags & 4 != 0 {
            a.max(8)
        } else if self.flags & 8 != 0 {
            a.max(16)
        } else {
            a
        }
    }

    pub fn footprint(&self) -> u64 {
        let (a, size) = (self.effective_align().max(1) as u64, self.size as u64);
        match self.count {
            0 => 0,
            1 => size,
            n => size.next_multiple_of(a) * (n as u64 - 1) + size,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TypeDesc {
    pub id: u32,
    pub name: String,
    pub first_index: i32,
    pub last_index: i32,
    pub size: u32,
    pub align: u32,
    pub flags: [u8; 4],
    pub fields: Vec<FieldDesc>,
    pub empty_slots: Vec<i32>,
}

impl TypeDesc {
    pub fn is_asset(&self) -> bool {
        self.first_index == -3
    }

    pub fn is_builtin(&self) -> bool {
        self.id < builtin::COUNT
    }

    pub fn field(&self, name: &str) -> Option<(usize, &FieldDesc)> {
        self.fields.iter().enumerate().find(|(_, f)| f.name == name)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schema {
    pub types: Vec<TypeDesc>,
}

impl Schema {
    pub fn index_of_id(&self, id: u32) -> Option<usize> {
        self.types.binary_search_by_key(&id, |t| t.id).ok()
    }

    pub fn index_of_name(&self, name: &str) -> Option<usize> {
        self.types.iter().position(|t| t.name == name)
    }

    pub fn get(&self, index: usize) -> Option<&TypeDesc> {
        self.types.get(index)
    }
}

pub fn type_layout_hash(slots: &[RawField], pool: &[u8], first_index: i32, last_index: i32, align: u32) -> u32 {
    let mut c = 0;
    for f in slots {
        c = crc32_msb(&f.type_id.to_le_bytes(), c);
        c = crc32_msb(&f.name_offset.to_le_bytes(), c);
        c = crc32_msb(&f.count.to_le_bytes(), c);
        c = crc32_msb(&f.flags.to_le_bytes(), c);
        c = crc32_msb(&f.offset.to_le_bytes(), c);
        c = crc32_msb(&f.run.to_le_bytes(), c);
    }
    c = crc32_msb(pool, c);
    c = crc32_msb(&first_index.to_le_bytes(), c);
    c = crc32_msb(&last_index.to_le_bytes(), c);
    crc32_msb(&align.to_le_bytes(), c)
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RawField {
    pub type_id: u32,
    pub size: u32,
    pub offset: u32,
    pub name_offset: u32,
    pub count: u16,
    pub flags: u16,
    pub align: u16,
    pub run: u16,
    pub type_ptr: u64,
}

impl RawField {
    pub fn is_empty(&self) -> bool {
        *self == RawField::default()
    }
}

pub fn compute_layout(fields: &[FieldDesc], sorted: bool) -> (Vec<u64>, u64, u32) {
    let mut order: Vec<usize> = (0..fields.len()).collect();
    if sorted {
        order.sort_by(|&a, &b| {
            let (x, y) = (&fields[a], &fields[b]);
            y.effective_align()
                .cmp(&x.effective_align())
                .then(y.size.cmp(&x.size))
                .then(y.count.cmp(&x.count))
                .then(x.type_id.cmp(&y.type_id))
                .then(x.index.cmp(&y.index))
        });
    }
    let (mut offsets, mut cur, mut align) = (vec![0; fields.len()], 0u64, 1u32);
    for i in order {
        let (a, n) = (fields[i].effective_align(), fields[i].footprint());
        if a == 0 || n == 0 {
            continue;
        }
        cur = cur.next_multiple_of(a as u64);
        offsets[i] = cur;
        cur += n;
        align = align.max(a);
    }
    (offsets, cur, align)
}

pub fn compute_runs(slots: &[RawField]) -> Vec<u16> {
    let mut runs = vec![0u16; slots.len()];
    for i in (0..slots.len()).rev() {
        let f = &slots[i];
        if f.is_empty() {
            continue;
        }
        let mut r = f.count;
        if let Some(n) = slots.get(i + 1)
            && !n.is_empty()
            && n.type_id == f.type_id
            && n.flags == f.flags
        {
            r = r.wrapping_add(runs[i + 1]);
        }
        runs[i] = r;
    }
    runs
}

fn pool_str(pool: &[u8], at: usize) -> Option<String> {
    let rest = pool.get(at..)?;
    let end = rest.iter().position(|&b| b == 0)?;
    Some(String::from_utf8_lossy(&rest[..end]).into_owned())
}

struct RawType {
    offset: usize,
    desc: TypeDesc,
    slots: Vec<RawField>,
    pool: Vec<u8>,
    field_targets: Vec<usize>,
}

fn parse_type(p: &mut Payload<'_>, at: usize) -> Result<RawType> {
    let mut r = p.reader(at)?;
    let first_index = r.i32()?;
    let last_index = r.i32()?;
    let size = r.u32()?;
    let align = r.u32()?;
    let pool_off = r.u32()? as usize;
    let pool_len = r.u32()? as usize;
    let flags: [u8; 4] = r.array()?;
    let id = r.u32()?;
    let n = last_index as i64 - first_index as i64 + 1;
    if n < 0 || pool_off as i64 != 0x20 + 0x20 * n || pool_len < 2 {
        return Err(
            p.bad(at, format!("type {id:#x}: slots {first_index}..={last_index}, pool {pool_off:#x}/{pool_len}"))
        );
    }
    let pool = p.slice(at + pool_off, pool_len)?.to_vec();
    let name = match (pool[0], pool_str(&pool, 1)) {
        (0, Some(s)) => s,
        _ => return Err(p.bad(at, format!("type {id:#x}: bad name pool"))),
    };
    let mut desc =
        TypeDesc { id, name, first_index, last_index, size, align, flags, fields: Vec::new(), empty_slots: Vec::new() };
    let (mut slots, mut field_targets) = (Vec::new(), Vec::new());
    let mut name_offsets = Vec::new();
    for k in 0..n as usize {
        let fo = at + 0x20 + 0x20 * k;
        let mut fr = p.reader(fo)?;
        let raw = RawField {
            type_id: fr.u32()?,
            size: fr.u32()?,
            offset: fr.u32()?,
            name_offset: fr.u32()?,
            count: fr.u16()?,
            flags: fr.u16()?,
            align: fr.u16()?,
            run: fr.u16()?,
            type_ptr: fr.u64()?,
        };
        slots.push(raw);
        let index = first_index + k as i32;
        if raw.is_empty() {
            desc.empty_slots.push(index);
            continue;
        }
        if raw.type_id == 0 {
            return Err(p.bad(fo, format!("{}: slot {index} has no type id but is not empty", desc.name)));
        }
        let fname = pool_str(&pool, raw.name_offset as usize)
            .ok_or_else(|| p.bad(fo, format!("{}: slot {index} name offset {:#x}", desc.name, raw.name_offset)))?;
        name_offsets.push((raw.name_offset as usize, fname.len() + 1));
        let target = p.ptr(fo + 0x18)?.ok_or_else(|| p.bad(fo, "field without type pointer"))?;
        field_targets.push(target);
        desc.fields.push(FieldDesc {
            index,
            name: fname,
            type_id: raw.type_id,
            type_index: usize::MAX,
            size: raw.size,
            offset: raw.offset,
            count: raw.count,
            flags: raw.flags,
            align: raw.align,
            run: raw.run,
        });
    }
    name_offsets.sort_unstable();
    let mut next = desc.name.len() + 2;
    for (o, len) in name_offsets {
        if o != next {
            return Err(p.bad(at, format!("{}: name pool not packed at {next:#x}", desc.name)));
        }
        next += len;
    }
    if next != pool_len {
        return Err(p.bad(at, format!("{}: name pool length {pool_len} != {next}", desc.name)));
    }
    Ok(RawType { offset: at, desc, slots, pool, field_targets })
}

pub(super) fn parse_schema(p: &mut Payload<'_>) -> Result<Schema> {
    let count = p.reader(0)?.u64()?;
    if count == 0 || count > (p.data.len() as u64) / 8 {
        return Err(p.bad(0, format!("type count {count}")));
    }
    let count = count as usize;
    let mut raws = Vec::with_capacity(count);
    let mut end = 8 + 8 * count;
    for i in 0..count {
        let at = p.ptr(8 + 8 * i)?.ok_or_else(|| p.bad(8 + 8 * i, "null type pointer"))?;
        if at != end.next_multiple_of(8) || p.data[end..at].iter().any(|&b| b != 0) {
            return Err(p.bad(at, format!("type descriptor {i} not packed after {end:#x}")));
        }
        let t = parse_type(p, at)?;
        end = at + 0x20 + 0x20 * t.slots.len() + t.pool.len();
        raws.push(t);
    }
    if end.next_multiple_of(4) != p.data.len() || p.data.get(end..).is_none_or(|t| t.iter().any(|&b| b != 0)) {
        return Err(p.bad(end, "REFL payload does not end after the last descriptor"));
    }
    p.all_relocs_used()?;
    let by_offset: HashMap<usize, usize> = raws.iter().enumerate().map(|(i, t)| (t.offset, i)).collect();
    let ids: Vec<u32> = raws.iter().map(|t| t.desc.id).collect();
    if ids.windows(2).any(|w| w[0] >= w[1]) {
        return Err(p.bad(0, "type ids not strictly ascending"));
    }
    let mut types = Vec::with_capacity(count);
    for t in &raws {
        let mut desc = t.desc.clone();
        for (f, target) in desc.fields.iter_mut().zip(&t.field_targets) {
            let ti = *by_offset.get(target).ok_or_else(|| p.bad(*target, "field type is not a listed descriptor"))?;
            if ids[ti] != f.type_id {
                return Err(p.bad(
                    t.offset,
                    format!("{}.{}: type id {:#x} != target {:#x}", desc.name, f.name, f.type_id, ids[ti]),
                ));
            }
            f.type_index = ti;
        }
        types.push(desc);
    }
    for (t, raw) in types.iter().zip(&raws) {
        validate_type(t, raw, &types).map_err(|m| p.bad(raw.offset, format!("type {} ({:#x}): {m}", t.name, t.id)))?;
    }
    Ok(Schema { types })
}

fn validate_type(t: &TypeDesc, raw: &RawType, types: &[TypeDesc]) -> std::result::Result<(), String> {
    for f in &t.fields {
        let target = &types[f.type_index];
        let (size, align) = if f.is_dynamic_array() { (0x10, 8) } else { (target.size, target.align) };
        if f.size != size || f.align as u32 != align || f.count == 0 || target.size == 0 {
            return Err(format!("field {}: size {:#x}/align {}/count {}", f.name, f.size, f.align, f.count));
        }
    }
    let runs = compute_runs(&raw.slots);
    if raw.slots.iter().zip(&runs).any(|(s, &r)| s.run != r) {
        return Err("field run counts differ from the recomputation".into());
    }
    if t.is_builtin() {
        let (name, size, align, nfields, ftype) = BUILTINS[t.id as usize];
        let fields_ok = t.fields.len() == nfields
            && t.fields.iter().enumerate().all(|(i, f)| {
                f.type_id == ftype && f.offset == (i as u32) * types[f.type_index].size && !f.is_dynamic_array()
            });
        if t.id == 0 || t.name != name || t.size != size || t.align != align || t.flags != [0, 1, 0, 0] || !fields_ok {
            return Err("builtin differs from the executable's table".into());
        }
        return Ok(());
    }
    if t.flags[1..] != [0, 0, 0] || t.flags[0] > 1 {
        return Err(format!("flags {:?}", t.flags));
    }
    match t.first_index {
        0 => {}
        -3 => {
            let want = [("__guid", builtin::GUID), ("__name", builtin::STRING), ("__base", builtin::DATA_REF)];
            let ok =
                t.fields.len() >= 3 && t.fields.iter().zip(want).all(|(f, (n, id))| f.name == n && f.type_id == id);
            if !ok || t.empty_slots.iter().any(|&i| i < 0) {
                return Err("asset type without __guid/__name/__base at −3..−1".into());
            }
        }
        i => return Err(format!("first field index {i}")),
    }
    let (offsets, size, align) = compute_layout(&t.fields, t.flags[0] == 1);
    if t.fields.iter().zip(&offsets).any(|(f, &o)| f.offset as u64 != o) || size != t.size as u64 || align != t.align {
        return Err("field offsets / size / align differ from the layout recomputation".into());
    }
    let hash = type_layout_hash(&raw.slots, &raw.pool, t.first_index, t.last_index, t.align);
    if hash != t.id {
        return Err(format!("layout hash {hash:#x} != id"));
    }
    Ok(())
}
