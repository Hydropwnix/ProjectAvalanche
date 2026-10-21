use crate::reader::Reader;
use crate::{Error, Result};

use super::container::{PtrTarget, Vault};
use super::fnv1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueKind {
    Bool,
    Int8,
    UInt8,
    Int16,
    UInt16,
    Int32,
    UInt32,
    Int64,
    UInt64,
    Float,
    Vector2,
    Vector3,
    Vector4,
    Text,
    Key,
    RefSpec,
    ClassRefSpec(u32),
    Raw,
}

impl ValueKind {
    pub fn from_type(name: &str, size: u32) -> Result<Self> {
        use ValueKind::*;
        let (kind, want) = match name {
            "EA::Reflection::Bool" => (Bool, 1),
            "EA::Reflection::Int8" => (Int8, 1),
            "EA::Reflection::UInt8" => (UInt8, 1),
            "EA::Reflection::Int16" => (Int16, 2),
            "EA::Reflection::UInt16" => (UInt16, 2),
            "EA::Reflection::Int32" => (Int32, 4),
            "EA::Reflection::UInt32" => (UInt32, 4),
            "EA::Reflection::Int64" => (Int64, 8),
            "EA::Reflection::UInt64" => (UInt64, 8),
            "EA::Reflection::Float" => (Float, 4),
            "Attrib::Types::Vector2" => (Vector2, 8),
            "Attrib::Types::Vector3" => (Vector3, 12),
            "Attrib::Types::Vector4" => (Vector4, 16),
            "EA::Reflection::Text" => (Text, 4),
            "Attrib::Key" => (Key, 4),
            "Attrib::RefSpec" => (RefSpec, 12),
            _ => match name.strip_prefix("Attrib::Gen::ClassRefSpec_") {
                Some(class) => (ClassRefSpec(fnv1(class.as_bytes())), 8),
                None => return Ok(Raw),
            },
        };
        if size != want {
            return Err(Error::invalid(format!("type {name}: size {size}, expected {want}")));
        }
        Ok(kind)
    }

    pub fn is_reference(self) -> bool {
        matches!(self, ValueKind::RefSpec | ValueKind::ClassRefSpec(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RefSpec {
    pub class: u32,
    pub collection: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawPointer {
    pub offset: u32,
    pub target: PtrTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Vector2([f32; 2]),
    Vector3([f32; 3]),
    Vector4([f32; 4]),
    Text(Option<String>),
    Key(u32),
    Ref(RefSpec),
    Raw { bytes: Vec<u8>, pointers: Vec<RawPointer> },
}

pub fn decode(vault: &Vault, kind: ValueKind, size: u32, at: PtrTarget, slots: &mut Vec<PtrTarget>) -> Result<Value> {
    let data = vault.file(at.file);
    let start = at.offset as usize;
    let bytes = data.get(start..start + size as usize).ok_or(Error::UnexpectedEof {
        offset: start,
        wanted: size as usize,
        available: data.len().saturating_sub(start),
    })?;
    let pointers: Vec<RawPointer> = vault.fixups[at.file as usize]
        .range(at.offset..at.offset + size)
        .map(|(&slot, &target)| RawPointer { offset: slot - at.offset, target })
        .collect();
    let mut r = Reader::be(bytes);
    let value = match kind {
        ValueKind::Text => match pointers.first() {
            Some(p) if p.offset == 0 => {
                slots.push(at);
                Value::Text(Some(vault.cstr(p.target)?))
            }
            None if bytes == [0; 4] => Value::Text(None),
            _ => return Err(Error::invalid(format!("Text at {at:?}: neither a pointer nor null"))),
        },
        ValueKind::Raw => {
            slots.extend(pointers.iter().map(|p| PtrTarget { file: at.file, offset: at.offset + p.offset }));
            return Ok(Value::Raw { bytes: bytes.to_vec(), pointers });
        }
        _ if !pointers.is_empty() => {
            return Err(Error::invalid(format!("{kind:?} value at {at:?} contains a pointer")));
        }
        ValueKind::Bool => match r.u8()? {
            b @ (0 | 1) => Value::Bool(b == 1),
            b => return Err(Error::invalid(format!("Bool at {at:?} is {b:#x}"))),
        },
        ValueKind::Int8 => Value::Int8(r.i8()?),
        ValueKind::UInt8 => Value::UInt8(r.u8()?),
        ValueKind::Int16 => Value::Int16(r.i16()?),
        ValueKind::UInt16 => Value::UInt16(r.u16()?),
        ValueKind::Int32 => Value::Int32(r.i32()?),
        ValueKind::UInt32 => Value::UInt32(r.u32()?),
        ValueKind::Int64 => Value::Int64(r.u64()? as i64),
        ValueKind::UInt64 => Value::UInt64(r.u64()?),
        ValueKind::Float => Value::Float(r.f32()?),
        ValueKind::Vector2 => Value::Vector2([r.f32()?, r.f32()?]),
        ValueKind::Vector3 => Value::Vector3(r.vec3()?),
        ValueKind::Vector4 => Value::Vector4([r.f32()?, r.f32()?, r.f32()?, r.f32()?]),
        ValueKind::Key => Value::Key(r.u32()?),
        ValueKind::RefSpec => {
            let (class, collection, unk) = (r.u32()?, r.u32()?, r.u32()?);
            if unk != 0 {
                return Err(Error::invalid(format!("RefSpec at {at:?}: third word {unk:#x} is not 0")));
            }
            Value::Ref(RefSpec { class, collection })
        }
        ValueKind::ClassRefSpec(class) => {
            let (collection, unk) = (r.u32()?, r.u32()?);
            if unk != 0 {
                return Err(Error::invalid(format!("ClassRefSpec at {at:?}: second word {unk:#x} is not 0")));
            }
            Value::Ref(RefSpec { class, collection })
        }
    };
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_types() {
        assert_eq!(ValueKind::from_type("EA::Reflection::Float", 4).unwrap(), ValueKind::Float);
        assert_eq!(
            ValueKind::from_type("Attrib::Gen::ClassRefSpec_ge_texture", 8).unwrap(),
            ValueKind::ClassRefSpec(fnv1(b"ge_texture"))
        );
        assert_eq!(ValueKind::from_type("SSXInstanceData", 64).unwrap(), ValueKind::Raw);
        assert!(ValueKind::from_type("Attrib::RefSpec", 8).is_err());
    }
}
