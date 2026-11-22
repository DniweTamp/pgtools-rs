//! Pure-Rust ASTC (Adaptive Scalable Texture Compression) LDR decoder supporting all 14 footprints
//! (`4x4`, `5x4`, `5x5`, `6x5`, `6x6`, `8x5`, `8x6`, `8x8`, `10x5`, `10x6`, `10x8`, `10x10`, `12x10`, `12x12`),
//! plus BC7 and BC6H (UF16/SF16) block decoders for full Switch (`XT1`/`BNTX`) and PC (`DDS`) parity.
//!
//! Ported from `Switch-Toolbox` (`Ryujinx.Graphics.Gal.Texture.ASTCDecoder`) and `XT1 Studio`.

use image::{ImageBuffer, RgbaImage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IntEncoding {
    JustBits,
    Quint,
    Trit,
}

#[derive(Debug, Clone, Copy)]
struct IntegerEncoded {
    encoding: IntEncoding,
    num_bits: i32,
    bit_value: i32,
    trit_value: i32,
    quint_value: i32,
}

impl IntegerEncoded {
    fn new(encoding: IntEncoding, num_bits: i32) -> Self {
        Self {
            encoding,
            num_bits,
            bit_value: 0,
            trit_value: 0,
            quint_value: 0,
        }
    }

    fn matches(&self, other: &Self) -> bool {
        self.encoding == other.encoding && self.num_bits == other.num_bits
    }

    fn bit_length(&self, num_vals: i32) -> i32 {
        let mut total = self.num_bits * num_vals;
        match self.encoding {
            IntEncoding::Trit => total += (num_vals * 8 + 4) / 5,
            IntEncoding::Quint => total += (num_vals * 7 + 2) / 3,
            IntEncoding::JustBits => {}
        }
        total
    }

    fn create_encoding(mut max_val: i32) -> Self {
        while max_val > 0 {
            let check = max_val + 1;
            if (check & (check - 1)) == 0 {
                return Self::new(IntEncoding::JustBits, max_val.count_ones() as i32);
            }
            if (check % 3 == 0) && ((check / 3) & ((check / 3) - 1)) == 0 {
                return Self::new(IntEncoding::Trit, (check / 3 - 1).count_ones() as i32);
            }
            if (check % 5 == 0) && ((check / 5) & ((check / 5) - 1)) == 0 {
                return Self::new(IntEncoding::Quint, (check / 5 - 1).count_ones() as i32);
            }
            max_val -= 1;
        }
        Self::new(IntEncoding::JustBits, 0)
    }
}

struct BitStream {
    bits: [u8; 16],
    pos: usize,
}

impl BitStream {
    fn from_bytes(bytes: &[u8]) -> Self {
        let mut bits = [0u8; 16];
        let copy_len = bytes.len().min(16);
        bits[..copy_len].copy_from_slice(&bytes[..copy_len]);
        Self { bits, pos: 0 }
    }

    fn read_bit_at(&self, idx: usize) -> i32 {
        if idx >= 128 {
            return 0;
        }
        ((self.bits[idx >> 3] >> (idx & 7)) & 1) as i32
    }

    fn read_bits(&mut self, len: usize) -> i32 {
        let mut ret = 0i32;
        for i in 0..len {
            if self.read_bit_at(self.pos + i) != 0 {
                ret |= 1 << i;
            }
        }
        self.pos += len;
        ret
    }

    fn write_bits(&mut self, val: i32, len: usize) {
        for i in 0..len {
            let bit_idx = self.pos + i;
            if bit_idx < 128 {
                if ((val >> i) & 1) != 0 {
                    self.bits[bit_idx >> 3] |= 1 << (bit_idx & 7);
                } else {
                    self.bits[bit_idx >> 3] &= !(1 << (bit_idx & 7));
                }
            }
        }
        self.pos += len;
    }
}

#[inline]
fn read_int_bits(val: i32, start: usize, end: usize) -> i32 {
    let mut ret = 0;
    for i in start..=end {
        if ((val >> i) & 1) != 0 {
            ret |= 1 << (i - start);
        }
    }
    ret
}

#[inline]
fn read_int_bit(val: i32, idx: usize) -> i32 {
    (val >> idx) & 1
}

#[inline]
fn replicate(val: i32, mut num_bits: i32, to_bit: i32) -> i32 {
    if num_bits <= 0 || to_bit <= 0 {
        return 0;
    }
    let temp = val & ((1 << num_bits) - 1);
    let mut ret = temp;
    let mut res_len = num_bits;
    while res_len < to_bit {
        let mut comp = 0;
        if num_bits > to_bit - res_len {
            let new_shift = to_bit - res_len;
            comp = num_bits - new_shift;
            num_bits = new_shift;
        }
        ret <<= num_bits;
        ret |= temp >> comp;
        res_len += num_bits;
    }
    ret
}

#[inline]
fn bit_transfer_signed(a: &mut i32, b: &mut i32) {
    *b >>= 1;
    *b |= *a & 0x80;
    *a >>= 1;
    *a &= 0x3F;
    if (*a & 0x20) != 0 {
        *a -= 0x40;
    }
}

fn decode_trit_block(bs: &mut BitStream, out: &mut Vec<IntegerEncoded>, nbits: i32) {
    let mut m = [0i32; 5];
    let mut t = [0i32; 5];
    let nb = nbits as usize;
    m[0] = bs.read_bits(nb);
    let mut big_t = bs.read_bits(2);
    m[1] = bs.read_bits(nb);
    big_t |= bs.read_bits(2) << 2;
    m[2] = bs.read_bits(nb);
    big_t |= bs.read_bits(1) << 4;
    m[3] = bs.read_bits(nb);
    big_t |= bs.read_bits(2) << 5;
    m[4] = bs.read_bits(nb);
    big_t |= bs.read_bits(1) << 7;

    let c: i32;
    if read_int_bits(big_t, 2, 4) == 7 {
        c = (read_int_bits(big_t, 5, 7) << 2) | read_int_bits(big_t, 0, 1);
        t[4] = 2;
        t[3] = 2;
    } else {
        c = read_int_bits(big_t, 0, 4);
        if read_int_bits(big_t, 5, 6) == 3 {
            t[4] = 2;
            t[3] = read_int_bit(big_t, 7);
        } else {
            t[4] = read_int_bit(big_t, 7);
            t[3] = read_int_bits(big_t, 5, 6);
        }
    }

    if read_int_bits(c, 0, 1) == 3 {
        t[2] = 2;
        t[1] = read_int_bit(c, 4);
        t[0] = (read_int_bit(c, 3) << 1) | (read_int_bit(c, 2) & !read_int_bit(c, 3));
    } else if read_int_bits(c, 2, 3) == 3 {
        t[2] = 2;
        t[1] = 2;
        t[0] = read_int_bits(c, 0, 1);
    } else {
        t[2] = read_int_bit(c, 4);
        t[1] = read_int_bits(c, 2, 3);
        t[0] = (read_int_bit(c, 1) << 1) | (read_int_bit(c, 0) & !read_int_bit(c, 1));
    }

    for i in 0..5 {
        out.push(IntegerEncoded {
            encoding: IntEncoding::Trit,
            num_bits: nbits,
            bit_value: m[i],
            trit_value: t[i],
            quint_value: 0,
        });
    }
}

fn decode_quint_block(bs: &mut BitStream, out: &mut Vec<IntegerEncoded>, nbits: i32) {
    let mut m = [0i32; 3];
    let mut q = [0i32; 3];
    let nb = nbits as usize;
    m[0] = bs.read_bits(nb);
    let mut big_q = bs.read_bits(3);
    m[1] = bs.read_bits(nb);
    big_q |= bs.read_bits(2) << 3;
    m[2] = bs.read_bits(nb);
    big_q |= bs.read_bits(2) << 5;

    if read_int_bits(big_q, 1, 2) == 3 && read_int_bits(big_q, 5, 6) == 0 {
        q[0] = 4;
        q[1] = 4;
        q[2] = (read_int_bit(big_q, 0) << 2)
            | ((read_int_bit(big_q, 4) & !read_int_bit(big_q, 0)) << 1)
            | (read_int_bit(big_q, 3) & !read_int_bit(big_q, 0));
    } else {
        let c: i32;
        if read_int_bits(big_q, 1, 2) == 3 {
            q[2] = 4;
            c = (read_int_bits(big_q, 3, 4) << 3)
                | ((!read_int_bits(big_q, 5, 6) & 3) << 1)
                | read_int_bit(big_q, 0);
        } else {
            q[2] = read_int_bits(big_q, 5, 6);
            c = read_int_bits(big_q, 0, 4);
        }
        if read_int_bits(c, 0, 2) == 5 {
            q[1] = 4;
            q[0] = read_int_bits(c, 3, 4);
        } else {
            q[1] = read_int_bits(c, 3, 4);
            q[0] = read_int_bits(c, 0, 2);
        }
    }

    for i in 0..3 {
        out.push(IntegerEncoded {
            encoding: IntEncoding::Quint,
            num_bits: nbits,
            bit_value: m[i],
            trit_value: 0,
            quint_value: q[i],
        });
    }
}

fn decode_integer_sequence(out: &mut Vec<IntegerEncoded>, bs: &mut BitStream, max_range: i32, num_values: i32) {
    let mut enc = IntegerEncoded::create_encoding(max_range);
    let mut decoded = 0;
    while decoded < num_values {
        match enc.encoding {
            IntEncoding::Quint => {
                decode_quint_block(bs, out, enc.num_bits);
                decoded += 3;
            }
            IntEncoding::Trit => {
                decode_trit_block(bs, out, enc.num_bits);
                decoded += 5;
            }
            IntEncoding::JustBits => {
                enc.bit_value = bs.read_bits(enc.num_bits as usize);
                out.push(enc);
                decoded += 1;
            }
        }
    }
}

#[derive(Default, Clone, Copy)]
struct TexelWeightParams {
    width: i32,
    height: i32,
    dual_plane: bool,
    max_weight: i32,
    error: bool,
    void_extent_ldr: bool,
    void_extent_hdr: bool,
}

impl TexelWeightParams {
    fn num_weight_values(&self) -> i32 {
        let mut ret = self.width * self.height;
        if self.dual_plane {
            ret *= 2;
        }
        ret
    }

    fn packed_bit_size(&self) -> i32 {
        let enc = IntegerEncoded::create_encoding(self.max_weight);
        enc.bit_length(self.num_weight_values())
    }
}

fn decode_block_info(bs: &mut BitStream) -> TexelWeightParams {
    let mut p = TexelWeightParams::default();
    let mode_bits = bs.read_bits(11) as u16;

    if (mode_bits & 0x01FF) == 0x1FC {
        if (mode_bits & 0x200) != 0 {
            p.void_extent_hdr = true;
        } else {
            p.void_extent_ldr = true;
        }
        if (mode_bits & 0x400) == 0 || bs.read_bits(1) == 0 {
            p.error = true;
        }
        return p;
    }

    if (mode_bits & 0xF) == 0 {
        p.error = true;
        return p;
    }
    if (mode_bits & 0x3) == 0 && (mode_bits & 0x1C0) == 0x1C0 {
        p.error = true;
        return p;
    }

    let layout = if (mode_bits & 0x1) != 0 || (mode_bits & 0x2) != 0 {
        if (mode_bits & 0x8) != 0 {
            if (mode_bits & 0x4) != 0 {
                if (mode_bits & 0x100) != 0 { 4 } else { 3 }
            } else {
                2
            }
        } else if (mode_bits & 0x4) != 0 {
            1
        } else {
            0
        }
    } else if (mode_bits & 0x100) != 0 {
        if (mode_bits & 0x80) != 0 {
            if (mode_bits & 0x20) != 0 { 8 } else { 7 }
        } else {
            9
        }
    } else if (mode_bits & 0x80) != 0 {
        6
    } else {
        5
    };

    let mut r = ((mode_bits >> 4) & 1) as i32;
    if layout < 5 {
        r |= ((mode_bits & 0x3) as i32) << 1;
    } else {
        r |= ((mode_bits & 0xC) as i32) >> 1;
    }
    if !(2..=7).contains(&r) {
        p.error = true;
        return p;
    }

    match layout {
        0 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 7) & 0x3) as i32;
            p.width = b + 4;
            p.height = a + 2;
        }
        1 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 7) & 0x3) as i32;
            p.width = b + 8;
            p.height = a + 2;
        }
        2 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 7) & 0x3) as i32;
            p.width = a + 2;
            p.height = b + 8;
        }
        3 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 7) & 0x1) as i32;
            p.width = a + 2;
            p.height = b + 6;
        }
        4 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 7) & 0x1) as i32;
            p.width = b + 2;
            p.height = a + 2;
        }
        5 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            p.width = 12;
            p.height = a + 2;
        }
        6 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            p.width = a + 2;
            p.height = 12;
        }
        7 => {
            p.width = 6;
            p.height = 10;
        }
        8 => {
            p.width = 10;
            p.height = 6;
        }
        9 => {
            let a = ((mode_bits >> 5) & 0x3) as i32;
            let b = ((mode_bits >> 9) & 0x3) as i32;
            p.width = a + 6;
            p.height = b + 6;
        }
        _ => {
            p.error = true;
            return p;
        }
    }

    let d = (layout != 9) && ((mode_bits & 0x400) != 0);
    let h = (layout != 9) && ((mode_bits & 0x200) != 0);
    p.max_weight = if h {
        [9, 11, 15, 19, 23, 31][(r - 2) as usize]
    } else {
        [1, 2, 3, 4, 5, 7][(r - 2) as usize]
    };
    p.dual_plane = d;
    p
}

#[derive(Clone, Copy, Default)]
struct AstcPixel {
    a: i16,
    r: i16,
    g: i16,
    b: i16,
}

impl AstcPixel {
    fn new(a: i32, r: i32, g: i32, b: i32) -> Self {
        Self {
            a: a as i16,
            r: r as i16,
            g: g as i16,
            b: b as i16,
        }
    }

    fn clamp_byte(&mut self) {
        self.a = self.a.clamp(0, 255);
        self.r = self.r.clamp(0, 255);
        self.g = self.g.clamp(0, 255);
        self.b = self.b.clamp(0, 255);
    }

    fn blue_contract(a: i32, r: i32, g: i32, b: i32) -> Self {
        Self::new(a, (r + b) >> 1, (g + b) >> 1, b)
    }

    fn get_comp(&self, idx: usize) -> i32 {
        match idx {
            0 => self.a as i32,
            1 => self.r as i32,
            2 => self.g as i32,
            3 => self.b as i32,
            _ => 0,
        }
    }

    fn set_comp(&mut self, idx: usize, v: i32) {
        match idx {
            0 => self.a = v as i16,
            1 => self.r = v as i16,
            2 => self.g = v as i16,
            3 => self.b = v as i16,
            _ => {}
        }
    }

    fn to_rgba_bytes(self) -> [u8; 4] {
        [
            self.r.clamp(0, 255) as u8,
            self.g.clamp(0, 255) as u8,
            self.b.clamp(0, 255) as u8,
            self.a.clamp(0, 255) as u8,
        ]
    }
}

fn compute_endpoints(
    ep: &mut [AstcPixel; 2],
    cv: &[i32],
    mode: u32,
    pos: &mut usize,
) {
    let mut next_val = || -> i32 {
        let v = cv.get(*pos).copied().unwrap_or(0);
        *pos += 1;
        v
    };

    match mode {
        0 => {
            let v0 = next_val();
            let v1 = next_val();
            ep[0] = AstcPixel::new(0xFF, v0, v0, v0);
            ep[1] = AstcPixel::new(0xFF, v1, v1, v1);
        }
        1 => {
            let v0 = next_val();
            let v1 = next_val();
            let l0 = (v0 >> 2) | (v1 & 0xC0);
            let l1 = (l0 + (v1 & 0x3F)).min(0xFF);
            ep[0] = AstcPixel::new(0xFF, l0, l0, l0);
            ep[1] = AstcPixel::new(0xFF, l1, l1, l1);
        }
        4 => {
            let (v0, v1, v2, v3) = (next_val(), next_val(), next_val(), next_val());
            ep[0] = AstcPixel::new(v2, v0, v0, v0);
            ep[1] = AstcPixel::new(v3, v1, v1, v1);
        }
        5 => {
            let (mut v0, mut v1, mut v2, mut v3) = (next_val(), next_val(), next_val(), next_val());
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            ep[0] = AstcPixel::new(v2, v0, v0, v0);
            ep[1] = AstcPixel::new(v2 + v3, v0 + v1, v0 + v1, v0 + v1);
            ep[0].clamp_byte();
            ep[1].clamp_byte();
        }
        6 => {
            let (v0, v1, v2, v3) = (next_val(), next_val(), next_val(), next_val());
            ep[0] = AstcPixel::new(0xFF, (v0 * v3) >> 8, (v1 * v3) >> 8, (v2 * v3) >> 8);
            ep[1] = AstcPixel::new(0xFF, v0, v1, v2);
        }
        8 => {
            let (v0, v1, v2, v3, v4, v5) = (
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
            );
            if v1 + v3 + v5 >= v0 + v2 + v4 {
                ep[0] = AstcPixel::new(0xFF, v0, v2, v4);
                ep[1] = AstcPixel::new(0xFF, v1, v3, v5);
            } else {
                ep[0] = AstcPixel::blue_contract(0xFF, v1, v3, v5);
                ep[1] = AstcPixel::blue_contract(0xFF, v0, v2, v4);
            }
        }
        9 => {
            let (mut v0, mut v1, mut v2, mut v3, mut v4, mut v5) = (
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
            );
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            bit_transfer_signed(&mut v5, &mut v4);
            if v1 + v3 + v5 >= 0 {
                ep[0] = AstcPixel::new(0xFF, v0, v2, v4);
                ep[1] = AstcPixel::new(0xFF, v0 + v1, v2 + v3, v4 + v5);
            } else {
                ep[0] = AstcPixel::blue_contract(0xFF, v0 + v1, v2 + v3, v4 + v5);
                ep[1] = AstcPixel::blue_contract(0xFF, v0, v2, v4);
            }
            ep[0].clamp_byte();
            ep[1].clamp_byte();
        }
        10 => {
            let (v0, v1, v2, v3, v4, v5) = (
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
            );
            ep[0] = AstcPixel::new(v4, (v0 * v3) >> 8, (v1 * v3) >> 8, (v2 * v3) >> 8);
            ep[1] = AstcPixel::new(v5, v0, v1, v2);
        }
        12 => {
            let (v0, v1, v2, v3, v4, v5, v6, v7) = (
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
            );
            if v1 + v3 + v5 >= v0 + v2 + v4 {
                ep[0] = AstcPixel::new(v6, v0, v2, v4);
                ep[1] = AstcPixel::new(v7, v1, v3, v5);
            } else {
                ep[0] = AstcPixel::blue_contract(v7, v1, v3, v5);
                ep[1] = AstcPixel::blue_contract(v6, v0, v2, v4);
            }
        }
        13 => {
            let (mut v0, mut v1, mut v2, mut v3, mut v4, mut v5, mut v6, mut v7) = (
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
                next_val(),
            );
            bit_transfer_signed(&mut v1, &mut v0);
            bit_transfer_signed(&mut v3, &mut v2);
            bit_transfer_signed(&mut v5, &mut v4);
            bit_transfer_signed(&mut v7, &mut v6);
            if v1 + v3 + v5 >= 0 {
                ep[0] = AstcPixel::new(v6, v0, v2, v4);
                ep[1] = AstcPixel::new(v6 + v7, v0 + v1, v2 + v3, v4 + v5);
            } else {
                ep[0] = AstcPixel::blue_contract(v6 + v7, v0 + v1, v2 + v3, v4 + v5);
                ep[1] = AstcPixel::blue_contract(v6, v0, v2, v4);
            }
            ep[0].clamp_byte();
            ep[1].clamp_byte();
        }
        _ => {
            ep[0] = AstcPixel::new(0xFF, 0, 0, 0);
            ep[1] = AstcPixel::new(0xFF, 255, 255, 255);
        }
    }
}

fn decode_color_values(
    out_vals: &mut [i32; 32],
    input_bytes: &[u8; 16],
    modes: &[u32; 4],
    num_partitions: usize,
    bits_for_color: i32,
) {
    let mut num_vals = 0i32;
    for i in 0..num_partitions {
        num_vals += (((modes[i] >> 2) + 1) << 1) as i32;
    }

    let mut range = 256i32;
    while {
        range -= 1;
        range > 0
    } {
        let enc = IntegerEncoded::create_encoding(range);
        if enc.bit_length(num_vals) <= bits_for_color {
            while {
                range -= 1;
                range > 0
            } {
                let new_enc = IntegerEncoded::create_encoding(range);
                if !new_enc.matches(&enc) {
                    break;
                }
            }
            range += 1;
            break;
        }
    }

    let mut seq = Vec::with_capacity(num_vals as usize + 8);
    let mut cbs = BitStream::from_bytes(input_bytes);
    decode_integer_sequence(&mut seq, &mut cbs, range, num_vals);

    let mut out_idx = 0usize;
    for item in seq {
        if out_idx >= 32 {
            break;
        }
        let bitlen = item.num_bits;
        let bitval = item.bit_value;
        let a = replicate(bitval & 1, 1, 9);
        let mut b = 0;
        let mut c = 0;
        let d: i32;

        match item.encoding {
            IntEncoding::JustBits => {
                out_vals[out_idx] = replicate(bitval, bitlen, 8);
                out_idx += 1;
                continue;
            }
            IntEncoding::Trit => {
                d = item.trit_value;
                match bitlen {
                    1 => c = 204,
                    2 => {
                        c = 93;
                        let bit = (bitval >> 1) & 1;
                        b = (bit << 8) | (bit << 4) | (bit << 2) | (bit << 1);
                    }
                    3 => {
                        c = 44;
                        let cb = (bitval >> 1) & 3;
                        b = (cb << 7) | (cb << 2) | cb;
                    }
                    4 => {
                        c = 22;
                        let dcb = (bitval >> 1) & 7;
                        b = (dcb << 6) | dcb;
                    }
                    5 => {
                        c = 11;
                        let edcb = (bitval >> 1) & 0xF;
                        b = (edcb << 5) | (edcb >> 2);
                    }
                    6 => {
                        c = 5;
                        let fedcb = (bitval >> 1) & 0x1F;
                        b = (fedcb << 4) | (fedcb >> 4);
                    }
                    _ => {}
                }
            }
            IntEncoding::Quint => {
                d = item.quint_value;
                match bitlen {
                    1 => c = 113,
                    2 => {
                        c = 54;
                        let bit = (bitval >> 1) & 1;
                        b = (bit << 8) | (bit << 3) | (bit << 2);
                    }
                    3 => {
                        c = 26;
                        let cb = (bitval >> 1) & 3;
                        b = (cb << 7) | (cb << 1) | (cb >> 1);
                    }
                    4 => {
                        c = 13;
                        let dcb = (bitval >> 1) & 7;
                        b = (dcb << 6) | (dcb >> 1);
                    }
                    5 => {
                        c = 6;
                        let edcb = (bitval >> 1) & 0xF;
                        b = (edcb << 5) | (edcb >> 3);
                    }
                    _ => {}
                }
            }
        }

        let mut t = d * c + b;
        t ^= a;
        t = (a & 0x80) | (t >> 2);
        out_vals[out_idx] = t;
        out_idx += 1;
    }
}

fn unquantize_texel_weight(item: &IntegerEncoded) -> i32 {
    let bitval = item.bit_value;
    let bitlen = item.num_bits;
    let a = replicate(bitval & 1, 1, 7);
    let mut b = 0;
    let mut c = 0;
    let d: i32;
    let mut result = 0;

    match item.encoding {
        IntEncoding::JustBits => {
            result = replicate(bitval, bitlen, 6);
        }
        IntEncoding::Trit => {
            d = item.trit_value.clamp(0, 2);
            match bitlen {
                0 => result = [0, 32, 63][d as usize],
                1 => c = 50,
                2 => {
                    c = 23;
                    let bit = (bitval >> 1) & 1;
                    b = (bit << 6) | (bit << 2) | bit;
                }
                3 => {
                    c = 11;
                    let cb = (bitval >> 1) & 3;
                    b = (cb << 5) | cb;
                }
                _ => {}
            }
            if bitlen > 0 {
                result = d * c + b;
                result ^= a;
                result = (a & 0x20) | (result >> 2);
            }
        }
        IntEncoding::Quint => {
            d = item.quint_value.clamp(0, 4);
            match bitlen {
                0 => result = [0, 16, 32, 47, 63][d as usize],
                1 => c = 28,
                2 => {
                    c = 13;
                    let bit = (bitval >> 1) & 1;
                    b = (bit << 6) | (bit << 1);
                }
                _ => {}
            }
            if bitlen > 0 {
                result = d * c + b;
                result ^= a;
                result = (a & 0x20) | (result >> 2);
            }
        }
    }

    if result > 32 {
        result += 1;
    }
    result
}

fn unquantize_texel_weights(
    out_weights: &mut [[i32; 144]; 2],
    weights: &[IntegerEncoded],
    params: &TexelWeightParams,
    block_w: usize,
    block_h: usize,
) {
    let mut unquantized = [[0i32; 144]; 2];
    let mut w_idx = 0usize;
    let mut i = 0usize;
    let total_grid = (params.width * params.height) as usize;

    while i < weights.len() && w_idx < total_grid.min(144) {
        unquantized[0][w_idx] = unquantize_texel_weight(&weights[i]);
        if params.dual_plane {
            i += 1;
            if i < weights.len() {
                unquantized[1][w_idx] = unquantize_texel_weight(&weights[i]);
            } else {
                break;
            }
        }
        w_idx += 1;
        i += 1;
    }

    let ds = (1024 + (block_w as i32 / 2)) / (block_w as i32 - 1).max(1);
    let dt = (1024 + (block_h as i32 / 2)) / (block_h as i32 - 1).max(1);
    let planes = if params.dual_plane { 2 } else { 1 };

    for plane in 0..planes {
        for t in 0..block_h {
            for s in 0..block_w {
                let cs = ds * (s as i32);
                let ct = dt * (t as i32);
                let gs = (cs * (params.width - 1) + 32) >> 6;
                let gt = (ct * (params.height - 1) + 32) >> 6;
                let js = gs >> 4;
                let fs = gs & 0xF;
                let jt = gt >> 4;
                let ft = gt & 0xF;

                let w11 = (fs * ft + 8) >> 4;
                let w10 = ft - w11;
                let w01 = fs - w11;
                let w00 = 16 - fs - ft + w11;

                let v0 = (js + jt * params.width) as usize;
                let p00 = if v0 < total_grid { unquantized[plane][v0] } else { 0 };
                let p01 = if v0 + 1 < total_grid { unquantized[plane][v0 + 1] } else { 0 };
                let p10 = if v0 + (params.width as usize) < total_grid {
                    unquantized[plane][v0 + (params.width as usize)]
                } else {
                    0
                };
                let p11 = if v0 + (params.width as usize) + 1 < total_grid {
                    unquantized[plane][v0 + (params.width as usize) + 1]
                } else {
                    0
                };

                out_weights[plane][t * block_w + s] =
                    (p00 * w00 + p01 * w01 + p10 * w10 + p11 * w11 + 8) >> 4;
            }
        }
    }
}

fn hash52(mut val: u32) -> u32 {
    val ^= val >> 15;
    val = val.wrapping_sub(val << 17);
    val = val.wrapping_add(val << 7);
    val = val.wrapping_add(val << 4);
    val ^= val >> 5;
    val = val.wrapping_add(val << 16);
    val ^= val >> 7;
    val ^= val >> 3;
    val ^= val << 6;
    val ^= val >> 17;
    val
}

fn select_2d_partition(
    mut seed: i32,
    mut x: i32,
    mut y: i32,
    partition_count: i32,
    is_small_block: bool,
) -> usize {
    if partition_count <= 1 {
        return 0;
    }
    let z = 0i32;
    if is_small_block {
        x <<= 1;
        y <<= 1;
    }
    seed += (partition_count - 1) * 1024;
    let rnum = hash52(seed as u32);

    let mut s1 = (rnum & 0xF) as i32;
    let mut s2 = ((rnum >> 4) & 0xF) as i32;
    let mut s3 = ((rnum >> 8) & 0xF) as i32;
    let mut s4 = ((rnum >> 12) & 0xF) as i32;
    let mut s5 = ((rnum >> 16) & 0xF) as i32;
    let mut s6 = ((rnum >> 20) & 0xF) as i32;
    let mut s7 = ((rnum >> 24) & 0xF) as i32;
    let mut s8 = ((rnum >> 28) & 0xF) as i32;
    let mut s9 = ((rnum >> 18) & 0xF) as i32;
    let mut s10 = ((rnum >> 22) & 0xF) as i32;
    let mut s11 = ((rnum >> 26) & 0xF) as i32;
    let mut s12 = (((rnum >> 30) | (rnum << 2)) & 0xF) as i32;

    s1 *= s1;
    s2 *= s2;
    s3 *= s3;
    s4 *= s4;
    s5 *= s5;
    s6 *= s6;
    s7 *= s7;
    s8 *= s8;
    s9 *= s9;
    s10 *= s10;
    s11 *= s11;
    s12 *= s12;

    let (sh1, sh2) = if (seed & 1) != 0 {
        (
            if (seed & 2) != 0 { 4 } else { 5 },
            if partition_count == 3 { 6 } else { 5 },
        )
    } else {
        (
            if partition_count == 3 { 6 } else { 5 },
            if (seed & 2) != 0 { 4 } else { 5 },
        )
    };
    let sh3 = if (seed & 0x10) != 0 { sh1 } else { sh2 };

    s1 >>= sh1;
    s2 >>= sh2;
    s3 >>= sh1;
    s4 >>= sh2;
    s5 >>= sh1;
    s6 >>= sh2;
    s7 >>= sh1;
    s8 >>= sh2;
    s9 >>= sh3;
    s10 >>= sh3;
    s11 >>= sh3;
    s12 >>= sh3;

    let mut a = (s1 * x + s2 * y + s11 * z + ((rnum >> 14) as i32)) & 0x3F;
    let mut b = (s3 * x + s4 * y + s12 * z + ((rnum >> 10) as i32)) & 0x3F;
    let mut c = (s5 * x + s6 * y + s9 * z + ((rnum >> 6) as i32)) & 0x3F;
    let mut d = (s7 * x + s8 * y + s10 * z + ((rnum >> 2) as i32)) & 0x3F;

    if partition_count < 4 {
        d = 0;
    }
    if partition_count < 3 {
        c = 0;
    }
    let _ = (&mut a, &mut b);
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

/// Decompress a single 16-byte ASTC block (`block_w` x `block_h`) into `[u8; 4]` RGBA8 pixels.
pub fn decompress_astc_block(
    block_bytes: &[u8; 16],
    out_pixels: &mut [[u8; 4]; 144],
    block_w: usize,
    block_h: usize,
) {
    let mut bs = BitStream::from_bytes(block_bytes);
    let params = decode_block_info(&mut bs);
    if params.error
        || params.void_extent_hdr
        || params.width as usize > block_w
        || params.height as usize > block_h
    {
        for p in out_pixels.iter_mut().take(block_w * block_h) {
            *p = [0, 0, 0, 0];
        }
        return;
    }

    if params.void_extent_ldr {
        for _ in 0..4 {
            bs.read_bits(13);
        }
        let r = (bs.read_bits(16) as u16 >> 8) as u8;
        let g = (bs.read_bits(16) as u16 >> 8) as u8;
        let b = (bs.read_bits(16) as u16 >> 8) as u8;
        let a = (bs.read_bits(16) as u16 >> 8) as u8;
        for p in out_pixels.iter_mut().take(block_w * block_h) {
            *p = [r, g, b, a];
        }
        return;
    }

    let num_partitions = (bs.read_bits(2) + 1) as usize;
    if num_partitions == 4 && params.dual_plane {
        return;
    }

    let mut cem = [0u32; 4];
    let partition_index: i32;
    let mut base_cem = 0u32;

    if num_partitions == 1 {
        cem[0] = bs.read_bits(4) as u32;
        partition_index = 0;
    } else {
        partition_index = bs.read_bits(10);
        base_cem = bs.read_bits(6) as u32;
    }

    let base_mode = base_cem & 3;
    let num_weight_bits = params.packed_bit_size();
    let mut rem_bits = 128 - num_weight_bits - (bs.pos as i32);

    let extra_cem_bits = if base_mode != 0 {
        match num_partitions {
            2 => 2,
            3 => 5,
            4 => 8,
            _ => 0,
        }
    } else {
        0
    };
    rem_bits -= extra_cem_bits;

    let plane_sel_bits = if params.dual_plane { 2 } else { 0 };
    rem_bits -= plane_sel_bits;

    let color_data_bits = rem_bits.max(0);
    let mut cem_stream = BitStream::from_bytes(&[0u8; 16]);
    while rem_bits > 0 {
        let nb = rem_bits.min(8) as usize;
        let v = bs.read_bits(nb);
        cem_stream.write_bits(v, nb);
        rem_bits -= 8;
    }

    let plane_indices = if plane_sel_bits > 0 {
        bs.read_bits(plane_sel_bits as usize)
    } else {
        0
    };

    if base_mode != 0 {
        let extra_cem = bs.read_bits(extra_cem_bits as usize) as u32;
        let mut temp_cem = ((extra_cem << 6) | base_cem) >> 2;
        let mut c_flags = [false; 4];
        let mut m_vals = [0u32; 4];
        for i in 0..num_partitions {
            c_flags[i] = (temp_cem & 1) != 0;
            temp_cem >>= 1;
        }
        for i in 0..num_partitions {
            m_vals[i] = temp_cem & 3;
            temp_cem >>= 2;
        }
        for i in 0..num_partitions {
            let mut mode = base_mode;
            if !c_flags[i] {
                mode = mode.saturating_sub(1);
            }
            cem[i] = (mode << 2) | m_vals[i];
        }
    } else if num_partitions > 1 {
        let m = base_cem >> 2;
        for i in 0..num_partitions {
            cem[i] = m;
        }
    }

    let mut color_values = [0i32; 32];
    decode_color_values(
        &mut color_values,
        &cem_stream.bits,
        &cem,
        num_partitions,
        color_data_bits,
    );

    let mut endpoints = [[AstcPixel::default(); 2]; 4];
    let mut cv_pos = 0usize;
    for i in 0..num_partitions {
        compute_endpoints(&mut endpoints[i], &color_values, cem[i], &mut cv_pos);
    }

    let mut weight_data = [0u8; 16];
    for i in 0..16 {
        weight_data[i] = block_bytes[15 - i].reverse_bits();
    }
    let clear_byte_start = ((num_weight_bits >> 3) + 1) as usize;
    if clear_byte_start > 0 && clear_byte_start <= 16 {
        let rem = (num_weight_bits % 8) as u32;
        weight_data[clear_byte_start - 1] &= ((1u16 << rem) - 1) as u8;
        for b in &mut weight_data[clear_byte_start..16] {
            *b = 0;
        }
    }

    let mut weight_vals = Vec::with_capacity(64);
    let mut wbs = BitStream::from_bytes(&weight_data);
    decode_integer_sequence(
        &mut weight_vals,
        &mut wbs,
        params.max_weight,
        params.num_weight_values(),
    );

    let mut weights = [[0i32; 144]; 2];
    unquantize_texel_weights(&mut weights, &weight_vals, &params, block_w, block_h);

    let is_small = (block_w * block_h) < 32;
    for j in 0..block_h {
        for i in 0..block_w {
            let part = select_2d_partition(
                partition_index,
                i as i32,
                j as i32,
                num_partitions as i32,
                is_small,
            )
            .min(num_partitions - 1);
            let mut pix = AstcPixel::default();
            for comp in 0..4 {
                let c0 = replicate(endpoints[part][0].get_comp(comp), 8, 16);
                let c1 = replicate(endpoints[part][1].get_comp(comp), 8, 16);
                let plane = if params.dual_plane && (((plane_indices + 1) & 3) as usize == comp) {
                    1
                } else {
                    0
                };
                let w = weights[plane][j * block_w + i];
                let fc = (c0 * (64 - w) + c1 * w + 32) / 64;
                let byte_val = if fc >= 65535 {
                    255
                } else {
                    ((255.0 * (fc as f64 / 65536.0) + 0.5) as i32).clamp(0, 255)
                };
                pix.set_comp(comp, byte_val);
            }
            out_pixels[j * block_w + i] = pix.to_rgba_bytes();
        }
    }
}

/// Decompress a full linear ASTC image of arbitrary footprint (`block_w` x `block_h`) into an `RgbaImage`.
pub fn decode_astc_linear_to_rgba8(
    linear: &[u8],
    width: u32,
    height: u32,
    block_w: usize,
    block_h: usize,
) -> RgbaImage {
    let w = width.max(1) as usize;
    let h = height.max(1) as usize;
    let bw = block_w.max(1);
    let bh = block_h.max(1);
    let blocks_x = (w + bw - 1) / bw;
    let blocks_y = (h + bh - 1) / bh;

    let mut img: RgbaImage = ImageBuffer::new(width.max(1), height.max(1));
    let mut block_pixels = [[0u8; 4]; 144];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let off = (by * blocks_x + bx) * 16;
            if off + 16 <= linear.len() {
                let arr: [u8; 16] = linear[off..off + 16].try_into().unwrap();
                decompress_astc_block(&arr, &mut block_pixels, bw, bh);
                for py in 0..bh {
                    let y = by * bh + py;
                    if y >= h {
                        break;
                    }
                    for px in 0..bw {
                        let x = bx * bw + px;
                        if x >= w {
                            break;
                        }
                        img.put_pixel(x as u32, y as u32, image::Rgba(block_pixels[py * bw + px]));
                    }
                }
            }
        }
    }
    img
}

// ============================================================================
// PURE-RUST ASTC LDR BLOCK ENCODER (ALL 14 FOOTPRINTS: 4x4 .. 12x12)
// ============================================================================

/// Compress a single `block_w x block_h` RGBA8 texel block into a 16-byte ASTC LDR block.
/// Uses Void-Extent LDR for uniform blocks and 1-partition 4x4 weight-grid `CEM=12` (RGBA Direct)
/// with 8-bit endpoints for general blocks, compatible with all 14 ASTC footprints (`4x4`..`12x12`).
pub fn compress_astc_block(
    pixels: &[[u8; 4]; 144],
    block_w: usize,
    block_h: usize,
) -> [u8; 16] {
    let bw = block_w.clamp(4, 12);
    let bh = block_h.clamp(4, 12);
    let count = bw * bh;
    let first = pixels[0];
    let is_uniform = (1..count).all(|i| pixels[i] == first);

    if is_uniform {
        // Void-Extent LDR block:
        // bits 0..12 = 0xDFC (mode_bits = 0x5FC, bit 11 = 1)
        // bits 12..64 = all 1s (4 x 13-bit 0x1FFF void-extent coords)
        // bits 64..128 = 4 x u16 UNORM16 colors
        let mut bs = BitStream::from_bytes(&[0u8; 16]);
        bs.write_bits(0x5FC, 11);
        bs.write_bits(1, 1);
        for _ in 0..4 {
            bs.write_bits(0x1FFF, 13);
        }
        bs.write_bits((first[0] as i32) * 257, 16);
        bs.write_bits((first[1] as i32) * 257, 16);
        bs.write_bits((first[2] as i32) * 257, 16);
        bs.write_bits((first[3] as i32) * 257, 16);
        return bs.bits;
    }

    // Find bounding-box endpoints [min_rgba, max_rgba]
    let mut min_c = [255u8; 4];
    let mut max_c = [0u8; 4];
    for p in pixels.iter().take(count) {
        for c in 0..4 {
            min_c[c] = min_c[c].min(p[c]);
            max_c[c] = max_c[c].max(p[c]);
        }
    }

    // Ensure CEM=12 non-blue-contracted ordering: r1 + g1 + b1 >= r0 + g0 + b0
    let sum0 = min_c[0] as u32 + min_c[1] as u32 + min_c[2] as u32;
    let sum1 = max_c[0] as u32 + max_c[1] as u32 + max_c[2] as u32;
    let (ep0, ep1) = if sum1 >= sum0 {
        (min_c, max_c)
    } else {
        (max_c, min_c)
    };

    // Mode 0x042: layout 0, 4x4 weight grid, max_weight = 3 (2 bits/weight JustBits = 32 bits)
    // num_partitions = 1 (2 bits = 0), CEM = 12 (4 bits = 0b1100)
    // Color bits = 128 - 32 - 17 = 79 bits -> 8 values with range 255 = JustBits(8)
    let mut bs = BitStream::from_bytes(&[0u8; 16]);
    bs.write_bits(0x042, 11);
    bs.write_bits(0, 2); // 1 partition
    bs.write_bits(12, 4); // CEM 12 (LDR RGBA Direct)

    // Write 8 color values: r0, r1, g0, g1, b0, b1, a0, a1 (8 bits each)
    let color_vals = [
        ep0[0], ep1[0], ep0[1], ep1[1], ep0[2], ep1[2], ep0[3], ep1[3],
    ];
    for &cv in &color_vals {
        bs.write_bits(cv as i32, 8);
    }

    // Compute 4x4 weight grid values (each in 0..=3) by sampling block texels at grid coords
    let dr = (ep1[0] as i32) - (ep0[0] as i32);
    let dg = (ep1[1] as i32) - (ep0[1] as i32);
    let db = (ep1[2] as i32) - (ep0[2] as i32);
    let da = (ep1[3] as i32) - (ep0[3] as i32);
    let len_sq = (dr * dr + dg * dg + db * db + da * da).max(1);

    let mut wbs = BitStream::from_bytes(&[0u8; 16]);
    for gy in 0..4usize {
        let py = ((gy * (bh - 1) + 1) / 3).min(bh - 1);
        for gx in 0..4usize {
            let px = ((gx * (bw - 1) + 1) / 3).min(bw - 1);
            let p = pixels[py * bw + px];
            let pr = (p[0] as i32) - (ep0[0] as i32);
            let pg = (p[1] as i32) - (ep0[1] as i32);
            let pb = (p[2] as i32) - (ep0[2] as i32);
            let pa = (p[3] as i32) - (ep0[3] as i32);
            let dot = pr * dr + pg * dg + pb * db + pa * da;
            let q = ((dot * 3 + (len_sq / 2)) / len_sq).clamp(0, 3);
            wbs.write_bits(q, 2);
        }
    }

    // Write reversed weight bits into the top 4 bytes (bytes 12..16)
    let mut out = bs.bits;
    for i in 0..4 {
        out[15 - i] = wbs.bits[i].reverse_bits();
    }
    out
}

/// Encode an `RgbaImage` into linear ASTC LDR blocks for footprint `(block_w, block_h)`.
pub fn encode_rgba8_to_astc_linear(
    img: &RgbaImage,
    block_w: usize,
    block_h: usize,
) -> Vec<u8> {
    let w = img.width().max(1) as usize;
    let h = img.height().max(1) as usize;
    let bw = block_w.clamp(4, 12);
    let bh = block_h.clamp(4, 12);
    let blocks_x = (w + bw - 1) / bw;
    let blocks_y = (h + bh - 1) / bh;

    let mut out = Vec::with_capacity(blocks_x * blocks_y * 16);
    let mut block_pixels = [[0u8; 4]; 144];

    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            for py in 0..bh {
                let y = (by * bh + py).min(h - 1);
                for px in 0..bw {
                    let x = (bx * bw + px).min(w - 1);
                    block_pixels[py * bw + px] = img.get_pixel(x as u32, y as u32).0;
                }
            }
            out.extend_from_slice(&compress_astc_block(&block_pixels, bw, bh));
        }
    }
    out
}

// ============================================================================
// PURE-RUST BC7 & BC6H BLOCK ENCODERS AND DECODERS
// ============================================================================

/// Encode a 4x4 RGBA8 block using BC7 Mode 6 (7-bit RGBA endpoints + 1-bit P-bits + 4-bit indices).
pub fn encode_bc7_block_mode6(pixels: &[[u8; 4]; 16]) -> [u8; 16] {
    let mut min_c = [255u8; 4];
    let mut max_c = [0u8; 4];
    for p in pixels {
        for c in 0..4 {
            min_c[c] = min_c[c].min(p[c]);
            max_c[c] = max_c[c].max(p[c]);
        }
    }

    let mut ep0 = min_c;
    let mut ep1 = max_c;

    let dr = (ep1[0] as i32) - (ep0[0] as i32);
    let dg = (ep1[1] as i32) - (ep0[1] as i32);
    let db = (ep1[2] as i32) - (ep0[2] as i32);
    let da = (ep1[3] as i32) - (ep0[3] as i32);
    let len_sq = (dr * dr + dg * dg + db * db + da * da).max(1);

    let mut indices = [0u8; 16];
    for (i, p) in pixels.iter().enumerate() {
        let pr = (p[0] as i32) - (ep0[0] as i32);
        let pg = (p[1] as i32) - (ep0[1] as i32);
        let pb = (p[2] as i32) - (ep0[2] as i32);
        let pa = (p[3] as i32) - (ep0[3] as i32);
        let dot = pr * dr + pg * dg + pb * db + pa * da;
        indices[i] = ((dot * 15 + (len_sq / 2)) / len_sq).clamp(0, 15) as u8;
    }

    // Anchor index 0 must have MSB == 0 (< 8). If >= 8, swap endpoints and invert indices.
    if indices[0] >= 8 {
        std::mem::swap(&mut ep0, &mut ep1);
        for idx in &mut indices {
            *idx = 15 - *idx;
        }
    }

    let mut bs = BitStream::from_bytes(&[0u8; 16]);
    // Mode 6 header: 6 zeros followed by a 1 (7 bits: 0x40)
    bs.write_bits(0x40, 7);
    // R0, R1, G0, G1, B0, B1, A0, A1 high 7 bits
    for c in 0..4 {
        bs.write_bits((ep0[c] >> 1) as i32, 7);
        bs.write_bits((ep1[c] >> 1) as i32, 7);
    }
    // P0, P1 (low bit of average of channels)
    let p0 = ((ep0[0] & 1) + (ep0[1] & 1) + (ep0[2] & 1) + (ep0[3] & 1) >= 2) as i32;
    let p1 = ((ep1[0] & 1) + (ep1[1] & 1) + (ep1[2] & 1) + (ep1[3] & 1) >= 2) as i32;
    bs.write_bits(p0, 1);
    bs.write_bits(p1, 1);
    // Anchor index 0 (3 bits) + indices 1..15 (4 bits each)
    bs.write_bits((indices[0] & 0x7) as i32, 3);
    for &idx in &indices[1..] {
        bs.write_bits((idx & 0xF) as i32, 4);
    }
    bs.bits
}

/// Encode an `RgbaImage` into linear BC7 blocks (16 bytes per 4x4 block).
pub fn encode_rgba8_to_bc7_linear(img: &RgbaImage) -> Vec<u8> {
    let w = img.width().max(1) as usize;
    let h = img.height().max(1) as usize;
    let bx_count = (w + 3) / 4;
    let by_count = (h + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 16);
    for by in 0..by_count {
        for bx in 0..bx_count {
            let mut blk = [[0u8; 4]; 16];
            for py in 0..4 {
                let y = (by * 4 + py).min(h - 1);
                for px in 0..4 {
                    let x = (bx * 4 + px).min(w - 1);
                    blk[py * 4 + px] = img.get_pixel(x as u32, y as u32).0;
                }
            }
            out.extend_from_slice(&encode_bc7_block_mode6(&blk));
        }
    }
    out
}

/// Decode a single 16-byte BC7 block into 16 RGBA8 pixels (supports Modes 0..7).
pub fn decode_bc7_block(block: &[u8]) -> [[u8; 4]; 16] {
    if block.len() < 16 {
        return [[0, 0, 0, 255]; 16];
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&block[0..16]);
    let mut bs = BitStream::from_bytes(&arr);

    let mut mode = 0usize;
    while mode < 8 && bs.read_bits(1) == 0 {
        mode += 1;
    }
    if mode >= 8 {
        return [[0, 0, 0, 0]; 16];
    }

    const WEIGHTS_2: [i32; 4] = [0, 21, 43, 64];
    const WEIGHTS_3: [i32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
    const WEIGHTS_4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

    // Fast exact path for Mode 6 (most common single-subset RGBA mode)
    if mode == 6 {
        let mut ep0 = [0i32; 4];
        let mut ep1 = [0i32; 4];
        for c in 0..4 {
            ep0[c] = bs.read_bits(7);
            ep1[c] = bs.read_bits(7);
        }
        let p0 = bs.read_bits(1);
        let p1 = bs.read_bits(1);
        for c in 0..4 {
            ep0[c] = (ep0[c] << 1) | p0;
            ep1[c] = (ep1[c] << 1) | p1;
        }
        let mut out = [[0u8; 4]; 16];
        for (i, dst) in out.iter_mut().enumerate() {
            let idx = if i == 0 {
                bs.read_bits(3) as usize
            } else {
                bs.read_bits(4) as usize
            };
            let w = WEIGHTS_4[idx & 15];
            for c in 0..4 {
                dst[c] = (((64 - w) * ep0[c] + w * ep1[c] + 32) >> 6).clamp(0, 255) as u8;
            }
        }
        return out;
    }

    // General BC7 decoder for Modes 0..5, 7
    let subsets = match mode {
        0 | 2 => 3usize,
        1 | 3 | 7 => 2usize,
        _ => 1usize,
    };
    let partition = if subsets > 1 {
        bs.read_bits(if mode == 0 { 4 } else { 6 }) as usize
    } else {
        0
    };
    let rotation = if mode == 4 || mode == 5 {
        bs.read_bits(2) as usize
    } else {
        0
    };
    let idx_mode = if mode == 4 { bs.read_bits(1) as usize } else { 0 };

    let color_bits = match mode {
        0 => 4,
        1 => 6,
        2 | 4 | 7 => 5,
        3 | 5 | 6 => 7,
        _ => 7,
    };
    let alpha_bits = match mode {
        4 => 6,
        5 => 8,
        6 => 7,
        7 => 5,
        _ => 0,
    };

    let mut ep = [[[0i32; 4]; 2]; 3];
    for ch in 0..3 {
        for s in 0..subsets {
            for e in 0..2 {
                ep[s][e][ch] = bs.read_bits(color_bits);
            }
        }
    }
    if alpha_bits > 0 {
        for s in 0..subsets {
            for e in 0..2 {
                ep[s][e][3] = bs.read_bits(alpha_bits);
            }
        }
    } else {
        for s in 0..subsets {
            ep[s][0][3] = 255;
            ep[s][1][3] = 255;
        }
    }

    // P-bits
    match mode {
        0 | 3 | 7 => {
            for s in 0..subsets {
                for e in 0..2 {
                    let p = bs.read_bits(1);
                    for ch in 0..3 {
                        ep[s][e][ch] = (ep[s][e][ch] << 1) | p;
                    }
                    if alpha_bits > 0 {
                        ep[s][e][3] = (ep[s][e][3] << 1) | p;
                    }
                }
            }
        }
        1 => {
            for s in 0..subsets {
                let p = bs.read_bits(1);
                for e in 0..2 {
                    for ch in 0..3 {
                        ep[s][e][ch] = (ep[s][e][ch] << 1) | p;
                    }
                }
            }
        }
        _ => {}
    }

    let c_prec = color_bits + matches!(mode, 0 | 1 | 3 | 7) as usize;
    let a_prec = alpha_bits + matches!(mode, 7) as usize;
    for s in 0..subsets {
        for e in 0..2 {
            for ch in 0..3 {
                ep[s][e][ch] = replicate(ep[s][e][ch], c_prec as i32, 8);
            }
            if alpha_bits > 0 {
                ep[s][e][3] = replicate(ep[s][e][3], a_prec as i32, 8);
            }
        }
    }

    const BC7_PART2: [u32; 64] = [
        0xCCCC, 0x8888, 0xEEEE, 0xECC8, 0xC880, 0xFEEC, 0xFEC8, 0xEC80,
        0xC800, 0xFFEC, 0xFE80, 0xE800, 0xFFE8, 0xFF00, 0xFFF0, 0xF000,
        0xF710, 0x008E, 0x7100, 0x08CE, 0x008C, 0x7310, 0x3100, 0x8CCE,
        0x088C, 0x3110, 0x6666, 0x366C, 0x17E8, 0x0FF0, 0x718E, 0x399C,
        0xAAAA, 0xF0F0, 0x5A5A, 0x33CC, 0x3C3C, 0x55AA, 0x9696, 0xA55A,
        0x73CE, 0x13C8, 0x324C, 0x3BDC, 0x6996, 0xC33C, 0x9966, 0x0660,
        0x0272, 0x04E4, 0x4E40, 0x2720, 0xC936, 0x936C, 0x39C6, 0x639C,
        0x9336, 0x9CC6, 0x817E, 0xE718, 0xCCF0, 0x0FCC, 0x7744, 0xEE22,
    ];
    const BC7_PART3: [u32; 64] = [
        0xAA685050, 0x6A5A5040, 0x5A5A4200, 0x5450A0A8, 0xA5A50000, 0xA0A05050, 0x5555A0A0, 0x5A5A5050,
        0xAA550000, 0xAA555500, 0xAAAA5500, 0x90909090, 0x94949494, 0xA4A4A4A4, 0xA9A59450, 0x2A0A4250,
        0xA5945040, 0x0A425054, 0xA5A5A500, 0x55A0A0A0, 0xA8A85454, 0x6A6A4040, 0xA4A45000, 0x1A1A0500,
        0x0050A4A4, 0xAAA59090, 0x14696914, 0x69691400, 0xA08585A0, 0xAA821414, 0x50A4A450, 0x6A5A0200,
        0xA9A58000, 0x5090A0A8, 0xA8A09050, 0x24242424, 0x00AA5500, 0x24924924, 0x24499224, 0x50A50A50,
        0x500AA550, 0xAAAA4444, 0x66660000, 0xA5A0A5A0, 0x50A050A0, 0x69286928, 0x44AAAA44, 0x66666600,
        0xAA444444, 0x54A854A8, 0x95809580, 0x96969600, 0xA85454A8, 0x80959580, 0xAA141414, 0x96960000,
        0xAAAA1414, 0xA05050A0, 0xA0A5A5A0, 0x96000000, 0x40804080, 0xA9A8A9A8, 0xAAAAAA44, 0x2A4A5254,
    ];
    const BC7_ANCHOR2: [u8; 64] = [
        15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
        15,  2,  8,  2,  2,  8,  8, 15,  2,  8,  2,  2,  8,  8,  2,  2,
        15, 15,  6,  8,  2,  8, 15, 15,  2,  8,  2,  2,  2, 15, 15,  6,
         6,  2,  6,  8, 15, 15,  2,  2, 15, 15, 15, 15, 15,  2,  2, 15,
    ];
    const BC7_ANCHOR3_A: [u8; 64] = [
         3,  3, 15, 15,  8,  3, 15, 15,  8,  8,  6,  6,  6,  5,  3,  3,
         3,  3,  8, 15,  3,  3,  6, 10,  5,  8,  8,  6,  8,  5, 15, 15,
         8, 15,  3,  5,  6, 10,  8, 15, 15,  3, 15,  5, 15, 15, 15, 15,
         3, 15,  5,  5,  5,  8,  5, 10,  5, 10,  8, 13, 15, 12,  3,  3,
    ];
    const BC7_ANCHOR3_B: [u8; 64] = [
        15,  8,  8,  3, 15, 15,  3,  8, 15, 15, 15, 15, 15, 15, 15,  8,
        15,  8, 15,  3, 15,  8, 15,  8,  3, 15,  6, 10, 15, 15, 10,  8,
        15,  3, 15, 10, 10,  8,  9, 10,  6, 15,  8, 15,  3,  6,  6,  8,
        15,  3, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,  3, 15, 15,  8,
    ];

    let idx_bits = match mode {
        0 | 1 => 3usize,
        2 | 3 | 5 | 7 => 2usize,
        4 => if idx_mode == 0 { 2usize } else { 3usize },
        _ => 4usize,
    };
    let weights_lut: &[i32] = match idx_bits {
        3 => &WEIGHTS_3,
        4 => &WEIGHTS_4,
        _ => &WEIGHTS_2,
    };

    let part_idx = partition & 63;
    let mut primary_weights = [0i32; 16];
    let mut out = [[0u8; 4]; 16];
    for (i, dst) in out.iter_mut().enumerate() {
        let subset = if subsets == 1 {
            0usize
        } else if subsets == 2 {
            ((BC7_PART2[part_idx] >> i) & 1) as usize
        } else {
            ((BC7_PART3[part_idx] >> (i * 2)) & 3).min(2) as usize
        };
        let is_anchor = i == 0
            || (subsets == 2 && i == BC7_ANCHOR2[part_idx] as usize)
            || (subsets == 3
                && (i == BC7_ANCHOR3_A[part_idx] as usize
                    || i == BC7_ANCHOR3_B[part_idx] as usize));
        let nb = if is_anchor { idx_bits - 1 } else { idx_bits };
        let idx = (bs.read_bits(nb) as usize).min(weights_lut.len() - 1);
        let w = weights_lut[idx];
        primary_weights[i] = w;
        if mode != 4 {
            for ch in 0..4 {
                dst[ch] = (((64 - w) * ep[subset][0][ch] + w * ep[subset][1][ch] + 32) >> 6)
                    .clamp(0, 255) as u8;
            }
        }
    }
    if mode == 4 || mode == 5 {
        let idx2_bits = if mode == 4 && idx_mode == 1 { 2usize } else { 3usize };
        let w2_lut: &[i32] = if idx2_bits == 2 { &WEIGHTS_2 } else { &WEIGHTS_3 };
        for (i, dst) in out.iter_mut().enumerate() {
            let nb = if i == 0 { idx2_bits - 1 } else { idx2_bits };
            let idx = (bs.read_bits(nb) as usize).min(w2_lut.len() - 1);
            let w2 = w2_lut[idx];
            if mode == 4 {
                let (w_rgb, w_a) = if idx_mode == 0 {
                    (primary_weights[i], w2)
                } else {
                    (w2, primary_weights[i])
                };
                for ch in 0..3 {
                    dst[ch] = (((64 - w_rgb) * ep[0][0][ch] + w_rgb * ep[0][1][ch] + 32) >> 6)
                        .clamp(0, 255) as u8;
                }
                dst[3] = (((64 - w_a) * ep[0][0][3] + w_a * ep[0][1][3] + 32) >> 6)
                    .clamp(0, 255) as u8;
            } else {
                dst[3] = (((64 - w2) * ep[0][0][3] + w2 * ep[0][1][3] + 32) >> 6)
                    .clamp(0, 255) as u8;
            }
            if rotation > 0 && rotation <= 3 {
                dst.swap(rotation - 1, 3);
            }
        }
    }
    out
}

/// Encode an `RgbaImage` into linear BC6H (Mode 3: 10-bit RGB endpoints + 4-bit indices) blocks.
pub fn encode_rgba8_to_bc6h_linear(img: &RgbaImage, _signed: bool) -> Vec<u8> {
    let w = img.width().max(1) as usize;
    let h = img.height().max(1) as usize;
    let bx_count = (w + 3) / 4;
    let by_count = (h + 3) / 4;
    let mut out = Vec::with_capacity(bx_count * by_count * 16);

    for by in 0..by_count {
        for bx in 0..bx_count {
            let mut blk = [[0u8; 4]; 16];
            let mut min_c = [255u8; 3];
            let mut max_c = [0u8; 3];
            for py in 0..4 {
                let y = (by * 4 + py).min(h - 1);
                for px in 0..4 {
                    let x = (bx * 4 + px).min(w - 1);
                    let p = img.get_pixel(x as u32, y as u32).0;
                    blk[py * 4 + px] = p;
                    for c in 0..3 {
                        min_c[c] = min_c[c].min(p[c]);
                        max_c[c] = max_c[c].max(p[c]);
                    }
                }
            }

            let mut ep0 = min_c;
            let mut ep1 = max_c;
            let dr = (ep1[0] as i32) - (ep0[0] as i32);
            let dg = (ep1[1] as i32) - (ep0[1] as i32);
            let db = (ep1[2] as i32) - (ep0[2] as i32);
            let len_sq = (dr * dr + dg * dg + db * db).max(1);

            let mut indices = [0u8; 16];
            for (i, p) in blk.iter().enumerate() {
                let pr = (p[0] as i32) - (ep0[0] as i32);
                let pg = (p[1] as i32) - (ep0[1] as i32);
                let pb = (p[2] as i32) - (ep0[2] as i32);
                let dot = pr * dr + pg * dg + pb * db;
                indices[i] = ((dot * 15 + (len_sq / 2)) / len_sq).clamp(0, 15) as u8;
            }
            if indices[0] >= 8 {
                std::mem::swap(&mut ep0, &mut ep1);
                for idx in &mut indices {
                    *idx = 15 - *idx;
                }
            }

            // Mode 3 (0b00011, 5 bits) + 6 x 10-bit RGB endpoints (60 bits) + 63 bits of indices = 128 bits
            let mut bs = BitStream::from_bytes(&[0u8; 16]);
            bs.write_bits(0x03, 5);
            for c in 0..3 {
                bs.write_bits(((ep0[c] as i32) << 2) | ((ep0[c] as i32) >> 6), 10);
            }
            for c in 0..3 {
                bs.write_bits(((ep1[c] as i32) << 2) | ((ep1[c] as i32) >> 6), 10);
            }
            bs.write_bits((indices[0] & 0x7) as i32, 3);
            for &idx in &indices[1..] {
                bs.write_bits((idx & 0xF) as i32, 4);
            }
            out.extend_from_slice(&bs.bits);
        }
    }
    out
}

#[inline]
fn bc6h_bits(v: u128, start: u32, len: u32) -> i32 {
    ((v >> start) & ((1u128 << len) - 1)) as i32
}

#[inline]
fn bc6h_bit(v: u128, pos: u32) -> i32 {
    ((v >> pos) & 1) as i32
}

#[inline]
fn bc6h_bits_rev(v: u128, start: u32, len: u32) -> i32 {
    let mut out = 0i32;
    for i in 0..len {
        out |= bc6h_bit(v, start + i) << (len - 1 - i);
    }
    out
}

#[inline]
fn bc6h_sign_extend(val: i32, bits: u32) -> i32 {
    let shift = 32 - bits;
    (val << shift) >> shift
}

#[inline]
fn bc6h_unquantize(comp: i32, bits: u32, signed: bool) -> i32 {
    if !signed {
        if bits >= 15 {
            comp
        } else if comp == 0 {
            0
        } else if comp == ((1 << bits) - 1) {
            0xFFFF
        } else {
            ((comp << 16) + 0x8000) >> bits
        }
    } else if bits >= 16 {
        comp
    } else {
        let (neg, c) = if comp < 0 { (true, -comp) } else { (false, comp) };
        let unq = if c == 0 {
            0
        } else if c >= ((1 << (bits - 1)) - 1) {
            0x7FFF
        } else {
            ((c << 15) + 0x4000) >> (bits - 1)
        };
        if neg { -unq } else { unq }
    }
}

#[inline]
fn bc6h_finish_unquantize(comp: i32, signed: bool) -> u16 {
    if !signed {
        ((comp * 31) >> 6) as u16
    } else {
        let scaled = if comp < 0 {
            -(((-comp) * 31) >> 5)
        } else {
            (comp * 31) >> 5
        };
        if scaled < 0 {
            (0x8000 | (-scaled)) as u16
        } else {
            scaled as u16
        }
    }
}

/// Decode a single 16-byte BC6H block (UF16 or SF16) into 16 RGBA8 pixels
/// following the Direct3D 11 / Khronos BPTC_FLOAT specification across all 14 modes.
pub fn decode_bc6h_block(block: &[u8], signed: bool) -> [[u8; 4]; 16] {
    if block.len() < 16 {
        return [[0, 0, 0, 255]; 16];
    }
    let mut arr = [0u8; 16];
    arr.copy_from_slice(&block[0..16]);
    let b = u128::from_le_bytes(arr);

    const BC6H_PART2: [u16; 32] = [
        0xCCCC, 0x8888, 0xEEEE, 0xECC8, 0xC880, 0xFEEC, 0xFEC8, 0xEC80,
        0xC800, 0xFFEC, 0xFE80, 0xE800, 0xFFE8, 0xFF00, 0xFFF0, 0xF000,
        0xF710, 0x008E, 0x7100, 0x08CE, 0x008C, 0x7310, 0x3100, 0x8CCE,
        0x088C, 0x3110, 0x6666, 0x366C, 0x17E8, 0x0FF0, 0x718E, 0x399C,
    ];
    const BC6H_ANCHOR2: [u8; 32] = [
        15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15, 15,
        15,  2,  8,  2,  2,  8,  8, 15,  2,  8,  2,  2,  8,  8,  2,  2,
    ];
    const WEIGHTS_3: [i32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
    const WEIGHTS_4: [i32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

    let mode2 = bc6h_bits(b, 0, 2);
    let mode5 = bc6h_bits(b, 0, 5);

    // ep[subset][endpoint_a_or_b][channel_rgb]
    let mut ep = [[[0i32; 3]; 2]; 2];
    let two_subsets: bool;
    let transformed: bool;
    let ep_bits: u32;
    let delta_bits: [u32; 3];

    if mode2 == 0b00 {
        // Mode 1: 2 subsets, transformed, (10.555, 10.555, 10.555)
        two_subsets = true;
        transformed = true;
        ep_bits = 10;
        delta_bits = [5, 5, 5];
        ep[0][0][0] = bc6h_bits(b, 5, 10);
        ep[0][0][1] = bc6h_bits(b, 15, 10);
        ep[0][0][2] = bc6h_bits(b, 25, 10);
        ep[0][1][0] = bc6h_bits(b, 35, 5);
        ep[0][1][1] = bc6h_bits(b, 45, 5);
        ep[0][1][2] = bc6h_bits(b, 55, 5);
        ep[1][0][0] = bc6h_bits(b, 65, 5);
        ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 2) << 4);
        ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 3) << 4);
        ep[1][1][0] = bc6h_bits(b, 71, 5);
        ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 40) << 4);
        ep[1][1][2] = bc6h_bit(b, 50)
            | (bc6h_bit(b, 60) << 1)
            | (bc6h_bit(b, 70) << 2)
            | (bc6h_bit(b, 76) << 3)
            | (bc6h_bit(b, 4) << 4);
    } else if mode2 == 0b01 {
        // Mode 2: 2 subsets, transformed, (7666, 7666, 7666)
        two_subsets = true;
        transformed = true;
        ep_bits = 7;
        delta_bits = [6, 6, 6];
        ep[0][0][0] = bc6h_bits(b, 5, 7);
        ep[0][0][1] = bc6h_bits(b, 15, 7);
        ep[0][0][2] = bc6h_bits(b, 25, 7);
        ep[0][1][0] = bc6h_bits(b, 35, 6);
        ep[0][1][1] = bc6h_bits(b, 45, 6);
        ep[0][1][2] = bc6h_bits(b, 55, 6);
        ep[1][0][0] = bc6h_bits(b, 65, 6);
        ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4) | (bc6h_bit(b, 2) << 5);
        ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4) | (bc6h_bit(b, 22) << 5);
        ep[1][1][0] = bc6h_bits(b, 71, 6);
        ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 3) << 4) | (bc6h_bit(b, 4) << 5);
        ep[1][1][2] = bc6h_bit(b, 12)
            | (bc6h_bit(b, 13) << 1)
            | (bc6h_bit(b, 23) << 2)
            | (bc6h_bit(b, 32) << 3)
            | (bc6h_bit(b, 34) << 4)
            | (bc6h_bit(b, 33) << 5);
    } else {
        match mode5 {
            0b00010 => {
                // Mode 3: 2 subsets, transformed, (11.555, 11.444, 11.444)
                two_subsets = true;
                transformed = true;
                ep_bits = 11;
                delta_bits = [5, 4, 4];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bit(b, 40) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bit(b, 49) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bit(b, 59) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 5);
                ep[0][1][1] = bc6h_bits(b, 45, 4);
                ep[0][1][2] = bc6h_bits(b, 55, 4);
                ep[1][0][0] = bc6h_bits(b, 65, 5);
                ep[1][0][1] = bc6h_bits(b, 41, 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4);
                ep[1][1][0] = bc6h_bits(b, 71, 5);
                ep[1][1][1] = bc6h_bits(b, 51, 4);
                ep[1][1][2] = bc6h_bit(b, 50)
                    | (bc6h_bit(b, 60) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3);
            }
            0b00110 => {
                // Mode 4: 2 subsets, transformed, (11.444, 11.555, 11.444)
                two_subsets = true;
                transformed = true;
                ep_bits = 11;
                delta_bits = [4, 5, 4];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bit(b, 39) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bit(b, 50) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bit(b, 59) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 4);
                ep[0][1][1] = bc6h_bits(b, 45, 5);
                ep[0][1][2] = bc6h_bits(b, 55, 4);
                ep[1][0][0] = bc6h_bits(b, 65, 4);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 75) << 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4);
                ep[1][1][0] = bc6h_bits(b, 71, 4);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 40) << 4);
                ep[1][1][2] = bc6h_bit(b, 69)
                    | (bc6h_bit(b, 60) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3);
            }
            0b01010 => {
                // Mode 5: 2 subsets, transformed, (11.444, 11.444, 11.555)
                two_subsets = true;
                transformed = true;
                ep_bits = 11;
                delta_bits = [4, 4, 5];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bit(b, 39) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bit(b, 49) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bit(b, 60) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 4);
                ep[0][1][1] = bc6h_bits(b, 45, 4);
                ep[0][1][2] = bc6h_bits(b, 55, 5);
                ep[1][0][0] = bc6h_bits(b, 65, 4);
                ep[1][0][1] = bc6h_bits(b, 41, 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 40) << 4);
                ep[1][1][0] = bc6h_bits(b, 71, 4);
                ep[1][1][1] = bc6h_bits(b, 51, 4);
                ep[1][1][2] = bc6h_bit(b, 50)
                    | (bc6h_bit(b, 69) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3)
                    | (bc6h_bit(b, 75) << 4);
            }
            0b01110 => {
                // Mode 6: 2 subsets, transformed, (9555, 9555, 9555)
                two_subsets = true;
                transformed = true;
                ep_bits = 9;
                delta_bits = [5, 5, 5];
                ep[0][0][0] = bc6h_bits(b, 5, 9);
                ep[0][0][1] = bc6h_bits(b, 15, 9);
                ep[0][0][2] = bc6h_bits(b, 25, 9);
                ep[0][1][0] = bc6h_bits(b, 35, 5);
                ep[0][1][1] = bc6h_bits(b, 45, 5);
                ep[0][1][2] = bc6h_bits(b, 55, 5);
                ep[1][0][0] = bc6h_bits(b, 65, 5);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4);
                ep[1][1][0] = bc6h_bits(b, 71, 5);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 40) << 4);
                ep[1][1][2] = bc6h_bit(b, 50)
                    | (bc6h_bit(b, 60) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3)
                    | (bc6h_bit(b, 34) << 4);
            }
            0b10010 => {
                // Mode 7: 2 subsets, transformed, (8666, 8555, 8555)
                two_subsets = true;
                transformed = true;
                ep_bits = 8;
                delta_bits = [6, 5, 5];
                ep[0][0][0] = bc6h_bits(b, 5, 8);
                ep[0][0][1] = bc6h_bits(b, 15, 8);
                ep[0][0][2] = bc6h_bits(b, 25, 8);
                ep[0][1][0] = bc6h_bits(b, 35, 6);
                ep[0][1][1] = bc6h_bits(b, 45, 5);
                ep[0][1][2] = bc6h_bits(b, 55, 5);
                ep[1][0][0] = bc6h_bits(b, 65, 6);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4);
                ep[1][1][0] = bc6h_bits(b, 71, 6);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 13) << 4);
                ep[1][1][2] = bc6h_bit(b, 50)
                    | (bc6h_bit(b, 60) << 1)
                    | (bc6h_bit(b, 23) << 2)
                    | (bc6h_bit(b, 33) << 3)
                    | (bc6h_bit(b, 34) << 4);
            }
            0b10110 => {
                // Mode 8: 2 subsets, transformed, (8555, 8666, 8555)
                two_subsets = true;
                transformed = true;
                ep_bits = 8;
                delta_bits = [5, 6, 5];
                ep[0][0][0] = bc6h_bits(b, 5, 8);
                ep[0][0][1] = bc6h_bits(b, 15, 8);
                ep[0][0][2] = bc6h_bits(b, 25, 8);
                ep[0][1][0] = bc6h_bits(b, 35, 5);
                ep[0][1][1] = bc6h_bits(b, 45, 6);
                ep[0][1][2] = bc6h_bits(b, 55, 5);
                ep[1][0][0] = bc6h_bits(b, 65, 5);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4) | (bc6h_bit(b, 23) << 5);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4);
                ep[1][1][0] = bc6h_bits(b, 71, 5);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 40) << 4) | (bc6h_bit(b, 33) << 5);
                ep[1][1][2] = bc6h_bit(b, 13)
                    | (bc6h_bit(b, 60) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3)
                    | (bc6h_bit(b, 34) << 4);
            }
            0b11010 => {
                // Mode 9: 2 subsets, transformed, (8555, 8555, 8666)
                two_subsets = true;
                transformed = true;
                ep_bits = 8;
                delta_bits = [5, 5, 6];
                ep[0][0][0] = bc6h_bits(b, 5, 8);
                ep[0][0][1] = bc6h_bits(b, 15, 8);
                ep[0][0][2] = bc6h_bits(b, 25, 8);
                ep[0][1][0] = bc6h_bits(b, 35, 5);
                ep[0][1][1] = bc6h_bits(b, 45, 5);
                ep[0][1][2] = bc6h_bits(b, 55, 6);
                ep[1][0][0] = bc6h_bits(b, 65, 5);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4) | (bc6h_bit(b, 23) << 5);
                ep[1][1][0] = bc6h_bits(b, 71, 5);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 40) << 4);
                ep[1][1][2] = bc6h_bit(b, 50)
                    | (bc6h_bit(b, 13) << 1)
                    | (bc6h_bit(b, 70) << 2)
                    | (bc6h_bit(b, 76) << 3)
                    | (bc6h_bit(b, 34) << 4)
                    | (bc6h_bit(b, 33) << 5);
            }
            0b11110 => {
                // Mode 10: 2 subsets, untransformed, (6666, 6666, 6666)
                two_subsets = true;
                transformed = false;
                ep_bits = 6;
                delta_bits = [6, 6, 6];
                ep[0][0][0] = bc6h_bits(b, 5, 6);
                ep[0][0][1] = bc6h_bits(b, 15, 6);
                ep[0][0][2] = bc6h_bits(b, 25, 6);
                ep[0][1][0] = bc6h_bits(b, 35, 6);
                ep[0][1][1] = bc6h_bits(b, 45, 6);
                ep[0][1][2] = bc6h_bits(b, 55, 6);
                ep[1][0][0] = bc6h_bits(b, 65, 6);
                ep[1][0][1] = bc6h_bits(b, 41, 4) | (bc6h_bit(b, 24) << 4) | (bc6h_bit(b, 21) << 5);
                ep[1][0][2] = bc6h_bits(b, 61, 4) | (bc6h_bit(b, 14) << 4) | (bc6h_bit(b, 22) << 5);
                ep[1][1][0] = bc6h_bits(b, 71, 6);
                ep[1][1][1] = bc6h_bits(b, 51, 4) | (bc6h_bit(b, 11) << 4) | (bc6h_bit(b, 31) << 5);
                ep[1][1][2] = bc6h_bit(b, 12)
                    | (bc6h_bit(b, 13) << 1)
                    | (bc6h_bit(b, 23) << 2)
                    | (bc6h_bit(b, 32) << 3)
                    | (bc6h_bit(b, 34) << 4)
                    | (bc6h_bit(b, 33) << 5);
            }
            0b00011 => {
                // Mode 11: 1 subset, untransformed, (10.10, 10.10, 10.10)
                two_subsets = false;
                transformed = false;
                ep_bits = 10;
                delta_bits = [10, 10, 10];
                ep[0][0][0] = bc6h_bits(b, 5, 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10);
                ep[0][1][0] = bc6h_bits(b, 35, 10);
                ep[0][1][1] = bc6h_bits(b, 45, 10);
                ep[0][1][2] = bc6h_bits(b, 55, 10);
            }
            0b00111 => {
                // Mode 12: 1 subset, transformed, (11.9, 11.9, 11.9)
                two_subsets = false;
                transformed = true;
                ep_bits = 11;
                delta_bits = [9, 9, 9];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bit(b, 44) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bit(b, 54) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bit(b, 64) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 9);
                ep[0][1][1] = bc6h_bits(b, 45, 9);
                ep[0][1][2] = bc6h_bits(b, 55, 9);
            }
            0b01011 => {
                // Mode 13: 1 subset, transformed, (12.8, 12.8, 12.8)
                two_subsets = false;
                transformed = true;
                ep_bits = 12;
                delta_bits = [8, 8, 8];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bits_rev(b, 43, 2) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bits_rev(b, 53, 2) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bits_rev(b, 63, 2) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 8);
                ep[0][1][1] = bc6h_bits(b, 45, 8);
                ep[0][1][2] = bc6h_bits(b, 55, 8);
            }
            0b01111 => {
                // Mode 14: 1 subset, transformed, (16.4, 16.4, 16.4)
                two_subsets = false;
                transformed = true;
                ep_bits = 16;
                delta_bits = [4, 4, 4];
                ep[0][0][0] = bc6h_bits(b, 5, 10) | (bc6h_bits_rev(b, 39, 6) << 10);
                ep[0][0][1] = bc6h_bits(b, 15, 10) | (bc6h_bits_rev(b, 49, 6) << 10);
                ep[0][0][2] = bc6h_bits(b, 25, 10) | (bc6h_bits_rev(b, 59, 6) << 10);
                ep[0][1][0] = bc6h_bits(b, 35, 4);
                ep[0][1][1] = bc6h_bits(b, 45, 4);
                ep[0][1][2] = bc6h_bits(b, 55, 4);
            }
            _ => {
                // Reserved modes (10011, 10111, 11011, 11111): return black with alpha 1.0
                return [[0, 0, 0, 255]; 16];
            }
        }
    }

    // Sign extension
    let num_subsets = if two_subsets { 2 } else { 1 };
    for c in 0..3 {
        if signed {
            ep[0][0][c] = bc6h_sign_extend(ep[0][0][c], ep_bits);
        }
        if transformed || signed {
            ep[0][1][c] = bc6h_sign_extend(ep[0][1][c], delta_bits[c]);
            if two_subsets {
                ep[1][0][c] = bc6h_sign_extend(ep[1][0][c], delta_bits[c]);
                ep[1][1][c] = bc6h_sign_extend(ep[1][1][c], delta_bits[c]);
            }
        }
    }

    // Inverse delta transform
    if transformed {
        let mask = (1i32 << ep_bits) - 1;
        for c in 0..3 {
            let base = ep[0][0][c];
            ep[0][1][c] = (ep[0][1][c] + base) & mask;
            if signed {
                ep[0][1][c] = bc6h_sign_extend(ep[0][1][c], ep_bits);
            }
            if two_subsets {
                ep[1][0][c] = (ep[1][0][c] + base) & mask;
                ep[1][1][c] = (ep[1][1][c] + base) & mask;
                if signed {
                    ep[1][0][c] = bc6h_sign_extend(ep[1][0][c], ep_bits);
                    ep[1][1][c] = bc6h_sign_extend(ep[1][1][c], ep_bits);
                }
            }
        }
    }

    // Unquantize endpoints
    for s in 0..num_subsets {
        for e in 0..2 {
            for c in 0..3 {
                ep[s][e][c] = bc6h_unquantize(ep[s][e][c], ep_bits, signed);
            }
        }
    }

    // Unpack palette indices & interpolate
    let partition = if two_subsets {
        (bc6h_bits(b, 77, 5) as usize) & 31
    } else {
        0
    };
    let part_mask = if two_subsets { BC6H_PART2[partition] } else { 0 };
    let anchor2 = if two_subsets { BC6H_ANCHOR2[partition] as usize } else { 0 };
    let mut bit_pos: u32 = if two_subsets { 82 } else { 65 };
    let idx_bits: u32 = if two_subsets { 3 } else { 4 };

    let mut out = [[0u8; 4]; 16];
    for (i, dst) in out.iter_mut().enumerate() {
        let subset = if two_subsets {
            ((part_mask >> i) & 1) as usize
        } else {
            0
        };
        let is_anchor = i == 0 || (two_subsets && i == anchor2);
        let nbits = if is_anchor { idx_bits - 1 } else { idx_bits };
        let idx = bc6h_bits(b, bit_pos, nbits) as usize;
        bit_pos += nbits;

        let w = if two_subsets {
            WEIGHTS_3[idx & 7]
        } else {
            WEIGHTS_4[idx & 15]
        };

        for c in 0..3 {
            let interp = (ep[subset][0][c] * (64 - w) + ep[subset][1][c] * w + 32) >> 6;
            let h16 = bc6h_finish_unquantize(interp, signed);
            let f = crate::io::F16(h16).to_f32();
            dst[c] = if f.is_nan() || f <= 0.0 {
                0
            } else {
                ((f / (1.0 + f)) * 255.0).round().clamp(0.0, 255.0) as u8
            };
        }
        dst[3] = 255;
    }
    out
}
