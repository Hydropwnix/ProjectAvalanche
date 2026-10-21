mod program;

use crate::ant::{Object, Schema, Struct, Value};
use crate::{Error, Result};

pub use program::*;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BinaryMath {
    pub position: u32,
    pub kind: u32,
    pub a: i16,
    pub b: i16,
    pub result: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointIo {
    pub position: u32,
    pub connectivity: u32,
    pub joint_index: u32,
    pub sqt: i16,
    pub scale: i16,
    pub quaternion: i16,
    pub translation: i16,
    pub local_space: bool,
    pub reader: bool,
    pub virtual_joint: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LengthRatio {
    pub position: u32,
    pub unk_connection_cardinality: u32,
    pub start_position: i16,
    pub position1: i16,
    pub position2: i16,
    pub start_offset: i16,
    pub offset1: i16,
    pub offset2: i16,
    pub result: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Magnitude {
    pub position: u32,
    pub vector: i16,
    pub result: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aim {
    pub position: u32,
    pub unk_reference: i16,
    pub joint_index: u32,
    pub target: i16,
    pub up_joint: i16,
    pub target_offset: i16,
    pub twist_amount: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetDrivenKey {
    pub position: u32,
    pub driver: i16,
    pub driven: i16,
    pub domain_a: i16,
    pub domain_mid: i16,
    pub domain_b: i16,
    pub range_a: i16,
    pub range_mid: i16,
    pub range_b: i16,
    pub power: i32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VectorBuildSplit {
    pub position: u32,
    pub vector: i16,
    pub x: i16,
    pub y: i16,
    pub z: i16,
    pub connections: i32,
    pub build: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Twist {
    pub position: u32,
    pub reference: i16,
    pub unk_joint_index: u32,
    pub amount: i16,
    pub driver: i16,
    pub gimbal: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointConstraint {
    pub position: u32,
    pub targets: [i16; 3],
    pub weights: [f32; 3],
    pub result: i16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Conditional {
    pub position: u32,
    pub a: i16,
    pub b: i16,
    pub result: i16,
    pub when_true: i16,
    pub when_false: i16,
    pub operation: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Psc {
    pub position: u32,
    pub root: i16,
    pub reference: i16,
    pub unk_joint_index: u32,
    pub unk_history: i16,
    pub num_bounces: u32,
    pub gravity: [f32; 3],
    pub sum_scalar: f32,
    pub bounce_scalars: Vec<f32>,
    pub bounce_delays: Vec<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VirtualJoint {
    pub index: i32,
    pub parent_index: i32,
    pub offset: [f32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofIo {
    pub position: u32,
    pub value: i16,
    pub index: u32,
    pub reader: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DofValue {
    pub dof_set_index: u32,
    pub dof_index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Debug {
    pub position: u32,
    pub sqt: bool,
    pub data: i16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rigamate {
    pub name: String,
    pub entry_count: u32,
    pub operation_positions: Vec<u32>,
    pub strings: Vec<String>,
    pub unk_strings_total_length: u32,
    pub vectors: Vec<(u32, [f32; 3])>,
    pub scalars: Vec<(u32, f32)>,
    pub binary_math: Vec<BinaryMath>,
    pub joint_io: Vec<JointIo>,
    pub length_ratio: Vec<LengthRatio>,
    pub magnitude: Vec<Magnitude>,
    pub aim: Vec<Aim>,
    pub set_driven_key: Vec<SetDrivenKey>,
    pub vector_build_split: Vec<VectorBuildSplit>,
    pub twist: Vec<Twist>,
    pub point_constraint: Vec<PointConstraint>,
    pub conditional: Vec<Conditional>,
    pub psc: Vec<Psc>,
    pub virtual_joints: Vec<VirtualJoint>,
    pub dof_io: Vec<DofIo>,
    pub dof_values: Vec<DofValue>,
    pub debug: Vec<Debug>,
    pub windows: Vec<i16>,
}

struct El<'a> {
    schema: &'a Schema,
    s: &'a Struct,
    class: &'static str,
}

impl El<'_> {
    fn v(&self, name: &str) -> Result<&Value> {
        self.s.own(self.schema, name).ok_or_else(|| Error::invalid(format!("{}: no field {name}", self.class)))
    }
    fn int(&self, name: &str) -> Result<i64> {
        self.v(name)?.as_i64().ok_or_else(|| Error::invalid(format!("{}: {name} is not an integer", self.class)))
    }
    fn i16(&self, name: &str) -> Result<i16> {
        i16::try_from(self.int(name)?).map_err(|_| Error::invalid(format!("{}: {name} out of range", self.class)))
    }
    fn u32(&self, name: &str) -> Result<u32> {
        u32::try_from(self.int(name)?).map_err(|_| Error::invalid(format!("{}: {name} out of range", self.class)))
    }
    fn i32(&self, name: &str) -> Result<i32> {
        i32::try_from(self.int(name)?).map_err(|_| Error::invalid(format!("{}: {name} out of range", self.class)))
    }
    fn f32(&self, name: &str) -> Result<f32> {
        self.v(name)?.as_f32().ok_or_else(|| Error::invalid(format!("{}: {name} is not a float", self.class)))
    }
    fn bool(&self, name: &str) -> Result<bool> {
        self.v(name)?.as_bool().ok_or_else(|| Error::invalid(format!("{}: {name} is not a bool", self.class)))
    }
    fn vec3(&self, name: &str) -> Result<[f32; 3]> {
        self.v(name)?.as_vec3().ok_or_else(|| Error::invalid(format!("{}: {name} is not a Vector3", self.class)))
    }
    fn array(&self, name: &str) -> Result<&[Value]> {
        self.v(name)?.as_array().ok_or_else(|| Error::invalid(format!("{}: {name} is not an array", self.class)))
    }
}

fn list<'a>(schema: &'a Schema, obj: &'a Object, field: &str, class: &'static str) -> Result<Vec<El<'a>>> {
    let v = obj.get(schema, field).ok_or_else(|| Error::invalid(format!("RigamateAsset: no field {field}")))?;
    let items = v.as_array().ok_or_else(|| Error::invalid(format!("RigamateAsset: {field} is not an array")))?;
    items
        .iter()
        .map(|x| {
            x.as_struct()
                .map(|s| El { schema, s, class })
                .ok_or_else(|| Error::invalid(format!("RigamateAsset: {field} element is not a struct")))
        })
        .collect()
}

impl Rigamate {
    pub const CLASS: &'static str = "RigamateAsset";

    pub fn parse(schema: &Schema, obj: &Object) -> Result<Rigamate> {
        let name = obj.name(schema).unwrap_or_default().to_string();
        let top = |f: &str| obj.get(schema, f).ok_or_else(|| Error::invalid(format!("RigamateAsset: no field {f}")));
        let int = |f: &str| top(f)?.as_i64().ok_or_else(|| Error::invalid(format!("RigamateAsset: {f}")));
        let entry_count = u32::try_from(int("EntryCount")?).map_err(|_| Error::invalid("RigamateAsset: EntryCount"))?;
        let ints = |f: &str| -> Result<Vec<i64>> {
            top(f)?
                .as_array()
                .ok_or_else(|| Error::invalid(format!("RigamateAsset: {f}")))?
                .iter()
                .map(|v| v.as_i64().ok_or_else(|| Error::invalid(format!("RigamateAsset: {f} element"))))
                .collect()
        };
        let operation_positions = ints("OperationPositions")?
            .into_iter()
            .map(|x| u32::try_from(x).map_err(|_| Error::invalid("RigamateAsset: operation position")))
            .collect::<Result<_>>()?;
        let windows = ints("Windows")?
            .into_iter()
            .map(|x| i16::try_from(x).map_err(|_| Error::invalid("RigamateAsset: window")))
            .collect::<Result<_>>()?;
        let table = top("StringTable")?.as_struct().ok_or_else(|| Error::invalid("RigamateAsset: StringTable"))?;
        let table = El { schema, s: table, class: "RigamateStringTable" };
        let strings = table
            .array("Strings")?
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| Error::invalid("RigamateStringTable: string")))
            .collect::<Result<_>>()?;
        let unk_strings_total_length = table.u32("TotalLength")?;
        let mut r = Rigamate {
            name,
            entry_count,
            operation_positions,
            strings,
            unk_strings_total_length,
            vectors: Vec::new(),
            scalars: Vec::new(),
            binary_math: Vec::new(),
            joint_io: Vec::new(),
            length_ratio: Vec::new(),
            magnitude: Vec::new(),
            aim: Vec::new(),
            set_driven_key: Vec::new(),
            vector_build_split: Vec::new(),
            twist: Vec::new(),
            point_constraint: Vec::new(),
            conditional: Vec::new(),
            psc: Vec::new(),
            virtual_joints: Vec::new(),
            dof_io: Vec::new(),
            dof_values: Vec::new(),
            debug: Vec::new(),
            windows,
        };
        for e in list(schema, obj, "VectorList", "RigamateVector")? {
            r.vectors.push((e.u32("Position")?, e.vec3("Value")?));
        }
        for e in list(schema, obj, "ScalarList", "RigamateScalar")? {
            r.scalars.push((e.u32("Position")?, e.f32("Value")?));
        }
        for e in list(schema, obj, "BinaryMathList", "RigamateBinaryMath")? {
            let (position, kind) = (e.u32("Position")?, e.u32("Type")?);
            r.binary_math.push(BinaryMath { position, kind, a: e.i16("A")?, b: e.i16("B")?, result: e.i16("Result")? });
        }
        for e in list(schema, obj, "JointIOList", "RigamateJointIO")? {
            r.joint_io.push(JointIo {
                position: e.u32("Position")?,
                connectivity: e.u32("Connectivity")?,
                joint_index: e.u32("JointIndex")?,
                sqt: e.i16("SQT")?,
                scale: e.i16("Scale")?,
                quaternion: e.i16("Quaternion")?,
                translation: e.i16("Translation")?,
                local_space: e.bool("LocalSpace")?,
                reader: e.bool("Reader")?,
                virtual_joint: e.u32("VirtualJoint")?,
            });
        }
        r.parse_more(schema, obj)?;
        Ok(r)
    }

    fn parse_more(&mut self, schema: &Schema, obj: &Object) -> Result<()> {
        for e in list(schema, obj, "LengthRatioList", "RigamateLengthRatio")? {
            self.length_ratio.push(LengthRatio {
                position: e.u32("Position")?,
                unk_connection_cardinality: e.u32("ConnectionCardinality")?,
                start_position: e.i16("StartPosition")?,
                position1: e.i16("Position1")?,
                position2: e.i16("Position2")?,
                start_offset: e.i16("StartOffset")?,
                offset1: e.i16("Offset1")?,
                offset2: e.i16("Offset2")?,
                result: e.i16("Result")?,
            });
        }
        for e in list(schema, obj, "MagnitudeList", "RigamateMagnitude")? {
            let position = e.u32("Position")?;
            self.magnitude.push(Magnitude { position, vector: e.i16("Vector")?, result: e.i16("Result")? });
        }
        for e in list(schema, obj, "AimList", "RigamateAim")? {
            self.aim.push(Aim {
                position: e.u32("Position")?,
                unk_reference: e.i16("Reference")?,
                joint_index: e.u32("JointIndex")?,
                target: e.i16("Target")?,
                up_joint: e.i16("UpJoint")?,
                target_offset: e.i16("TargetOffset")?,
                twist_amount: e.i16("TwistAmount")?,
            });
        }
        for e in list(schema, obj, "SetDrivenKeyList", "RigamateSetDrivenKey")? {
            self.set_driven_key.push(SetDrivenKey {
                position: e.u32("Position")?,
                driver: e.i16("Driver")?,
                driven: e.i16("Driven")?,
                domain_a: e.i16("DomainA")?,
                domain_mid: e.i16("DomainMid")?,
                domain_b: e.i16("DomainB")?,
                range_a: e.i16("RangeA")?,
                range_mid: e.i16("RangeMid")?,
                range_b: e.i16("RangeB")?,
                power: e.i32("Power")?,
            });
        }
        for e in list(schema, obj, "VectorBuildSplitList", "RigamateVectorBuildSplit")? {
            self.vector_build_split.push(VectorBuildSplit {
                position: e.u32("Position")?,
                vector: e.i16("Vector")?,
                x: e.i16("X")?,
                y: e.i16("Y")?,
                z: e.i16("Z")?,
                connections: e.i32("Connections")?,
                build: e.bool("Build")?,
            });
        }
        for e in list(schema, obj, "TwistList", "RigamateTwist")? {
            self.twist.push(Twist {
                position: e.u32("Position")?,
                reference: e.i16("Reference")?,
                unk_joint_index: e.u32("JointIndex")?,
                amount: e.i16("Amount")?,
                driver: e.i16("Driver")?,
                gimbal: e.bool("Gimbal")?,
            });
        }
        for e in list(schema, obj, "PointConstraintList", "RigamatePointConstraint")? {
            self.point_constraint.push(PointConstraint {
                position: e.u32("Position")?,
                targets: [e.i16("Target1")?, e.i16("Target2")?, e.i16("Target3")?],
                weights: [e.f32("Weight1")?, e.f32("Weight2")?, e.f32("Weight3")?],
                result: e.i16("Result")?,
            });
        }
        for e in list(schema, obj, "ConditionalList", "RigamateConditional")? {
            self.conditional.push(Conditional {
                position: e.u32("Position")?,
                a: e.i16("A")?,
                b: e.i16("B")?,
                result: e.i16("Result")?,
                when_true: e.i16("True")?,
                when_false: e.i16("False")?,
                operation: e.i32("Operation")?,
            });
        }
        for e in list(schema, obj, "PSCList", "RigamatePSC")? {
            let floats = |f: &str| -> Result<Vec<f32>> {
                e.array(f)?
                    .iter()
                    .map(|v| v.as_f32().ok_or_else(|| Error::invalid(format!("RigamatePSC: {f}"))))
                    .collect()
            };
            let delays = e
                .array("BounceDelays")?
                .iter()
                .map(|v| v.as_i64().and_then(|x| u32::try_from(x).ok()))
                .collect::<Option<Vec<u32>>>()
                .ok_or_else(|| Error::invalid("RigamatePSC: BounceDelays"))?;
            self.psc.push(Psc {
                position: e.u32("Position")?,
                root: e.i16("Root")?,
                reference: e.i16("Reference")?,
                unk_joint_index: e.u32("JointIndex")?,
                unk_history: e.i16("History")?,
                num_bounces: e.u32("NumBounces")?,
                gravity: e.vec3("Gravity")?,
                sum_scalar: e.f32("SumScalar")?,
                bounce_scalars: floats("BounceScalars")?,
                bounce_delays: delays,
            });
        }
        for e in list(schema, obj, "VirtualJointList", "RigamateVirtualJoint")? {
            let (index, parent_index) = (e.i32("Index")?, e.i32("ParentIndex")?);
            self.virtual_joints.push(VirtualJoint { index, parent_index, offset: e.vec3("Offset")? });
        }
        for e in list(schema, obj, "DofIOList", "RigamateDofIO")? {
            let (position, value) = (e.u32("Position")?, e.i16("Value")?);
            self.dof_io.push(DofIo { position, value, index: e.u32("Index")?, reader: e.bool("Reader")? });
        }
        for e in list(schema, obj, "DofValueList", "RigamateDofValue")? {
            let dof_set_index = e.u32("DofSetIndex")?;
            self.dof_values.push(DofValue { dof_set_index, dof_index: e.u32("DofIndex")? });
        }
        for e in list(schema, obj, "DebugList", "RigamateDebug")? {
            self.debug.push(Debug { position: e.u32("Position")?, sqt: e.bool("Sqt")?, data: e.i16("Data")? });
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_tables_follow_the_loaders() {
        assert_eq!(JOINT_BASE[8], 0x20);
        assert_eq!(JOINT_BASE[8] + 0xF + 0x1E + 0x3C, 0x7A + 0xF);
        assert_eq!(SPLIT_BASE[1..], [6, 7, 9, 8, 0xA, 0xB, 0xC]);
    }
}
