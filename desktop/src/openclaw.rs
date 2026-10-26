use crate::{adb::Adb, device::Device};
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenClawCredentials {
    pub device_id: String,
    pub public_key: String,
    pub private_key: String,
    pub device_token: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealAmooData {
    #[serde(default)]
    pub account_scope: Option<String>,
    pub selected_session: Option<String>,
    pub gateway_version: String,
    pub gateway_online: bool,
    pub default_agent_id: String,
    pub agents: Vec<RealAgentInfo>,
    pub sessions: Vec<RealSessionInfo>,
    pub cron_jobs: Vec<RealCronInfo>,
    pub skills: Vec<RealSkillInfo>,
    pub total_skills_count: usize,
    pub eligible_skills_count: usize,
    pub memory_initialized: bool,
    pub memory_bm25_entries: usize,
    pub memory_engine_label: String,
    pub memory_data_dir: String,
    pub memory_provider: String,
    pub memory_entries: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealAgentInfo {
    pub id: String,
    pub name: String,
    pub model: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealSessionInfo {
    pub key: String,
    pub session_id: String,
    pub title: String,
    pub agent: String,
    pub status: String,
    pub status_kind: u8,
    pub updated_at_ms: i64,
    pub meta: String,
    pub model: String,
    pub provider: String,
    pub messages: Vec<RealChatMessage>,
    #[serde(default)]
    pub history_loaded: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealChatMessage {
    #[serde(default)]
    pub usage: crate::amoo_usage::UsageMetrics,
    #[serde(default)]
    pub usage_session_key: String,
    #[serde(default)]
    pub usage_message_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub run_id: String,
    #[serde(default)]
    pub stop_reason: String,
    pub role: String,
    pub text: String,
    pub thinking: Vec<String>,
    pub tools_summary: String,
    /// Explicit authorization failures from tool results, never inferred from a slow reply.
    #[serde(default)]
    pub tool_authorization_failures: Vec<String>,
    pub timestamp_ms: i64,
    pub time_label: String,
    pub is_error: bool,
    pub error_message: Option<String>,
    pub model: String,
    pub provider: String,
    #[serde(default)]
    pub attachments: Vec<RealAttachment>,
}

/// Original service attachment references. A blob is opaque unless it is an
/// explicit HTTP(S) URL; clients must not treat it as base64 or a local filename.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealAttachment {
    pub kind: String,
    pub mime: String,
    pub name: String,
    pub url: String,
    pub blob: String,
    pub path: String,
    pub size_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealCronInfo {
    #[serde(default)]
    pub session_key: String,
    #[serde(default)]
    pub status: String,
    pub id: String,
    pub name: String,
    pub schedule: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RealSkillInfo {
    pub skill_key: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub enabled: bool,
    pub eligible: bool,
    pub prompt: String,
}

// ============================================================================
// Pure-Rust SHA-512 & TweetNaCl Ed25519 (RFC 8032)
// ============================================================================

const SHA512_K: [u64; 80] = [
    0x428a2f98d728ae22,
    0x7137449123ef65cd,
    0xb5c0fbcfec4d3b2f,
    0xe9b5dba58189dbbc,
    0x3956c25bf348b538,
    0x59f111f1b605d019,
    0x923f82a4af194f9b,
    0xab1c5ed5da6d8118,
    0xd807aa98a3030242,
    0x12835b0145706fbe,
    0x243185be4ee4b28c,
    0x550c7dc3d5ffb4e2,
    0x72be5d74f27b896f,
    0x80deb1fe3b1696b1,
    0x9bdc06a725c71235,
    0xc19bf174cf692694,
    0xe49b69c19ef14ad2,
    0xefbe4786384f25e3,
    0x0fc19dc68b8cd5b5,
    0x240ca1cc77ac9c65,
    0x2de92c6f592b0275,
    0x4a7484aa6ea6e483,
    0x5cb0a9dcbd41fbd4,
    0x76f988da831153b5,
    0x983e5152ee66dfab,
    0xa831c66d2db43210,
    0xb00327c898fb213f,
    0xbf597fc7beef0ee4,
    0xc6e00bf33da88fc2,
    0xd5a79147930aa725,
    0x06ca6351e003826f,
    0x142929670a0e6e70,
    0x27b70a8546d22ffc,
    0x2e1b21385c26c926,
    0x4d2c6dfc5ac42aed,
    0x53380d139d95b3df,
    0x650a73548baf63de,
    0x766a0abb3c77b2a8,
    0x81c2c92e47edaee6,
    0x92722c851482353b,
    0xa2bfe8a14cf10364,
    0xa81a664bbc423001,
    0xc24b8b70d0f89791,
    0xc76c51a30654be30,
    0xd192e819d6ef5218,
    0xd69906245565a910,
    0xf40e35855771202a,
    0x106aa07032bbd1b8,
    0x19a4c116b8d2d0c8,
    0x1e376c085141ab53,
    0x2748774cdf8eeb99,
    0x34b0bcb5e19b48a8,
    0x391c0cb3c5c95a63,
    0x4ed8aa4ae3418acb,
    0x5b9cca4f7763e373,
    0x682e6ff3d6b2b8a3,
    0x748f82ee5defb2fc,
    0x78a5636f43172f60,
    0x84c87814a1f0ab72,
    0x8cc702081a6439ec,
    0x90befffa23631e28,
    0xa4506cebde82bde9,
    0xbef9a3f7b2c67915,
    0xc67178f2e372532b,
    0xca273eceea26619c,
    0xd186b8c721c0c207,
    0xeada7dd6cde0eb1e,
    0xf57d4f7fee6ed178,
    0x06f067aa72176fba,
    0x0a637dc5a2c898a6,
    0x113f9804bef90dae,
    0x1b710b35131c471b,
    0x28db77f523047d84,
    0x32caab7b40c72493,
    0x3c9ebe0a15c9bebc,
    0x431d67c49c100d4c,
    0x4cc5d4becb3e42b6,
    0x597f299cfc657e2a,
    0x5fcb6fab3ad6faec,
    0x6c44198c4a475817,
];

pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut h: [u64; 8] = [
        0x6a09e667f3bcc908,
        0xbb67ae8584caa73b,
        0x3c6ef372fe94f82b,
        0xa54ff53a5f1d36f1,
        0x510e527fade682d1,
        0x9b05688c2b3e6c1f,
        0x1f83d9abfb41bd6b,
        0x5be0cd19137e2179,
    ];
    let bit_len = (data.len() as u128) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 128) != 112 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in padded.chunks_exact(128) {
        let mut w = [0u64; 80];
        for (i, word) in chunk.chunks_exact(8).enumerate() {
            w[i] = u64::from_be_bytes([
                word[0], word[1], word[2], word[3], word[4], word[5], word[6], word[7],
            ]);
        }
        for i in 16..80 {
            let s0 = w[i - 15].rotate_right(1) ^ w[i - 15].rotate_right(8) ^ (w[i - 15] >> 7);
            let s1 = w[i - 2].rotate_right(19) ^ w[i - 2].rotate_right(61) ^ (w[i - 2] >> 6);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut a = h[0];
        let mut b = h[1];
        let mut c = h[2];
        let mut d = h[3];
        let mut e = h[4];
        let mut f = h[5];
        let mut g = h[6];
        let mut hh = h[7];

        for i in 0..80 {
            let s1 = e.rotate_right(14) ^ e.rotate_right(18) ^ e.rotate_right(41);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA512_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(28) ^ a.rotate_right(34) ^ a.rotate_right(39);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
        h[5] = h[5].wrapping_add(f);
        h[6] = h[6].wrapping_add(g);
        h[7] = h[7].wrapping_add(hh);
    }

    let mut out = [0u8; 64];
    for (i, word) in h.iter().enumerate() {
        out[i * 8..i * 8 + 8].copy_from_slice(&word.to_be_bytes());
    }
    out
}

type Gf = [i64; 16];

const GF0: Gf = [0; 16];
const GF1: Gf = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const D2: Gf = [
    0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0, 0xd130, 0xeef3, 0x80f2, 0x198e,
    0xfce7, 0x56df, 0xd9dc, 0x2406,
];
const BX: Gf = [
    0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c, 0xdc5c, 0xfdd6, 0xe231, 0xc0a4,
    0x53fe, 0xcd6e, 0x36d3, 0x2169,
];
const BY: Gf = [
    0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666,
    0x6666, 0x6666, 0x6666, 0x6666,
];
const L_SCALAR: [i64; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

fn car25519(o: &mut Gf) {
    for i in 0..16 {
        o[i] += 1 << 16;
        let c = o[i] >> 16;
        if i < 15 {
            o[i + 1] += c - 1;
        } else {
            o[0] += 38 * (c - 1);
        }
        o[i] -= c << 16;
    }
}

fn sel25519(p: &mut Gf, q: &mut Gf, b: i64) {
    let c = !(b - 1);
    for i in 0..16 {
        let t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}

fn gf_add(a: &Gf, b: &Gf) -> Gf {
    let mut o = GF0;
    for i in 0..16 {
        o[i] = a[i] + b[i];
    }
    o
}

fn gf_sub(a: &Gf, b: &Gf) -> Gf {
    let mut o = GF0;
    for i in 0..16 {
        o[i] = a[i] - b[i];
    }
    o
}

fn gf_mul(a: &Gf, b: &Gf) -> Gf {
    let mut t = [0i64; 31];
    for i in 0..16 {
        for j in 0..16 {
            t[i + j] += a[i] * b[j];
        }
    }
    for i in 0..15 {
        t[i] += 38 * t[i + 16];
    }
    let mut o = GF0;
    o.copy_from_slice(&t[..16]);
    car25519(&mut o);
    car25519(&mut o);
    o
}

fn gf_sq(a: &Gf) -> Gf {
    gf_mul(a, a)
}

fn inv25519(i: &Gf) -> Gf {
    let mut c = *i;
    for a in (0..=253).rev() {
        c = gf_sq(&c);
        if a != 2 && a != 4 {
            c = gf_mul(&c, i);
        }
    }
    c
}

fn pack25519(n: &Gf) -> [u8; 32] {
    let mut t = *n;
    car25519(&mut t);
    car25519(&mut t);
    car25519(&mut t);
    let mut m = GF0;
    for _ in 0..2 {
        m[0] = t[0] - 0xffed;
        for i in 1..15 {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        let b = (m[15] >> 16) & 1;
        m[14] &= 0xffff;
        sel25519(&mut t, &mut m, 1 - b);
    }
    let mut o = [0u8; 32];
    for i in 0..16 {
        o[2 * i] = (t[i] & 0xff) as u8;
        o[2 * i + 1] = (t[i] >> 8) as u8;
    }
    o
}

fn par25519(a: &Gf) -> u8 {
    let d = pack25519(a);
    d[0] & 1
}

fn cswap(p: &mut [Gf; 4], q: &mut [Gf; 4], b: u8) {
    for i in 0..4 {
        sel25519(&mut p[i], &mut q[i], b as i64);
    }
}

fn pt_add(p: &mut [Gf; 4], q: &[Gf; 4]) {
    let a = gf_sub(&p[1], &p[0]);
    let t = gf_sub(&q[1], &q[0]);
    let a = gf_mul(&a, &t);
    let b = gf_add(&p[0], &p[1]);
    let t = gf_add(&q[0], &q[1]);
    let b = gf_mul(&b, &t);
    let c = gf_mul(&p[3], &q[3]);
    let c = gf_mul(&c, &D2);
    let d = gf_mul(&p[2], &q[2]);
    let d = gf_add(&d, &d);
    let e = gf_sub(&b, &a);
    let f = gf_sub(&d, &c);
    let g = gf_add(&d, &c);
    let h = gf_add(&b, &a);
    p[0] = gf_mul(&e, &f);
    p[1] = gf_mul(&h, &g);
    p[2] = gf_mul(&g, &f);
    p[3] = gf_mul(&e, &h);
}

fn scalarmult_base(s: &[u8; 32]) -> [Gf; 4] {
    let mut q = [BX, BY, GF1, gf_mul(&BX, &BY)];
    let mut p = [GF0, GF1, GF1, GF0];
    for i in (0..=255).rev() {
        let b = (s[i / 8] >> (i & 7)) & 1;
        cswap(&mut p, &mut q, b);
        pt_add(&mut q, &p);
        let p_copy = p;
        pt_add(&mut p, &p_copy);
        cswap(&mut p, &mut q, b);
    }
    p
}

fn pt_pack(p: &[Gf; 4]) -> [u8; 32] {
    let zi = inv25519(&p[2]);
    let tx = gf_mul(&p[0], &zi);
    let ty = gf_mul(&p[1], &zi);
    let mut r = pack25519(&ty);
    r[31] ^= par25519(&tx) << 7;
    r
}

fn mod_l(x: &mut [i64; 64]) -> [u8; 32] {
    for i in (32..=63).rev() {
        let mut carry = 0i64;
        for j in (i - 32)..(i - 12) {
            x[j] += carry - 16 * x[i] * L_SCALAR[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= carry << 8;
        }
        x[i - 12] += carry;
        x[i] = 0;
    }
    let mut carry = 0i64;
    for j in 0..32 {
        x[j] += carry - (x[31] >> 4) * L_SCALAR[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for j in 0..32 {
        x[j] -= carry * L_SCALAR[j];
    }
    let mut r = [0u8; 32];
    for i in 0..32 {
        x[i + 1] += x[i] >> 8;
        r[i] = (x[i] & 255) as u8;
    }
    r
}

fn reduce_scalar(h: &[u8; 64]) -> [u8; 32] {
    let mut x = [0i64; 64];
    for i in 0..64 {
        x[i] = h[i] as i64;
    }
    mod_l(&mut x)
}

pub fn ed25519_sign(seed: &[u8; 32], public_key: &[u8; 32], message: &[u8]) -> [u8; 64] {
    let mut d = sha512(seed);
    d[0] &= 248;
    d[31] &= 127;
    d[31] |= 64;

    let mut r_in = Vec::with_capacity(32 + message.len());
    r_in.extend_from_slice(&d[32..64]);
    r_in.extend_from_slice(message);
    let r_hash = sha512(&r_in);
    let r_scalar = reduce_scalar(&r_hash);

    let r_pt = scalarmult_base(&r_scalar);
    let r_bytes = pt_pack(&r_pt);

    let mut k_in = Vec::with_capacity(64 + message.len());
    k_in.extend_from_slice(&r_bytes);
    k_in.extend_from_slice(public_key);
    k_in.extend_from_slice(message);
    let k_hash = sha512(&k_in);
    let k_scalar = reduce_scalar(&k_hash);

    let mut s_x = [0i64; 64];
    for i in 0..32 {
        s_x[i] = r_scalar[i] as i64;
    }
    for i in 0..32 {
        for j in 0..32 {
            s_x[i + j] += (k_scalar[i] as i64) * (d[j] as i64);
        }
    }
    let s_bytes = mod_l(&mut s_x);

    let mut sig = [0u8; 64];
    sig[..32].copy_from_slice(&r_bytes);
    sig[32..].copy_from_slice(&s_bytes);
    sig
}

// ============================================================================
// Pure-Rust Snappy & LevelDB LocalStorage Scanner for OpenClaw Credentials
// ============================================================================

fn read_varint(data: &[u8], mut pos: usize) -> Option<(u64, usize)> {
    let mut result = 0u64;
    let mut shift = 0u32;
    while pos < data.len() && shift < 64 {
        let byte = data[pos];
        pos += 1;
        result |= ((byte & 0x7f) as u64) << shift;
        if (byte & 0x80) == 0 {
            return Some((result, pos));
        }
        shift += 7;
    }
    None
}

fn snappy_decompress(src: &[u8]) -> Option<Vec<u8>> {
    let (uncompressed_len, mut pos) = read_varint(src, 0)?;
    if uncompressed_len > 16 * 1024 * 1024 {
        return None;
    }
    let mut out = Vec::with_capacity(uncompressed_len as usize);
    while pos < src.len() {
        let tag = src[pos];
        pos += 1;
        match tag & 0x03 {
            0 => {
                let len_code = (tag >> 2) as usize;
                let lit_len = if len_code < 60 {
                    len_code + 1
                } else {
                    let extra_bytes = len_code - 59;
                    if pos + extra_bytes > src.len() {
                        return None;
                    }
                    let mut l = 0usize;
                    for i in 0..extra_bytes {
                        l |= (src[pos + i] as usize) << (8 * i);
                    }
                    pos += extra_bytes;
                    l + 1
                };
                if pos + lit_len > src.len() {
                    return None;
                }
                out.extend_from_slice(&src[pos..pos + lit_len]);
                pos += lit_len;
            }
            1 => {
                if pos >= src.len() {
                    return None;
                }
                let length = (((tag >> 2) & 0x07) as usize) + 4;
                let offset = (((tag >> 5) as usize) << 8) | (src[pos] as usize);
                pos += 1;
                if offset == 0 || offset > out.len() {
                    return None;
                }
                for _ in 0..length {
                    let b = out[out.len() - offset];
                    out.push(b);
                }
            }
            2 => {
                if pos + 2 > src.len() {
                    return None;
                }
                let length = ((tag >> 2) as usize) + 1;
                let offset = u16::from_le_bytes([src[pos], src[pos + 1]]) as usize;
                pos += 2;
                if offset == 0 || offset > out.len() {
                    return None;
                }
                for _ in 0..length {
                    let b = out[out.len() - offset];
                    out.push(b);
                }
            }
            3 => {
                if pos + 4 > src.len() {
                    return None;
                }
                let length = ((tag >> 2) as usize) + 1;
                let offset =
                    u32::from_le_bytes([src[pos], src[pos + 1], src[pos + 2], src[pos + 3]])
                        as usize;
                pos += 4;
                if offset == 0 || offset > out.len() {
                    return None;
                }
                for _ in 0..length {
                    let b = out[out.len() - offset];
                    out.push(b);
                }
            }
            _ => unreachable!(),
        }
    }
    Some(out)
}

fn decode_ldb_blocks(raw: &[u8]) -> Vec<u8> {
    let mut combined = Vec::new();
    // Footer is 48 bytes at the end of an .ldb file
    if raw.len() >= 48 && &raw[raw.len() - 8..] == &[0x57, 0xfb, 0x80, 0x8b, 0x24, 0x75, 0x47, 0xdb]
    {
        let footer_start = raw.len() - 48;
        if let Some((_meta_off, p1)) = read_varint(raw, footer_start) {
            if let Some((_meta_sz, p2)) = read_varint(raw, p1) {
                if let Some((idx_off, p3)) = read_varint(raw, p2) {
                    if let Some((idx_sz, _)) = read_varint(raw, p3) {
                        let idx_off = idx_off as usize;
                        let idx_sz = idx_sz as usize;
                        if idx_off + idx_sz < raw.len() {
                            let idx_block = match raw[idx_off + idx_sz] {
                                0 => Some(raw[idx_off..idx_off + idx_sz].to_vec()),
                                1 => snappy_decompress(&raw[idx_off..idx_off + idx_sz]),
                                _ => None,
                            };
                            if let Some(ib) = idx_block {
                                if ib.len() >= 4 {
                                    let num_restarts = u32::from_le_bytes([
                                        ib[ib.len() - 4],
                                        ib[ib.len() - 3],
                                        ib[ib.len() - 2],
                                        ib[ib.len() - 1],
                                    ])
                                        as usize;
                                    let limit = ib.len().saturating_sub(4 * (num_restarts + 1));
                                    let mut pos = 0usize;
                                    while pos < limit {
                                        let Some((_shared, p_a)) = read_varint(&ib, pos) else {
                                            break;
                                        };
                                        let Some((unshared, p_b)) = read_varint(&ib, p_a) else {
                                            break;
                                        };
                                        let Some((val_len, p_c)) = read_varint(&ib, p_b) else {
                                            break;
                                        };
                                        let val_start = p_c + (unshared as usize);
                                        let val_end = val_start + (val_len as usize);
                                        if val_end > limit {
                                            break;
                                        }
                                        if let Some((b_off, vp1)) = read_varint(&ib, val_start) {
                                            if let Some((b_sz, _)) = read_varint(&ib, vp1) {
                                                let b_off = b_off as usize;
                                                let b_sz = b_sz as usize;
                                                if b_off + b_sz < raw.len() {
                                                    match raw[b_off + b_sz] {
                                                        0 => combined.extend_from_slice(
                                                            &raw[b_off..b_off + b_sz],
                                                        ),
                                                        1 => {
                                                            if let Some(dec) = snappy_decompress(
                                                                &raw[b_off..b_off + b_sz],
                                                            ) {
                                                                combined.extend_from_slice(&dec);
                                                            }
                                                        }
                                                        _ => {}
                                                    }
                                                }
                                            }
                                        }
                                        pos = val_end;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if combined.is_empty() {
        combined.extend_from_slice(raw);
    }
    combined
}

fn extract_json_objects_containing(haystack: &str, needle: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut search_from = 0;
    while let Some(rel_pos) = haystack[search_from..].find(needle) {
        let pos = search_from + rel_pos;
        search_from = pos + needle.len();
        let before = &haystack[..pos];
        for (brace_idx, ch) in before.char_indices().rev().take(800) {
            if ch == '{' {
                let slice = &haystack[brace_idx..];
                let mut depth = 0i32;
                let mut in_str = false;
                let mut esc = false;
                for (end_offset, c) in slice.char_indices().take(2400) {
                    if in_str {
                        if esc {
                            esc = false;
                        } else if c == '\\' {
                            esc = true;
                        } else if c == '"' {
                            in_str = false;
                        }
                    } else if c == '"' {
                        in_str = true;
                    } else if c == '{' {
                        depth += 1;
                    } else if c == '}' {
                        depth -= 1;
                        if depth == 0 {
                            let candidate = &slice[..=end_offset];
                            if let Ok(val) = serde_json::from_str::<Value>(candidate) {
                                out.push(val);
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
    out
}

static CREDENTIALS_CACHE: OnceLock<Mutex<Option<(Instant, Option<OpenClawCredentials>)>>> =
    OnceLock::new();
static ANDROID_ID_CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

pub fn load_openclaw_credentials() -> Option<OpenClawCredentials> {
    let cache = CREDENTIALS_CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((cached_at, ref creds)) = *guard {
            if cached_at.elapsed() < Duration::from_secs(60) {
                return creds.clone();
            }
        }
    }
    let loaded = load_openclaw_credentials_uncached();
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), loaded.clone()));
    }
    loaded
}

pub fn save_openclaw_credentials(creds: &OpenClawCredentials) -> Result<()> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .context("缺少 HOME 目录")?;
    let cache_path = home.join(".config/stepx/openclaw_auth.json");
    if let Some(parent) = cache_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&cache_path, serde_json::to_string_pretty(creds)?)?;
    if let Some(cache) = CREDENTIALS_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
    Ok(())
}

fn load_openclaw_credentials_uncached() -> Option<OpenClawCredentials> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let cache_path = home.join(".config/stepx/openclaw_auth.json");
    if let Ok(text) = fs::read_to_string(&cache_path) {
        if let Ok(creds) = serde_json::from_str::<OpenClawCredentials>(&text) {
            if !creds.device_id.is_empty() && !creds.device_token.is_empty() {
                return Some(creds);
            }
        }
    }

    let leveldb_dirs = [
        home.join(".config/BraveSoftware/Brave-Origin/Default/Local Storage/leveldb"),
        home.join(".config/BraveSoftware/Brave-Browser/Default/Local Storage/leveldb"),
        home.join(".config/google-chrome/Default/Local Storage/leveldb"),
        home.join(".config/chromium/Default/Local Storage/leveldb"),
    ];

    let mut device_id = String::new();
    let mut public_key = String::new();
    let mut private_key = String::new();
    let mut device_token = String::new();

    for dir in &leveldb_dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext != "ldb" && ext != "log" {
                continue;
            }
            let Ok(raw) = fs::read(&path) else {
                continue;
            };
            let decoded = if ext == "ldb" {
                decode_ldb_blocks(&raw)
            } else {
                raw
            };
            let text = String::from_utf8_lossy(&decoded);
            if device_id.is_empty() && text.contains("\"privateKey\"") {
                for val in extract_json_objects_containing(&text, "\"privateKey\"") {
                    if let (Some(id), Some(pk), Some(sk)) = (
                        val.get("deviceId").and_then(Value::as_str),
                        val.get("publicKey").and_then(Value::as_str),
                        val.get("privateKey").and_then(Value::as_str),
                    ) {
                        device_id = id.to_string();
                        public_key = pk.to_string();
                        private_key = sk.to_string();
                        break;
                    }
                }
            }
            if device_token.is_empty() && text.contains("operator.admin") {
                for val in extract_json_objects_containing(&text, "operator.admin") {
                    if let Some(tok) = val
                        .get("tokens")
                        .and_then(|t| t.get("operator"))
                        .and_then(|o| o.get("token"))
                        .and_then(Value::as_str)
                    {
                        device_token = tok.to_string();
                        break;
                    } else if val.get("role").and_then(Value::as_str) == Some("operator") {
                        if let Some(tok) = val.get("token").and_then(Value::as_str) {
                            device_token = tok.to_string();
                            break;
                        }
                    }
                }
            }
            if !device_id.is_empty() && !device_token.is_empty() {
                break;
            }
        }
        if !device_id.is_empty() && !device_token.is_empty() {
            break;
        }
    }

    if !device_id.is_empty()
        && !public_key.is_empty()
        && !private_key.is_empty()
        && !device_token.is_empty()
    {
        let creds = OpenClawCredentials {
            device_id,
            public_key,
            private_key,
            device_token,
        };
        let _ = save_openclaw_credentials(&creds);
        return Some(creds);
    }
    None
}

// ============================================================================
// Minimal RFC 6455 WebSocket Client for OpenClaw Gateway (ws://127.0.0.1:18789)
// ============================================================================

pub struct GatewayWs {
    stream: TcpStream,
    buf: Vec<u8>,
    next_id: u64,
    pub gateway_version: String,
    request_deadline: Option<Instant>,
    _forward: Option<GatewayForward>,
}

struct GatewayForward {
    adb: Adb,
    serial: String,
    port: String,
}
impl Drop for GatewayForward {
    fn drop(&mut self) {
        let _ = self.adb.run_timeout(
            &["-s", &self.serial, "forward", "--remove", &self.port],
            Duration::from_secs(3),
        );
    }
}

impl GatewayWs {
    pub fn connect(adb: &Adb, device: &Device) -> Result<Self> {
        device.verify(adb)?;
        let port: u16 = adb
            .run(&["-s", &device.transport, "forward", "tcp:0", "tcp:18789"])?
            .trim()
            .parse()
            .context("无法建立当前手机的 OpenClaw 后台连接")?;
        let forward = GatewayForward {
            adb: adb.clone(),
            serial: device.transport.clone(),
            port: format!("tcp:{port}"),
        };
        let mut ws = Self::connect_at(adb, Some(device), port)?;
        ws._forward = Some(forward);
        Ok(ws)
    }

    pub fn connect_local(adb: &Adb, device: Option<&Device>) -> Result<Self> {
        Self::connect_at(adb, device, 18789)
    }

    fn connect_at(adb: &Adb, device: Option<&Device>, port: u16) -> Result<Self> {
        let addr: SocketAddr = ([127, 0, 0, 1], port).into();
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))
            .context("无法连接手机 OpenClaw 网关 (127.0.0.1:18789)")?;
        stream.set_read_timeout(Some(Duration::from_secs(6)))?;
        stream.set_write_timeout(Some(Duration::from_secs(6)))?;

        let req = concat!(
            "GET / HTTP/1.1\r\n",
            "Host: 127.0.0.1:18789\r\n",
            "Origin: http://127.0.0.1:18789\r\n",
            "Upgrade: websocket\r\n",
            "Connection: Upgrade\r\n",
            "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
            "Sec-WebSocket-Version: 13\r\n",
            "\r\n"
        );
        stream.write_all(req.as_bytes())?;
        stream.flush()?;

        let mut buf = Vec::with_capacity(4096);
        let mut tmp = [0u8; 2048];
        let header_end = loop {
            let n = stream.read(&mut tmp)?;
            if n == 0 {
                bail!("OpenClaw 网关握手连接被关闭");
            }
            buf.extend_from_slice(&tmp[..n]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
            if buf.len() > 16384 {
                bail!("OpenClaw 网关 HTTP 响应头过长");
            }
        };
        let header_str = String::from_utf8_lossy(&buf[..header_end]);
        if !header_str.contains("101") {
            bail!(
                "OpenClaw 网关拒绝升级 WebSocket: {}",
                header_str.lines().next().unwrap_or("")
            );
        }
        buf.drain(..header_end);

        let mut ws = Self {
            stream,
            buf,
            next_id: 1,
            gateway_version: String::new(),
            request_deadline: None,
            _forward: None,
        };

        // 1. Read connect.challenge
        let challenge_raw = ws.read_frame()?;
        let challenge: Value =
            serde_json::from_str(&challenge_raw).context("无法解析 OpenClaw connect.challenge")?;
        let nonce = challenge
            .get("payload")
            .and_then(|p| p.get("nonce"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 2. Build connect request with device signature if credentials exist, else derived token
        let signed_at_ms = chrono::Utc::now().timestamp_millis();
        let connect_req = if let Some(creds) = load_openclaw_credentials() {
            let pk_bytes_vec = URL_SAFE_NO_PAD
                .decode(creds.public_key.trim_end_matches('='))
                .context("无效的 OpenClaw publicKey")?;
            let sk_bytes_vec = URL_SAFE_NO_PAD
                .decode(creds.private_key.trim_end_matches('='))
                .context("无效的 OpenClaw privateKey")?;
            if pk_bytes_vec.len() != 32 || sk_bytes_vec.len() != 32 {
                bail!("OpenClaw Ed25519 密钥长度异常");
            }
            let mut pk = [0u8; 32];
            let mut sk = [0u8; 32];
            pk.copy_from_slice(&pk_bytes_vec);
            sk.copy_from_slice(&sk_bytes_vec);

            let scopes = [
                "operator.admin",
                "operator.read",
                "operator.write",
                "operator.approvals",
                "operator.pairing",
            ];
            let v2_payload = format!(
                "v2|{}|openclaw-control-ui|webchat|operator|{}|{}|{}|{}",
                creds.device_id,
                scopes.join(","),
                signed_at_ms,
                creds.device_token,
                nonce
            );
            let sig = ed25519_sign(&sk, &pk, v2_payload.as_bytes());
            let sig_b64 = URL_SAFE_NO_PAD.encode(sig);

            json!({
                "type": "req",
                "id": "connect-1",
                "method": "connect",
                "params": {
                    "minProtocol": 3,
                    "maxProtocol": 3,
                    "client": {
                        "id": "openclaw-control-ui",
                        "version": "control-ui",
                        "platform": "Linux x86_64",
                        "mode": "webchat"
                    },
                    "role": "operator",
                    "scopes": scopes,
                    "device": {
                        "id": creds.device_id,
                        "publicKey": creds.public_key,
                        "signature": sig_b64,
                        "signedAt": signed_at_ms,
                        "nonce": nonce
                    },
                    "caps": ["tool-events"],
                    "auth": {
                        "token": creds.device_token,
                        "deviceToken": creds.device_token
                    }
                }
            })
        } else {
            let android_id = if let Some(d) = device {
                let key = if d.serial.is_empty() {
                    &d.transport
                } else {
                    &d.serial
                };
                let cache = ANDROID_ID_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
                let cached = cache
                    .lock()
                    .ok()
                    .and_then(|g| g.get(key).cloned())
                    .filter(|s| !s.is_empty());
                if let Some(id) = cached {
                    id
                } else {
                    let id = adb
                        .shell(&d.transport, &["settings", "get", "secure", "android_id"])
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if !id.is_empty() {
                        if let Ok(mut g) = cache.lock() {
                            g.insert(key.clone(), id.clone());
                        }
                    }
                    id
                }
            } else {
                "23392eef6ce29cb0".to_string()
            };
            let tok = crate::device::derive_openclaw_token(&android_id);
            json!({
                "type": "req",
                "id": "connect-1",
                "method": "connect",
                "params": {
                    "minProtocol": 3,
                    "maxProtocol": 3,
                    "client": {
                        "id": "openclaw-control-ui",
                        "version": "control-ui",
                        "platform": "Linux x86_64",
                        "mode": "webchat"
                    },
                    "role": "operator",
                    "scopes": ["operator.admin", "operator.read", "operator.write"],
                    "auth": {
                        "token": tok
                    }
                }
            })
        };

        ws.write_text_frame(&serde_json::to_string(&connect_req)?)?;
        let hello = ws.wait_response("connect-1")?;
        ws.gateway_version = hello
            .get("server")
            .and_then(|s| s.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("2026.4.26-stepclaw.21")
            .to_string();
        Ok(ws)
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.call_with_timeout(method, params, Duration::from_secs(6))
    }

    pub fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        self.next_id += 1;
        let id = format!("req-{}", self.next_id);
        let req = json!({
            "type": "req",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_text_frame(&serde_json::to_string(&req)?)?;
        self.request_deadline = Some(Instant::now() + timeout);
        let response = self.wait_response(&id);
        self.request_deadline = None;
        let _ = self.stream.set_read_timeout(Some(Duration::from_secs(6)));
        response
    }

    fn wait_response(&mut self, expected_id: &str) -> Result<Value> {
        let mut frames = 0usize;
        while self.request_deadline.is_some() || frames < 32 {
            frames += 1;
            let raw = self.read_frame()?;
            let Ok(val) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            if val.get("type").and_then(Value::as_str) == Some("res")
                && val.get("id").and_then(Value::as_str) == Some(expected_id)
            {
                if val.get("ok").and_then(Value::as_bool) == Some(true) {
                    return Ok(val.get("payload").cloned().unwrap_or(Value::Null));
                } else {
                    let msg = val
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(Value::as_str)
                        .unwrap_or("OpenClaw RPC 调用失败");
                    bail!("{msg}");
                }
            }
        }
        bail!("等待 OpenClaw 响应超时 ({expected_id})")
    }

    fn read_exact_buf(&mut self, n: usize) -> Result<Vec<u8>> {
        let mut tmp = [0u8; 4096];
        while self.buf.len() < n {
            if let Some(deadline) = self.request_deadline {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .context("OpenClaw 请求超时，请刷新确认执行结果")?;
                self.stream
                    .set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
            }
            let read_n = self.stream.read(&mut tmp)?;
            if read_n == 0 {
                bail!("OpenClaw WebSocket 连接已断开");
            }
            self.buf.extend_from_slice(&tmp[..read_n]);
        }
        let out: Vec<u8> = self.buf.drain(..n).collect();
        Ok(out)
    }

    fn read_frame(&mut self) -> Result<String> {
        loop {
            let hdr = self.read_exact_buf(2)?;
            let opcode = hdr[0] & 0x0f;
            let masked = (hdr[1] & 0x80) != 0;
            let mut len = (hdr[1] & 0x7f) as usize;
            if len == 126 {
                let ext = self.read_exact_buf(2)?;
                len = u16::from_be_bytes([ext[0], ext[1]]) as usize;
            } else if len == 127 {
                let ext = self.read_exact_buf(8)?;
                len = u64::from_be_bytes([
                    ext[0], ext[1], ext[2], ext[3], ext[4], ext[5], ext[6], ext[7],
                ]) as usize;
            }
            if len > 16 * 1024 * 1024 {
                bail!("OpenClaw WebSocket 数据帧过大");
            }
            let mask_key = if masked {
                let m = self.read_exact_buf(4)?;
                Some([m[0], m[1], m[2], m[3]])
            } else {
                None
            };
            let mut payload = self.read_exact_buf(len)?;
            if let Some(m) = mask_key {
                for (i, b) in payload.iter_mut().enumerate() {
                    *b ^= m[i & 3];
                }
            }
            match opcode {
                0x8 => bail!("OpenClaw WebSocket 已关闭"),
                0x9 => {
                    // Ping -> Pong
                    let _ = self.write_frame(0x8A, &payload);
                }
                0xA => continue,
                _ => return Ok(String::from_utf8_lossy(&payload).into_owned()),
            }
        }
    }

    fn write_text_frame(&mut self, text: &str) -> Result<()> {
        self.write_frame(0x81, text.as_bytes())
    }

    fn write_frame(&mut self, first_byte: u8, payload: &[u8]) -> Result<()> {
        let mask: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];
        let mut frame = Vec::with_capacity(14 + payload.len());
        frame.push(first_byte);
        let len = payload.len();
        if len < 126 {
            frame.push(0x80 | (len as u8));
        } else if len <= 65535 {
            frame.push(0x80 | 126);
            frame.extend_from_slice(&(len as u16).to_be_bytes());
        } else {
            frame.push(0x80 | 127);
            frame.extend_from_slice(&(len as u64).to_be_bytes());
        }
        frame.extend_from_slice(&mask);
        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask[i & 3]);
        }
        self.stream.write_all(&frame)?;
        self.stream.flush()?;
        Ok(())
    }
}

// ============================================================================
// High-level OpenClaw + StepClaw Amoo Data Sync & Operations
// ============================================================================

fn clean_openclaw_user_text(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("Conversation info (untrusted metadata):") {
        if let Some(idx) = trimmed.find("```\n\n") {
            let after = trimmed[idx + 5..].trim();
            if !after.is_empty() {
                return after.to_string();
            }
        } else if let Some(idx) = trimmed.rfind("```") {
            let after = trimmed[idx + 3..].trim();
            if !after.is_empty() {
                return after.to_string();
            }
        }
    }
    trimmed.to_string()
}

pub fn format_timestamp_ms(ms: i64) -> String {
    if ms <= 0 {
        return "刚刚".into();
    }
    use chrono::{Local, TimeZone};
    if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
        let now = Local::now();
        if dt.date_naive() == now.date_naive() {
            dt.format("%H:%M").to_string()
        } else {
            dt.format("%m-%d %H:%M").to_string()
        }
    } else {
        "刚刚".into()
    }
}

pub fn parse_chat_history_messages(messages_arr: &[Value]) -> Vec<RealChatMessage> {
    let mut out = Vec::new();
    for msg in messages_arr {
        let role = msg
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("assistant")
            .to_string();
        let timestamp_ms = msg.get("timestamp").and_then(Value::as_i64).unwrap_or(0);
        let time_label = format_timestamp_ms(timestamp_ms);
        let stop_reason = msg.get("stopReason").and_then(Value::as_str).unwrap_or("");
        let error_message = msg
            .get("errorMessage")
            .and_then(Value::as_str)
            .map(str::to_string);
        let model = msg
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or("stepos-model-auto")
            .to_string();
        let provider = msg
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("stepos")
            .to_string();

        let mut text_parts = Vec::new();
        let mut thinking_steps = Vec::new();
        let mut tool_names = Vec::new();

        if let Some(content_arr) = msg.get("content").and_then(Value::as_array) {
            for item in content_arr {
                let kind = item.get("type").and_then(Value::as_str).unwrap_or("text");
                match kind {
                    "text" => {
                        if let Some(t) = item.get("text").and_then(Value::as_str) {
                            let cleaned = if role == "user" {
                                clean_openclaw_user_text(t)
                            } else {
                                t.trim().to_string()
                            };
                            if !cleaned.is_empty() {
                                text_parts.push(cleaned);
                            }
                        }
                    }
                    "thinking" => {
                        if let Some(th) = item.get("thinking").and_then(Value::as_str) {
                            let th = th.trim();
                            if !th.is_empty() {
                                thinking_steps.push(th.to_string());
                            }
                        }
                    }
                    "toolCall" | "tool_use" => {
                        let name = item.get("name").and_then(Value::as_str).unwrap_or("tool");
                        tool_names.push(name.to_string());
                        thinking_steps.push(format!("调用工具：{name}"));
                    }
                    _ => {}
                }
            }
        } else if let Some(t) = msg.get("content").and_then(Value::as_str) {
            let cleaned = if role == "user" {
                clean_openclaw_user_text(t)
            } else {
                t.trim().to_string()
            };
            if !cleaned.is_empty() {
                text_parts.push(cleaned);
            }
        }

        let is_error = stop_reason == "error" || error_message.is_some();
        let mut text = text_parts.join("\n");
        if role != "user" && text.is_empty() {
            if let Some(err) = &error_message {
                text = format!("手机 OpenClaw 模型请求返回错误：{err}\n模型：{provider}/{model}");
            }
        }

        if role == "user" && text.is_empty() {
            continue;
        }

        let tools_summary = if is_error {
            format!("请求失败 · {}", error_message.as_deref().unwrap_or("error"))
        } else if !tool_names.is_empty() {
            tool_names.join(" · ")
        } else {
            format!("{provider}/{model}")
        };

        if role != "user" && thinking_steps.is_empty() {
            if is_error {
                thinking_steps.push(format!("1. 会话路由至模型：{provider}/{model}"));
                thinking_steps.push(format!(
                    "2. 模型代理 (127.0.0.1:18791) 返回状态：{}",
                    error_message.as_deref().unwrap_or("error")
                ));
            } else {
                thinking_steps.push(format!("1. 模型推理：{provider}/{model}"));
            }
        }

        out.push(RealChatMessage {
            usage: crate::amoo_usage::parse_message(msg),
            role,
            text,
            thinking: thinking_steps,
            tools_summary,
            timestamp_ms,
            time_label,
            is_error,
            error_message,
            model,
            provider,
            ..Default::default()
        });
    }
    out
}

/// The desktop Amoo page reads Amoo's AgentBus service, never its screen.
pub fn fetch_real_amoo_data(adb: &Adb, device: &Device) -> Result<RealAmooData> {
    crate::amoo::fetch(adb, device, None)
}

pub fn send_openclaw_chat(
    adb: &Adb,
    device: &Device,
    session_key: &str,
    message: &str,
) -> Result<RealAmooData> {
    let mut ws = GatewayWs::connect(adb, device)?;
    let idempotency_key = format!(
        "{:08x}-{:04x}-{:04x}-{:04x}-{:012x}",
        rand::random::<u32>(),
        rand::random::<u16>(),
        rand::random::<u16>(),
        rand::random::<u16>(),
        rand::random::<u64>() & 0xffffffffffff
    );
    let _ = ws.call(
        "chat.send",
        json!({
            "sessionKey": session_key,
            "message": message,
            "idempotencyKey": idempotency_key,
        }),
    )?;
    // Wait briefly for the gateway to process and record the assistant turn / error
    std::thread::sleep(Duration::from_millis(900));
    drop(ws);
    fetch_real_amoo_data(adb, device)
}

pub fn reset_openclaw_session(
    adb: &Adb,
    device: &Device,
    session_key: &str,
) -> Result<RealAmooData> {
    let mut ws = GatewayWs::connect(adb, device)?;
    let _ = ws.call(
        "sessions.reset",
        json!({
            "key": session_key,
        }),
    );
    drop(ws);
    fetch_real_amoo_data(adb, device)
}

pub fn set_openclaw_skill_enabled(
    adb: &Adb,
    device: &Device,
    skill_key: &str,
    enabled: bool,
) -> Result<RealAmooData> {
    let mut ws = GatewayWs::connect(adb, device)?;
    let _ = ws.call(
        "skills.update",
        json!({
            "skillKey": skill_key,
            "enabled": enabled,
        }),
    );
    drop(ws);
    fetch_real_amoo_data(adb, device)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_events_do_not_extend_the_request_deadline() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let event = br#"{"type":"event","event":"tick"}"#;
            let mut frame = vec![0x81, event.len() as u8];
            frame.extend_from_slice(event);
            for _ in 0..30 {
                if stream.write_all(&frame).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let mut ws = GatewayWs {
            stream: TcpStream::connect(addr).unwrap(),
            buf: vec![],
            next_id: 0,
            gateway_version: String::new(),
            request_deadline: None,
            _forward: None,
        };
        let start = Instant::now();
        assert!(ws
            .call_with_timeout("read-only-test", json!({}), Duration::from_millis(70))
            .is_err());
        assert!(start.elapsed() < Duration::from_millis(500));
        assert!(ws.request_deadline.is_none());
        drop(ws);
        server.join().unwrap();
    }

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn rfc8032_ed25519_test_vector_1() {
        let seed_vec = unhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60");
        let pk_vec = unhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a");
        let expected_sig = unhex(
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
        );
        let mut seed = [0u8; 32];
        let mut pk = [0u8; 32];
        seed.copy_from_slice(&seed_vec);
        pk.copy_from_slice(&pk_vec);
        let sig = ed25519_sign(&seed, &pk, b"");
        assert_eq!(&sig[..], &expected_sig[..]);
    }

    #[test]
    fn clean_user_metadata_prefix() {
        let raw = "Conversation info (untrusted metadata):\n```json\n{\n  \"message_id\": \"123\"\n}\n```\n\n你好";
        assert_eq!(clean_openclaw_user_text(raw), "你好");
    }

    #[test]
    #[ignore = "requires personal browser credentials and an explicitly connected phone"]
    fn live_openclaw_gateway_sync_check() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let dir = home.join(".config/BraveSoftware/Brave-Origin/Default/Local Storage/leveldb");
        for entry in fs::read_dir(&dir).unwrap().flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) == Some("ldb") {
                let raw = fs::read(&p).unwrap();
                let dec = decode_ldb_blocks(&raw);
                let txt = String::from_utf8_lossy(&dec);
                println!(
                    "LDB {:?}: raw={} dec={} has_pk={} has_op={}",
                    p.file_name(),
                    raw.len(),
                    dec.len(),
                    txt.contains("privateKey"),
                    txt.contains("operator")
                );
            }
        }
        let creds = load_openclaw_credentials()
            .expect("should load OpenClaw credentials from leveldb/cache");
        assert!(!creds.device_id.is_empty());
        assert!(!creds.device_token.is_empty());
        if let Ok(adb) = crate::adb::Adb::discover() {
            if let Ok(device) = crate::device::Device::inspect(&adb, &std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit test phone")) {
                let data = fetch_real_amoo_data(&adb, &device)
                    .expect("should fetch real amoo data from phone");
                println!(
                    "LIVE_SYNC: version={} sessions={} skills={}/{} mem_init={}",
                    data.gateway_version,
                    data.sessions.len(),
                    data.eligible_skills_count,
                    data.total_skills_count,
                    data.memory_initialized
                );
                for s in &data.sessions {
                    println!(
                        "SESSION: key={} title={} status={} meta={} msgs={}",
                        s.key,
                        s.title,
                        s.status,
                        s.meta,
                        s.messages.len()
                    );
                    for m in &s.messages {
                        println!(
                            "  MSG [{}] ({}): {} (err={:?})",
                            m.role, m.time_label, m.text, m.error_message
                        );
                    }
                }
                for sk in &data.skills {
                    println!(
                        "SKILL: key={} name={} display={} enabled={} eligible={} desc={}",
                        sk.skill_key,
                        sk.name,
                        sk.display_name,
                        sk.enabled,
                        sk.eligible,
                        sk.description
                    );
                }
            }
        }
    }
}
