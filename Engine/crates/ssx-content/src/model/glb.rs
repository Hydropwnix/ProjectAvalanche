const MAGIC: u32 = 0x4654_6C67;
const VERSION: u32 = 2;
const CHUNK_JSON: u32 = 0x4E4F_534A;
const CHUNK_BIN: u32 = 0x004E_4942;

pub struct Glb<'a> {
    pub json: &'a [u8],
    pub bin: Option<&'a [u8]>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub json: std::ops::Range<u64>,
    pub bin: Option<std::ops::Range<u64>>,
}

fn u32_at(data: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(data.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

pub fn layout(
    size: u64,
    read: &mut dyn FnMut(u64, usize) -> Result<Vec<u8>, String>,
    max_json: u64,
) -> Result<Layout, String> {
    let header = if size >= 12 { read(0, 12)? } else { Vec::new() };
    if header.len() < 12 || u32_at(&header, 0) != Some(MAGIC) {
        return Err("not a binary glTF (no \"glTF\" header)".into());
    }
    let version = u32_at(&header, 4).unwrap_or(0);
    if version != VERSION {
        return Err(format!("binary glTF version {version}; only version 2 is read"));
    }
    let stated = u64::from(u32_at(&header, 8).unwrap_or(0));
    if stated != size {
        return Err(format!("the header states {stated} bytes, the file has {size}"));
    }
    let mut at = 12u64;
    let mut json = None;
    let mut bin = None;
    while at < size {
        let head = if size - at >= 8 { read(at, 8)? } else { Vec::new() };
        let (Some(len), Some(kind)) = (u32_at(&head, 0), u32_at(&head, 4)) else {
            return Err(format!("a chunk header at byte {at} is cut short"));
        };
        let len = u64::from(len);
        if !len.is_multiple_of(4) {
            return Err(format!("chunk at byte {at}: its length {len} is not a multiple of 4"));
        }
        let start = at + 8;
        let Some(end) = start.checked_add(len).filter(|&e| e <= size) else {
            return Err(format!("chunk at byte {at}: {len} bytes stated, {} left", size - start.min(size)));
        };
        match kind {
            CHUNK_JSON if json.is_none() && bin.is_none() => {
                if len > max_json {
                    return Err(format!("the JSON chunk has {len} bytes; at most {max_json} are read"));
                }
                json = Some(start..end);
            }
            CHUNK_JSON => return Err("a second JSON chunk".into()),
            CHUNK_BIN if json.is_none() => return Err("the BIN chunk comes before the JSON chunk".into()),
            CHUNK_BIN if bin.is_some() => return Err("a second BIN chunk".into()),
            CHUNK_BIN => bin = Some(start..end),
            other => return Err(format!("a chunk of type 0x{other:08x} at byte {at} (only JSON and BIN are read)")),
        }
        at = end;
    }
    let json = json.ok_or("no JSON chunk")?;
    Ok(Layout { json, bin })
}

pub fn split(data: &[u8], max_json: u64) -> Result<Glb<'_>, String> {
    let mut read = |at: u64, len: usize| -> Result<Vec<u8>, String> {
        Ok(data.get(at as usize..at as usize + len).map(<[u8]>::to_vec).unwrap_or_default())
    };
    let l = layout(data.len() as u64, &mut read, max_json)?;
    let slice = |r: &std::ops::Range<u64>| &data[r.start as usize..r.end as usize];
    Ok(Glb { json: slice(&l.json), bin: l.bin.as_ref().map(slice) })
}

pub fn assemble(json: &[u8], bin: Option<&[u8]>) -> Vec<u8> {
    let pad = |n: usize| (4 - n % 4) % 4;
    let json_len = json.len() + pad(json.len());
    let bin_len = bin.map(|b| b.len() + pad(b.len()));
    let total = 12 + 8 + json_len + bin_len.map_or(0, |l| 8 + l);
    let mut out = Vec::with_capacity(total);
    for w in [MAGIC, VERSION, total as u32, json_len as u32, CHUNK_JSON] {
        out.extend(w.to_le_bytes());
    }
    out.extend(json);
    out.resize(out.len() + pad(json.len()), b' ');
    if let (Some(b), Some(len)) = (bin, bin_len) {
        out.extend((len as u32).to_le_bytes());
        out.extend(CHUNK_BIN.to_le_bytes());
        out.extend(b);
        out.resize(out.len() + pad(b.len()), 0);
    }
    out
}
