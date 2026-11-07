use std::collections::BTreeMap;

pub type RowMatrix = [[f32; 4]; 4];

pub const MAX_JOINTS: usize = 512;

pub const BIND_ROTATION_TOLERANCE: f32 = 1e-3;
pub const BIND_TRANSLATION_TOLERANCE: f32 = 1e-3;

#[derive(Clone, Debug, PartialEq)]
pub struct SkeletonJoint {
    pub name: String,
    pub parent: Option<usize>,
    pub inverse_bind: RowMatrix,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Skeleton {
    pub joints: Vec<SkeletonJoint>,
    pub face: Option<String>,
    by_name: BTreeMap<String, usize>,
}

impl Skeleton {
    pub fn new(joints: Vec<SkeletonJoint>, face: Option<String>) -> Result<Skeleton, String> {
        if joints.is_empty() || joints.len() > MAX_JOINTS {
            return Err(format!("{} joints; a skeleton has 1 to {MAX_JOINTS}", joints.len()));
        }
        let mut by_name = BTreeMap::new();
        for (i, j) in joints.iter().enumerate() {
            if j.parent.is_some_and(|p| p >= i) {
                return Err(format!(
                    "joint {i} {}: its parent does not come before it",
                    ssx_core::text::quoted(&j.name)
                ));
            }
            if j.inverse_bind.iter().flatten().any(|v| !v.is_finite()) {
                return Err(format!(
                    "joint {}: a bind matrix with a value that is not finite",
                    ssx_core::text::quoted(&j.name)
                ));
            }
            if by_name.insert(j.name.clone(), i).is_some() {
                return Err(format!("the joint name {} is given twice", ssx_core::text::quoted(&j.name)));
            }
        }
        Ok(Skeleton { joints, face, by_name })
    }

    pub fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    pub fn len(&self) -> usize {
        self.joints.len()
    }

    pub fn is_empty(&self) -> bool {
        self.joints.is_empty()
    }

    pub fn is_below(&self, mut j: usize, ancestor: usize) -> bool {
        loop {
            if j == ancestor {
                return true;
            }
            match self.joints.get(j).and_then(|n| n.parent) {
                Some(p) => j = p,
                None => return false,
            }
        }
    }

    pub fn bind_mismatch(&self, j: usize, m: &RowMatrix) -> Option<String> {
        let want = &self.joints[j].inverse_bind;
        let mut worst_r = 0.0f32;
        for r in 0..3 {
            for c in 0..3 {
                worst_r = worst_r.max((m[r][c] - want[r][c]).abs());
            }
        }
        let worst_t = (0..3).map(|c| (m[3][c] - want[3][c]).abs()).fold(0.0f32, f32::max);
        if worst_r <= BIND_ROTATION_TOLERANCE && worst_t <= BIND_TRANSLATION_TOLERANCE {
            return None;
        }
        Some(format!(
            "joint {}: its inverse bind matrix differs from the skeleton's bind pose{} by {worst_r:.2e} in rotation \
             and scale and {worst_t:.2e} m in translation (at most {BIND_ROTATION_TOLERANCE:e} and \
             {BIND_TRANSLATION_TOLERANCE:e} m): a moved or changed skeleton is simulation content, refused in this \
             phase",
            ssx_core::text::quoted(&self.joints[j].name),
            self.face.as_deref().map_or(String::new(), |f| format!(" (face offsets of {f})"))
        ))
    }
}

pub fn mul(a: &RowMatrix, b: &RowMatrix) -> RowMatrix {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..4).map(|k| a[i][k] * b[k][j]).sum()))
}

pub fn det3(m: &RowMatrix) -> f64 {
    let e = |r: usize, c: usize| f64::from(m[r][c]);
    e(0, 0) * (e(1, 1) * e(2, 2) - e(1, 2) * e(2, 1)) - e(0, 1) * (e(1, 0) * e(2, 2) - e(1, 2) * e(2, 0))
        + e(0, 2) * (e(1, 0) * e(2, 1) - e(1, 1) * e(2, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joint(name: &str, parent: Option<usize>, t: [f32; 3]) -> SkeletonJoint {
        let mut m = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
        m[3] = [-t[0], -t[1], -t[2], 1.0];
        SkeletonJoint { name: name.into(), parent, inverse_bind: m }
    }

    #[test]
    fn a_moved_joint_is_another_skeleton() {
        let s =
            Skeleton::new(vec![joint("Hips", None, [0.0, 1.0, 0.0]), joint("Head", Some(0), [0.0, 1.7, 0.0])], None)
                .unwrap();
        let mut m = s.joints[1].inverse_bind;
        m[3][1] -= 0.0005;
        assert_eq!(s.bind_mismatch(1, &m), None);
        m[3][1] -= 0.0015;
        assert!(s.bind_mismatch(1, &m).unwrap().contains("simulation content"));
        let mut r = s.joints[1].inverse_bind;
        r[0][0] = 1.002;
        assert!(s.bind_mismatch(1, &r).is_some(), "scaled by 0.2 %");
        assert!(s.is_below(1, 0) && !s.is_below(0, 1));
        let dup = Skeleton::new(vec![joint("A", None, [0.0; 3]), joint("A", Some(0), [0.0; 3])], None);
        assert!(dup.unwrap_err().contains("twice"));
        let order = Skeleton::new(vec![joint("A", Some(1), [0.0; 3]), joint("B", None, [0.0; 3])], None);
        assert!(order.unwrap_err().contains("does not come before"));
    }
}
