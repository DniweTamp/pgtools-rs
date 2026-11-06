//! 4x4 transformation matrices, 4D vectors, and PlatinumGames 6-order Euler rotations
//! (1:1 port of `lib/bayonetta/linalg.rb`).

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Vector {
    pub data: [f32; 4],
}

impl Default for Vector {
    fn default() -> Self {
        Self::new(0.0, 0.0, 0.0, 1.0)
    }
}

impl Vector {
    #[inline]
    pub const fn new(x: f32, y: f32, z: f32, w: f32) -> Self {
        Self { data: [x, y, z, w] }
    }

    #[inline]
    pub const fn vec3(x: f32, y: f32, z: f32) -> Self {
        Self { data: [x, y, z, 1.0] }
    }

    #[inline]
    pub const fn dir3(x: f32, y: f32, z: f32) -> Self {
        Self { data: [x, y, z, 0.0] }
    }

    #[inline]
    pub fn x(&self) -> f32 {
        self.data[0]
    }
    #[inline]
    pub fn y(&self) -> f32 {
        self.data[1]
    }
    #[inline]
    pub fn z(&self) -> f32 {
        self.data[2]
    }
    #[inline]
    pub fn w(&self) -> f32 {
        self.data[3]
    }

    #[inline]
    pub fn set_x(&mut self, v: f32) {
        self.data[0] = v;
    }
    #[inline]
    pub fn set_y(&mut self, v: f32) {
        self.data[1] = v;
    }
    #[inline]
    pub fn set_z(&mut self, v: f32) {
        self.data[2] = v;
    }
    #[inline]
    pub fn set_w(&mut self, v: f32) {
        self.data[3] = v;
    }

    #[inline]
    pub fn dot(self, other: Self) -> f32 {
        self.x() * other.x() + self.y() * other.y() + self.z() * other.z()
    }

    #[inline]
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }

    pub fn normalize(mut self) -> Self {
        let l = self.length();
        if l != 0.0 && !l.is_nan() {
            self.data[0] /= l;
            self.data[1] /= l;
            self.data[2] /= l;
        } else {
            self.data[0] = 0.0;
            self.data[1] = 0.0;
            self.data[2] = 0.0;
        }
        self
    }
}

impl Add for Vector {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self::new(
            self.x() + rhs.x(),
            self.y() + rhs.y(),
            self.z() + rhs.z(),
            self.w(),
        )
    }
}

impl Sub for Vector {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self::new(
            self.x() - rhs.x(),
            self.y() - rhs.y(),
            self.z() - rhs.z(),
            self.w(),
        )
    }
}

impl Mul<f32> for Vector {
    type Output = Self;
    fn mul(self, scalar: f32) -> Self::Output {
        Self::new(
            self.x() * scalar,
            self.y() * scalar,
            self.z() * scalar,
            self.w(),
        )
    }
}

impl Neg for Vector {
    type Output = Self;
    fn neg(self) -> Self::Output {
        Self::new(-self.x(), -self.y(), -self.z(), self.w())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Matrix {
    pub data: [[f32; 4]; 4],
}

impl Default for Matrix {
    fn default() -> Self {
        Self::identity()
    }
}

impl Matrix {
    pub const fn identity() -> Self {
        Self {
            data: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub const fn zero() -> Self {
        Self {
            data: [[0.0; 4]; 4],
        }
    }

    pub fn transpose(&self) -> Self {
        let mut m = Self::zero();
        for i in 0..4 {
            for j in 0..4 {
                m.data[i][j] = self.data[j][i];
            }
        }
        m
    }

    pub fn inverse(&self) -> Self {
        let d = &self.data;
        let s0 = d[0][0] * d[1][1] - d[1][0] * d[0][1];
        let s1 = d[0][0] * d[1][2] - d[1][0] * d[0][2];
        let s2 = d[0][0] * d[1][3] - d[1][0] * d[0][3];
        let s3 = d[0][1] * d[1][2] - d[1][1] * d[0][2];
        let s4 = d[0][1] * d[1][3] - d[1][1] * d[0][3];
        let s5 = d[0][2] * d[1][3] - d[1][2] * d[0][3];

        let c5 = d[2][2] * d[3][3] - d[3][2] * d[2][3];
        let c4 = d[2][1] * d[3][3] - d[3][1] * d[2][3];
        let c3 = d[2][1] * d[3][2] - d[3][1] * d[2][2];
        let c2 = d[2][0] * d[3][3] - d[3][0] * d[2][3];
        let c1 = d[2][0] * d[3][2] - d[3][0] * d[2][2];
        let c0 = d[2][0] * d[3][1] - d[3][0] * d[2][1];

        let det = s0 * c5 - s1 * c4 + s2 * c3 + s3 * c2 - s4 * c1 + s5 * c0;
        let invdet = if det.abs() > 1e-12 { 1.0 / det } else { 0.0 };

        let mut m = Self::zero();
        m.data[0][0] = (d[1][1] * c5 - d[1][2] * c4 + d[1][3] * c3) * invdet;
        m.data[0][1] = (-d[0][1] * c5 + d[0][2] * c4 - d[0][3] * c3) * invdet;
        m.data[0][2] = (d[3][1] * s5 - d[3][2] * s4 + d[3][3] * s3) * invdet;
        m.data[0][3] = (-d[2][1] * s5 + d[2][2] * s4 - d[2][3] * s3) * invdet;

        m.data[1][0] = (-d[1][0] * c5 + d[1][2] * c2 - d[1][3] * c1) * invdet;
        m.data[1][1] = (d[0][0] * c5 - d[0][2] * c2 + d[0][3] * c1) * invdet;
        m.data[1][2] = (-d[3][0] * s5 + d[3][2] * s2 - d[3][3] * s1) * invdet;
        m.data[1][3] = (d[2][0] * s5 - d[2][2] * s2 + d[2][3] * s1) * invdet;

        m.data[2][0] = (d[1][0] * c4 - d[1][1] * c2 + d[1][3] * c0) * invdet;
        m.data[2][1] = (-d[0][0] * c4 + d[0][1] * c2 - d[0][3] * c0) * invdet;
        m.data[2][2] = (d[3][0] * s4 - d[3][1] * s2 + d[3][3] * s0) * invdet;
        m.data[2][3] = (-d[2][0] * s4 + d[2][1] * s2 - d[2][3] * s0) * invdet;

        m.data[3][0] = (-d[1][0] * c3 + d[1][1] * c1 - d[1][2] * c0) * invdet;
        m.data[3][1] = (d[0][0] * c3 - d[0][1] * c1 + d[0][2] * c0) * invdet;
        m.data[3][2] = (-d[3][0] * s3 + d[3][1] * s1 - d[3][2] * s0) * invdet;
        m.data[3][3] = (d[2][0] * s3 - d[2][1] * s1 + d[2][2] * s0) * invdet;
        m
    }

    pub fn translation(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::identity();
        m.data[0][3] = x;
        m.data[1][3] = y;
        m.data[2][3] = z;
        m
    }

    pub fn inverse_translation(x: f32, y: f32, z: f32) -> Self {
        Self::translation(-x, -y, -z)
    }

    pub fn scaling(x: f32, y: f32, z: f32) -> Self {
        let mut m = Self::identity();
        m.data[0][0] = x;
        m.data[1][1] = y;
        m.data[2][2] = z;
        m
    }

    pub fn inverse_scaling(x: f32, y: f32, z: f32) -> Self {
        Self::scaling(1.0 / x, 1.0 / y, 1.0 / z)
    }

    pub fn axis_angle(angle: f32, axis: Vector) -> Self {
        let v = axis.normalize();
        let (x, y, z) = (v.x(), v.y(), v.z());
        let c = angle.cos();
        let d = 1.0 - c;
        let s = angle.sin();
        Self {
            data: [
                [x * x * d + c, x * y * d - z * s, x * z * d + y * s, 0.0],
                [y * x * d + z * s, y * y * d + c, y * z * d - x * s, 0.0],
                [z * x * d - y * s, z * y * d + x * s, z * z * d + c, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn rotation(rx: f32, ry: f32, rz: f32, center: Option<[f32; 3]>, order: Option<i8>) -> Self {
        let (mt1, mt2) = if let Some([cx, cy, cz]) = center {
            (Self::translation(-cx, -cy, -cz), Self::translation(cx, cy, cz))
        } else {
            (Self::identity(), Self::identity())
        };
        let ax = Self::axis_angle(rx, Vector::vec3(1.0, 0.0, 0.0));
        let ay = Self::axis_angle(ry, Vector::vec3(0.0, 1.0, 0.0));
        let az = Self::axis_angle(rz, Vector::vec3(0.0, 0.0, 1.0));

        let rot = match order {
            Some(0) => ax * ay * az,
            Some(1) => ax * az * ay,
            Some(2) => ay * ax * az,
            Some(3) => ay * az * ax,
            Some(4) => az * ax * ay,
            _ => az * ay * ax,
        };
        mt2 * rot * mt1
    }

    pub fn inverse_rotation(
        rx: f32,
        ry: f32,
        rz: f32,
        center: Option<[f32; 3]>,
        order: Option<i8>,
    ) -> Self {
        let (mt1, mt2) = if let Some([cx, cy, cz]) = center {
            (Self::translation(-cx, -cy, -cz), Self::translation(cx, cy, cz))
        } else {
            (Self::identity(), Self::identity())
        };
        let ax = Self::axis_angle(-rx, Vector::vec3(1.0, 0.0, 0.0));
        let ay = Self::axis_angle(-ry, Vector::vec3(0.0, 1.0, 0.0));
        let az = Self::axis_angle(-rz, Vector::vec3(0.0, 0.0, 1.0));

        let rot = match order {
            Some(0) => az * ay * ax,
            Some(1) => ay * az * ax,
            Some(2) => az * ax * ay,
            Some(3) => ax * az * ay,
            Some(4) => ay * ax * az,
            _ => ax * ay * az,
        };
        mt2 * rot * mt1
    }

    pub fn transformation(
        translate: [f32; 3],
        rotate: [f32; 3],
        scale: [f32; 3],
        parent_scale: [f32; 3],
        order: Option<i8>,
    ) -> Self {
        Self::translation(translate[0], translate[1], translate[2])
            * Self::inverse_scaling(parent_scale[0], parent_scale[1], parent_scale[2])
            * Self::rotation(rotate[0], rotate[1], rotate[2], None, order)
            * Self::scaling(scale[0], scale[1], scale[2])
    }

    pub fn inverse_transformation(
        translate: [f32; 3],
        rotate: [f32; 3],
        scale: [f32; 3],
        parent_scale: [f32; 3],
        order: Option<i8>,
    ) -> Self {
        Self::inverse_scaling(scale[0], scale[1], scale[2])
            * Self::inverse_rotation(rotate[0], rotate[1], rotate[2], None, order)
            * Self::scaling(parent_scale[0], parent_scale[1], parent_scale[2])
            * Self::inverse_translation(translate[0], translate[1], translate[2])
    }
}

impl Add for Matrix {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        let mut m = Self::zero();
        for i in 0..4 {
            for j in 0..4 {
                m.data[i][j] = self.data[i][j] + rhs.data[i][j];
            }
        }
        m
    }
}

impl Mul for Matrix {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self::Output {
        let mut m = Self::zero();
        for i in 0..4 {
            for j in 0..4 {
                let mut sum = 0.0;
                for k in 0..4 {
                    sum += self.data[i][k] * rhs.data[k][j];
                }
                m.data[i][j] = sum;
            }
        }
        m
    }
}

impl Mul<Vector> for Matrix {
    type Output = Vector;
    fn mul(self, rhs: Vector) -> Self::Output {
        let d = &self.data;
        let v = &rhs.data;
        Vector::new(
            d[0][0] * v[0] + d[0][1] * v[1] + d[0][2] * v[2] + d[0][3] * v[3],
            d[1][0] * v[0] + d[1][1] * v[1] + d[1][2] * v[2] + d[1][3] * v[3],
            d[2][0] * v[0] + d[2][1] * v[1] + d[2][2] * v[2] + d[2][3] * v[3],
            d[3][0] * v[0] + d[3][1] * v[1] + d[3][2] * v[2] + d[3][3] * v[3],
        )
    }
}

impl Mul<f32> for Matrix {
    type Output = Self;
    fn mul(self, scalar: f32) -> Self::Output {
        let mut m = Self::zero();
        for i in 0..4 {
            for j in 0..4 {
                m.data[i][j] = self.data[i][j] * scalar;
            }
        }
        m
    }
}
