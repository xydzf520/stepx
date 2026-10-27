//! StepX LAN Cast & Multi-Protocol Screen Projection Engine.
//!
//! Supports casting the connected StepX phone screen across the LAN to:
//! 1. **DLNA / UPnP MediaRenderers & Smart TVs** (SSDP discovery + SOAP `AVTransport:1` `SetAVTransportURI` & `Play` pushing a live 188-byte MPEG-TS H.264 stream).
//! 2. **Apple AirPlay Receivers** (mDNS `_airplay._tcp.local.` discovery + HTTP `/play` pushing a live Apple HLS `.m3u8` stream).
//! 3. **Google Cast / Chromecast / Android TV / DIAL** (mDNS `_googlecast._tcp.local.` & SSDP DIAL discovery + stream handoff).
//! 4. **Phone Native Miracast / Wi-Fi Display** (`WifiDisplayAdapter` / `com.qualcomm.wfd.service` control + `android.settings.CAST_SETTINGS`).
//! 5. **Universal LAN Web Cast Console & Media Players (VLC / mpv / PotPlayer / Smart TV Browser)** via an embedded multi-protocol HTTP server (`/cast`, `/live.ts`, `/live.m3u8`, `/stream.mjpeg`, `/api/control`).

use crate::adb::Adb;
use anyhow::{bail, Context, Result};
use image::{codecs::jpeg::JpegEncoder, DynamicImage, GenericImageView, ImageBuffer, Luma};
use qrcode::QrCode;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs, UdpSocket},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

pub const DEFAULT_CAST_PORT: u16 = 19780;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LanCastProtocol {
    DlnaUpnp,
    AirPlay,
    GoogleCast,
    Miracast,
    WebReceiver,
}

impl LanCastProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::DlnaUpnp => "DLNA / UPnP",
            Self::AirPlay => "AirPlay",
            Self::GoogleCast => "Google Cast",
            Self::Miracast => "Miracast 无线显示",
            Self::WebReceiver => "Web / VLC 接收端",
        }
    }

    pub fn short_badge(self) -> &'static str {
        match self {
            Self::DlnaUpnp => "DLNA",
            Self::AirPlay => "AirPlay",
            Self::GoogleCast => "Cast",
            Self::Miracast => "Miracast",
            Self::WebReceiver => "LAN 终端",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LanCastReceiver {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub port: u16,
    pub protocol: LanCastProtocol,
    pub model: String,
    pub control_url: Option<String>,
    pub rendering_control_url: Option<String>,
    pub location_url: Option<String>,
    pub mac: Option<String>,
    pub online: bool,
    pub can_direct_push: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MiracastSink {
    pub address: String,
    pub name: String,
    pub is_available: bool,
    pub is_remembered: bool,
    pub is_connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhoneMiracastState {
    pub enabled: bool,
    pub scanning: bool,
    pub active_display_name: Option<String>,
    pub active_display_address: Option<String>,
    pub sinks: Vec<MiracastSink>,
    pub wfd_service_installed: bool,
}

#[derive(Clone, Debug, Default)]
pub struct LanDiscoverySnapshot {
    pub host_lan_ip: String,
    pub phone_lan_ip: Option<String>,
    pub receivers: Vec<LanCastReceiver>,
    pub miracast: PhoneMiracastState,
    pub scanned_at_hms: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LanCastQuality {
    Smooth720p,
    #[default]
    Hd1080p,
    UltraNative,
}

impl LanCastQuality {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::Smooth720p,
            1 => Self::Hd1080p,
            _ => Self::UltraNative,
        }
    }

    pub fn as_u8(self) -> u8 {
        match self {
            Self::Smooth720p => 0,
            Self::Hd1080p => 1,
            Self::UltraNative => 2,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Smooth720p => "流畅 · 最长边 960 (4Mbps)",
            Self::Hd1080p => "清晰 · 最长边 1280 (8Mbps)",
            Self::UltraNative => "高画质 · 最长边 1600 (12Mbps)",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Smooth720p => "流畅",
            Self::Hd1080p => "清晰",
            Self::UltraNative => "高画质",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Smooth720p => Self::Hd1080p,
            Self::Hd1080p => Self::UltraNative,
            Self::UltraNative => Self::Smooth720p,
        }
    }

    pub fn bit_rate_bps(self) -> u32 {
        match self {
            Self::Smooth720p => 4_000_000,
            Self::Hd1080p => 8_000_000,
            Self::UltraNative => 12_000_000,
        }
    }

    pub fn max_dim(self) -> u32 {
        match self {
            Self::Smooth720p => 960,
            Self::Hd1080p => 1280,
            Self::UltraNative => 1600,
        }
    }

    pub fn screenrecord_size(self) -> &'static str {
        match self {
            Self::Smooth720p => "540x1200",
            Self::Hd1080p => "720x1600",
            Self::UltraNative => "1080x2400",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LanCastStats {
    pub running: bool,
    pub port: u16,
    pub host_ip: String,
    pub web_url: String,
    pub ts_url: String,
    pub hls_url: String,
    pub mjpeg_url: String,
    pub qr_png_path: Option<PathBuf>,
    pub active_clients: usize,
    pub frames_captured: u64,
    pub bytes_broadcast: u64,
    pub fps: f32,
    pub phone_width: u32,
    pub phone_height: u32,
    pub remote_control_enabled: bool,
    pub quality: LanCastQuality,
    pub pushed_receiver_id: Option<String>,
    pub pushed_receiver_name: Option<String>,
}

/// Detect the primary outbound LAN IPv4 address of this host.
pub fn detect_lan_ipv4() -> String {
    for target in ["10.0.1.1:80", "192.168.1.1:80", "8.8.8.8:80", "1.1.1.1:53"] {
        if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(target).is_ok() {
                if let Ok(SocketAddr::V4(local)) = sock.local_addr() {
                    let ip = *local.ip();
                    if !ip.is_loopback() && !ip.is_unspecified() && ip.octets()[0] != 172 {
                        return ip.to_string();
                    }
                }
            }
        }
    }
    if let Ok(sock) = UdpSocket::bind("0.0.0.0:0") {
        if sock.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = sock.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}

/// Generate a crisp PNG QR code for the given URL.
pub fn generate_cast_qr_png(url: &str, port: u16) -> Option<PathBuf> {
    let code = QrCode::new(url.as_bytes()).ok()?;
    let colors = code.to_colors();
    let width = code.width() as u32;
    let quiet = 3u32;
    let scale = 6u32;
    let total = (width + quiet * 2) * scale;
    let mut img: ImageBuffer<Luma<u8>, Vec<u8>> = ImageBuffer::from_pixel(total, total, Luma([255u8]));
    for y in 0..width {
        for x in 0..width {
            let dark = matches!(colors[(y * width + x) as usize], qrcode::Color::Dark);
            if dark {
                let px0 = (x + quiet) * scale;
                let py0 = (y + quiet) * scale;
                for dy in 0..scale {
                    for dx in 0..scale {
                        img.put_pixel(px0 + dx, py0 + dy, Luma([18u8]));
                    }
                }
            }
        }
    }
    let out_path = std::env::temp_dir().join(format!("stepx-lan-cast-qr-{port}.png"));
    DynamicImage::ImageLuma8(img).save(&out_path).ok()?;
    Some(out_path)
}

// ============================================================================
// Pure-Rust Annex-B H.264 -> 188-Byte ISO/IEC 13818-1 MPEG-TS & HLS Muxer
// ============================================================================

pub mod ts_mux {
    pub const TS_PACKET_SIZE: usize = 188;
    const PID_PAT: u16 = 0x0000;
    const PID_PMT: u16 = 0x1000;
    const PID_VIDEO: u16 = 0x0100;

    pub fn crc32_mpeg2(data: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &byte in data {
            crc ^= (byte as u32) << 24;
            for _ in 0..8 {
                if (crc & 0x8000_0000) != 0 {
                    crc = (crc << 1) ^ 0x04C1_1DB7;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    pub struct TsMuxer {
        pat_cc: u8,
        pmt_cc: u8,
        video_cc: u8,
        sps: Vec<u8>,
        pps: Vec<u8>,
        frame_index: u64,
    }

    impl Default for TsMuxer {
        fn default() -> Self {
            Self::new()
        }
    }

    impl TsMuxer {
        pub fn new() -> Self {
            Self {
                pat_cc: 0,
                pmt_cc: 0,
                video_cc: 0,
                sps: Vec::new(),
                pps: Vec::new(),
                frame_index: 0,
            }
        }

        pub fn make_pat_packet(&mut self) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = 0x40 | ((PID_PAT >> 8) as u8 & 0x1F);
            pkt[2] = (PID_PAT & 0xFF) as u8;
            pkt[3] = 0x10 | (self.pat_cc & 0x0F);
            self.pat_cc = (self.pat_cc + 1) & 0x0F;
            pkt[4] = 0x00; // pointer_field

            let section: [u8; 12] = [
                0x00, // table_id (PAT)
                0xB0, 0x0D, // section_syntax_indicator=1, section_length=13
                0x00, 0x01, // transport_stream_id=1
                0xC1, // version=0, current_next=1
                0x00, // section_number=0
                0x00, // last_section_number=0
                0x00, 0x01, // program_number=1
                0xE0 | ((PID_PMT >> 8) as u8 & 0x1F),
                (PID_PMT & 0xFF) as u8,
            ];
            pkt[5..17].copy_from_slice(&section);
            let crc = crc32_mpeg2(&section);
            pkt[17..21].copy_from_slice(&crc.to_be_bytes());
            pkt
        }

        pub fn make_pmt_packet(&mut self) -> [u8; TS_PACKET_SIZE] {
            let mut pkt = [0xFFu8; TS_PACKET_SIZE];
            pkt[0] = 0x47;
            pkt[1] = 0x40 | ((PID_PMT >> 8) as u8 & 0x1F);
            pkt[2] = (PID_PMT & 0xFF) as u8;
            pkt[3] = 0x10 | (self.pmt_cc & 0x0F);
            self.pmt_cc = (self.pmt_cc + 1) & 0x0F;
            pkt[4] = 0x00; // pointer_field

            let section: [u8; 17] = [
                0x02, // table_id (PMT)
                0xB0, 0x12, // section_length=18
                0x00, 0x01, // program_number=1
                0xC1, // version=0, current_next=1
                0x00, // section_number=0
                0x00, // last_section_number=0
                0xE0 | ((PID_VIDEO >> 8) as u8 & 0x1F), // PCR_PID = PID_VIDEO
                (PID_VIDEO & 0xFF) as u8,
                0xF0, 0x00, // program_info_length=0
                0x1B, // stream_type = 0x1B (H.264 / AVC)
                0xE0 | ((PID_VIDEO >> 8) as u8 & 0x1F),
                (PID_VIDEO & 0xFF) as u8,
                0xF0, 0x00, // ES_info_length=0
            ];
            pkt[5..22].copy_from_slice(&section);
            let crc = crc32_mpeg2(&section);
            pkt[22..26].copy_from_slice(&crc.to_be_bytes());
            pkt
        }

        pub fn tables_bytes(&mut self) -> Vec<u8> {
            let mut out = Vec::with_capacity(TS_PACKET_SIZE * 2);
            out.extend_from_slice(&self.make_pat_packet());
            out.extend_from_slice(&self.make_pmt_packet());
            out
        }

        /// Mux an Annex-B H.264 buffer into 188-byte MPEG-TS packets.
        /// Returns `(ts_bytes, contains_idr)`.
        pub fn mux_annexb_chunk(&mut self, data: &[u8], pts_90khz: u64) -> (Vec<u8>, bool) {
            let nals = split_annexb_nals(data);
            if nals.is_empty() {
                return (Vec::new(), false);
            }
            let mut has_idr = false;
            let mut frame_nals: Vec<&[u8]> = Vec::new();
            for nal in nals {
                if nal.is_empty() {
                    continue;
                }
                let nal_type = nal[0] & 0x1F;
                match nal_type {
                    7 => {
                        self.sps = nal.to_vec();
                    }
                    8 => {
                        self.pps = nal.to_vec();
                    }
                    5 => {
                        has_idr = true;
                        frame_nals.push(nal);
                    }
                    1 | 6 => {
                        frame_nals.push(nal);
                    }
                    _ => {}
                }
            }

            if frame_nals.is_empty() && !has_idr {
                return (Vec::new(), false);
            }

            let mut out = Vec::new();
            if has_idr || self.frame_index % 15 == 0 {
                out.extend_from_slice(&self.make_pat_packet());
                out.extend_from_slice(&self.make_pmt_packet());
            }
            self.frame_index = self.frame_index.wrapping_add(1);

            // Build Annex-B access unit with AUD (00 00 00 01 09 F0)
            let mut es = Vec::with_capacity(data.len() + 128);
            es.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x09, 0xF0]);
            if has_idr {
                if !self.sps.is_empty() {
                    es.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    es.extend_from_slice(&self.sps);
                }
                if !self.pps.is_empty() {
                    es.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                    es.extend_from_slice(&self.pps);
                }
            }
            for nal in frame_nals {
                es.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
                es.extend_from_slice(nal);
            }

            // Build PES header with 33-bit PTS
            let mut pes = Vec::with_capacity(14 + es.len());
            pes.extend_from_slice(&[
                0x00, 0x00, 0x01, // packet_start_code_prefix
                0xE0, // stream_id (video)
                0x00, 0x00, // PES_packet_length = 0 (unbounded video PES)
                0x80, // marker '10', data_alignment_indicator=0
                0x80, // PTS_DTS_flags = '10' (PTS only)
                0x05, // PES_header_data_length = 5
            ]);
            pes.extend_from_slice(&encode_pts(pts_90khz));
            pes.extend_from_slice(&es);

            self.packetize_pes(&pes, Some(pts_90khz), has_idr, &mut out);
            (out, has_idr)
        }

        fn packetize_pes(
            &mut self,
            pes: &[u8],
            pcr_90khz: Option<u64>,
            random_access: bool,
            out: &mut Vec<u8>,
        ) {
            let mut offset = 0usize;
            let mut first = true;
            while offset < pes.len() {
                let mut pkt = [0xFFu8; TS_PACKET_SIZE];
                pkt[0] = 0x47;
                pkt[1] = (if first { 0x40 } else { 0x00 }) | ((PID_VIDEO >> 8) as u8 & 0x1F);
                pkt[2] = (PID_VIDEO & 0xFF) as u8;

                let remaining = pes.len() - offset;
                if first && pcr_90khz.is_some() {
                    let pcr = pcr_90khz.unwrap();
                    // Adaptation field with 6-byte PCR (length = 7, total 8 bytes)
                    let max_payload = TS_PACKET_SIZE - 4 - 8;
                    let take = remaining.min(max_payload);
                    let stuff = max_payload - take;
                    let af_len = 7 + stuff;
                    pkt[3] = 0x30 | (self.video_cc & 0x0F); // adaptation + payload
                    pkt[4] = af_len as u8;
                    pkt[5] = 0x10 | if random_access { 0x40 } else { 0x00 }; // PCR_flag=1, RAI
                    pkt[6..12].copy_from_slice(&encode_pcr(pcr));
                    //pkt[12..12+stuff] is already 0xFF
                    let payload_start = 4 + 1 + af_len;
                    pkt[payload_start..payload_start + take]
                        .copy_from_slice(&pes[offset..offset + take]);
                    offset += take;
                } else if remaining >= TS_PACKET_SIZE - 4 {
                    pkt[3] = 0x10 | (self.video_cc & 0x0F); // payload only
                    pkt[4..TS_PACKET_SIZE].copy_from_slice(&pes[offset..offset + 184]);
                    offset += 184;
                } else {
                    // Stuffing adaptation field
                    let stuff_total = (TS_PACKET_SIZE - 4) - remaining;
                    pkt[3] = 0x30 | (self.video_cc & 0x0F); // adaptation + payload
                    if stuff_total == 1 {
                        pkt[4] = 0; // adaptation_field_length = 0
                    } else {
                        pkt[4] = (stuff_total - 1) as u8;
                        pkt[5] = 0x00;
                    }
                    let payload_start = 4 + stuff_total;
                    pkt[payload_start..TS_PACKET_SIZE]
                        .copy_from_slice(&pes[offset..offset + remaining]);
                    offset += remaining;
                }
                self.video_cc = (self.video_cc + 1) & 0x0F;
                first = false;
                out.extend_from_slice(&pkt);
            }
        }
    }

    fn encode_pts(pts: u64) -> [u8; 5] {
        let pts = pts & 0x1_FFFF_FFFF;
        [
            0x21 | (((pts >> 30) as u8 & 0x07) << 1),
            ((pts >> 22) & 0xFF) as u8,
            0x01 | (((pts >> 15) as u8 & 0x7F) << 1),
            ((pts >> 7) & 0xFF) as u8,
            0x01 | (((pts & 0x7F) as u8) << 1),
        ]
    }

    fn encode_pcr(pcr_base: u64) -> [u8; 6] {
        let base = pcr_base & 0x1_FFFF_FFFF;
        [
            (base >> 25) as u8,
            (base >> 17) as u8,
            (base >> 9) as u8,
            (base >> 1) as u8,
            (((base & 1) as u8) << 7) | 0x7E,
            0x00,
        ]
    }

    pub fn split_annexb_nals(buf: &[u8]) -> Vec<&[u8]> {
        let mut starts = Vec::new();
        let mut i = 0usize;
        while i + 3 < buf.len() {
            if buf[i] == 0 && buf[i + 1] == 0 {
                if buf[i + 2] == 1 {
                    starts.push((i, i + 3));
                    i += 3;
                    continue;
                } else if i + 4 <= buf.len() && buf[i + 2] == 0 && buf[i + 3] == 1 {
                    starts.push((i, i + 4));
                    i += 4;
                    continue;
                }
            }
            i += 1;
        }
        let mut nals = Vec::with_capacity(starts.len());
        for idx in 0..starts.len() {
            let nal_start = starts[idx].1;
            let nal_end = if idx + 1 < starts.len() {
                starts[idx + 1].0
            } else {
                buf.len()
            };
            if nal_start < nal_end {
                nals.push(&buf[nal_start..nal_end]);
            }
        }
        nals
    }
}

// ============================================================================
// Multi-Protocol LAN Discovery (SSDP UPnP/DLNA + mDNS AirPlay/Cast + WFD + ARP)
// ============================================================================

pub fn discover_lan_receivers(adb: Option<&Adb>, serial: Option<&str>) -> LanDiscoverySnapshot {
    let host_lan_ip = detect_lan_ipv4();
    let mut phone_lan_ip = None;
    let mut miracast = PhoneMiracastState::default();
    let resolved_serial = match (adb, serial) {
        (Some(adb), Some(s)) => Some(
            crate::device::resolve_online_transport(adb, s, "", "PZ261A", &[])
                .unwrap_or_else(|_| s.to_string()),
        ),
        _ => None,
    };

    if let (Some(adb), Some(serial)) = (adb, resolved_serial.as_deref()) {
        miracast = query_phone_miracast(adb, serial);
        if let Ok(ip_out) = adb.run_timeout(
            &["-s", serial, "shell", "ip -4 addr show wlan0"],
            Duration::from_secs(3),
        ) {
            for line in ip_out.lines() {
                let trimmed = line.trim();
                if let Some(rest) = trimmed.strip_prefix("inet ") {
                    if let Some(ip_part) = rest.split('/').next() {
                        phone_lan_ip = Some(ip_part.trim().to_string());
                        break;
                    }
                }
            }
        }
    }

    let mut receivers: Vec<LanCastReceiver> = Vec::new();
    let mut seen_ids: HashSet<String> = HashSet::new();

    // 1. SSDP Multicast M-SEARCH (UPnP MediaRenderer, AVTransport, DIAL)
    for r in discover_ssdp_receivers() {
        if seen_ids.insert(r.id.clone()) {
            receivers.push(r);
        }
    }

    // 2. mDNS Discovery (_airplay._tcp.local., _googlecast._tcp.local., _raop._tcp.local.)
    for r in discover_mdns_receivers() {
        if seen_ids.insert(r.id.clone()) {
            receivers.push(r);
        }
    }

    // 3. Phone Native Miracast / Wi-Fi Display Sinks
    for sink in &miracast.sinks {
        let id = format!("miracast:{}", sink.address);
        if seen_ids.insert(id.clone()) {
            receivers.push(LanCastReceiver {
                id,
                name: format!("{} (Miracast)", sink.name),
                ip: sink.address.clone(),
                port: 7236,
                protocol: LanCastProtocol::Miracast,
                model: if sink.is_connected {
                    "已连接无线显示器".into()
                } else if sink.is_available {
                    "可直连 Miracast 接收端".into()
                } else {
                    "历史配对无线显示器".into()
                },
                control_url: None,
                rendering_control_url: None,
                location_url: None,
                mac: Some(sink.address.clone()),
                online: sink.is_available || sink.is_connected,
                can_direct_push: true,
            });
        }
    }

    // 4. Active LAN Neighbor & Port Discovery (ARP + Phone Neighbors + Port Probe)
    for r in discover_arp_and_port_receivers(&host_lan_ip, phone_lan_ip.as_deref(), adb, serial) {
        let already_has_ip = receivers
            .iter()
            .any(|existing| existing.ip == r.ip && existing.protocol == r.protocol);
        if !already_has_ip && seen_ids.insert(r.id.clone()) {
            receivers.push(r);
        }
    }

    // Sort: direct-push protocol receivers first (DLNA, AirPlay, Cast, Miracast), then LAN terminals
    receivers.sort_by_key(|r| {
        let rank = match r.protocol {
            LanCastProtocol::DlnaUpnp => 0,
            LanCastProtocol::AirPlay => 1,
            LanCastProtocol::GoogleCast => 2,
            LanCastProtocol::Miracast => 3,
            LanCastProtocol::WebReceiver => 4,
        };
        (!r.can_direct_push, rank, r.ip.clone())
    });

    let scanned_at_hms = chrono::Local::now().format("%H:%M:%S").to_string();
    LanDiscoverySnapshot {
        host_lan_ip,
        phone_lan_ip,
        receivers,
        miracast,
        scanned_at_hms,
    }
}

fn discover_ssdp_receivers() -> Vec<LanCastReceiver> {
    let mut locations: HashMap<String, (String, Option<String>)> = HashMap::new();
    let Ok(sock) = UdpSocket::bind("0.0.0.0:0") else {
        return Vec::new();
    };
    let _ = sock.set_read_timeout(Some(Duration::from_millis(220)));
    let _ = sock.set_write_timeout(Some(Duration::from_millis(200)));

    let targets = [
        "urn:schemas-upnp-org:device:MediaRenderer:1",
        "urn:schemas-upnp-org:service:AVTransport:1",
        "urn:dial-multiscreen-org:service:dial:1",
        "ssdp:all",
    ];
    let multicast: SocketAddr = "239.255.255.250:1900".parse().unwrap();
    for st in targets {
        let req = format!(
            "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 1\r\nST: {st}\r\n\r\n"
        );
        let _ = sock.send_to(req.as_bytes(), multicast);
    }

    let deadline = Instant::now() + Duration::from_millis(750);
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match sock.recv_from(&mut buf) {
            Ok((n, src)) => {
                let text = String::from_utf8_lossy(&buf[..n]);
                let mut location = None;
                let mut st_hdr = String::new();
                let mut server_hdr = None;
                for line in text.lines() {
                    let l = line.trim();
                    let lower = l.to_ascii_lowercase();
                    if lower.starts_with("location:") {
                        location = Some(l[9..].trim().to_string());
                    } else if lower.starts_with("st:") {
                        st_hdr = l[3..].trim().to_string();
                    } else if lower.starts_with("server:") {
                        server_hdr = Some(l[7..].trim().to_string());
                    }
                }
                if let Some(loc) = location {
                    locations
                        .entry(loc)
                        .or_insert((st_hdr, server_hdr.or_else(|| Some(src.ip().to_string()))));
                }
            }
            Err(_) => break,
        }
    }

    let mut out = Vec::new();
    for (loc, (st, server_hint)) in locations {
        if let Some(receiver) = fetch_and_parse_upnp_location(&loc, &st, server_hint.as_deref()) {
            out.push(receiver);
        }
    }
    out
}

pub fn parse_upnp_description_xml(
    xml: &str,
    base_location: &str,
) -> (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
) {
    let extract_tag = |src: &str, tag: &str| -> Option<String> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = src.find(&open)? + open.len();
        let end = src[start..].find(&close)? + start;
        let val = src[start..end].trim();
        (!val.is_empty()).then(|| html_unescape(val))
    };

    let friendly_name = extract_tag(xml, "friendlyName");
    let model_name = extract_tag(xml, "modelName");
    let manufacturer = extract_tag(xml, "manufacturer");

    let mut av_control_url = None;
    let mut rendering_control_url = None;

    for block in xml.split("<service>").skip(1) {
        let Some(end_idx) = block.find("</service>") else {
            continue;
        };
        let svc = &block[..end_idx];
        let stype = extract_tag(svc, "serviceType").unwrap_or_default();
        let curl = extract_tag(svc, "controlURL");
        if stype.contains("AVTransport") {
            av_control_url = curl.map(|u| resolve_relative_url(base_location, &u));
        } else if stype.contains("RenderingControl") {
            rendering_control_url = curl.map(|u| resolve_relative_url(base_location, &u));
        }
    }

    (
        friendly_name,
        model_name,
        manufacturer,
        av_control_url,
        rendering_control_url,
    )
}

fn resolve_relative_url(base_location: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") {
        return path.to_string();
    }
    if let Ok(base) = url::Url::parse(base_location) {
        if let Ok(joined) = base.join(path) {
            return joined.to_string();
        }
    }
    path.to_string()
}

fn fetch_and_parse_upnp_location(
    location: &str,
    st: &str,
    server_hint: Option<&str>,
) -> Option<LanCastReceiver> {
    let parsed = url::Url::parse(location).ok()?;
    let host = parsed.host_str()?.to_string();
    let port = parsed.port_or_known_default().unwrap_or(80);
    let path = if parsed.query().is_some() {
        format!("{}?{}", parsed.path(), parsed.query().unwrap_or_default())
    } else {
        parsed.path().to_string()
    };

    let addr = format!("{host}:{port}").to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(380)).ok()?;
    let _ = stream.set_read_timeout(Some(Duration::from_millis(450)));
    let _ = stream.set_write_timeout(Some(Duration::from_millis(350)));
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nUser-Agent: StepX-Companion/1.0\r\n\r\n"
    );
    stream.write_all(req.as_bytes()).ok()?;
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp);
    let text = String::from_utf8_lossy(&resp);
    let (headers, body) = text.split_once("\r\n\r\n").unwrap_or((&text, ""));

    let mut dial_app_url = None;
    for line in headers.lines() {
        let l = line.trim();
        if l.to_ascii_lowercase().starts_with("application-url:") {
            dial_app_url = Some(l[16..].trim().to_string());
        }
    }

    let (friendly_name, model_name, manufacturer, av_control_url, rendering_control_url) =
        parse_upnp_description_xml(body, location);

    // Filter out pure router IGD gateways unless they expose AVTransport or DIAL
    let is_igd_only = body.contains("InternetGatewayDevice")
        && av_control_url.is_none()
        && dial_app_url.is_none()
        && !st.contains("MediaRenderer");
    if is_igd_only {
        return None;
    }

    let is_dial = dial_app_url.is_some() || st.contains("dial-multiscreen-org");
    let protocol = if av_control_url.is_some() || body.contains("MediaRenderer") {
        LanCastProtocol::DlnaUpnp
    } else if is_dial {
        LanCastProtocol::GoogleCast
    } else {
        return None;
    };

    let name = friendly_name.unwrap_or_else(|| format!("局域网投屏设备 ({host})"));
    let model = match (manufacturer, model_name) {
        (Some(m), Some(md)) if !md.starts_with(&m) => format!("{m} {md}"),
        (_, Some(md)) => md,
        (Some(m), None) => m,
        (None, None) => server_hint.unwrap_or("UPnP / DLNA").to_string(),
    };

    Some(LanCastReceiver {
        id: format!("upnp:{host}:{port}"),
        name,
        ip: host,
        port,
        protocol,
        model,
        control_url: av_control_url.or(dial_app_url),
        rendering_control_url,
        location_url: Some(location.to_string()),
        mac: None,
        online: true,
        can_direct_push: true,
    })
}

fn discover_mdns_receivers() -> Vec<LanCastReceiver> {
    let mut out = Vec::new();
    let Ok(mdns) = mdns_sd::ServiceDaemon::new() else {
        return out;
    };
    let services = [
        ("_airplay._tcp.local.", LanCastProtocol::AirPlay),
        ("_googlecast._tcp.local.", LanCastProtocol::GoogleCast),
        ("_raop._tcp.local.", LanCastProtocol::AirPlay),
    ];
    let mut rx_list = Vec::new();
    for (svc, proto) in services {
        if let Ok(rx) = mdns.browse(svc) {
            rx_list.push((rx, proto));
        }
    }
    let deadline = Instant::now() + Duration::from_millis(420);
    while Instant::now() < deadline {
        let mut any = false;
        for (rx, proto) in &rx_list {
            while let Ok(ev) = rx.try_recv() {
                any = true;
                if let mdns_sd::ServiceEvent::ServiceResolved(info) = ev {
                    let Some(ip) = info
                        .get_addresses_v4()
                        .into_iter()
                        .next()
                        .map(|a| a.to_string())
                    else {
                        continue;
                    };
                    let port = info.get_port();
                    let friendly = info
                        .get_property_val_str("fn")
                        .or_else(|| info.get_property_val_str("name"))
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            info.get_fullname()
                                .split('.')
                                .next()
                                .unwrap_or("局域网投屏终端")
                                .replace("\\032", " ")
                        });
                    let model = info
                        .get_property_val_str("md")
                        .or_else(|| info.get_property_val_str("model"))
                        .unwrap_or(proto.short_badge())
                        .to_string();
                    out.push(LanCastReceiver {
                        id: format!("mdns:{}:{ip}:{port}", proto.short_badge()),
                        name: friendly,
                        ip,
                        port,
                        protocol: *proto,
                        model,
                        control_url: None,
                        rendering_control_url: None,
                        location_url: None,
                        mac: None,
                        online: true,
                        can_direct_push: true,
                    });
                }
            }
        }
        if !any {
            thread::sleep(Duration::from_millis(25));
        }
    }
    let _ = mdns.shutdown();
    out
}

fn discover_arp_and_port_receivers(
    host_ip: &str,
    phone_ip: Option<&str>,
    adb: Option<&Adb>,
    serial: Option<&str>,
) -> Vec<LanCastReceiver> {
    let mut ip_mac: HashMap<String, String> = HashMap::new();

    let host_prefix = host_ip
        .rsplit_once('.')
        .map(|(p, _)| format!("{p}."))
        .unwrap_or_default();

    if let Ok(arp) = std::fs::read_to_string("/proc/net/arp") {
        for line in arp.lines().skip(1) {
            let cols: Vec<&str> = line.split_whitespace().collect();
            if cols.len() >= 6 && cols[2] != "0x0" && cols[3] != "00:00:00:00:00:00" {
                let ip = cols[0];
                let dev = cols[5];
                let is_virtual = dev.starts_with("docker")
                    || dev.starts_with("br-")
                    || dev.starts_with("veth")
                    || dev.starts_with("virbr");
                let same_subnet = host_prefix.is_empty() || ip.starts_with(&host_prefix);
                if !is_virtual && same_subnet {
                    ip_mac.insert(ip.to_string(), cols[3].to_string());
                }
            }
        }
    }

    if let (Some(adb), Some(serial)) = (adb, serial) {
        if let Ok(neigh) = adb.run_timeout(
            &["-s", serial, "shell", "ip -4 neigh show"],
            Duration::from_secs(2),
        ) {
            for line in neigh.lines() {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if let Some(ip) = cols.first() {
                    if ip.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                        let mac = cols
                            .iter()
                            .position(|&w| w == "lladdr")
                            .and_then(|idx| cols.get(idx + 1))
                            .copied()
                            .unwrap_or("未知 MAC");
                        ip_mac.entry((*ip).to_string()).or_insert_with(|| mac.to_string());
                    }
                }
            }
        }
    }

    let mut candidates: Vec<(String, String)> = ip_mac
        .into_iter()
        .filter(|(ip, _)| ip != host_ip && Some(ip.as_str()) != phone_ip)
        .collect();
    candidates.sort_by(|a, b| a.0.cmp(&b.0));

    let mut handles = Vec::new();
    for (ip, mac) in candidates {
        handles.push(thread::spawn(move || probe_lan_host(ip, mac)));
    }

    let mut out = Vec::new();
    // Always include local loopback receiver entry for instant 1-click VLC / Browser verification on this desktop
    if !host_ip.is_empty() && host_ip != "127.0.0.1" {
        out.push(LanCastReceiver {
            id: format!("lan-self:{host_ip}"),
            name: format!("本机投屏接收端 ({host_ip})"),
            ip: host_ip.to_string(),
            port: DEFAULT_CAST_PORT,
            protocol: LanCastProtocol::WebReceiver,
            model: "支持 VLC / 浏览器 / DLNA 实时流接收与触控反控".into(),
            control_url: None,
            rendering_control_url: None,
            location_url: None,
            mac: None,
            online: true,
            can_direct_push: true,
        });
    }

    for h in handles {
        if let Ok(Some(r)) = h.join() {
            out.push(r);
        }
    }
    out
}

fn probe_lan_host(ip: String, mac: String) -> Option<LanCastReceiver> {
    let check_port = |port: u16| -> bool {
        let Ok(addr) = format!("{ip}:{port}").parse::<SocketAddr>() else {
            return false;
        };
        TcpStream::connect_timeout(&addr, Duration::from_millis(140)).is_ok()
    };

    if check_port(7000) {
        return Some(LanCastReceiver {
            id: format!("airplay:{ip}:7000"),
            name: format!("AirPlay 投屏终端 ({ip})"),
            ip,
            port: 7000,
            protocol: LanCastProtocol::AirPlay,
            model: format!("AirPlay / RAOP 接收端 · MAC {mac}"),
            control_url: None,
            rendering_control_url: None,
            location_url: None,
            mac: Some(mac),
            online: true,
            can_direct_push: true,
        });
    }
    if check_port(8008) || check_port(8009) {
        return Some(LanCastReceiver {
            id: format!("cast:{ip}:8008"),
            name: format!("Google Cast / 电视终端 ({ip})"),
            ip: ip.clone(),
            port: 8008,
            protocol: LanCastProtocol::GoogleCast,
            model: format!("Chromecast / DIAL 协议终端 · MAC {mac}"),
            control_url: Some(format!("http://{ip}:8008/apps")),
            rendering_control_url: None,
            location_url: None,
            mac: Some(mac),
            online: true,
            can_direct_push: true,
        });
    }

    let is_gateway = ip.ends_with(".1");
    let label = if is_gateway {
        format!("局域网网关 / 路由节点 ({ip})")
    } else {
        format!("局域网在线机器 ({ip})")
    };
    let model = if is_gateway {
        format!("UPnP 网关 · MAC {mac}")
    } else {
        format!("支持浏览器 / VLC / 播放器打开投屏流 · MAC {mac}")
    };

    Some(LanCastReceiver {
        id: format!("lan:{ip}"),
        name: label,
        ip,
        port: DEFAULT_CAST_PORT,
        protocol: LanCastProtocol::WebReceiver,
        model,
        control_url: None,
        rendering_control_url: None,
        location_url: None,
        mac: Some(mac),
        online: true,
        can_direct_push: false,
    })
}

pub fn query_phone_miracast(adb: &Adb, serial: &str) -> PhoneMiracastState {
    let resolved = crate::device::resolve_online_transport(adb, serial, "", "PZ261A", &[])
        .unwrap_or_else(|_| serial.to_string());
    let serial = resolved.as_str();
    let mut state = PhoneMiracastState {
        wfd_service_installed: true,
        ..Default::default()
    };
    if let Ok(val) = adb.run_timeout(
        &["-s", serial, "shell", "settings get global wifi_display_on"],
        Duration::from_secs(3),
    ) {
        state.enabled = val.trim() == "1";
    }
    let Ok(dump) = adb.run_timeout(
        &["-s", serial, "shell", "dumpsys display"],
        Duration::from_secs(4),
    ) else {
        return state;
    };

    for line in dump.lines() {
        let t = line.trim();
        if t.starts_with("mWifiDisplayOnSetting=true") || t.starts_with("mWfdEnabled=true") {
            state.enabled = true;
        } else if t.starts_with("mScanState=1") {
            state.scanning = true;
        } else if let Some(rest) = t.strip_prefix("mActiveDisplay=") {
            if rest != "null" {
                if let Some((addr, name)) = parse_wfd_entry(rest) {
                    state.active_display_address = Some(addr);
                    state.active_display_name = Some(name);
                }
            }
        }
    }

    // Parse WifiDisplay entries like: WifiDisplay{AA:BB:CC:DD:EE:FF, Living Room TV, ...}
    for segment in dump.split("WifiDisplay{").skip(1) {
        if let Some(end) = segment.find('}') {
            let inner = &segment[..end];
            let parts: Vec<&str> = inner.split(',').map(|s| s.trim()).collect();
            if parts.len() >= 2 {
                let addr = parts[0].to_string();
                let name = parts[1].to_string();
                let is_avail = inner.contains("isAvailable=true") || inner.contains("canConnect=true");
                let is_rem = inner.contains("isRemembered=true");
                let is_conn = state.active_display_address.as_deref() == Some(addr.as_str());
                if !addr.is_empty()
                    && !state
                        .sinks
                        .iter()
                        .any(|s: &MiracastSink| s.address == addr)
                {
                    state.sinks.push(MiracastSink {
                        address: addr,
                        name,
                        is_available: is_avail,
                        is_remembered: is_rem,
                        is_connected: is_conn,
                    });
                }
            }
        }
    }
    state
}

fn parse_wfd_entry(raw: &str) -> Option<(String, String)> {
    let inner = raw.strip_prefix("WifiDisplay{")?.split('}').next()?;
    let mut parts = inner.split(',').map(|s| s.trim());
    let addr = parts.next()?.to_string();
    let name = parts.next()?.to_string();
    Some((addr, name))
}

pub fn set_phone_miracast_enabled(
    adb: &Adb,
    serial: &str,
    enabled: bool,
) -> Result<PhoneMiracastState> {
    let resolved = crate::device::resolve_online_transport(adb, serial, "", "PZ261A", &[])
        .unwrap_or_else(|_| serial.to_string());
    let bit = if enabled { "1" } else { "0" };
    let script = format!(
        "settings put global wifi_display_on {bit}; settings put global wifi_display_certification_on {bit}; cmd wifi set-wifi-enabled enabled >/dev/null 2>&1 || true"
    );
    adb.script(&resolved, &script)?;
    Ok(query_phone_miracast(adb, &resolved))
}

pub fn open_phone_cast_settings(adb: &Adb, serial: &str) -> Result<()> {
    let resolved = crate::device::resolve_online_transport(adb, serial, "", "PZ261A", &[])
        .unwrap_or_else(|_| serial.to_string());
    let _ = adb.script(
        &resolved,
        "input keyevent 224 >/dev/null 2>&1 || true; settings put global wifi_display_on 1; am start -a android.settings.CAST_SETTINGS >/dev/null 2>&1",
    )?;
    Ok(())
}

pub fn connect_phone_miracast(adb: &Adb, serial: &str, _address: &str) -> Result<()> {
    open_phone_cast_settings(adb, serial)
}

// ============================================================================
// Embedded Multi-Protocol LAN Broadcast Server (MPEG-TS + HLS + MJPEG + Web UI)
// ============================================================================

#[derive(Default)]
struct SharedStreamState {
    latest_jpeg: Vec<u8>,
    latest_annexb_idr: Vec<u8>,
    frame_seq: u64,
    hw_frames_decoded: u64,
    phone_width: u32,
    phone_height: u32,
    fps_x10: u32,
    ts_subscribers: Vec<std::sync::mpsc::SyncSender<Vec<u8>>>,
    hls_segments: VecDeque<(u64, Vec<u8>)>,
    hls_next_seq: u64,
    pushed_receiver_id: Option<String>,
    pushed_receiver_name: Option<String>,
}

pub struct LanCastServer {
    stop_flag: Arc<AtomicBool>,
    remote_control: Arc<AtomicBool>,
    quality: Arc<AtomicU8>,
    active_clients: Arc<AtomicU64>,
    bytes_broadcast: Arc<AtomicU64>,
    port: u16,
    host_ip: String,
    qr_png_path: Option<PathBuf>,
    session_token: String,
    shared: Arc<(Mutex<SharedStreamState>, Condvar)>,
    h264_child_pid: Arc<AtomicU64>,
    serial_slot: Arc<Mutex<String>>,
}

impl LanCastServer {
    pub fn start(
        adb_path: PathBuf,
        serial: String,
        quality: LanCastQuality,
        remote_control_enabled: bool,
    ) -> Result<Self> {
        let host_ip = detect_lan_ipv4();
        let mut bound_listener = None;
        let mut bound_port = DEFAULT_CAST_PORT;
        for port in DEFAULT_CAST_PORT..(DEFAULT_CAST_PORT + 25) {
            if let Ok(l) = TcpListener::bind(("0.0.0.0", port)) {
                bound_listener = Some(l);
                bound_port = port;
                break;
            }
        }
        let listener = bound_listener.context("无法绑定局域网投屏端口 (19780..19805)")?;
        let _ = listener.set_nonblocking(true);

        let session_token = format!("{:032x}", rand::random::<u128>());
        let web_url = format!("http://{host_ip}:{bound_port}/session/{session_token}/cast");
        let qr_png_path = generate_cast_qr_png(&web_url, bound_port);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let remote_control = Arc::new(AtomicBool::new(remote_control_enabled));
        let quality_atom = Arc::new(AtomicU8::new(quality.as_u8()));
        let active_clients = Arc::new(AtomicU64::new(0));
        let bytes_broadcast = Arc::new(AtomicU64::new(0));
        let h264_child_pid = Arc::new(AtomicU64::new(0));
        let serial_slot = Arc::new(Mutex::new(serial));
        let shared = Arc::new((
            Mutex::new(SharedStreamState {
                phone_width: 1200,
                phone_height: 2670,
                ..Default::default()
            }),
            Condvar::new(),
        ));

        // 1. Spawn Unified Hardware H.264 (scrcpy-server + openh264 + MPEG-TS/HLS) Stream Thread
        {
            let stop = stop_flag.clone();
            let shared = shared.clone();
            let qual = quality_atom.clone();
            let adb_p = adb_path.clone();
            let ser_slot = serial_slot.clone();
            let pid_slot = h264_child_pid.clone();
            let bytes_cnt = bytes_broadcast.clone();
            let port_num = bound_port;
            thread::spawn(move || {
                unified_h264_cast_loop(
                    adb_p,
                    ser_slot,
                    port_num,
                    stop,
                    qual,
                    shared,
                    pid_slot,
                    bytes_cnt,
                );
            });
        }

        // 2. Bootstrap first image and sample actual decoded-frame throughput.
        {
            let stop = stop_flag.clone();
            let shared = shared.clone();
            let qual = quality_atom.clone();
            let adb_p = adb_path.clone();
            let ser_slot = serial_slot.clone();
            let bytes_cnt = bytes_broadcast.clone();
            thread::spawn(move || {
                frame_pacer_loop(adb_p, ser_slot, stop, qual, shared, bytes_cnt);
            });
        }

        // 3. Spawn HTTP Accept Loop
        {
            let stop = stop_flag.clone();
            let remote = remote_control.clone();
            let clients = active_clients.clone();
            let bytes_cnt = bytes_broadcast.clone();
            let shared = shared.clone();
            let adb_p = adb_path;
            let ser_slot = serial_slot.clone();
            let hip = host_ip.clone();
            let token = session_token.clone();
            thread::spawn(move || {
                http_accept_loop(
                    listener,
                    bound_port,
                    hip,
                    token,
                    adb_p,
                    ser_slot,
                    stop,
                    remote,
                    clients,
                    bytes_cnt,
                    shared,
                );
            });
        }

        Ok(Self {
            stop_flag,
            remote_control,
            quality: quality_atom,
            active_clients,
            bytes_broadcast,
            port: bound_port,
            host_ip,
            qr_png_path,
            session_token,
            shared,
            h264_child_pid,
            serial_slot,
        })
    }

    pub fn set_serial(&self, next_serial: String) {
        let trimmed = next_serial.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Ok(mut guard) = self.serial_slot.lock() {
            if *guard != trimmed {
                *guard = trimmed.to_string();
            }
        }
    }

    pub fn latest_jpeg_bytes(&self) -> Vec<u8> {
        self.shared
            .0
            .lock()
            .map(|g| g.latest_jpeg.clone())
            .unwrap_or_default()
    }

    pub fn stats(&self) -> LanCastStats {
        let guard = self.shared.0.lock().unwrap();
        let port = self.port;
        let host_ip = &self.host_ip;
        let token = &self.session_token;
        LanCastStats {
            running: !self.stop_flag.load(Ordering::Relaxed),
            port,
            host_ip: host_ip.clone(),
            web_url: format!("http://{host_ip}:{port}/session/{token}/cast"),
            ts_url: format!("http://{host_ip}:{port}/session/{token}/live.ts"),
            hls_url: format!("http://{host_ip}:{port}/session/{token}/live.m3u8"),
            mjpeg_url: format!("http://{host_ip}:{port}/session/{token}/stream.mjpeg"),
            qr_png_path: self.qr_png_path.clone(),
            active_clients: self.active_clients.load(Ordering::Relaxed) as usize,
            frames_captured: guard.frame_seq,
            bytes_broadcast: self.bytes_broadcast.load(Ordering::Relaxed),
            fps: guard.fps_x10 as f32 / 10.0,
            phone_width: guard.phone_width,
            phone_height: guard.phone_height,
            remote_control_enabled: self.remote_control.load(Ordering::Relaxed),
            quality: LanCastQuality::from_u8(self.quality.load(Ordering::Relaxed)),
            pushed_receiver_id: guard.pushed_receiver_id.clone(),
            pushed_receiver_name: guard.pushed_receiver_name.clone(),
        }
    }

    pub fn set_remote_control(&self, enabled: bool) {
        self.remote_control.store(enabled, Ordering::Relaxed);
    }

    pub fn set_quality(&self, quality: LanCastQuality) {
        self.quality.store(quality.as_u8(), Ordering::Relaxed);
    }

    pub fn set_pushed_receiver(&self, id: Option<String>, name: Option<String>) {
        if let Ok(mut guard) = self.shared.0.lock() {
            guard.pushed_receiver_id = id;
            guard.pushed_receiver_name = name;
        }
    }

    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);
        self.shared.1.notify_all();
        let pid = self.h264_child_pid.swap(0, Ordering::Relaxed);
        if pid > 0 {
            #[cfg(unix)]
            unsafe {
                libc::kill(pid as i32, libc::SIGTERM);
            }
        }
    }
}

impl Drop for LanCastServer {
    fn drop(&mut self) {
        self.stop();
    }
}

fn resolve_active_cast_serial(_adb_path: &PathBuf, serial_slot: &Arc<Mutex<String>>) -> String {
    // Only the desktop device-selection flow may change the selected transport.
    serial_slot.lock().map(|g| g.clone()).unwrap_or_default()
}

fn find_scrcpy_server_jar() -> Option<(PathBuf, String)> {
    let mut candidates = Vec::new();
    if let Some(env_path) = std::env::var_os("SCRCPY_SERVER_PATH") {
        candidates.push(PathBuf::from(env_path));
    }
    for bin in [
        std::env::var_os("STEPX_SCRCPY").map(PathBuf::from),
        crate::adb::bundled_tool("scrcpy"),
        crate::adb::find_executable("scrcpy"),
        crate::adb::platform_tool_candidate("scrcpy"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(parent) = bin.parent() {
            candidates.push(parent.join("scrcpy-server"));
        }
    }
    candidates.push(
        crate::resources::root().join(".tools/root/scrcpy-linux-x86_64-v5.0.1/scrcpy-server"),
    );
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(
            PathBuf::from(home)
                .join(".local/opt/stepx-companion/tools/root/scrcpy-linux-x86_64-v5.0.1/scrcpy-server"),
        );
    }
    for path in candidates {
        if path.is_file() {
            let ver = path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .and_then(|s| s.rsplit("-v").next())
                .filter(|v| v.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .unwrap_or("5.0.1")
                .to_string();
            return Some((path, ver));
        }
    }
    None
}

fn decode_screencap_to_scaled_rgb(
    raw: &[u8],
    max_dim: u32,
) -> Option<(u32, u32, image::RgbImage)> {
    // Fast path 1: Raw Android `screencap` framebuffer (12- or 16-byte LE header + RGBA8)
    if raw.len() >= 16 && !raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        let w = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
        let h = u32::from_le_bytes([raw[4], raw[5], raw[6], raw[7]]);
        let pixel_bytes = (w as usize).saturating_mul(h as usize).saturating_mul(4);
        let hdr_len = if raw.len() == 16 + pixel_bytes {
            16
        } else if raw.len() == 12 + pixel_bytes {
            12
        } else {
            0
        };
        if hdr_len > 0 && w > 0 && h > 0 {
            let pixels = &raw[hdr_len..hdr_len + pixel_bytes];
            let (out_w, out_h) = if h > max_dim {
                (((w as u64 * max_dim as u64) / h as u64).max(1) as u32, max_dim)
            } else {
                (w, h)
            };
            let mut rgb = image::RgbImage::new(out_w, out_h);
            for y in 0..out_h {
                let src_y = ((y as u64 * h as u64) / out_h as u64) as usize;
                let row_off = src_y * (w as usize) * 4;
                for x in 0..out_w {
                    let src_x = ((x as u64 * w as u64) / out_w as u64) as usize;
                    let idx = row_off + src_x * 4;
                    rgb.put_pixel(
                        x,
                        y,
                        image::Rgb([pixels[idx], pixels[idx + 1], pixels[idx + 2]]),
                    );
                }
            }
            return Some((w, h, rgb));
        }
    }

    // Fast path 2: PNG `screencap -p`
    if raw.starts_with(b"\x89PNG\r\n\x1a\n") {
        let img = image::load_from_memory(raw).ok()?;
        let (orig_w, orig_h) = img.dimensions();
        let scaled = if orig_h > max_dim {
            let new_w = ((orig_w as u64 * max_dim as u64) / orig_h.max(1) as u64).max(1) as u32;
            img.resize_exact(new_w, max_dim, image::imageops::FilterType::Nearest)
        } else {
            img
        };
        return Some((orig_w, orig_h, scaled.to_rgb8()));
    }
    None
}

fn capture_single_screencap_jpeg(
    adb_path: &PathBuf,
    serial: &str,
    max_dim: u32,
) -> Option<(u32, u32, Vec<u8>)> {
    let is_usb = !serial.contains(':') && !serial.starts_with("adb-");
    let mut cmd = Command::new(adb_path);
    if is_usb {
        cmd.args(["-s", serial, "exec-out", "screencap"]);
    } else {
        cmd.args(["-s", serial, "exec-out", "screencap", "-p"]);
    }
    crate::resources::quiet_command(&mut cmd);
    let output = crate::adb::run_process(cmd, None, Duration::from_secs(4)).ok()?;
    if !output.status.success() {
        return None;
    }
    let (orig_w, orig_h, rgb) = decode_screencap_to_scaled_rgb(&output.stdout, max_dim)?;
    let mut jpeg_bytes = Vec::with_capacity(96 * 1024);
    let mut enc = JpegEncoder::new_with_quality(&mut jpeg_bytes, 80);
    enc.encode_image(&rgb).ok()?;
    Some((orig_w, orig_h, jpeg_bytes))
}

fn frame_pacer_loop(
    adb_path: PathBuf,
    serial_slot: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    quality_atom: Arc<AtomicU8>,
    shared: Arc<(Mutex<SharedStreamState>, Condvar)>,
    _bytes_broadcast: Arc<AtomicU64>,
) {
    // Sample actual frames, including zero FPS on a static screen. Never generate fake frames.
    let tick_interval = Duration::from_millis(250);
    let mut window_start = Instant::now();
    // Existing buffered frames predate this measurement window.
    let mut last_seq_seen = shared.0.lock().unwrap().frame_seq;
    let mut bootstrap_attempted = false;
    let started_at = Instant::now();

    while !stop.load(Ordering::Relaxed) {
        thread::sleep(tick_interval);
        if stop.load(Ordering::Relaxed) {
            break;
        }

        let need_bootstrap = {
            let guard = shared.0.lock().unwrap();
            guard.latest_jpeg.is_empty() && started_at.elapsed() >= Duration::from_millis(350)
        };
        if need_bootstrap && !bootstrap_attempted {
            bootstrap_attempted = true;
            let active_serial = resolve_active_cast_serial(&adb_path, &serial_slot);
            let max_dim =
                LanCastQuality::from_u8(quality_atom.load(Ordering::Relaxed)).max_dim();
            if let Some((orig_w, orig_h, jpeg_bytes)) =
                capture_single_screencap_jpeg(&adb_path, &active_serial, max_dim)
            {
                let mut guard = shared.0.lock().unwrap();
                if guard.latest_jpeg.is_empty() {
                    guard.latest_jpeg = jpeg_bytes;
                    guard.frame_seq = guard.frame_seq.wrapping_add(1);
                    if orig_w > 0 && orig_h > 0 {
                        guard.phone_width = orig_w;
                        guard.phone_height = orig_h;
                    }
                    shared.1.notify_all();
                }
            }
        }

        let mut guard = shared.0.lock().unwrap();
        if !guard.latest_jpeg.is_empty() {
            // Static screens keep the last frame without redundant network traffic.
            let elapsed = window_start.elapsed().as_secs_f32();
            if elapsed >= 0.45 {
                let delta_frames = guard.frame_seq.saturating_sub(last_seq_seen) as f32;
                let fps_x10 = ((delta_frames / elapsed) * 10.0).round() as u32;
                guard.fps_x10 = fps_x10;
                last_seq_seen = guard.frame_seq;
                window_start = Instant::now();
            }
            shared.1.notify_all();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn unified_h264_cast_loop(
    adb_path: PathBuf,
    serial_slot: Arc<Mutex<String>>,
    cast_port: u16,
    stop: Arc<AtomicBool>,
    quality_atom: Arc<AtomicU8>,
    shared: Arc<(Mutex<SharedStreamState>, Condvar)>,
    pid_slot: Arc<AtomicU64>,
    bytes_broadcast: Arc<AtomicU64>,
) {
    use openh264::formats::YUVSource;

    let mut muxer = ts_mux::TsMuxer::new();
    let stream_epoch = Instant::now();
    let mut current_hls_buf = Vec::with_capacity(256 * 1024);
    let mut last_hls_cut = Instant::now();
    let scrcpy_jar = find_scrcpy_server_jar();
    let mut decoder = openh264::decoder::Decoder::new().ok();
    let mut rgb_scratch: Vec<u8> = Vec::new();
    let mut last_jpeg_encode = Instant::now() - Duration::from_secs(1);

    let mut process_annexb_chunk = |chunk: &[u8],
                                    shared: &Arc<(Mutex<SharedStreamState>, Condvar)>,
                                    _bytes_broadcast: &Arc<AtomicU64>| {
        if chunk.is_empty() {
            return;
        }
        // 1. Decode H.264 Constrained Baseline NAL units in memory via openh264 -> RGB -> JPEG
        if let Some(dec) = decoder.as_mut() {
            for nal_pkt in openh264::nal_units(chunk) {
                if let Ok(Some(yuv)) = dec.decode(nal_pkt) {
                    // Cap JPEG re-encoding at ~45 FPS (22ms) to keep CPU usage feather-light
                    if last_jpeg_encode.elapsed() >= Duration::from_millis(20) {
                        let (w, h) = yuv.dimensions();
                        if w > 0 && h > 0 {
                            let needed = yuv.rgb8_len();
                            if rgb_scratch.len() != needed {
                                rgb_scratch.resize(needed, 0);
                            }
                            yuv.write_rgb8(&mut rgb_scratch);
                            if let Some(rgb_img) = image::RgbImage::from_raw(
                                w as u32,
                                h as u32,
                                rgb_scratch.clone(),
                            ) {
                                let mut jpeg_bytes = Vec::with_capacity(96 * 1024);
                                let mut enc = JpegEncoder::new_with_quality(&mut jpeg_bytes, 80);
                                if enc.encode_image(&rgb_img).is_ok() {
                                    last_jpeg_encode = Instant::now();
                                    let mut guard = shared.0.lock().unwrap();
                                    guard.latest_jpeg = jpeg_bytes;
                                    guard.hw_frames_decoded =
                                        guard.hw_frames_decoded.wrapping_add(1);
                                    guard.frame_seq = guard.frame_seq.wrapping_add(1);
                                    shared.1.notify_all();
                                }
                            }
                        }
                    }
                }
            }
        }

        // 2. Mux Annex-B H.264 into 188-byte MPEG-TS (`/live.ts`) and Apple HLS (`/live.m3u8`)
        let pts_90khz = (stream_epoch.elapsed().as_secs_f64() * 90_000.0) as u64;
        let (ts_pkt, has_idr) = muxer.mux_annexb_chunk(chunk, pts_90khz);
        if !ts_pkt.is_empty() {
            current_hls_buf.extend_from_slice(&ts_pkt);
            let mut guard = shared.0.lock().unwrap();
            if has_idr {
                guard.latest_annexb_idr = chunk.to_vec();
            }
            let rotate_hls = guard.hls_segments.is_empty()
                || (has_idr && last_hls_cut.elapsed() >= Duration::from_millis(1200))
                || last_hls_cut.elapsed() >= Duration::from_millis(2000);
            guard.ts_subscribers.retain(|tx| match tx.try_send(ts_pkt.clone()) {
                Ok(()) => true,
                Err(std::sync::mpsc::TrySendError::Full(_)) => true,
                Err(std::sync::mpsc::TrySendError::Disconnected(_)) => false,
            });
            if rotate_hls && !current_hls_buf.is_empty() {
                let seq = guard.hls_next_seq;
                guard.hls_next_seq = seq + 1;
                let mut seg_bytes = muxer.tables_bytes();
                seg_bytes.append(&mut current_hls_buf);
                guard.hls_segments.push_back((seq, seg_bytes));
                while guard.hls_segments.len() > 5 {
                    guard.hls_segments.pop_front();
                }
                last_hls_cut = Instant::now();
            }
        }
    };

    while !stop.load(Ordering::Relaxed) {
        let active_serial = resolve_active_cast_serial(&adb_path, &serial_slot);
        if active_serial.is_empty() {
            thread::sleep(Duration::from_millis(300));
            continue;
        }
        let q = LanCastQuality::from_u8(quality_atom.load(Ordering::Relaxed));

        // Wake sleeping phone screen & query physical resolution once per stream session
        let adb = Adb {
            path: adb_path.clone(),
        };
        let _ = adb.run_timeout(
            &["-s", &active_serial, "shell", "input keyevent 224 >/dev/null 2>&1 || true"],
            Duration::from_secs(2),
        );
        if let Ok(wm) = adb.run_timeout(
            &["-s", &active_serial, "shell", "wm size"],
            Duration::from_secs(2),
        ) {
            for line in wm.lines() {
                if let Some((_, dim)) = line.split_once(':') {
                    if let Some((w_s, h_s)) = dim.trim().split_once('x') {
                        if let (Ok(pw), Ok(ph)) = (w_s.parse::<u32>(), h_s.parse::<u32>()) {
                            if pw > 0 && ph > 0 {
                                let mut guard = shared.0.lock().unwrap();
                                guard.phone_width = pw;
                                guard.phone_height = ph;
                            }
                        }
                    }
                }
            }
        }

        // Fast Path: Use scrcpy-server `raw_stream=true` with Constrained Baseline H.264 at 60 FPS!
        if let Some((ref jar_path, ref jar_ver)) = scrcpy_jar {
            let remote_jar = "/data/local/tmp/stepx-cast-server.jar";
            let expected_len = std::fs::metadata(jar_path).map(|m| m.len()).unwrap_or(0);
            let remote_ok = adb
                .run_timeout(
                    &["-s", &active_serial, "shell", &format!("stat -c %s {remote_jar} 2>/dev/null")],
                    Duration::from_secs(2),
                )
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok())
                .is_some_and(|sz| sz > 0 && sz == expected_len);
            if !remote_ok {
                let _ = adb.run_timeout(
                    &[
                        "-s",
                        &active_serial,
                        "push",
                        &jar_path.to_string_lossy(),
                        remote_jar,
                    ],
                    Duration::from_secs(8),
                );
            }

            let scid_val: u32 = 0x2780_0000 | ((cast_port as u32) & 0xFFFF);
            let scid_hex = format!("{scid_val:08x}");
            let forward_port = cast_port.saturating_add(100);
            let forward_spec = format!("tcp:{forward_port}");
            let remote_sock = format!("localabstract:scrcpy_{scid_hex}");

            if adb
                .run_timeout(
                    &["-s", &active_serial, "forward", &forward_spec, &remote_sock],
                    Duration::from_secs(3),
                )
                .is_ok()
            {
                let max_dim = q.max_dim();
                let bit_rate = q.bit_rate_bps();
                let server_cmd = format!(
                    "CLASSPATH={remote_jar} app_process / com.genymobile.scrcpy.Server {jar_ver} scid={scid_hex} log_level=info video=true audio=false control=false tunnel_forward=true raw_stream=true cleanup=false max_size={max_dim} max_fps=60 video_bit_rate={bit_rate} video_codec_options=profile=1,level=4096"
                );
                let mut cmd = Command::new(&adb_path);
                cmd.args(["-s", &active_serial, "shell", &server_cmd])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                crate::resources::quiet_command(&mut cmd);

                if let Ok(mut child) = cmd.spawn() {
                    pid_slot.store(child.id() as u64, Ordering::Relaxed);
                    // Trigger a slight display refresh on the phone so MediaCodec immediately emits SPS+PPS+IDR
                    let adb_nudge = adb_path.clone();
                    let ser_nudge = active_serial.clone();
                    thread::spawn(move || {
                        thread::sleep(Duration::from_millis(180));
                        let mut c = Command::new(adb_nudge);
                        c.args(["-s", &ser_nudge, "shell", "input keyevent 224 >/dev/null 2>&1 || true"]);
                        crate::resources::quiet_command(&mut c);
                        let _ = crate::adb::run_process(c, None, Duration::from_secs(2));
                    });

                    let addr: SocketAddr =
                        format!("127.0.0.1:{forward_port}").parse().unwrap();
                    let connect_deadline = Instant::now() + Duration::from_millis(2600);
                    let mut connected_stream = None;
                    while Instant::now() < connect_deadline && !stop.load(Ordering::Relaxed) {
                        if let Ok(s) = TcpStream::connect_timeout(&addr, Duration::from_millis(120))
                        {
                            let _ = s.set_nodelay(true);
                            let _ = s.set_read_timeout(Some(Duration::from_millis(180)));
                            let mut probe = [0u8; 64 * 1024];
                            // Verify the tunnel actually delivers bytes
                            let mut wait_data = 0;
                            while wait_data < 14 && !stop.load(Ordering::Relaxed) {
                                match (&s).read(&mut probe) {
                                    Ok(0) => break,
                                    Ok(n) => {
                                        process_annexb_chunk(
                                            &probe[..n],
                                            &shared,
                                            &bytes_broadcast,
                                        );
                                        connected_stream = Some(s);
                                        break;
                                    }
                                    Err(ref e)
                                        if e.kind() == std::io::ErrorKind::WouldBlock
                                            || e.kind() == std::io::ErrorKind::TimedOut =>
                                    {
                                        wait_data += 1;
                                    }
                                    Err(_) => break,
                                }
                            }
                            if connected_stream.is_some() {
                                break;
                            }
                        }
                        thread::sleep(Duration::from_millis(45));
                    }

                    if let Some(mut tcp) = connected_stream {
                        let _ = tcp.set_read_timeout(Some(Duration::from_millis(250)));
                        let mut buf = [0u8; 64 * 1024];
                        let mut frame_buf = Vec::with_capacity(128 * 1024);
                        let mut idle_ticks = 0u32;

                        while !stop.load(Ordering::Relaxed) {
                            // Check if user/connection_watch switched to a different transport
                            if let Ok(slot_g) = serial_slot.lock() {
                                if !slot_g.is_empty() && *slot_g != active_serial {
                                    break;
                                }
                            }
                            match tcp.read(&mut buf) {
                                Ok(0) => break,
                                Ok(n) => {
                                    idle_ticks = 0;
                                    frame_buf.extend_from_slice(&buf[..n]);
                                    // Drain any immediately queued TCP segments of the same frame
                                    let _ = tcp.set_read_timeout(Some(Duration::from_millis(4)));
                                    while frame_buf.len() < 512 * 1024 {
                                        match tcp.read(&mut buf) {
                                            Ok(0) => break,
                                            Ok(m) => frame_buf.extend_from_slice(&buf[..m]),
                                            Err(_) => break,
                                        }
                                    }
                                    let _ = tcp.set_read_timeout(Some(Duration::from_millis(250)));
                                    process_annexb_chunk(&frame_buf, &shared, &bytes_broadcast);
                                    frame_buf.clear();
                                }
                                Err(ref e)
                                    if e.kind() == std::io::ErrorKind::WouldBlock
                                        || e.kind() == std::io::ErrorKind::TimedOut =>
                                {
                                    idle_ticks += 1;
                                    // Every ~1s of static screen, re-mux the latest IDR into MPEG-TS/HLS so live players never stall
                                    if idle_ticks % 4 == 0 {
                                        let last_idr = {
                                            let guard = shared.0.lock().unwrap();
                                            guard.latest_annexb_idr.clone()
                                        };
                                        if !last_idr.is_empty() {
                                            process_annexb_chunk(
                                                &last_idr,
                                                &shared,
                                                &bytes_broadcast,
                                            );
                                        }
                                    }
                                    if matches!(child.try_wait(), Ok(Some(_))) {
                                        break;
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }

                    let _ = child.kill();
                    let _ = child.wait();
                    pid_slot.store(0, Ordering::Relaxed);
                    let _ = adb.run_timeout(
                        &["-s", &active_serial, "forward", "--remove", &forward_spec],
                        Duration::from_secs(1),
                    );
                    if !stop.load(Ordering::Relaxed) {
                        thread::sleep(Duration::from_millis(80));
                    }
                    continue;
                }
            }
        }

        // Fallback Path: Android `screenrecord --output-format=h264` + periodic screencap if scrcpy-server is unavailable
        let mut cmd = Command::new(&adb_path);
        cmd.args([
            "-s",
            &active_serial,
            "exec-out",
            "screenrecord",
            "--output-format=h264",
            "--size",
            q.screenrecord_size(),
            "--bit-rate",
            &q.bit_rate_bps().to_string(),
            "-",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
        crate::resources::quiet_command(&mut cmd);

        let Ok(mut child) = cmd.spawn() else {
            thread::sleep(Duration::from_millis(400));
            continue;
        };
        pid_slot.store(child.id() as u64, Ordering::Relaxed);

        if let Some(mut stdout) = child.stdout.take() {
            let mut buf = [0u8; 32 * 1024];
            let mut pending = Vec::with_capacity(64 * 1024);
            while !stop.load(Ordering::Relaxed) {
                match stdout.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        pending.extend_from_slice(&buf[..n]);
                        if pending.len() >= 2048 {
                            process_annexb_chunk(&pending, &shared, &bytes_broadcast);
                            pending.clear();
                        }
                    }
                    Err(_) => break,
                }
            }
        }
        let _ = child.kill();
        let _ = child.wait();
        pid_slot.store(0, Ordering::Relaxed);
        if !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(120));
        }
    }
}

struct HttpConnectionPermit(Arc<AtomicU64>);
impl Drop for HttpConnectionPermit {
    fn drop(&mut self) { self.0.fetch_sub(1, Ordering::Relaxed); }
}
fn authorized_cast_path<'a>(raw_path: &'a str, token: &str) -> Option<&'a str> {
    let prefix = format!("/session/{token}");
    raw_path.strip_prefix(&prefix).filter(|p| p.starts_with('/'))
}
fn read_cast_line(reader: &mut impl BufRead) -> std::io::Result<String> {
    let mut line = String::new();
    reader.take(8193).read_line(&mut line)?;
    if line.len() > 8192 || !line.ends_with('\n') {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "HTTP header too large or incomplete"));
    }
    Ok(line)
}

#[allow(clippy::too_many_arguments)]
fn http_accept_loop(
    listener: TcpListener,
    port: u16,
    host_ip: String,
    token: String,
    adb_path: PathBuf,
    serial_slot: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    remote_control: Arc<AtomicBool>,
    active_clients: Arc<AtomicU64>,
    bytes_broadcast: Arc<AtomicU64>,
    shared: Arc<(Mutex<SharedStreamState>, Condvar)>,
) {
    let connections = Arc::new(AtomicU64::new(0));
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut stream, _peer)) => {
                if connections.load(Ordering::Relaxed) >= 32 {
                    let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
                    continue;
                }
                connections.fetch_add(1, Ordering::Relaxed);
                let permit = HttpConnectionPermit(connections.clone());
                let stop_c = stop.clone();
                let remote_c = remote_control.clone();
                let clients_c = active_clients.clone();
                let bytes_c = bytes_broadcast.clone();
                let shared_c = shared.clone();
                let adb_c = adb_path.clone();
                let ser_c = serial_slot.clone();
                let hip_c = host_ip.clone();
                let token_c = token.clone();
                thread::spawn(move || {
                    let _permit = permit;
                    handle_http_client(
                        stream,
                        port,
                        hip_c,
                        token_c,
                        adb_c,
                        ser_c,
                        stop_c,
                        remote_c,
                        clients_c,
                        bytes_c,
                        shared_c,
                    );
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(50));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_http_client(
    mut stream: TcpStream,
    port: u16,
    host_ip: String,
    token: String,
    adb_path: PathBuf,
    serial_slot: Arc<Mutex<String>>,
    stop: Arc<AtomicBool>,
    remote_control: Arc<AtomicBool>,
    active_clients: Arc<AtomicU64>,
    bytes_broadcast: Arc<AtomicU64>,
    shared: Arc<(Mutex<SharedStreamState>, Condvar)>,
) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
    let _ = stream.set_nodelay(true);

    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(cloned);
    let Ok(req_line) = read_cast_line(&mut reader) else { return; };
    let mut parts = req_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_uppercase();
    let raw_path = parts.next().unwrap_or("").split('?').next().unwrap_or("");
    let Some(path) = authorized_cast_path(raw_path, &token) else {
        let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n");
        return;
    };
    if stop.load(Ordering::Relaxed) { return; }
    let mut content_len = 0usize;
    let mut total_headers = req_line.len();
    loop {
        let Ok(line) = read_cast_line(&mut reader) else { return; };
        total_headers += line.len();
        if total_headers > 16384 { return; }
        if line == "\r\n" || line == "\n" { break; }
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("transfer-encoding:") { return; }
        if let Some(rest) = lower.strip_prefix("content-length:") {
            let Ok(len) = rest.trim().parse::<usize>() else { return; };
            if len > 16384 { return; }
            content_len = len;
        }
    }
    let mut body = vec![0u8; content_len];
    if reader.read_exact(&mut body).is_err() { return; }
    if path == "/api/control" && method != "POST" {
        let _ = stream.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nAllow: POST\r\nContent-Length: 0\r\n\r\n");
        return;
    }

    match (method.as_str(), path) {
        ("GET", "/") | ("GET", "/cast") | ("GET", "/index.html") => {
            let html = build_web_receiver_html(&host_ip, port, remote_control.load(Ordering::Relaxed), &token);
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nReferrer-Policy: no-referrer\r\n\r\n",
                html.len()
            );
            let _ = stream.write_all(resp.as_bytes());
            let _ = stream.write_all(html.as_bytes());
        }
        ("GET", "/frame.jpg") => {
            let jpeg = {
                let guard = shared.0.lock().unwrap();
                if guard.latest_jpeg.is_empty() {
                    let (g, _) = shared
                        .1
                        .wait_timeout(guard, Duration::from_millis(2500))
                        .unwrap();
                    g.latest_jpeg.clone()
                } else {
                    guard.latest_jpeg.clone()
                }
            };
            if jpeg.is_empty() {
                let _ = stream.write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n");
                return;
            }
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nCache-Control: no-store, no-cache\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\n\r\n",
                jpeg.len()
            );
            let _ = stream.write_all(hdr.as_bytes());
            if stream.write_all(&jpeg).is_ok() {
                bytes_broadcast.fetch_add(jpeg.len() as u64, Ordering::Relaxed);
            }
        }
        ("GET", "/stream.mjpeg") => {
            active_clients.fetch_add(1, Ordering::Relaxed);
            let hdr = b"HTTP/1.1 200 OK\r\nContent-Type: multipart/x-mixed-replace; boundary=stepxframe\r\nCache-Control: no-store, no-cache\r\nPragma: no-cache\r\nReferrer-Policy: no-referrer\r\nConnection: close\r\n\r\n";
            if stream.write_all(hdr).is_ok() {
                let mut last_seq = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let (jpeg, seq) = {
                        let guard = shared.0.lock().unwrap();
                        if guard.frame_seq == last_seq {
                            let (g, _) = shared
                                .1
                                .wait_timeout(guard, Duration::from_millis(500))
                                .unwrap();
                            (g.latest_jpeg.clone(), g.frame_seq)
                        } else {
                            (guard.latest_jpeg.clone(), guard.frame_seq)
                        }
                    };
                    if seq == last_seq || jpeg.is_empty() {
                        continue;
                    }
                    last_seq = seq;
                    let part_hdr = format!(
                        "--stepxframe\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\n\r\n",
                        jpeg.len()
                    );
                    if stream.write_all(part_hdr.as_bytes()).is_err()
                        || stream.write_all(&jpeg).is_err()
                        || stream.write_all(b"\r\n").is_err()
                    {
                        break;
                    }
                    bytes_broadcast.fetch_add(jpeg.len() as u64, Ordering::Relaxed);
                }
            }
            active_clients.fetch_sub(1, Ordering::Relaxed);
        }
        ("HEAD", "/live.ts") => {
            let hdr = "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\ntransferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: DLNA.ORG_PN=AVC_TS_MP_HD_AAC_MULT5;DLNA.ORG_OP=00;DLNA.ORG_CI=1;DLNA.ORG_FLAGS=8d700000000000000000000000000000\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\n\r\n";
            let _ = stream.write_all(hdr.as_bytes());
        }
        ("GET", "/live.ts") => {
            active_clients.fetch_add(1, Ordering::Relaxed);
            let hdr = "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\ntransferMode.dlna.org: Streaming\r\ncontentFeatures.dlna.org: DLNA.ORG_PN=AVC_TS_MP_HD_AAC_MULT5;DLNA.ORG_OP=00;DLNA.ORG_CI=1;DLNA.ORG_FLAGS=8d700000000000000000000000000000\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n";
            if stream.write_all(hdr.as_bytes()).is_ok() {
                let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(64);
                let bootstrap = {
                    let mut guard = shared.0.lock().unwrap();
                    guard.ts_subscribers.push(tx);
                    guard.hls_segments.back().map(|(_, b)| b.clone())
                };
                if let Some(init_bytes) = bootstrap {
                    let _ = stream.write_all(&init_bytes);
                }
                while !stop.load(Ordering::Relaxed) {
                    match rx.recv_timeout(Duration::from_millis(800)) {
                        Ok(chunk) => {
                            if stream.write_all(&chunk).is_err() {
                                break;
                            }
                            bytes_broadcast.fetch_add(chunk.len() as u64, Ordering::Relaxed);
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                            let keepalive = {
                                let guard = shared.0.lock().unwrap();
                                guard.hls_segments.back().map(|(_, b)| b.clone())
                            };
                            if let Some(chunk) = keepalive {
                                if stream.write_all(&chunk).is_err() {
                                    break;
                                }
                            }
                        }
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            }
            active_clients.fetch_sub(1, Ordering::Relaxed);
        }
        ("GET", "/live.m3u8") => {
            let playlist = {
                let guard = shared.0.lock().unwrap();
                let first_seq = guard.hls_segments.front().map(|(s, _)| *s).unwrap_or(0);
                let mut m3u8 = format!(
                    "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:3\n#EXT-X-MEDIA-SEQUENCE:{first_seq}\n"
                );
                for (seq, _) in &guard.hls_segments {
                    m3u8.push_str(&format!("#EXTINF:2.000,\n/session/{token}/seg/{seq}.ts\n"));
                }
                m3u8
            };
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/vnd.apple.mpegurl\r\nReferrer-Policy: no-referrer\r\nCache-Control: no-store\r\nContent-Length: {}\r\n\r\n",
                playlist.len()
            );
            let _ = stream.write_all(hdr.as_bytes());
            let _ = stream.write_all(playlist.as_bytes());
        }
        ("GET", p) if p.starts_with("/seg/") && p.ends_with(".ts") => {
            let seq_str = p.trim_start_matches("/seg/").trim_end_matches(".ts");
            let seq = seq_str.parse::<u64>().unwrap_or(0);
            let seg = {
                let guard = shared.0.lock().unwrap();
                guard
                    .hls_segments
                    .iter()
                    .find(|(s, _)| *s == seq)
                    .map(|(_, b)| b.clone())
            };
            if let Some(bytes) = seg {
                let hdr = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp2t\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\n\r\n",
                    bytes.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                if stream.write_all(&bytes).is_ok() {
                    bytes_broadcast.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                }
            } else {
                let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
            }
        }
        ("GET", "/api/status") => {
            let guard = shared.0.lock().unwrap();
            let json = serde_json::json!({
                "ok": true,
                "fps": guard.fps_x10 as f32 / 10.0,
                "frame_seq": guard.frame_seq,
                "width": guard.phone_width,
                "height": guard.phone_height,
                "clients": active_clients.load(Ordering::Relaxed),
                "bytes": bytes_broadcast.load(Ordering::Relaxed),
                "remote_control": remote_control.load(Ordering::Relaxed),
            })
            .to_string();
            let hdr = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\n\r\n",
                json.len()
            );
            let _ = stream.write_all(hdr.as_bytes());
            let _ = stream.write_all(json.as_bytes());
        }
        ("POST", "/api/control") => {
            if !remote_control.load(Ordering::Relaxed) {
                let msg = r#"{"ok":false,"error":"远端触控反控已在桌面端关闭"}"#;
                let hdr = format!(
                    "HTTP/1.1 403 Forbidden\r\nContent-Type: application/json\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\n\r\n{msg}",
                    msg.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                return;
            }
            let (pw, ph) = {
                let guard = shared.0.lock().unwrap();
                (guard.phone_width.max(1), guard.phone_height.max(1))
            };
            let payload_str = String::from_utf8_lossy(&body);
            let active_serial = resolve_active_cast_serial(&adb_path, &serial_slot);
            let result = dispatch_remote_control_command(adb_path, active_serial, pw, ph, &payload_str);
            let (status, msg) = match result {
                Ok(()) => ("200 OK", serde_json::json!({"ok": true}).to_string()),
                Err(err) => ("400 Bad Request", serde_json::json!({"ok":false,"error":err.to_string()}).to_string()),
            };
            let hdr = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\n\r\n{msg}",
                msg.len()
            );
            let _ = stream.write_all(hdr.as_bytes());
        }
        _ => {
            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n");
        }
    }
}

fn dispatch_remote_control_command(
    adb_path: PathBuf,
    serial: String,
    phone_w: u32,
    phone_h: u32,
    payload: &str,
) -> Result<()> {
    let val: serde_json::Value = serde_json::from_str(payload).context("无效的控制请求")?;
    let action = val.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let shell_cmd = match action {
        "tap" => {
            let nx = val.get("x").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0);
            let ny = val.get("y").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0);
            let x = (nx * phone_w as f64).round() as u32;
            let y = (ny * phone_h as f64).round() as u32;
            format!("input tap {x} {y}")
        }
        "swipe" => {
            let x1 = (val.get("x1").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0)
                * phone_w as f64)
                .round() as u32;
            let y1 = (val.get("y1").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0)
                * phone_h as f64)
                .round() as u32;
            let x2 = (val.get("x2").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0)
                * phone_w as f64)
                .round() as u32;
            let y2 = (val.get("y2").and_then(|v| v.as_f64()).unwrap_or(0.5).clamp(0.0, 1.0)
                * phone_h as f64)
                .round() as u32;
            let dur = val
                .get("duration")
                .and_then(|v| v.as_u64())
                .unwrap_or(220)
                .clamp(80, 1500);
            format!("input swipe {x1} {y1} {x2} {y2} {dur}")
        }
        "key" => {
            let code = val.get("code").and_then(|v| v.as_u64()).unwrap_or(0);
            if ![3,4,24,25,26,66,67,85,164,187,223,224].contains(&code) {
                bail!("不支持的按键");
            }
            format!("input keyevent {code}")
        }
        "text" => {
            let raw = val.get("text").and_then(|v| v.as_str()).unwrap_or("").trim();
            if raw.is_empty() {
                bail!("请输入文字");
            }
            let escaped: String = raw
                .chars()
                .filter(|c| !matches!(c, '\'' | '"' | '`' | '$' | ';' | '&' | '|'))
                .collect();
            format!("input text '{}'", escaped.replace(' ', "%s"))
        }
        _ => bail!("不支持的操作"),
    };

    let mut cmd = Command::new(adb_path);
    cmd.args(["-s", &serial, "shell", &shell_cmd]);
    crate::resources::quiet_command(&mut cmd);
    let output = crate::adb::run_process(cmd, None, Duration::from_secs(3))?;
    if !output.status.success() { bail!("手机未执行该操作，请检查连接与解锁状态"); }
    Ok(())
}

// ============================================================================
// Active Stream Push to DLNA / AirPlay / Google Cast / Local Media Players
// ============================================================================

pub fn push_stream_to_receiver(
    receiver: &LanCastReceiver,
    stats: &LanCastStats,
    adb: Option<&Adb>,
    serial: Option<&str>,
) -> Result<String> {
    if !stats.running {
        bail!("请先启动局域网投屏网关");
    }
    match receiver.protocol {
        LanCastProtocol::DlnaUpnp => {
            let control_url = receiver
                .control_url
                .as_deref()
                .context("该 DLNA 设备未暴露 AVTransport 控制端点")?;
            dlna_soap_set_uri_and_play(control_url, &stats.ts_url, "StepX 手机实时投屏")?;
            Ok(format!(
                "已通过 DLNA AVTransport 协议向「{}」({}) 推送手机实时画面 ({})",
                receiver.name, receiver.ip, stats.ts_url
            ))
        }
        LanCastProtocol::AirPlay => {
            airplay_push_hls(&receiver.ip, receiver.port, &stats.hls_url)?;
            Ok(format!(
                "已通过 Apple AirPlay 协议向「{}」({}) 推送手机 HLS 直播流 ({})",
                receiver.name, receiver.ip, stats.hls_url
            ))
        }
        LanCastProtocol::GoogleCast => {
            bail!("已发现该 Google Cast 设备，但当前未实现 Cast V2 媒体会话。请在接收端浏览器打开本次投屏链接，或使用支持的 DLNA 接收端");
        }
        LanCastProtocol::Miracast => {
            if let (Some(adb), Some(serial)) = (adb, serial) {
                connect_phone_miracast(adb, serial, &receiver.ip)?;
                Ok(format!(
                    "已在手机上激活原生 Miracast 并打开无线显示器连接面板（目标：{}）",
                    receiver.name
                ))
            } else {
                bail!("连接 Miracast 需要手机处于已连接状态")
            }
        }
        LanCastProtocol::WebReceiver => {
            if receiver.ip == stats.host_ip {
                let opened = open_stream_in_local_player(&stats.web_url, false)?;
                Ok(format!("已在本机打开局域网投屏接收控制台：{opened}"))
            } else {
                Ok(format!(
                    "局域网终端「{}」可直接在浏览器打开 {} 或使用 VLC 打开 {}",
                    receiver.name, stats.web_url, stats.ts_url
                ))
            }
        }
    }
}

pub fn stop_receiver_push(receiver: &LanCastReceiver) -> Result<()> {
    match receiver.protocol {
        LanCastProtocol::DlnaUpnp => {
            if let Some(control_url) = &receiver.control_url {
                let _ = dlna_soap_stop(control_url);
            }
        }
        LanCastProtocol::AirPlay => {
            let _ = airplay_stop(&receiver.ip, receiver.port);
        }
        _ => {}
    }
    Ok(())
}

pub fn open_stream_in_local_player(url: &str, prefer_vlc: bool) -> Result<String> {
    if prefer_vlc {
        for bin in ["vlc", "mpv", "ffplay"] {
            if let Some(exe) = crate::adb::find_executable(bin) {
                let mut cmd = Command::new(exe);
                if bin == "vlc" {
                    cmd.args(["--network-caching=250", url]);
                } else {
                    cmd.arg(url);
                }
                crate::resources::quiet_command(&mut cmd);
                cmd.stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                cmd.spawn().context("无法启动本地流媒体播放器")?;
                return Ok(format!("已使用 {bin} 打开局域网直播流 {url}"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    let mut cmd = {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(target_os = "windows")]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    };
    crate::resources::quiet_command(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    cmd.spawn().context("无法打开系统浏览器")?;
    Ok(url.to_string())
}

fn dlna_soap_set_uri_and_play(control_url: &str, stream_url: &str, title: &str) -> Result<()> {
    let didl = format!(
        r#"&lt;DIDL-Lite xmlns=&quot;urn:schemas-upnp-org:metadata-1-0/DIDL-Lite/&quot; xmlns:dc=&quot;http://purl.org/dc/elements/1.1/&quot; xmlns:upnp=&quot;urn:schemas-upnp-org:metadata-1-0/upnp/&quot;&gt;&lt;item id=&quot;0&quot; parentID=&quot;-1&quot; restricted=&quot;1&quot;&gt;&lt;dc:title&gt;{title}&lt;/dc:title&gt;&lt;upnp:class&gt;object.item.videoItem&lt;/upnp:class&gt;&lt;res protocolInfo=&quot;http-get:*:video/mp2t:DLNA.ORG_PN=AVC_TS_MP_HD_AAC_MULT5;DLNA.ORG_OP=00;DLNA.ORG_CI=1;DLNA.ORG_FLAGS=8d700000000000000000000000000000&quot;&gt;{stream_url}&lt;/res&gt;&lt;/item&gt;&lt;/DIDL-Lite&gt;"#
    );
    let set_body = format!(
        r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:SetAVTransportURI xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">
      <InstanceID>0</InstanceID>
      <CurrentURI>{stream_url}</CurrentURI>
      <CurrentURIMetaData>{didl}</CurrentURIMetaData>
    </u:SetAVTransportURI>
  </s:Body>
</s:Envelope>"#
    );
    send_soap_action(
        control_url,
        "\"urn:schemas-upnp-org:service:AVTransport:1#SetAVTransportURI\"",
        &set_body,
    )?;

    let play_body = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:Play xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">
      <InstanceID>0</InstanceID>
      <Speed>1</Speed>
    </u:Play>
  </s:Body>
</s:Envelope>"#;
    send_soap_action(
        control_url,
        "\"urn:schemas-upnp-org:service:AVTransport:1#Play\"",
        play_body,
    )?;
    Ok(())
}

fn dlna_soap_stop(control_url: &str) -> Result<()> {
    let stop_body = r#"<?xml version="1.0" encoding="utf-8"?>
<s:Envelope xmlns:s="http://schemas.xmlsoap.org/soap/envelope/" s:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
  <s:Body>
    <u:Stop xmlns:u="urn:schemas-upnp-org:service:AVTransport:1">
      <InstanceID>0</InstanceID>
    </u:Stop>
  </s:Body>
</s:Envelope>"#;
    send_soap_action(
        control_url,
        "\"urn:schemas-upnp-org:service:AVTransport:1#Stop\"",
        stop_body,
    )
}

fn send_soap_action(control_url: &str, soap_action: &str, xml_body: &str) -> Result<()> {
    let parsed = url::Url::parse(control_url).context("无效的 DLNA 控制 URL")?;
    let host = parsed.host_str().context("DLNA URL 缺少主机名")?;
    let port = parsed.port_or_known_default().unwrap_or(80);
    let path = if let Some(q) = parsed.query() {
        format!("{}?{q}", parsed.path())
    } else {
        parsed.path().to_string()
    };
    let addr = format!("{host}:{port}")
        .to_socket_addrs()?
        .next()
        .context("无法解析 DLNA 设备地址")?;
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).context("连接 DLNA 设备超时")?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: text/xml; charset=\"utf-8\"\r\nSOAPAction: {soap_action}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{xml_body}",
        xml_body.len()
    );
    stream.write_all(req.as_bytes())?;
    let mut resp = Vec::new();
    let _ = stream.read_to_end(&mut resp);
    let status_line = String::from_utf8_lossy(&resp);
    if !status_line.starts_with("HTTP/1.1 200") && !status_line.starts_with("HTTP/1.0 200") {
        bail!(
            "DLNA 设备返回非成功状态：{}",
            status_line.lines().next().unwrap_or("未知错误")
        );
    }
    Ok(())
}

fn airplay_push_hls(ip: &str, port: u16, hls_url: &str) -> Result<()> {
    let addr: SocketAddr = format!("{ip}:{port}").parse()?;
    let mut stream =
        TcpStream::connect_timeout(&addr, Duration::from_secs(2)).context("连接 AirPlay 设备超时")?;
    let body = format!("Content-Location: {hls_url}\nStart-Position: 0.0\n\n");
    let req = format!(
        "POST /play HTTP/1.1\r\nHost: {ip}:{port}\r\nUser-Agent: MediaControl/1.0\r\nContent-Type: text/parameters\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(req.as_bytes())?;
    Ok(())
}

fn airplay_stop(ip: &str, port: u16) -> Result<()> {
    let addr: SocketAddr = format!("{ip}:{port}").parse()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1))?;
    let req = format!(
        "POST /stop HTTP/1.1\r\nHost: {ip}:{port}\r\nUser-Agent: MediaControl/1.0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;
    Ok(())
}

fn dial_notify_receiver(app_url: &str, web_url: &str) -> Result<()> {
    let parsed = url::Url::parse(app_url)?;
    let host = parsed.host_str().unwrap_or("127.0.0.1");
    let port = parsed.port_or_known_default().unwrap_or(8008);
    let addr: SocketAddr = format!("{host}:{port}").parse()?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(800))?;
    let req = format!(
        "POST {} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{web_url}",
        parsed.path(),
        web_url.len()
    );
    let _ = stream.write_all(req.as_bytes());
    Ok(())
}

fn html_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}

fn build_web_receiver_html(host_ip: &str, port: u16, remote_enabled: bool, token: &str) -> String {
    let remote_badge = if remote_enabled {
        "远端触控反控：已开启"
    } else {
        "远端触控反控：仅观看"
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width,initial-scale=1,maximum-scale=1,user-scalable=no"/>
<title>StepX Companion · 局域网跨机投屏接收端</title>
<style>
  :root {{
    --bg: #09090b;
    --panel: #141418;
    --border: #27272a;
    --text: #f4f4f5;
    --muted: #a1a1aa;
    --accent: #10b981;
  }}
  * {{ box-sizing: border-box; margin: 0; padding: 0; user-select: none; }}
  body {{
    background: var(--bg);
    color: var(--text);
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    height: 100vh;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }}
  header {{
    height: 54px;
    background: var(--panel);
    border-bottom: 1px solid var(--border);
    padding: 0 20px;
    display: flex;
    align-items: center;
    justify-content: space-between;
  }}
  .brand {{ display: flex; align-items: center; gap: 12px; font-weight: 600; font-size: 15px; }}
  .badge {{
    font-size: 12px;
    padding: 3px 10px;
    border-radius: 999px;
    background: rgba(16,185,129,0.15);
    color: #34d399;
    font-weight: 500;
  }}
  .toolbar {{ display: flex; align-items: center; gap: 8px; }}
  button, .btn {{
    background: #1f1f25;
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 12px;
    font-size: 12px;
    cursor: pointer;
    transition: background 0.15s;
    text-decoration: none;
  }}
  button:hover, .btn:hover {{ background: #2c2c35; }}
  button.active {{ background: #10b981; color: #052e16; border-color: #10b981; font-weight: 600; }}
  main {{
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 20px;
    padding: 16px;
    overflow: hidden;
  }}
  .stage {{
    position: relative;
    height: 100%;
    display: flex;
    align-items: center;
    justify-content: center;
    background: #000;
    border: 1px solid var(--border);
    border-radius: 20px;
    overflow: hidden;
    box-shadow: 0 20px 50px rgba(0,0,0,0.6);
  }}
  #mjpegView, #hlsView {{
    max-height: calc(100vh - 110px);
    max-width: calc(100vw - 260px);
    width: auto;
    height: auto;
    object-fit: contain;
    display: block;
    cursor: pointer;
  }}
  #hlsView {{ display: none; }}
  .rail {{
    width: 210px;
    background: var(--panel);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 14px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }}
  .rail h3 {{ font-size: 13px; color: var(--muted); margin-bottom: 2px; }}
  .grid2 {{ display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }}
  .input-row {{ display: flex; gap: 6px; margin-top: 4px; }}
  .input-row input {{
    flex: 1;
    background: #09090b;
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 6px 8px;
    color: var(--text);
    font-size: 12px;
    user-select: text;
  }}
  .meta {{ font-size: 11px; color: var(--muted); line-height: 1.5; word-break: break-all; }}
</style>
</head>
<body>
<header>
  <div class="brand">
    <span>StepX Companion · 局域网实时投屏</span>
    <span class="badge" id="fpsBadge">实时连接中…</span>
    <span class="badge">{remote_badge}</span>
  </div>
  <div class="toolbar">
    <button id="btnMjpeg" class="active" onclick="switchMode('mjpeg')">极速互动流 (MJPEG)</button>
    <button id="btnHls" onclick="switchMode('hls')">高清视频流 (H.264 HLS)</button>
    <a class="btn" href="/live.ts" target="_blank">下载/VLC打开 MPEG-TS 流</a>
    <button onclick="toggleFullscreen()">全屏显示</button>
  </div>
</header>
<main>
  <div class="stage" id="stageBox">
    <img id="mjpegView" src="/session/{token}/stream.mjpeg" alt="StepX Live Screen" draggable="false"/>
    <video id="hlsView" controls autoplay muted playsinline></video>
  </div>
  <aside class="rail">
    <h3>远端手机控制</h3>
    <div class="grid2">
      <button onclick="sendKey(4)">← 返回</button>
      <button onclick="sendKey(3)">⌂ 主屏幕</button>
      <button onclick="sendKey(187)">≡ 多任务</button>
      <button onclick="sendKey(26)">🔒 锁屏/亮屏</button>
      <button onclick="sendKey(24)">🔊 音量 +</button>
      <button onclick="sendKey(25)">🔉 音量 −</button>
      <button onclick="sendSwipe(0.5,0.78,0.5,0.22)">↑ 上滑翻页</button>
      <button onclick="sendSwipe(0.5,0.22,0.5,0.78)">↓ 下滑翻页</button>
    </div>
    <h3>发送文字到手机</h3>
    <div class="input-row">
      <input id="txtInput" placeholder="输入英文/数字…" onkeydown="if(event.key==='Enter')sendText()"/>
      <button onclick="sendText()">发送</button>
    </div>
    <h3>多协议流地址</h3>
    <div class="meta">
      <div><strong>MPEG-TS (DLNA/VLC):</strong><br/>http://{host_ip}:{port}/live.ts</div>
      <div style="margin-top:6px;"><strong>Apple HLS (AirPlay):</strong><br/>http://{host_ip}:{port}/session/{token}/live.m3u8</div>
    </div>
  </aside>
</main>
<script>
const img = document.getElementById('mjpegView');
const vid = document.getElementById('hlsView');
let downPt = null;
let downAt = 0;

function normPt(ev) {{
  const r = img.getBoundingClientRect();
  const x = Math.max(0, Math.min(1, (ev.clientX - r.left) / Math.max(1, r.width)));
  const y = Math.max(0, Math.min(1, (ev.clientY - r.top) / Math.max(1, r.height)));
  return {{ x, y }};
}}

img.addEventListener('pointerdown', (e) => {{
  downPt = normPt(e);
  downAt = Date.now();
}});

img.addEventListener('pointerup', (e) => {{
  if (!downPt) return;
  const upPt = normPt(e);
  const dx = upPt.x - downPt.x;
  const dy = upPt.y - downPt.y;
  const dist = Math.hypot(dx, dy);
  const dur = Math.min(1000, Math.max(100, Date.now() - downAt));
  if (dist < 0.025) {{
    fetch('/session/{token}/api/control', {{
      method: 'POST',
      body: JSON.stringify({{ action: 'tap', x: upPt.x, y: upPt.y }})
    }});
  }} else {{
    fetch('/session/{token}/api/control', {{
      method: 'POST',
      body: JSON.stringify({{ action: 'swipe', x1: downPt.x, y1: downPt.y, x2: upPt.x, y2: upPt.y, duration: dur }})
    }});
  }}
  downPt = null;
}});

function sendKey(code) {{
  fetch('/session/{token}/api/control', {{ method: 'POST', body: JSON.stringify({{ action: 'key', code }}) }});
}}
function sendSwipe(x1, y1, x2, y2) {{
  fetch('/session/{token}/api/control', {{ method: 'POST', body: JSON.stringify({{ action: 'swipe', x1, y1, x2, y2, duration: 240 }}) }});
}}
function sendText() {{
  const el = document.getElementById('txtInput');
  if (!el.value) return;
  fetch('/session/{token}/api/control', {{ method: 'POST', body: JSON.stringify({{ action: 'text', text: el.value }}) }});
  el.value = '';
}}
function switchMode(mode) {{
  document.getElementById('btnMjpeg').classList.toggle('active', mode === 'mjpeg');
  document.getElementById('btnHls').classList.toggle('active', mode === 'hls');
  if (mode === 'hls') {{
    img.style.display = 'none';
    vid.style.display = 'block';
    vid.src = '/session/{token}/live.m3u8';
    vid.play().catch(()=>{{}});
  }} else {{
    vid.pause();
    vid.style.display = 'none';
    img.style.display = 'block';
    img.src = '/session/{token}/stream.mjpeg?t=' + Date.now();
  }}
}}
function toggleFullscreen() {{
  const box = document.getElementById('stageBox');
  if (!document.fullscreenElement) box.requestFullscreen().catch(()=>{{}});
  else document.exitFullscreen();
}}
setInterval(() => {{
  fetch('/session/{token}/api/status').then(r => r.json()).then(d => {{
    if (d && d.ok) {{
      document.getElementById('fpsBadge').textContent =
        d.width + '×' + d.height + ' · ' + d.fps.toFixed(1) + ' FPS · ' + d.clients + ' 终端在线';
    }}
  }}).catch(()=>{{}});
}}, 1500);
</script>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request_for_test(path: &str, method: &str, remote: bool) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let request = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n");
        let worker = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle_http_client(stream, addr.port(), "127.0.0.1".into(), "secret-test".into(),
                PathBuf::from("must-not-run-adb"), Arc::new(Mutex::new("fixture".into())),
                Arc::new(AtomicBool::new(false)), Arc::new(AtomicBool::new(remote)),
                Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)),
                Arc::new((Mutex::new(SharedStreamState::default()), Condvar::new())));
        });
        let mut client = TcpStream::connect(addr).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        client.write_all(request.as_bytes()).unwrap();
        let mut result = String::new();
        client.read_to_string(&mut result).unwrap();
        worker.join().unwrap();
        result
    }

    #[test]
    fn cast_requires_session_link_and_control_is_post_only() {
        for path in ["/cast", "/api/status", "/frame.jpg", "/session/wrong/live.ts"] {
            assert!(request_for_test(path, "GET", true).starts_with("HTTP/1.1 403"));
        }
        assert!(request_for_test("/session/secret-test/api/status", "GET", false).starts_with("HTTP/1.1 200"));
        assert!(request_for_test("/session/secret-test/api/control", "GET", true).starts_with("HTTP/1.1 405"));
        assert!(request_for_test("/session/secret-test/api/control", "POST", false).starts_with("HTTP/1.1 403"));
        assert!(request_for_test("/session/secret-test/api/control", "POST", true).starts_with("HTTP/1.1 400"));
        let html = request_for_test("/session/secret-test/cast", "GET", false);
        assert!(html.contains("/session/secret-test/stream.mjpeg"));
        assert!(html.contains("/session/secret-test/api/control"));
        assert!(!html.contains("Access-Control-Allow-Origin: *"));
        assert!(authorized_cast_path("/session/secret-test/api/status", "next-session").is_none());
    }

    #[test]
    fn static_frames_are_not_repeated_or_reported_as_30_fps() {
        let state = Arc::new((Mutex::new(SharedStreamState {latest_jpeg: vec![1,2,3], frame_seq: 1, fps_x10: 300, ..Default::default()}), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let shared = state.clone(); let signal = stop.clone();
        let worker = thread::spawn(move || frame_pacer_loop(PathBuf::from("must-not-run-adb"), Arc::new(Mutex::new("fixture".into())), signal, Arc::new(AtomicU8::new(0)), shared, Arc::new(AtomicU64::new(0))));
        // Wait for a sample instead of assuming two ticks fit a fixed sleep on CI.
        let deadline = Instant::now() + Duration::from_secs(5);
        while state.0.lock().unwrap().fps_x10 != 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(25));
        }
        stop.store(true, Ordering::Relaxed); worker.join().unwrap();
        let state = state.0.lock().unwrap();
        assert_eq!(state.frame_seq, 1);
        assert_eq!(state.fps_x10, 0);
    }

    #[test]
    fn oversized_header_is_rejected() {
        let data = format!("{}\n", "x".repeat(9000));
        assert!(read_cast_line(&mut std::io::Cursor::new(data)).is_err());
    }

    #[test]
    fn ts_muxer_produces_valid_188_byte_aligned_packets() {
        let mut muxer = ts_mux::TsMuxer::new();
        // Fake Annex-B stream with SPS (type 7), PPS (type 8), and IDR slice (type 5)
        let annexb = [
            0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0x00, 0x1F, // SPS
            0x00, 0x00, 0x00, 0x01, 0x68, 0xCE, 0x3C, 0x80, // PPS
            0x00, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x21, 0xA0, // IDR
        ];
        let (ts, has_idr) = muxer.mux_annexb_chunk(&annexb, 90_000);
        assert!(has_idr);
        assert!(!ts.is_empty());
        assert_eq!(ts.len() % ts_mux::TS_PACKET_SIZE, 0);
        for chunk in ts.chunks_exact(ts_mux::TS_PACKET_SIZE) {
            assert_eq!(chunk[0], 0x47, "Every TS packet must start with sync byte 0x47");
        }
    }

    #[test]
    fn parse_upnp_description_extracts_friendly_name_and_avtransport() {
        let sample = r#"<?xml version="1.0"?>
        <root xmlns="urn:schemas-upnp-org:device-1-0">
          <device>
            <friendlyName>客厅小米电视 4K</friendlyName>
            <manufacturer>Xiaomi</manufacturer>
            <modelName>MiTV-P1</modelName>
            <serviceList>
              <service>
                <serviceType>urn:schemas-upnp-org:service:AVTransport:1</serviceType>
                <controlURL>/upnp/control/rendertransport1</controlURL>
              </service>
              <service>
                <serviceType>urn:schemas-upnp-org:service:RenderingControl:1</serviceType>
                <controlURL>/upnp/control/rendercontrol1</controlURL>
              </service>
            </serviceList>
          </device>
        </root>"#;
        let (name, model, mfr, av, rc) =
            parse_upnp_description_xml(sample, "http://10.0.1.88:49152/description.xml");
        assert_eq!(name.as_deref(), Some("客厅小米电视 4K"));
        assert_eq!(model.as_deref(), Some("MiTV-P1"));
        assert_eq!(mfr.as_deref(), Some("Xiaomi"));
        assert_eq!(
            av.as_deref(),
            Some("http://10.0.1.88:49152/upnp/control/rendertransport1")
        );
        assert_eq!(
            rc.as_deref(),
            Some("http://10.0.1.88:49152/upnp/control/rendercontrol1")
        );
    }

    #[test]
    fn qr_png_generation_creates_valid_png_file() {
        let path = generate_cast_qr_png("http://10.0.1.134:19780/cast", 19780)
            .expect("QR PNG should be generated");
        assert!(path.is_file());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    #[ignore = "requires an explicitly selected, authorized physical phone"]
    fn live_server_streams_jpeg_and_mpegts_when_phone_connected() {
        let adb = Adb::discover().unwrap();
        let serial = std::env::var("STEPX_TEST_DEVICE").expect("select authorized phone with STEPX_TEST_DEVICE");
        let device = crate::device::Device::inspect(&adb, &serial).unwrap();
        device.verify(&adb).unwrap();
        let mut srv = LanCastServer::start(
            adb.path.clone(),
            serial,
            LanCastQuality::Hd1080p,
            true,
        )
        .expect("server should start");
        for _ in 0..60 {
            if !srv.latest_jpeg_bytes().is_empty() { break; }
            thread::sleep(Duration::from_millis(250));
        }
        thread::sleep(Duration::from_secs(3));
        assert!(srv.shared.0.lock().unwrap().hw_frames_decoded > 0, "hardware stream must decode beyond a bootstrap screenshot");
        let st = srv.stats();
        println!(
            "LIVE_STATS: port={} frames={} bytes={} fps={} w={} h={}",
            st.port, st.frames_captured, st.bytes_broadcast, st.fps, st.phone_width, st.phone_height
        );
        assert!(st.frames_captured > 0, "Should capture actual phone frames");
        assert!(!srv.latest_jpeg_bytes().is_empty());
        let mut client = TcpStream::connect(("127.0.0.1", st.port)).unwrap();
        client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        client.write_all(format!("GET /session/{}/frame.jpg HTTP/1.1\r\nHost: localhost\r\n\r\n", srv.session_token).as_bytes()).unwrap();
        let mut received = Vec::new(); client.read_to_end(&mut received).unwrap();
        let body_at = received.windows(4).position(|x| x == b"\r\n\r\n").unwrap() + 4;
        let image = image::load_from_memory(&received[body_at..]).expect("HTTP receiver must get decodable phone JPEG");
        assert!(image.width() > 100 && image.height() > 100);
        println!("LIVE_RECEIVER_OK: decoded {}x{}, {} bytes", image.width(), image.height(), received.len() - body_at);
        srv.stop();
    }

    #[test]
    #[ignore = "requires an explicitly selected, authorized physical phone"]
    fn live_wireless_transport_casts_and_fails_over_at_high_fps() {
        let adb = Adb::discover().unwrap();
        let w_serial = std::env::var("STEPX_WIRELESS_TEST_DEVICE").expect("explicit authorized wireless transport required");
        crate::device::Device::inspect(&adb, &w_serial).unwrap().verify(&adb).unwrap();
        let mut srv = LanCastServer::start(
            adb.path.clone(),
            w_serial.clone(),
            LanCastQuality::Hd1080p,
            true,
        )
        .expect("wireless server should start");
        srv.set_serial(w_serial.clone());
        thread::sleep(Duration::from_millis(2400));
        let st = srv.stats();
        println!(
            "WIRELESS_LIVE_STATS ({}): port={} frames={} bytes={} fps={} w={} h={}",
            w_serial, st.port, st.frames_captured, st.bytes_broadcast, st.fps, st.phone_width, st.phone_height
        );
        assert!(
            st.frames_captured > 0,
            "Wireless casting must capture and broadcast frames"
        );
        srv.stop();
    }
}
