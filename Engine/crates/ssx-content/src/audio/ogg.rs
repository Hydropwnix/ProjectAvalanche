#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct OggInfo {
    pub channels: u16,
    pub rate: u32,
    pub frames: u64,
}

const HEADER: usize = 27;
const MAX_PACKET: usize = 1 << 20;
const BOS: u8 = 0x02;
const EOS: u8 = 0x04;
const CONTINUED: u8 = 0x01;

fn crc_table() -> &'static [u32; 256] {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut r = (i as u32) << 24;
            for _ in 0..8 {
                r = if r & 0x8000_0000 != 0 { (r << 1) ^ 0x04C1_1DB7 } else { r << 1 };
            }
            *e = r;
        }
        t
    })
}

pub(super) fn page_crc(page: &[u8]) -> u32 {
    let t = crc_table();
    let mut crc = 0u32;
    for (i, &byte) in page.iter().enumerate() {
        let b = if (22..26).contains(&i) { 0 } else { byte };
        crc = (crc << 8) ^ t[((crc >> 24) as u8 ^ b) as usize];
    }
    crc
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(super) fn walk(b: &[u8]) -> Result<OggInfo, String> {
    let mut at = 0usize;
    let mut serial = None;
    let mut sequence = 0u32;
    let mut last_granule: Option<u64> = None;
    let mut packet: Vec<u8> = Vec::new();
    let mut packets = 0usize;
    let mut id: Option<(u16, u32)> = None;
    let mut eos = false;
    while at < b.len() {
        if eos {
            return Err(format!("bytes after the last page (EOS) at {at}: a second stream or trailing data"));
        }
        if b.len() - at < HEADER || &b[at..at + 4] != b"OggS" {
            return Err(format!("no page at byte {at} (cut short, or bytes between pages)"));
        }
        if b[at + 4] != 0 {
            return Err(format!("page {sequence}: stream structure version {} (only 0)", b[at + 4]));
        }
        let flags = b[at + 5];
        if flags & !0x07 != 0 {
            return Err(format!("page {sequence}: unknown header flags {flags:#04x}"));
        }
        let granule = u64::from_le_bytes(b[at + 6..at + 14].try_into().unwrap_or([0; 8]));
        let page_serial = u32le(b, at + 14);
        let page_sequence = u32le(b, at + 18);
        let stated_crc = u32le(b, at + 22);
        let n = usize::from(b[at + 26]);
        if b.len() - at < HEADER + n {
            return Err(format!("page {sequence}: its segment table is cut short"));
        }
        let lacing = &b[at + HEADER..at + HEADER + n];
        let body_len: usize = lacing.iter().map(|&l| usize::from(l)).sum();
        let end = at + HEADER + n + body_len;
        if end > b.len() {
            return Err(format!(
                "page {sequence}: its body is cut short ({} of {body_len} bytes)",
                b.len() - at - HEADER - n
            ));
        }
        let page = &b[at..end];
        let crc = page_crc(page);
        if crc != stated_crc {
            return Err(format!("page {sequence} (byte {at}): CRC {crc:#010x}, the page states {stated_crc:#010x}"));
        }
        match serial {
            None => serial = Some(page_serial),
            Some(s) if s != page_serial => {
                return Err(format!(
                    "page {sequence}: a second logical stream (serial {page_serial:#x}, first {s:#x})"
                ));
            }
            _ => {}
        }
        if page_sequence != sequence {
            return Err(format!("page sequence {page_sequence} where {sequence} comes (a page lost or repeated)"));
        }
        let first = sequence == 0;
        if first != (flags & BOS != 0) {
            return Err(format!(
                "page {sequence}: the beginning-of-stream flag {}",
                if first { "missing" } else { "repeated" }
            ));
        }
        if flags & CONTINUED != 0 && packet.is_empty() {
            return Err(format!("page {sequence}: continues a packet that no page began"));
        }
        if flags & CONTINUED == 0 && !packet.is_empty() {
            return Err(format!("page {sequence}: a packet left unfinished by the page before"));
        }
        eos = flags & EOS != 0;
        let mut ended = false;
        let mut body = &page[HEADER + n..];
        for (s, &l) in lacing.iter().enumerate() {
            let (seg, rest) = body.split_at(usize::from(l));
            body = rest;
            if packet.len() + seg.len() > MAX_PACKET {
                return Err(format!("a packet over {MAX_PACKET} bytes (page {sequence})"));
            }
            packet.extend_from_slice(seg);
            if l < 255 {
                packets += 1;
                ended = true;
                header_packet(packets, &packet, &mut id, sequence)?;
                packet.clear();
                if packets == 3 && s + 1 < n {
                    return Err(format!("page {sequence}: audio in the setup header's page (it begins a fresh page)"));
                }
            }
        }
        if granule != u64::MAX {
            if !ended {
                return Err(format!("page {sequence}: a granule position on a page where no packet ends"));
            }
            if last_granule.is_some_and(|g| granule < g) {
                return Err(format!("page {sequence}: the granule position goes back"));
            }
            if packets > 3 || granule != 0 {
                last_granule = Some(granule);
            }
        }
        if first && packets != 1 {
            return Err("the first page holds more than the identification header".into());
        }
        sequence = sequence.checked_add(1).ok_or("more than 2^32 pages")?;
        at = end;
    }
    if !eos {
        return Err("no last page (EOS): the file is cut short".into());
    }
    if !packet.is_empty() {
        return Err("the last page leaves a packet unfinished".into());
    }
    if packets < 4 {
        return Err(format!("{packets} packets: no audio after the three Vorbis headers"));
    }
    let (channels, rate) = id.ok_or("no Vorbis identification header")?;
    let frames = last_granule.ok_or("no page states a granule position")?;
    Ok(OggInfo { channels, rate, frames })
}

fn header_packet(k: usize, p: &[u8], id: &mut Option<(u16, u32)>, page: u32) -> Result<(), String> {
    let want = match k {
        1 => 1,
        2 => 3,
        3 => 5,
        _ => {
            if p.first().is_some_and(|b| b & 1 != 0) {
                return Err(format!("page {page}: a header packet where audio comes (packet {k})"));
            }
            return Ok(());
        }
    };
    if p.len() < 7 || p[0] != want || &p[1..7] != b"vorbis" {
        return Err(format!("packet {k} is not the Vorbis {} header", ["identification", "comment", "setup"][k - 1]));
    }
    if k == 1 {
        if p.len() != 30 {
            return Err(format!("an identification header of {} bytes (30)", p.len()));
        }
        let version = u32le(p, 7);
        let (channels, rate) = (u16::from(p[11]), u32le(p, 12));
        let (bs0, bs1) = (p[28] & 0xF, p[28] >> 4);
        if version != 0 {
            return Err(format!("Vorbis version {version} (only 0)"));
        }
        if channels == 0 || rate == 0 {
            return Err(format!("{channels} channels at {rate} Hz"));
        }
        if !(6..=13).contains(&bs0) || !(6..=13).contains(&bs1) || bs0 > bs1 {
            return Err(format!("block sizes 2^{bs0} / 2^{bs1} (from 2^6 to 2^13, the short not above the long)"));
        }
        if p[29] & 1 == 0 {
            return Err("the identification header's framing bit is not set".into());
        }
        *id = Some((channels, rate));
    }
    Ok(())
}
