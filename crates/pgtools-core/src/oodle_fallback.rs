//! Pure-Rust open-source Oodle (`Kraken` / `Mermaid` / `Selkie` / `LZNA` — `ooz`) decompressor
//! fallback for `.pkz` archives when `oo2core_*_win64.dll` is not available on disk.
//!
//! Ported from the open-source `ooz` (`kraken.cpp`) reference implementation:
//! supports 256 KB Oodle quanta, uncompressed/memset/whole-match blocks, 3-stream interleaved
//! Huffman entropy decoding (`Type 0`, `Type 1`, `Type 2`), Kraken (`decoder_type == 6`) Mode 0
//! (delta literals) and Mode 1 (raw literals), and Mermaid/Selkie (`decoder_type == 10`) Mode 0/1.

use crate::{PgError, Result};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, Default)]
struct KrakenHeader {
    decoder_type: i32,
    restart_decoder: bool,
    uncompressed: bool,
    use_checksums: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct KrakenQuantumHeader {
    compressed_size: usize,
    checksum: u32,
    _flag1: u8,
    _flag2: u8,
    whole_match_distance: usize,
}

fn parse_header(p: &[u8]) -> Option<(KrakenHeader, usize)> {
    if p.len() < 2 {
        return None;
    }
    let b0 = p[0];
    if (b0 & 0x0F) != 0x0C || ((b0 >> 4) & 3) != 0 {
        return None;
    }
    let b1 = p[1];
    let decoder_type = (b1 & 0x3F) as i32;
    if !matches!(decoder_type, 6 | 10 | 5 | 11) {
        return None;
    }
    Some((
        KrakenHeader {
            decoder_type,
            restart_decoder: ((b0 >> 7) & 1) != 0,
            uncompressed: ((b0 >> 6) & 1) != 0,
            use_checksums: (b1 >> 7) != 0,
        },
        2,
    ))
}

fn parse_quantum_header(p: &[u8], use_checksum: bool) -> Option<(KrakenQuantumHeader, usize)> {
    if p.len() < 3 {
        return None;
    }
    let v = ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | (p[2] as u32);
    let size = v & 0x3FFFF;
    if size != 0x3FFFF {
        let compressed_size = (size + 1) as usize;
        let flag1 = ((v >> 18) & 1) as u8;
        let flag2 = ((v >> 19) & 1) as u8;
        if use_checksum {
            if p.len() < 6 {
                return None;
            }
            let checksum = ((p[3] as u32) << 16) | ((p[4] as u32) << 8) | (p[5] as u32);
            return Some((
                KrakenQuantumHeader {
                    compressed_size,
                    checksum,
                    _flag1: flag1,
                    _flag2: flag2,
                    whole_match_distance: 0,
                },
                6,
            ));
        } else {
            return Some((
                KrakenQuantumHeader {
                    compressed_size,
                    checksum: 0,
                    _flag1: flag1,
                    _flag2: flag2,
                    whole_match_distance: 0,
                },
                3,
            ));
        }
    }
    let special = v >> 18;
    if special == 1 {
        if p.len() < 4 {
            return None;
        }
        return Some((
            KrakenQuantumHeader {
                compressed_size: 0,
                checksum: p[3] as u32,
                _flag1: 0,
                _flag2: 0,
                whole_match_distance: 0,
            },
            4,
        ));
    }
    None
}

struct BitReader<'a> {
    buf: &'a [u8],
    p: isize,
    bits: u32,
    bitpos: i32,
}

impl<'a> BitReader<'a> {
    fn new_forward(buf: &'a [u8]) -> Self {
        Self {
            buf,
            p: 0,
            bits: 0,
            bitpos: 24,
        }
    }

    fn new_backward(buf: &'a [u8]) -> Self {
        Self {
            buf,
            p: buf.len() as isize,
            bits: 0,
            bitpos: 24,
        }
    }

    #[inline]
    fn refill(&mut self) {
        while self.bitpos > 0 {
            let b = if self.p >= 0 && (self.p as usize) < self.buf.len() {
                self.buf[self.p as usize] as u32
            } else {
                0
            };
            self.bits |= b << self.bitpos;
            self.bitpos -= 8;
            self.p += 1;
        }
    }

    #[inline]
    fn refill_backwards(&mut self) {
        while self.bitpos > 0 {
            self.p -= 1;
            let b = if self.p >= 0 && (self.p as usize) < self.buf.len() {
                self.buf[self.p as usize] as u32
            } else {
                0
            };
            self.bits |= b << self.bitpos;
            self.bitpos -= 8;
        }
    }

    #[inline]
    fn read_bit(&mut self) -> u32 {
        self.refill();
        let r = self.bits >> 31;
        self.bits <<= 1;
        self.bitpos += 1;
        r
    }

    #[inline]
    fn read_bits_no_refill(&mut self, n: i32) -> u32 {
        if n <= 0 {
            return 0;
        }
        let r = self.bits >> (32 - n);
        self.bits <<= n;
        self.bitpos += n;
        r
    }

    #[inline]
    fn read_gamma(&mut self) -> Option<i32> {
        let n = if self.bits != 0 {
            self.bits.leading_zeros() as i32
        } else {
            32
        };
        let n2 = 2 * n + 2;
        if n2 >= 24 {
            return None;
        }
        self.bitpos += n2;
        let r = (self.bits >> (32 - n2)) as i32;
        self.bits <<= n2;
        Some(r - 2)
    }

    #[inline]
    fn read_gamma_x(&mut self, forced: i32) -> Option<u32> {
        if self.bits != 0 {
            let n = self.bits.leading_zeros() as i32;
            if n >= 24 {
                return None;
            }
            let r = (self.bits >> (31 - n - forced)) + (((n - 1) as u32) << forced);
            let shift = n + forced + 1;
            self.bits <<= shift;
            self.bitpos += shift;
            Some(r)
        } else {
            Some(0)
        }
    }

    #[inline]
    fn read_distance(&mut self, v: u32) -> u32 {
        let rv = if v < 0xF0 {
            let n = (v >> 4) + 4;
            let w = (self.bits | 1).rotate_left(n);
            self.bitpos += n as i32;
            let m = (2u32 << n) - 1;
            self.bits = w & !m;
            ((w & m) << 4).wrapping_add(v & 0xF).wrapping_sub(248)
        } else {
            let n = v - 0xF0 + 4;
            let w = (self.bits | 1).rotate_left(n);
            self.bitpos += n as i32;
            let m = (2u32 << n) - 1;
            self.bits = w & !m;
            let mut rv = 8_322_816u32.wrapping_add((w & m) << 12);
            self.refill();
            rv = rv.wrapping_add(self.bits >> 20);
            self.bitpos += 12;
            self.bits <<= 12;
            rv
        };
        self.refill();
        rv
    }

    #[inline]
    fn read_distance_b(&mut self, v: u32) -> u32 {
        let rv = if v < 0xF0 {
            let n = (v >> 4) + 4;
            let w = (self.bits | 1).rotate_left(n);
            self.bitpos += n as i32;
            let m = (2u32 << n) - 1;
            self.bits = w & !m;
            ((w & m) << 4).wrapping_add(v & 0xF).wrapping_sub(248)
        } else {
            let n = v - 0xF0 + 4;
            let w = (self.bits | 1).rotate_left(n);
            self.bitpos += n as i32;
            let m = (2u32 << n) - 1;
            self.bits = w & !m;
            let mut rv = 8_322_816u32.wrapping_add((w & m) << 12);
            self.refill_backwards();
            rv = rv.wrapping_add(self.bits >> 20);
            self.bitpos += 12;
            self.bits <<= 12;
            rv
        };
        self.refill_backwards();
        rv
    }

    #[inline]
    fn read_length(&mut self) -> Option<u32> {
        if self.bits == 0 {
            return None;
        }
        let mut n = self.bits.leading_zeros() as i32;
        if n > 12 {
            return None;
        }
        self.bitpos += n;
        self.bits <<= n;
        self.refill();
        n += 7;
        self.bitpos += n;
        let rv = (self.bits >> (32 - n)).wrapping_sub(64);
        self.bits <<= n;
        self.refill();
        Some(rv)
    }

    #[inline]
    fn read_length_b(&mut self) -> Option<u32> {
        if self.bits == 0 {
            return None;
        }
        let mut n = self.bits.leading_zeros() as i32;
        if n > 12 {
            return None;
        }
        self.bitpos += n;
        self.bits <<= n;
        self.refill_backwards();
        n += 7;
        self.bitpos += n;
        let rv = (self.bits >> (32 - n)).wrapping_sub(64);
        self.bits <<= n;
        self.refill_backwards();
        Some(rv)
    }
}

#[inline]
fn log2_round_up(v: u32) -> i32 {
    if v > 1 {
        (32 - (v - 1).leading_zeros()) as i32
    } else {
        0
    }
}

struct HuffmanTree {
    num_syms: usize,
    max_code_len: usize,
    used_syms: usize,
    last_used_sym: usize,
    min_used_codelen: usize,
    max_used_codelen: usize,
    single_symbol: u8,
    codelen_count: [usize; 17],
    symlen: [u8; 258],
    c_syms: [u8; 516],
    bits_to_len: [u8; 2048],
    bits_to_sym: [u8; 2048],
}

impl HuffmanTree {
    fn new(num_syms: usize, max_code_len: usize) -> Self {
        Self {
            num_syms,
            max_code_len,
            used_syms: 0,
            last_used_sym: 0,
            min_used_codelen: 0,
            max_used_codelen: 0,
            single_symbol: 0,
            codelen_count: [0; 17],
            symlen: [0; 258],
            c_syms: [0; 516],
            bits_to_len: [0; 2048],
            bits_to_sym: [0; 2048],
        }
    }

    fn decode(&mut self, bits: &mut BitReader) -> bool {
        if bits.read_bit() != 0 {
            let symbits = log2_round_up(self.num_syms as u32);
            let mut sym = 0usize;
            let mut avg_bits_x4 = symbits * 4;
            let mut min_codelen = 32usize;
            let mut max_codelen = 0usize;
            let forced_bits = bits.read_bits_no_refill(2) as i32;
            let mut skip_initial_zeros = bits.read_bit() != 0;

            loop {
                if !skip_initial_zeros {
                    let Some(g) = bits.read_gamma() else {
                        return false;
                    };
                    let n = (g + 1) as usize;
                    if sym + n > self.num_syms {
                        return false;
                    }
                    for slot in &mut self.symlen[sym..sym + n] {
                        *slot = 0;
                    }
                    sym += n;
                    if sym >= self.num_syms {
                        break;
                    }
                    bits.refill();
                }
                skip_initial_zeros = false;

                let Some(g) = bits.read_gamma() else {
                    return false;
                };
                let mut n = (g + 1) as usize;
                if sym + n > self.num_syms {
                    return false;
                }
                bits.refill();
                while n > 0 {
                    let Some(p) = bits.read_gamma_x(forced_bits) else {
                        return false;
                    };
                    bits.refill();
                    let delta = (-((p & 1) as i32)) ^ ((p >> 1) as i32);
                    let codelen = delta + ((avg_bits_x4 + 2) >> 2);
                    if !(1..=16).contains(&codelen) {
                        return false;
                    }
                    let cl = codelen as usize;
                    self.used_syms += 1;
                    self.codelen_count[cl] += 1;
                    if cl < min_codelen {
                        min_codelen = cl;
                    }
                    if cl > max_codelen {
                        max_codelen = cl;
                    }
                    avg_bits_x4 = codelen + ((3 * avg_bits_x4 + 2) >> 2);
                    self.last_used_sym = sym;
                    self.symlen[sym] = cl as u8;
                    sym += 1;
                    n -= 1;
                }
                if sym >= self.num_syms {
                    break;
                }
            }
            self.min_used_codelen = min_codelen;
            self.max_used_codelen = max_codelen;
            true
        } else {
            let sym_bits = log2_round_up(self.num_syms as u32);
            self.used_syms = bits.read_bits_no_refill(sym_bits) as usize;
            if self.used_syms > self.num_syms {
                return false;
            }
            if self.used_syms == 0 {
                return true;
            }
            if self.used_syms != 1 {
                let mut min_codelen = 32usize;
                let mut max_codelen = 0usize;
                self.symlen[..self.num_syms].fill(0);
                let codelen_bits = bits.read_bits_no_refill(3) as i32;
                if codelen_bits > 5 {
                    return false;
                }
                for _ in 0..self.used_syms {
                    bits.refill();
                    let sym = bits.read_bits_no_refill(sym_bits) as usize;
                    let codelen = (bits.read_bits_no_refill(codelen_bits) + 1) as usize;
                    if codelen > 16 || sym >= self.num_syms {
                        return false;
                    }
                    self.codelen_count[codelen] += 1;
                    self.last_used_sym = sym;
                    self.symlen[sym] = codelen as u8;
                    if codelen < min_codelen {
                        min_codelen = codelen;
                    }
                    if codelen > max_codelen {
                        max_codelen = codelen;
                    }
                }
                self.min_used_codelen = min_codelen;
                self.max_used_codelen = max_codelen;
                true
            } else {
                bits.refill();
                let s = bits.read_bits_no_refill(sym_bits) as usize;
                if s >= self.num_syms {
                    return false;
                }
                self.single_symbol = s as u8;
                self.last_used_sym = s;
                self.symlen[s] = 0;
                self.c_syms[0] = s as u8;
                true
            }
        }
    }

    fn setup_bits(&mut self) -> bool {
        if self.used_syms <= 1
            || self.min_used_codelen == 0
            || self.max_used_codelen == 0
            || self.max_used_codelen > self.max_code_len
        {
            return false;
        }
        let mut local_arr = [0usize; 17];
        let mut x = 0usize;
        let mut y = 0usize;
        let mut z = 0usize;
        for i in self.min_used_codelen..=self.max_used_codelen {
            local_arr[i] = x;
            z = self.codelen_count[i] + y;
            y = 2 * z;
            x += self.codelen_count[i];
        }
        if z != (1usize << self.max_used_codelen) {
            return false;
        }
        local_arr[0] = self.used_syms;
        for i in 0..=self.last_used_sym {
            let cl = self.symlen[i] as usize;
            let dst_idx = local_arr[cl];
            self.c_syms[dst_idx] = i as u8;
            local_arr[cl] += 1;
        }
        true
    }

    fn make_lookup_tables(&mut self) -> bool {
        if !self.setup_bits() {
            return false;
        }
        let mut cursym_idx = 0usize;
        let mut p = 0usize;
        for i in self.min_used_codelen..self.max_code_len {
            let mut n = self.codelen_count[i];
            if n != 0 {
                let shift = self.max_code_len - i;
                let count = n << shift;
                self.bits_to_len[p..p + count].fill(i as u8);
                let m = 1usize << shift;
                while n > 0 {
                    let sym = self.c_syms[cursym_idx];
                    cursym_idx += 1;
                    self.bits_to_sym[p..p + m].fill(sym);
                    p += m;
                    n -= 1;
                }
            }
        }
        let n = self.codelen_count[self.max_code_len];
        if n != 0 {
            self.bits_to_len[p..p + n].fill(self.max_code_len as u8);
            self.bits_to_sym[p..p + n].copy_from_slice(&self.c_syms[cursym_idx..cursym_idx + n]);
        }
        true
    }
}

#[inline]
fn reverse_bits_array_2048(input: &[u8; 2048], output: &mut [u8; 2048]) {
    for (i, &val) in input.iter().enumerate() {
        let rev_idx = ((i as u16).reverse_bits() >> 5) as usize;
        output[rev_idx] = val;
    }
}

fn kraken_decode_bytes_core(
    huff: &HuffmanTree,
    src_buf: &[u8],
    split_mid_idx: usize,
    output: &mut [u8],
) -> bool {
    if split_mid_idx > src_buf.len() {
        return false;
    }
    let mut bits2len_rev = [0u8; 2048];
    let mut bits2sym_rev = [0u8; 2048];
    reverse_bits_array_2048(&huff.bits_to_len, &mut bits2len_rev);
    reverse_bits_array_2048(&huff.bits_to_sym, &mut bits2sym_rev);

    let mut src: isize = 0;
    let mut src_bits: u32 = 0;
    let mut src_bitpos: i32 = 0;

    let src_mid_org: isize = split_mid_idx as isize;
    let mut src_mid: isize = src_mid_org;
    let mut src_mid_bits: u32 = 0;
    let mut src_mid_bitpos: i32 = 0;

    let mut src_end: isize = src_buf.len() as isize;
    let mut src_end_bits: u32 = 0;
    let mut src_end_bitpos: i32 = 0;

    let mut dst: usize = 0;
    let mut dst_end: usize = output.len();

    let read_u32_le_at = |pos: isize| -> u32 {
        let mut b = [0u8; 4];
        for i in 0..4 {
            let idx = pos + i;
            if idx >= 0 && (idx as usize) < src_buf.len() {
                b[i as usize] = src_buf[idx as usize];
            }
        }
        u32::from_le_bytes(b)
    };
    let read_u16_le_at = |pos: isize| -> u32 {
        let mut b = [0u8; 2];
        for i in 0..2 {
            let idx = pos + i;
            if idx >= 0 && (idx as usize) < src_buf.len() {
                b[i as usize] = src_buf[idx as usize];
            }
        }
        u16::from_le_bytes(b) as u32
    };

    if src_end - src_mid >= 4 && dst_end >= 6 {
        dst_end -= 5;
        src_end -= 4;
        while dst < dst_end && src <= src_mid && src_mid <= src_end {
            src_bits |= read_u32_le_at(src).wrapping_shl(src_bitpos as u32);
            src += ((31 - src_bitpos) >> 3) as isize;

            src_end_bits |= read_u32_le_at(src_end)
                .swap_bytes()
                .wrapping_shl(src_end_bitpos as u32);
            src_end -= ((31 - src_end_bitpos) >> 3) as isize;

            src_mid_bits |= read_u32_le_at(src_mid).wrapping_shl(src_mid_bitpos as u32);
            src_mid += ((31 - src_mid_bitpos) >> 3) as isize;

            src_bitpos |= 0x18;
            src_end_bitpos |= 0x18;
            src_mid_bitpos |= 0x18;

            let k0 = (src_bits & 0x7FF) as usize;
            let n0 = bits2len_rev[k0] as i32;
            src_bits >>= n0;
            src_bitpos -= n0;
            output[dst] = bits2sym_rev[k0];

            let k1 = (src_end_bits & 0x7FF) as usize;
            let n1 = bits2len_rev[k1] as i32;
            src_end_bits >>= n1;
            src_end_bitpos -= n1;
            output[dst + 1] = bits2sym_rev[k1];

            let k2 = (src_mid_bits & 0x7FF) as usize;
            let n2 = bits2len_rev[k2] as i32;
            src_mid_bits >>= n2;
            src_mid_bitpos -= n2;
            output[dst + 2] = bits2sym_rev[k2];

            let k3 = (src_bits & 0x7FF) as usize;
            let n3 = bits2len_rev[k3] as i32;
            src_bits >>= n3;
            src_bitpos -= n3;
            output[dst + 3] = bits2sym_rev[k3];

            let k4 = (src_end_bits & 0x7FF) as usize;
            let n4 = bits2len_rev[k4] as i32;
            src_end_bits >>= n4;
            src_end_bitpos -= n4;
            output[dst + 4] = bits2sym_rev[k4];

            let k5 = (src_mid_bits & 0x7FF) as usize;
            let n5 = bits2len_rev[k5] as i32;
            src_mid_bits >>= n5;
            src_mid_bitpos -= n5;
            output[dst + 5] = bits2sym_rev[k5];

            dst += 6;
        }
        dst_end += 5;
        src -= (src_bitpos >> 3) as isize;
        src_bitpos &= 7;
        src_end += 4 + ((src_end_bitpos >> 3) as isize);
        src_end_bitpos &= 7;
        src_mid -= (src_mid_bitpos >> 3) as isize;
        src_mid_bitpos &= 7;
    }

    loop {
        if dst >= dst_end {
            break;
        }
        if src_mid - src <= 1 {
            if src_mid - src == 1 && src >= 0 && (src as usize) < src_buf.len() {
                src_bits |= (src_buf[src as usize] as u32) << src_bitpos;
            }
        } else {
            src_bits |= read_u16_le_at(src) << src_bitpos;
        }
        let k = (src_bits & 0x7FF) as usize;
        let n = bits2len_rev[k] as i32;
        src_bitpos -= n;
        src_bits >>= n;
        output[dst] = bits2sym_rev[k];
        dst += 1;
        src += ((7 - src_bitpos) >> 3) as isize;
        src_bitpos &= 7;

        if dst < dst_end {
            if src_end - src_mid <= 1 {
                if src_end - src_mid == 1 && src_mid >= 0 && (src_mid as usize) < src_buf.len() {
                    let b = src_buf[src_mid as usize] as u32;
                    src_end_bits |= b << src_end_bitpos;
                    src_mid_bits |= b << src_mid_bitpos;
                }
            } else {
                let v = read_u16_le_at(src_end - 2);
                let v_swapped = ((v >> 8) | (v << 8)) & 0xFFFF;
                src_end_bits |= v_swapped << src_end_bitpos;
                src_mid_bits |= read_u16_le_at(src_mid) << src_mid_bitpos;
            }
            let ke = (src_end_bits & 0x7FF) as usize;
            let ne = bits2len_rev[ke] as i32;
            output[dst] = bits2sym_rev[ke];
            dst += 1;
            src_end_bitpos -= ne;
            src_end_bits >>= ne;
            src_end -= ((7 - src_end_bitpos) >> 3) as isize;
            src_end_bitpos &= 7;

            if dst < dst_end {
                let km = (src_mid_bits & 0x7FF) as usize;
                let nm = bits2len_rev[km] as i32;
                output[dst] = bits2sym_rev[km];
                dst += 1;
                src_mid_bitpos -= nm;
                src_mid_bits >>= nm;
                src_mid += ((7 - src_mid_bitpos) >> 3) as isize;
                src_mid_bitpos &= 7;
            }
        }
        if src > src_mid || src_mid > src_end {
            return false;
        }
    }

    src == src_mid_org && src_end == src_mid
}

fn kraken_decode_bytes_type12(src: &[u8], output: &mut [u8], typ: u8) -> bool {
    let mut huff = HuffmanTree::new(256, 11);
    let mut bits = BitReader::new_forward(src);
    if bits.read_bit() != 0 || !huff.decode(&mut bits) {
        return false;
    }
    let bytes_parsed = (bits.p - (((24 - bits.bitpos) / 8) as isize)) as usize;
    if bytes_parsed > src.len() {
        return false;
    }
    let rem = &src[bytes_parsed..];
    if huff.used_syms <= 1 {
        if !rem.is_empty() {
            return false;
        }
        output.fill(huff.single_symbol);
        return true;
    }
    if !huff.make_lookup_tables() {
        return false;
    }
    if typ == 1 {
        if rem.len() < 3 {
            return false;
        }
        let split_mid = u16::from_le_bytes([rem[0], rem[1]]) as usize;
        kraken_decode_bytes_core(&huff, &rem[2..], split_mid, output)
    } else {
        if rem.len() < 6 {
            return false;
        }
        let half_output_size = (output.len() + 1) >> 1;
        let split_mid =
            (rem[0] as usize) | ((rem[1] as usize) << 8) | ((rem[2] as usize) << 16);
        let after3 = &rem[3..];
        if split_mid > after3.len() || split_mid < 2 || after3.len() - split_mid < 3 {
            return false;
        }
        let split_left = u16::from_le_bytes([after3[0], after3[1]]) as usize;
        if split_mid - 2 < split_left + 2 {
            return false;
        }
        let right_slice = &after3[split_mid..];
        let split_right = u16::from_le_bytes([right_slice[0], right_slice[1]]) as usize;
        if right_slice.len() - 2 < split_right + 2 {
            return false;
        }
        let (out_left, out_right) = output.split_at_mut(half_output_size);
        if !kraken_decode_bytes_core(&huff, &after3[2..split_mid], split_left, out_left) {
            return false;
        }
        kraken_decode_bytes_core(&huff, &right_slice[2..], split_right, out_right)
    }
}

/// Returns `(bytes_consumed_from_src, decoded_vec)`
fn kraken_decode_bytes(src: &[u8], max_output_size: usize) -> Option<(usize, Vec<u8>)> {
    if src.len() < 5 {
        return None;
    }
    let typ = src[0] >> 5;
    match typ {
        0 => {
            let src_size = (((src[0] as usize) << 16)
                | ((src[1] as usize) << 8)
                | (src[2] as usize))
                & 0x3FFFF;
            if src_size > max_output_size || src.len() < 3 + src_size {
                return None;
            }
            Some((3 + src_size, src[3..3 + src_size].to_vec()))
        }
        1 | 2 => {
            let lo_bits = u32::from_be_bytes([src[1], src[2], src[3], src[4]]);
            let src_size = (lo_bits & 0x3FFFF) as usize;
            if src.len() < 5 + src_size {
                return None;
            }
            let num_elems =
                ((((lo_bits >> 18) | ((src[0] as u32) << 14)) & 0x3FFFF) as usize) + 1;
            if num_elems > max_output_size || src_size >= num_elems {
                return None;
            }
            let mut out = vec![0u8; num_elems];
            if !kraken_decode_bytes_type12(&src[5..5 + src_size], &mut out, typ) {
                return None;
            }
            Some((5 + src_size, out))
        }
        _ => None,
    }
}

struct KrakenLzTable {
    lit_stream: Vec<u8>,
    flag_stream: Vec<u8>,
    dist_stream: Vec<i32>,
    len_stream: Vec<usize>,
}

fn kraken_unpack_offsets(
    src: &[u8],
    packed_dist: &[u8],
    packed_litlen: &[u8],
) -> Option<(Vec<i32>, Vec<usize>)> {
    let mut bits_a = BitReader::new_forward(src);
    bits_a.refill();
    let mut bits_b = BitReader::new_backward(src);
    bits_b.refill_backwards();

    if bits_b.bits == 0 {
        return None;
    }
    let mut n = bits_b.bits.leading_zeros() as i32;
    if n > 18 {
        return None;
    }
    bits_b.bitpos += n;
    bits_b.bits <<= n;
    bits_b.refill_backwards();
    n += 1;
    let extralong_size = ((bits_b.bits >> (32 - n)).wrapping_sub(1)) as usize;
    bits_b.bitpos += n;
    bits_b.bits <<= n;
    bits_b.refill_backwards();

    let mut dist_stream = vec![0i32; packed_dist.len()];
    let mut i = 0usize;
    while i + 1 < packed_dist.len() {
        dist_stream[i] = -(bits_a.read_distance(packed_dist[i] as u32) as i32);
        dist_stream[i + 1] = -(bits_b.read_distance_b(packed_dist[i + 1] as u32) as i32);
        i += 2;
    }
    if i < packed_dist.len() {
        dist_stream[i] = -(bits_a.read_distance(packed_dist[i] as u32) as i32);
    }

    let mut extralong = vec![0u32; extralong_size];
    let mut j = 0usize;
    while j + 1 < extralong_size {
        extralong[j] = bits_a.read_length()?;
        extralong[j + 1] = bits_b.read_length_b()?;
        j += 2;
    }
    if j < extralong_size {
        extralong[j] = bits_a.read_length()?;
    }

    let pa = bits_a.p - (((24 - bits_a.bitpos) >> 3) as isize);
    let pb = bits_b.p + (((24 - bits_b.bitpos) >> 3) as isize);
    if pa != pb {
        return None;
    }

    let mut len_stream = Vec::with_capacity(packed_litlen.len());
    let mut el_idx = 0usize;
    for &b in packed_litlen {
        let mut v = b as usize;
        if v == 255 {
            let extra = *extralong.get(el_idx)? as usize;
            el_idx += 1;
            v = extra + 255;
        }
        len_stream.push(v + 3);
    }
    Some((dist_stream, len_stream))
}

fn kraken_read_lz_table(
    mode: i32,
    mut src: &[u8],
    dst_buf: &mut [u8],
    dst_offset: usize,
    dst_count: usize,
) -> Option<KrakenLzTable> {
    if mode > 1 || src.len() < 13 {
        return None;
    }
    if dst_offset == 0 {
        if dst_buf.len() < 8 || src.len() < 8 {
            return None;
        }
        dst_buf[0..8].copy_from_slice(&src[0..8]);
        src = &src[8..];
    }
    let max_out = 2 * dst_count + 32;
    let (n1, lit_stream) = kraken_decode_bytes(src, max_out)?;
    src = &src[n1..];
    let (n2, flag_stream) = kraken_decode_bytes(src, max_out)?;
    if flag_stream.is_empty() {
        return None;
    }
    src = &src[n2..];
    let (n3, packed_dist) = kraken_decode_bytes(src, max_out)?;
    src = &src[n3..];
    let (n4, packed_litlen) = kraken_decode_bytes(src, max_out)?;
    src = &src[n4..];

    let (dist_stream, len_stream) = kraken_unpack_offsets(src, &packed_dist, &packed_litlen)?;
    Some(KrakenLzTable {
        lit_stream,
        flag_stream,
        dist_stream,
        len_stream,
    })
}

fn kraken_process_lz_runs(
    mode: i32,
    lzt: &KrakenLzTable,
    dst_buf: &mut [u8],
    dst_offset: usize,
    dst_size: usize,
) -> bool {
    let dst_end = dst_offset + dst_size;
    let mut dst = dst_offset + if dst_offset == 0 { 8 } else { 0 };
    let mut lit_idx = 0usize;
    let mut len_idx = 0usize;
    let mut dist_idx = 0usize;
    let mut recent_distance = [0i32, 0, 0, -8, -8, -8, 0];
    let mut last_lit_dist: isize = -8;

    for &f in &lzt.flag_stream {
        let mut litlen = (f & 3) as usize;
        let disttype = ((f >> 6) & 3) as usize;
        let mut copy_len = ((f >> 2) & 0xF) as usize;

        if litlen == 3 {
            let Some(&nl) = lzt.len_stream.get(len_idx) else {
                return false;
            };
            len_idx += 1;
            litlen = nl;
        }
        recent_distance[6] = lzt.dist_stream.get(dist_idx).copied().unwrap_or(-8);

        if dst + litlen > dst_end || lit_idx + litlen > lzt.lit_stream.len() {
            return false;
        }
        if mode == 0 {
            for k in 0..litlen {
                let ref_pos = (dst + k) as isize + last_lit_dist;
                if ref_pos < 0 || (ref_pos as usize) >= dst + k {
                    return false;
                }
                dst_buf[dst + k] =
                    lzt.lit_stream[lit_idx + k].wrapping_add(dst_buf[ref_pos as usize]);
            }
        } else {
            dst_buf[dst..dst + litlen].copy_from_slice(&lzt.lit_stream[lit_idx..lit_idx + litlen]);
        }
        dst += litlen;
        lit_idx += litlen;

        let distance = recent_distance[disttype + 3];
        for r in (1..=disttype).rev() {
            recent_distance[r + 3] = recent_distance[r + 2];
        }
        recent_distance[3] = distance;
        last_lit_dist = distance as isize;
        if disttype == 3 {
            dist_idx += 1;
        }

        let copyfrom_isize = (dst as isize) + (distance as isize);
        if copyfrom_isize < 0 {
            return false;
        }
        let copyfrom = copyfrom_isize as usize;

        let actual_copy = if copy_len != 15 {
            copy_len + 2
        } else {
            let Some(&extra) = lzt.len_stream.get(len_idx) else {
                return false;
            };
            len_idx += 1;
            copy_len = 14 + extra;
            copy_len
        };
        if dst + actual_copy > dst_end {
            return false;
        }
        for k in 0..actual_copy {
            dst_buf[dst + k] = dst_buf[copyfrom + k];
        }
        dst += actual_copy;
    }

    if dist_idx != lzt.dist_stream.len() || len_idx != lzt.len_stream.len() {
        return false;
    }
    let final_len = dst_end.saturating_sub(dst);
    if final_len != lzt.lit_stream.len().saturating_sub(lit_idx) {
        return false;
    }
    if mode == 0 {
        for k in 0..final_len {
            let ref_pos = (dst + k) as isize + last_lit_dist;
            if ref_pos < 0 || (ref_pos as usize) >= dst + k {
                return false;
            }
            dst_buf[dst + k] =
                lzt.lit_stream[lit_idx + k].wrapping_add(dst_buf[ref_pos as usize]);
        }
    } else {
        dst_buf[dst..dst + final_len]
            .copy_from_slice(&lzt.lit_stream[lit_idx..lit_idx + final_len]);
    }
    true
}

fn kraken_decode_quantum(
    dst_buf: &mut [u8],
    mut dst_offset: usize,
    dst_end: usize,
    src: &[u8],
) -> Option<usize> {
    let mut src_pos = 0usize;
    while dst_offset < dst_end {
        let dst_count = (dst_end - dst_offset).min(0x20000);
        if src.len() - src_pos < 4 {
            return None;
        }
        let chunkhdr = ((src[src_pos] as usize) << 16)
            | ((src[src_pos + 1] as usize) << 8)
            | (src[src_pos + 2] as usize);
        if (chunkhdr & 0x800000) == 0 {
            let (src_used, decoded) = kraken_decode_bytes(&src[src_pos..], dst_count)?;
            if decoded.len() != dst_count {
                return None;
            }
            dst_buf[dst_offset..dst_offset + dst_count].copy_from_slice(&decoded);
            src_pos += src_used;
        } else {
            src_pos += 3;
            let src_used = chunkhdr & 0x7FFFF;
            let mode = ((chunkhdr >> 19) & 0xF) as i32;
            if src.len() - src_pos < src_used {
                return None;
            }
            if src_used < dst_count {
                let lzt = kraken_read_lz_table(
                    mode,
                    &src[src_pos..src_pos + src_used],
                    dst_buf,
                    dst_offset,
                    dst_count,
                )?;
                if !kraken_process_lz_runs(mode, &lzt, dst_buf, dst_offset, dst_count) {
                    return None;
                }
            } else if src_used > dst_count || mode != 0 {
                return None;
            } else {
                dst_buf[dst_offset..dst_offset + dst_count]
                    .copy_from_slice(&src[src_pos..src_pos + dst_count]);
            }
            src_pos += src_used;
        }
        dst_offset += dst_count;
    }
    Some(src_pos)
}

/// Decompress an Oodle `Kraken` stream in pure Rust without requiring `oo2core_*.dll`.
pub fn decompress_oodle_fallback(compressed: &[u8], uncompressed_size: usize) -> Result<Vec<u8>> {
    if uncompressed_size == 0 {
        return Ok(Vec::new());
    }
    if uncompressed_size > crate::io::MAX_ARCHIVE_ALLOC_BYTES {
        return Err(PgError::InvalidData(format!(
            "Oodle uncompressed size {uncompressed_size} exceeds safety limit ({})",
            crate::io::MAX_ARCHIVE_ALLOC_BYTES
        )));
    }
    if compressed.len() == uncompressed_size {
        return Ok(compressed.to_vec());
    }

    let mut dst = vec![0u8; uncompressed_size + 64];
    let mut offset = 0usize;
    let mut src_pos = 0usize;
    let mut hdr = KrakenHeader::default();

    while offset < uncompressed_size {
        if (offset & 0x3FFFF) == 0 {
            let (parsed_hdr, adv) = parse_header(&compressed[src_pos..]).ok_or_else(|| {
                PgError::InvalidData("Invalid Oodle Kraken block header".to_string())
            })?;
            hdr = parsed_hdr;
            src_pos += adv;
        }

        let max_block_size = if hdr.decoder_type != 5 && hdr.decoder_type != 11 {
            0x40000
        } else {
            0x4000
        };
        let dst_bytes_left = (uncompressed_size - offset).min(max_block_size);

        if hdr.uncompressed {
            if compressed.len() - src_pos < dst_bytes_left {
                return Err(PgError::InvalidData(
                    "Truncated uncompressed Oodle quantum".into(),
                ));
            }
            dst[offset..offset + dst_bytes_left]
                .copy_from_slice(&compressed[src_pos..src_pos + dst_bytes_left]);
            src_pos += dst_bytes_left;
            offset += dst_bytes_left;
            continue;
        }

        let (qhdr, q_adv) =
            parse_quantum_header(&compressed[src_pos..], hdr.use_checksums).ok_or_else(|| {
                PgError::InvalidData("Invalid Oodle quantum header".to_string())
            })?;
        src_pos += q_adv;

        if compressed.len() - src_pos < qhdr.compressed_size || qhdr.compressed_size > dst_bytes_left
        {
            return Err(PgError::InvalidData(
                "Oodle quantum size out of bounds".into(),
            ));
        }

        if qhdr.compressed_size == 0 {
            if qhdr.whole_match_distance != 0 {
                if qhdr.whole_match_distance > offset {
                    return Err(PgError::InvalidData(
                        "Oodle whole-match distance out of bounds".into(),
                    ));
                }
                for k in 0..dst_bytes_left {
                    dst[offset + k] = dst[offset + k - qhdr.whole_match_distance];
                }
            } else {
                dst[offset..offset + dst_bytes_left].fill(qhdr.checksum as u8);
            }
            offset += dst_bytes_left;
            continue;
        }

        if qhdr.compressed_size == dst_bytes_left {
            dst[offset..offset + dst_bytes_left]
                .copy_from_slice(&compressed[src_pos..src_pos + dst_bytes_left]);
            src_pos += dst_bytes_left;
            offset += dst_bytes_left;
            continue;
        }

        if hdr.decoder_type == 6 {
            let used = kraken_decode_quantum(
                &mut dst,
                offset,
                offset + dst_bytes_left,
                &compressed[src_pos..src_pos + qhdr.compressed_size],
            )
            .ok_or_else(|| PgError::InvalidData("Oodle Kraken quantum decode failed".into()))?;
            if used != qhdr.compressed_size {
                return Err(PgError::InvalidData(
                    "Oodle Kraken quantum consumed size mismatch".into(),
                ));
            }
            src_pos += used;
            offset += dst_bytes_left;
        } else {
            return Err(PgError::Unsupported(format!(
                "Oodle sub-codec {} requires oo2core_*.dll",
                hdr.decoder_type
            )));
        }
    }

    dst.truncate(uncompressed_size);
    Ok(dst)
}
