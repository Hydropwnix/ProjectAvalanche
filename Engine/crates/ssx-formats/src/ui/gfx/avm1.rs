use super::bits::SwfReader;
use crate::{Error, Result};

#[derive(Clone, Debug, PartialEq)]
pub enum PushValue {
    String(String),
    Float(f32),
    Null,
    Undefined,
    Register(u8),
    Bool(bool),
    Double(f64),
    Int(i32),
    Constant(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum ActionData {
    None,
    GotoFrame(u16),
    GetUrl {
        url: String,
        target: String,
    },
    StoreRegister(u8),
    ConstantPool(Vec<String>),
    WaitForFrame {
        frame: u16,
        skip: u8,
    },
    SetTarget(String),
    GotoLabel(String),
    WaitForFrame2 {
        skip: u8,
    },
    DefineFunction2 {
        name: String,
        register_count: u8,
        flags: u16,
        params: Vec<(u8, String)>,
        code_size: u16,
    },
    Try {
        flags: u8,
        try_size: u16,
        catch_size: u16,
        finally_size: u16,
        catch_name: Option<String>,
        catch_register: Option<u8>,
    },
    With {
        size: u16,
    },
    Push(Vec<PushValue>),
    Jump(i16),
    If(i16),
    GetUrl2(u8),
    DefineFunction {
        name: String,
        params: Vec<String>,
        code_size: u16,
    },
    GotoFrame2 {
        flags: u8,
        scene_bias: Option<u16>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Action {
    pub offset: u32,
    pub op: u8,
    pub data: ActionData,
}

impl Action {
    pub fn end(&self, len: u32) -> u32 {
        self.offset + len
    }
}

pub fn op_name(op: u8) -> Option<&'static str> {
    Some(match op {
        0x00 => "End",
        0x04 => "NextFrame",
        0x05 => "PreviousFrame",
        0x06 => "Play",
        0x07 => "Stop",
        0x08 => "ToggleQuality",
        0x09 => "StopSounds",
        0x0A => "Add",
        0x0B => "Subtract",
        0x0C => "Multiply",
        0x0D => "Divide",
        0x0E => "Equals",
        0x0F => "Less",
        0x10 => "And",
        0x11 => "Or",
        0x12 => "Not",
        0x13 => "StringEquals",
        0x14 => "StringLength",
        0x15 => "StringExtract",
        0x17 => "Pop",
        0x18 => "ToInteger",
        0x1C => "GetVariable",
        0x1D => "SetVariable",
        0x20 => "SetTarget2",
        0x21 => "StringAdd",
        0x22 => "GetProperty",
        0x23 => "SetProperty",
        0x24 => "CloneSprite",
        0x25 => "RemoveSprite",
        0x26 => "Trace",
        0x27 => "StartDrag",
        0x28 => "EndDrag",
        0x29 => "StringLess",
        0x2A => "Throw",
        0x2B => "CastOp",
        0x2C => "ImplementsOp",
        0x2D => "FSCommand2",
        0x30 => "RandomNumber",
        0x31 => "MBStringLength",
        0x32 => "CharToAscii",
        0x33 => "AsciiToChar",
        0x34 => "GetTime",
        0x35 => "MBStringExtract",
        0x36 => "MBCharToAscii",
        0x37 => "MBAsciiToChar",
        0x3A => "Delete",
        0x3B => "Delete2",
        0x3C => "DefineLocal",
        0x3D => "CallFunction",
        0x3E => "Return",
        0x3F => "Modulo",
        0x40 => "NewObject",
        0x41 => "DefineLocal2",
        0x42 => "InitArray",
        0x43 => "InitObject",
        0x44 => "TypeOf",
        0x45 => "TargetPath",
        0x46 => "Enumerate",
        0x47 => "Add2",
        0x48 => "Less2",
        0x49 => "Equals2",
        0x4A => "ToNumber",
        0x4B => "ToString",
        0x4C => "PushDuplicate",
        0x4D => "StackSwap",
        0x4E => "GetMember",
        0x4F => "SetMember",
        0x50 => "Increment",
        0x51 => "Decrement",
        0x52 => "CallMethod",
        0x53 => "NewMethod",
        0x54 => "InstanceOf",
        0x55 => "Enumerate2",
        0x60 => "BitAnd",
        0x61 => "BitOr",
        0x62 => "BitXor",
        0x63 => "BitLShift",
        0x64 => "BitRShift",
        0x65 => "BitURShift",
        0x66 => "StrictEquals",
        0x67 => "Greater",
        0x68 => "StringGreater",
        0x69 => "Extends",
        0x81 => "GotoFrame",
        0x83 => "GetURL",
        0x87 => "StoreRegister",
        0x88 => "ConstantPool",
        0x8A => "WaitForFrame",
        0x8B => "SetTarget",
        0x8C => "GoToLabel",
        0x8D => "WaitForFrame2",
        0x8E => "DefineFunction2",
        0x8F => "Try",
        0x94 => "With",
        0x96 => "Push",
        0x99 => "Jump",
        0x9A => "GetURL2",
        0x9B => "DefineFunction",
        0x9D => "If",
        0x9E => "Call",
        0x9F => "GotoFrame2",
        _ => return None,
    })
}

fn read_data(op: u8, body: &[u8]) -> Result<ActionData> {
    let mut r = SwfReader::new(body);
    let data = match op {
        0x81 => ActionData::GotoFrame(r.u16()?),
        0x83 => ActionData::GetUrl { url: r.string()?, target: r.string()? },
        0x87 => ActionData::StoreRegister(r.u8()?),
        0x88 => {
            let n = r.u16()?;
            ActionData::ConstantPool((0..n).map(|_| r.string()).collect::<Result<_>>()?)
        }
        0x8A => ActionData::WaitForFrame { frame: r.u16()?, skip: r.u8()? },
        0x8B => ActionData::SetTarget(r.string()?),
        0x8C => ActionData::GotoLabel(r.string()?),
        0x8D => ActionData::WaitForFrame2 { skip: r.u8()? },
        0x8E => {
            let name = r.string()?;
            let n = r.u16()?;
            let register_count = r.u8()?;
            let hi = r.u8()?;
            let lo = r.u8()?;
            let flags = u16::from(hi) << 8 | u16::from(lo);
            let params = (0..n).map(|_| Ok((r.u8()?, r.string()?))).collect::<Result<_>>()?;
            ActionData::DefineFunction2 { name, register_count, flags, params, code_size: r.u16()? }
        }
        0x8F => {
            let flags = r.u8()?;
            let (try_size, catch_size, finally_size) = (r.u16()?, r.u16()?, r.u16()?);
            let (catch_name, catch_register) =
                if flags & 0x04 != 0 { (None, Some(r.u8()?)) } else { (Some(r.string()?), None) };
            ActionData::Try { flags, try_size, catch_size, finally_size, catch_name, catch_register }
        }
        0x94 => ActionData::With { size: r.u16()? },
        0x96 => {
            let mut values = Vec::new();
            while !r.at_end() {
                let t = r.u8()?;
                values.push(match t {
                    0 => PushValue::String(r.string()?),
                    1 => PushValue::Float(r.f32()?),
                    2 => PushValue::Null,
                    3 => PushValue::Undefined,
                    4 => PushValue::Register(r.u8()?),
                    5 => PushValue::Bool(r.u8()? != 0),
                    6 => PushValue::Double(r.f64_swapped()?),
                    7 => PushValue::Int(r.i32()?),
                    8 => PushValue::Constant(u16::from(r.u8()?)),
                    9 => PushValue::Constant(r.u16()?),
                    _ => return Err(Error::invalid(format!("push type {t}"))),
                });
            }
            ActionData::Push(values)
        }
        0x99 => ActionData::Jump(r.i16()?),
        0x9A => ActionData::GetUrl2(r.u8()?),
        0x9B => {
            let name = r.string()?;
            let n = r.u16()?;
            let params = (0..n).map(|_| r.string()).collect::<Result<_>>()?;
            ActionData::DefineFunction { name, params, code_size: r.u16()? }
        }
        0x9D => ActionData::If(r.i16()?),
        0x9F => {
            let flags = r.u8()?;
            let scene_bias = if flags & 0x02 != 0 { Some(r.u16()?) } else { None };
            ActionData::GotoFrame2 { flags, scene_bias }
        }
        _ => ActionData::None,
    };
    if !r.at_end() {
        return Err(Error::invalid(format!("action {op:#04x}: {} bytes left in its record", r.remaining())));
    }
    Ok(data)
}

pub fn decode(code: &[u8]) -> Result<Vec<Action>> {
    let mut r = SwfReader::new(code);
    let mut out = Vec::new();
    while !r.at_end() {
        let offset = r.pos() as u32;
        let op = r.u8()?;
        if op_name(op).is_none() {
            return Err(Error::invalid(format!("unknown action code {op:#04x} at {offset:#x}")));
        }
        let data = if op >= 0x80 {
            let n = usize::from(r.u16()?);
            read_data(op, r.bytes(n)?).map_err(|e| Error::invalid(format!("action {op:#04x} at {offset:#x}: {e}")))?
        } else {
            ActionData::None
        };
        out.push(Action { offset, op, data });
    }
    Ok(out)
}

fn boundaries(actions: &[Action], len: u32) -> Vec<u32> {
    let mut b: Vec<u32> = actions.iter().map(|a| a.offset).collect();
    b.push(len);
    b
}

pub fn branch_target(next: u32, delta: i16, bounds: &[u32]) -> Option<(u32, bool)> {
    let on = |t: i64| t >= 0 && bounds.binary_search(&(t as u32)).is_ok();
    let t = i64::from(next) + i64::from(delta);
    if on(t) {
        return Some((t as u32, false));
    }
    let w = i64::from(next) + i64::from(delta as u16);
    on(w).then_some((w as u32, true))
}

pub fn action_boundaries(actions: &[Action], len: u32) -> Vec<u32> {
    boundaries(actions, len)
}

pub fn validate(actions: &[Action], len: u32) -> Result<usize> {
    let mut wrapped = 0;
    let bounds = boundaries(actions, len);
    let on_boundary = |at: i64| at >= 0 && bounds.binary_search(&(at as u32)).is_ok();
    match actions.last() {
        Some(a) if a.op == 0 && a.offset + 1 == len => {}
        _ => return Err(Error::invalid("block does not end with End")),
    }
    let mut scopes: Vec<(u32, u32)> = vec![(len, 4)];
    let mut pool: Option<usize> = None;
    for (i, a) in actions.iter().enumerate() {
        while scopes.len() > 1 && a.offset >= scopes.last().map_or(len, |s| s.0) {
            scopes.pop();
        }
        let next = bounds[i + 1];
        let (scope_end, regs) = *scopes.last().expect("root scope");
        let fail = |what: &str| Err(Error::invalid(format!("action {:#04x} at {:#x}: {what}", a.op, a.offset)));
        match &a.data {
            ActionData::Jump(d) | ActionData::If(d) => match branch_target(next, *d, &bounds) {
                Some((t, w)) if t <= scope_end => wrapped += usize::from(w),
                _ => return fail("branch target off an action boundary"),
            },
            ActionData::DefineFunction { code_size, .. } | ActionData::DefineFunction2 { code_size, .. } => {
                let end = next + u32::from(*code_size);
                if !on_boundary(i64::from(end)) || end > scope_end {
                    return fail("function body off an action boundary");
                }
                let limit = match &a.data {
                    ActionData::DefineFunction2 { register_count, .. } => u32::from(*register_count),
                    _ => 4,
                };
                if let ActionData::DefineFunction2 { params, .. } = &a.data
                    && params.iter().any(|(reg, _)| u32::from(*reg) >= limit.max(1))
                {
                    return fail("parameter register out of range");
                }
                scopes.push((end, limit));
            }
            ActionData::Try { try_size, catch_size, finally_size, .. } => {
                let mut at = next;
                for s in [try_size, catch_size, finally_size] {
                    at += u32::from(*s);
                    if !on_boundary(i64::from(at)) || at > scope_end {
                        return fail("try block off an action boundary");
                    }
                }
            }
            ActionData::With { size } => {
                let end = next + u32::from(*size);
                if !on_boundary(i64::from(end)) || end > scope_end {
                    return fail("with block off an action boundary");
                }
            }
            ActionData::ConstantPool(p) => pool = Some(p.len()),
            ActionData::StoreRegister(r) if u32::from(*r) >= regs => return fail("register out of range"),
            ActionData::Push(values) => {
                for v in values {
                    match v {
                        PushValue::Constant(c) if pool.is_none_or(|n| usize::from(*c) >= n) => {
                            return fail("constant outside the pool");
                        }
                        PushValue::Register(r) if u32::from(*r) >= regs => return fail("register out of range"),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(wrapped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_push_and_pool() {
        let code =
            [0x88, 6, 0, 2, 0, b'a', 0, b'b', 0, 0x96, 9, 0, 8, 1, 7, 7, 0, 0, 0, 4, 0, 0x1C, 0x99, 2, 0, 0, 0, 0x00];
        let a = decode(&code).unwrap();
        assert_eq!(a.len(), 5);
        assert_eq!(a[0].data, ActionData::ConstantPool(vec!["a".into(), "b".into()]));
        assert_eq!(
            a[1].data,
            ActionData::Push(vec![PushValue::Constant(1), PushValue::Int(7), PushValue::Register(0)])
        );
        validate(&a, code.len() as u32).unwrap();
    }

    #[test]
    fn rejects_bad_structure() {
        let code = [0x99, 2, 0, 0xFE, 0xFF, 0x00];
        let a = decode(&code).unwrap();
        assert!(validate(&a, code.len() as u32).is_err());
        assert!(decode(&[0x01, 0x00]).is_err());
        assert!(decode(&[0x87, 2, 0, 1, 2, 0x00]).is_err());
        let code = [0x96, 2, 0, 8, 0, 0x00];
        assert!(validate(&decode(&code).unwrap(), code.len() as u32).is_err());
    }

    #[test]
    fn long_branch_wraps() {
        let mut code = vec![0x9D, 2, 0, 0x03, 0x80];
        code.extend(std::iter::repeat_n(0x07u8, 0x8003));
        code.push(0x00);
        let a = decode(&code).unwrap();
        assert_eq!(validate(&a, code.len() as u32).unwrap(), 1);
    }

    #[test]
    fn function2_scopes_registers() {
        let body = [0x96, 2, 0, 4, 2, 0x3E];
        let mut code = vec![0x8E, 9, 0, b'f', 0, 0, 0, 3, 0, 0];
        code.extend_from_slice(&(body.len() as u16).to_le_bytes());
        code.extend_from_slice(&body);
        code.push(0x00);
        let a = decode(&code).unwrap();
        validate(&a, code.len() as u32).unwrap();
        let code = [0x96, 2, 0, 4, 5, 0x00];
        assert!(validate(&decode(&code).unwrap(), code.len() as u32).is_err());
    }
}
