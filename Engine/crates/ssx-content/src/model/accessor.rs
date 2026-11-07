use gltf_json::Root;
use gltf_json::accessor::{ComponentType, Type};
use gltf_json::validation::Checked;

pub type Layout = (Type, ComponentType, bool);

pub const F32_VEC2: Layout = (Type::Vec2, ComponentType::F32, false);
pub const F32_VEC3: Layout = (Type::Vec3, ComponentType::F32, false);
pub const F32_VEC4: Layout = (Type::Vec4, ComponentType::F32, false);

fn components(t: Type) -> Option<usize> {
    match t {
        Type::Scalar => Some(1),
        Type::Vec2 => Some(2),
        Type::Vec3 => Some(3),
        Type::Vec4 => Some(4),
        Type::Mat2 | Type::Mat3 | Type::Mat4 => None,
    }
}

fn size_of(c: ComponentType) -> usize {
    match c {
        ComponentType::I8 | ComponentType::U8 => 1,
        ComponentType::I16 | ComponentType::U16 => 2,
        ComponentType::U32 | ComponentType::F32 => 4,
    }
}

fn component(c: ComponentType, normalized: bool, b: &[u8]) -> f32 {
    let le2 = || [b[0], b[1]];
    let le4 = || [b[0], b[1], b[2], b[3]];
    match (c, normalized) {
        (ComponentType::F32, _) => f32::from_le_bytes(le4()),
        (ComponentType::U8, true) => f32::from(b[0]) / 255.0,
        (ComponentType::I8, true) => (f32::from(b[0] as i8) / 127.0).max(-1.0),
        (ComponentType::U16, true) => f32::from(u16::from_le_bytes(le2())) / 65535.0,
        (ComponentType::I16, true) => (f32::from(i16::from_le_bytes(le2())) / 32767.0).max(-1.0),
        (ComponentType::U8, false) => f32::from(b[0]),
        (ComponentType::I8, false) => f32::from(b[0] as i8),
        (ComponentType::U16, false) => f32::from(u16::from_le_bytes(le2())),
        (ComponentType::I16, false) => f32::from(i16::from_le_bytes(le2())),
        (ComponentType::U32, _) => u32::from_le_bytes(le4()) as f32,
    }
}

fn index_value(c: ComponentType, b: &[u8]) -> Option<u32> {
    match c {
        ComponentType::U8 => Some(u32::from(b[0])),
        ComponentType::U16 => Some(u32::from(u16::from_le_bytes([b[0], b[1]]))),
        ComponentType::U32 => Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]])),
        _ => None,
    }
}

fn view_bytes<'a>(root: &Root, buffers: &[&'a [u8]], view: usize) -> Result<(&'a [u8], Option<usize>), String> {
    let v = root.buffer_views.get(view).ok_or_else(|| format!("bufferViews[{view}] does not exist"))?;
    let b = v.buffer.value();
    let data = buffers.get(b).ok_or_else(|| format!("bufferViews[{view}]: buffers[{b}] does not exist"))?;
    let declared = root.buffers.get(b).map_or(0, |b| b.byte_length.0);
    let start = v.byte_offset.map_or(0, |o| o.0);
    let end = start.checked_add(v.byte_length.0).filter(|&e| e <= declared && e <= data.len() as u64);
    let Some(end) = end else {
        return Err(format!("bufferViews[{view}]: bytes {start}+{} lie outside buffers[{b}]", v.byte_length.0));
    };
    if v.byte_length.0 == 0 {
        return Err(format!("bufferViews[{view}]: no bytes"));
    }
    let stride = v.byte_stride.map(|s| s.0);
    if let Some(s) = stride
        && (!(4..=252).contains(&s) || s % 4 != 0)
    {
        return Err(format!("bufferViews[{view}]: byteStride {s} (4 to 252, a multiple of 4)"));
    }
    Ok((&data[start as usize..end as usize], stride))
}

fn elements<'a>(
    data: &'a [u8],
    offset: u64,
    stride: usize,
    elem: usize,
    count: u64,
    what: &str,
) -> Result<impl Iterator<Item = &'a [u8]>, String> {
    let need = (count - 1)
        .checked_mul(stride as u64)
        .and_then(|x| x.checked_add(offset))
        .and_then(|x| x.checked_add(elem as u64));
    if need.is_none_or(|n| n > data.len() as u64) {
        return Err(format!(
            "{what}: {count} elements of {elem} bytes, stride {stride}, from byte {offset}, do not fit its view of {} bytes",
            data.len()
        ));
    }
    Ok((0..count as usize).map(move |i| &data[offset as usize + i * stride..offset as usize + i * stride + elem]))
}

pub fn read_f32(
    root: &Root,
    buffers: &[&[u8]],
    index: usize,
    accept: &[Layout],
    max_count: u64,
) -> Result<(Vec<f32>, usize), String> {
    let what = format!("accessors[{index}]");
    let a = root.accessors.get(index).ok_or_else(|| format!("{what} does not exist"))?;
    let (Checked::Valid(t), Checked::Valid(c)) = (&a.type_, &a.component_type) else {
        return Err(format!("{what}: an unknown type or component type"));
    };
    let (t, c) = (*t, c.0);
    if !accept.iter().any(|&(at, ac, an)| at == t && ac == c && an == a.normalized) {
        return Err(format!(
            "{what}: {t:?} of {c:?}{} is not accepted here",
            if a.normalized { " (normalised)" } else { "" }
        ));
    }
    let n = components(t).ok_or_else(|| format!("{what}: a matrix"))?;
    let count = a.count.0;
    if count == 0 || count > max_count {
        return Err(format!("{what}: {count} elements (1 to {max_count} are read)"));
    }
    let cs = size_of(c);
    let elem = cs * n;
    let mut out = vec![0.0f32; count as usize * n];
    if let Some(view) = a.buffer_view {
        let (data, stride) = view_bytes(root, buffers, view.value())?;
        let offset = a.byte_offset.map_or(0, |o| o.0);
        let view_offset = root.buffer_views[view.value()].byte_offset.map_or(0, |o| o.0);
        if !offset.is_multiple_of(cs as u64) || !view_offset.is_multiple_of(cs as u64) {
            return Err(format!("{what}: an offset not aligned to its {cs}-byte components"));
        }
        let stride = stride.unwrap_or(elem);
        if stride < elem {
            return Err(format!("{what}: byteStride {stride} below its element of {elem} bytes"));
        }
        for (i, e) in elements(data, offset, stride, elem, count, &what)?.enumerate() {
            for k in 0..n {
                out[i * n + k] = component(c, a.normalized, &e[k * cs..]);
            }
        }
    } else if a.sparse.is_none() {
        return Err(format!("{what}: neither a buffer view nor sparse values"));
    }
    if let Some(s) = &a.sparse {
        let positions = sparse_indices(root, buffers, s, count, &what)?;
        let (data, stride) = view_bytes(root, buffers, s.values.buffer_view.value())?;
        if stride.is_some() {
            return Err(format!("{what}: sparse values with a byteStride"));
        }
        for (e, at) in elements(data, s.values.byte_offset.0, elem, elem, positions.len() as u64, &what)?.zip(positions)
        {
            for k in 0..n {
                out[at as usize * n + k] = component(c, a.normalized, &e[k * cs..]);
            }
        }
    }
    if out.iter().any(|v| !v.is_finite()) {
        return Err(format!("{what}: a value that is not a finite number"));
    }
    Ok((out, n))
}

pub fn read_matrices(root: &Root, buffers: &[&[u8]], index: usize, count: u64) -> Result<Vec<[[f32; 4]; 4]>, String> {
    let what = format!("accessors[{index}] (inverse bind matrices)");
    let a = root.accessors.get(index).ok_or_else(|| format!("{what} does not exist"))?;
    let (Checked::Valid(Type::Mat4), Checked::Valid(c)) = (&a.type_, &a.component_type) else {
        return Err(format!("{what}: not MAT4 (glTF 2.0 §3.7.3.1)"));
    };
    if c.0 != ComponentType::F32 || a.normalized {
        return Err(format!("{what}: MAT4 of floats, not {:?}", c.0));
    }
    if a.sparse.is_some() {
        return Err(format!("{what}: sparse matrices are not read"));
    }
    if a.count.0 != count {
        return Err(format!("{what}: {} matrices for {count} joints", a.count.0));
    }
    let view = a.buffer_view.ok_or_else(|| format!("{what}: no buffer view"))?;
    let (data, stride) = view_bytes(root, buffers, view.value())?;
    let offset = a.byte_offset.map_or(0, |o| o.0);
    let view_offset = root.buffer_views[view.value()].byte_offset.map_or(0, |o| o.0);
    if !offset.is_multiple_of(4) || !view_offset.is_multiple_of(4) {
        return Err(format!("{what}: an offset not aligned to its 4-byte components"));
    }
    let stride = stride.unwrap_or(64);
    if stride < 64 {
        return Err(format!("{what}: byteStride {stride} below its matrix of 64 bytes"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for e in elements(data, offset, stride, 64, count, &what)? {
        let m: [[f32; 4]; 4] = std::array::from_fn(|r| {
            std::array::from_fn(|c| f32::from_le_bytes(std::array::from_fn(|k| e[(r * 4 + c) * 4 + k])))
        });
        if m.iter().flatten().any(|v| !v.is_finite()) {
            return Err(format!("{what}: a value that is not a finite number"));
        }
        out.push(m);
    }
    Ok(out)
}

fn sparse_indices(
    root: &Root,
    buffers: &[&[u8]],
    s: &gltf_json::accessor::sparse::Sparse,
    count: u64,
    what: &str,
) -> Result<Vec<u32>, String> {
    let n = s.count.0;
    if n == 0 || n > count {
        return Err(format!("{what}: {n} sparse values for {count} elements"));
    }
    let Checked::Valid(ic) = &s.indices.component_type else {
        return Err(format!("{what}: sparse indices of an unknown component type"));
    };
    let ic = ic.0;
    let (data, stride) = view_bytes(root, buffers, s.indices.buffer_view.value())?;
    if stride.is_some() {
        return Err(format!("{what}: sparse indices with a byteStride"));
    }
    let size = size_of(ic);
    let mut out: Vec<u32> = Vec::with_capacity(n as usize);
    for e in elements(data, s.indices.byte_offset.0, size, size, n, what)? {
        let i = index_value(ic, e).ok_or_else(|| format!("{what}: sparse indices of {ic:?}"))?;
        if u64::from(i) >= count || out.last().is_some_and(|&last| i <= last) {
            return Err(format!("{what}: sparse index {i} out of order or not below {count}"));
        }
        out.push(i);
    }
    Ok(out)
}

pub fn read_indices(
    root: &Root,
    buffers: &[&[u8]],
    index: usize,
    vertices: u64,
    max_count: u64,
) -> Result<Vec<u32>, String> {
    let what = format!("accessors[{index}] (indices)");
    let a = root.accessors.get(index).ok_or_else(|| format!("{what} does not exist"))?;
    let (Checked::Valid(Type::Scalar), Checked::Valid(c)) = (&a.type_, &a.component_type) else {
        return Err(format!("{what}: not a scalar of a known component type"));
    };
    let c = c.0;
    if a.normalized || !matches!(c, ComponentType::U8 | ComponentType::U16 | ComponentType::U32) {
        return Err(format!("{what}: indices must be unsigned bytes, shorts or ints, not normalised"));
    }
    if a.sparse.is_some() {
        return Err(format!("{what}: sparse indices are not read"));
    }
    let count = a.count.0;
    if count == 0 || count > max_count {
        return Err(format!("{what}: {count} indices (1 to {max_count} are read)"));
    }
    let view = a.buffer_view.ok_or_else(|| format!("{what}: no buffer view"))?;
    let (data, stride) = view_bytes(root, buffers, view.value())?;
    if stride.is_some() {
        return Err(format!("{what}: an index view with a byteStride"));
    }
    let size = size_of(c);
    let offset = a.byte_offset.map_or(0, |o| o.0);
    let view_offset = root.buffer_views[view.value()].byte_offset.map_or(0, |o| o.0);
    if !offset.is_multiple_of(size as u64) || !view_offset.is_multiple_of(size as u64) {
        return Err(format!("{what}: an offset not aligned to its {size}-byte indices"));
    }
    let mut out = Vec::with_capacity(count as usize);
    for e in elements(data, offset, size, size, count, &what)? {
        let i = index_value(c, e).unwrap_or(u32::MAX);
        if u64::from(i) >= vertices {
            return Err(format!("{what}: index {i} of {vertices} vertices"));
        }
        out.push(i);
    }
    Ok(out)
}
