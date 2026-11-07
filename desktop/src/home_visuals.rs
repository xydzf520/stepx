//! Pure SVG chart builders for the desktop Home hub (`Page::Home`).
//!
//! Every builder returns a self-contained `<svg>` document string that the UI rasterizes
//! through resvg. The documents deliberately contain no `<text>` nodes: labels are drawn
//! natively by the UI so CJK typography, theming and hit-testing stay consistent, and so
//! rendering never depends on which system fonts resvg can find.
//!
//! All inputs are clamped; NaN / out-of-range values never produce malformed markup.

use std::f32::consts::PI;
use std::fmt::Write as _;

/// An sRGB color with straight alpha used for SVG `fill` / `stroke` attributes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paint {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// 0.0 ..= 1.0
    pub alpha: f32,
}

impl Paint {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, alpha: 1.0 }
    }

    pub fn with_alpha(self, alpha: f32) -> Self {
        Self {
            alpha: clamp01(alpha),
            ..self
        }
    }

    /// Composites this (possibly translucent) colour over an opaque `matte`, returning an
    /// opaque colour. The host renderer uploads resvg's premultiplied pixels as if they were
    /// straight alpha, so translucent fills render too dark; charts therefore emit only
    /// opaque colours, pre-blended against the surface they sit on.
    pub fn over(self, matte: Paint) -> Paint {
        let a = clamp01(self.alpha);
        let mix = |fg: u8, bg: u8| -> u8 {
            (fg as f32 * a + bg as f32 * (1.0 - a)).round().clamp(0.0, 255.0) as u8
        };
        Paint {
            r: mix(self.r, matte.r),
            g: mix(self.g, matte.g),
            b: mix(self.b, matte.b),
            alpha: 1.0,
        }
    }

    /// `fill="#RRGGBB" fill-opacity="a"`
    pub fn fill_attr(&self) -> String {
        self.fill().to_string()
    }

    /// `stroke="#RRGGBB" stroke-opacity="a"`
    pub fn stroke_attr(&self) -> String {
        self.stroke().to_string()
    }

    #[inline]
    fn fill(self) -> FillAttr {
        FillAttr(self)
    }

    #[inline]
    fn stroke(self) -> StrokeAttr {
        StrokeAttr(self)
    }
}

#[derive(Clone, Copy, Debug)]
struct FillAttr(Paint);

impl std::fmt::Display for FillAttr {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r##"fill="#{r:02X}{g:02X}{b:02X}" fill-opacity="{a}""##,
            r = self.0.r,
            g = self.0.g,
            b = self.0.b,
            a = fmt_num(clamp01(self.0.alpha))
        )
    }
}

#[derive(Clone, Copy, Debug)]
struct StrokeAttr(Paint);

impl std::fmt::Display for StrokeAttr {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            r##"stroke="#{r:02X}{g:02X}{b:02X}" stroke-opacity="{a}""##,
            r = self.0.r,
            g = self.0.g,
            b = self.0.b,
            a = fmt_num(clamp01(self.0.alpha))
        )
    }
}

#[inline]
fn clamp01(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug)]
struct FmtNum(f32);

impl std::fmt::Display for FmtNum {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let v = if self.0.is_finite() {
            self.0.clamp(-1.0e12, 1.0e12)
        } else {
            0.0
        };
        let scaled = (v * 100.0).round() as i64;
        if scaled == 0 {
            return f.write_str("0");
        }
        let sign = if scaled < 0 { "-" } else { "" };
        let abs = scaled.unsigned_abs();
        let int_part = abs / 100;
        let frac = abs % 100;
        if frac == 0 {
            write!(f, "{sign}{int_part}")
        } else if frac % 10 == 0 {
            write!(f, "{sign}{int_part}.{}", frac / 10)
        } else {
            write!(f, "{sign}{int_part}.{frac:02}")
        }
    }
}

#[cfg(test)]
impl PartialEq<&str> for FmtNum {
    fn eq(&self, other: &&str) -> bool {
        self.to_string() == *other
    }
}

/// Compact, locale-independent number formatting (max 2 decimals, no `-0`).
#[inline]
fn fmt_num(v: f32) -> FmtNum {
    FmtNum(v)
}

#[inline]
fn svg_open_with_capacity(width: f32, height: f32, cap: usize) -> String {
    let mut s = String::with_capacity(cap);
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}">"#,
        w = fmt_num(width),
        h = fmt_num(height)
    );
    s
}

/// Point on a circle; `turn` 0.0 is 12 o'clock and increases clockwise.
#[inline]
fn polar(cx: f32, cy: f32, r: f32, turn: f32) -> (f32, f32) {
    let a = turn * 2.0 * PI - PI / 2.0;
    let (sin_a, cos_a) = a.sin_cos();
    (cx + r * cos_a, cy + r * sin_a)
}

#[derive(Clone, Copy, Debug)]
struct ArcPath {
    cx: f32,
    cy: f32,
    r: f32,
    start: f32,
    end: f32,
}

impl std::fmt::Display for ArcPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let span = (self.end - self.start).clamp(0.0, 1.0);
        if span >= 0.9999 {
            let (x0, y0) = polar(self.cx, self.cy, self.r, self.start);
            let (x1, y1) = polar(self.cx, self.cy, self.r, self.start + 0.5);
            return write!(
                f,
                "M {} {} A {r} {r} 0 1 1 {} {} A {r} {r} 0 1 1 {} {}",
                fmt_num(x0),
                fmt_num(y0),
                fmt_num(x1),
                fmt_num(y1),
                fmt_num(x0),
                fmt_num(y0),
                r = fmt_num(self.r)
            );
        }
        let (x0, y0) = polar(self.cx, self.cy, self.r, self.start);
        let (x1, y1) = polar(self.cx, self.cy, self.r, self.start + span);
        let large = if span > 0.5 { 1 } else { 0 };
        write!(
            f,
            "M {} {} A {r} {r} 0 {large} 1 {} {}",
            fmt_num(x0),
            fmt_num(y0),
            fmt_num(x1),
            fmt_num(y1),
            r = fmt_num(self.r)
        )
    }
}

/// Stroked arc path from `start` to `end` (turn fractions, clockwise). Callers ensure
/// `0 <= start < end <= start + 1`; a full turn is split into two half arcs because a
/// single SVG arc command cannot describe a closed circle.
#[inline]
fn arc_path(cx: f32, cy: f32, r: f32, start: f32, end: f32) -> ArcPath {
    ArcPath {
        cx,
        cy,
        r,
        start,
        end,
    }
}

// ============================================================================
// Ring gauge (KPI)
// ============================================================================

/// Circular progress ring. `fraction` is clamped to `0..=1`; zero renders only the track.
pub fn ring_gauge(size: f32, stroke: f32, fraction: f32, track: Paint, fill: Paint) -> String {
    let size = size.max(8.0);
    let stroke = stroke.clamp(1.0, size / 3.0);
    let c = size / 2.0;
    let r = c - stroke / 2.0 - 0.5;
    let fraction = clamp01(fraction);
    let mut s = svg_open_with_capacity(size, size, 384);
    let _ = write!(
        s,
        r#"<circle cx="{c}" cy="{c}" r="{r}" fill="none" {track} stroke-width="{w}"/>"#,
        c = fmt_num(c),
        r = fmt_num(r),
        track = track.stroke(),
        w = fmt_num(stroke)
    );
    if fraction > 0.0 {
        // Keep a visible stub for tiny non-zero values.
        let span = fraction.max(0.025);
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" {fill} stroke-width="{w}" stroke-linecap="round"/>"#,
            d = arc_path(c, c, r, 0.0, span),
            fill = fill.stroke(),
            w = fmt_num(stroke)
        );
    }
    s.push_str("</svg>");
    s
}

// ============================================================================
// Donut chart (app usage share)
// ============================================================================

/// Multi-segment donut. Segment values are relative weights; non-finite or negative weights
/// are ignored. `gap` is the blank between segments in turn fractions (e.g. `0.012`).
pub fn donut(size: f32, stroke: f32, segments: &[(f32, Paint)], track: Paint, gap: f32) -> String {
    let size = size.max(8.0);
    let stroke = stroke.clamp(1.0, size / 3.0);
    let c = size / 2.0;
    let r = c - stroke / 2.0 - 0.5;
    let mut s = svg_open_with_capacity(size, size, 256 + segments.len() * 128);
    let _ = write!(
        s,
        r#"<circle cx="{c}" cy="{c}" r="{r}" fill="none" {track} stroke-width="{w}"/>"#,
        c = fmt_num(c),
        r = fmt_num(r),
        track = track.stroke(),
        w = fmt_num(stroke)
    );
    let mut valid_len = 0usize;
    let mut total = 0.0f32;
    for &(v, _) in segments {
        if v.is_finite() && v > 0.0 {
            valid_len += 1;
            total += v;
        }
    }
    if total > 0.0 {
        let gap = if valid_len > 1 {
            gap.clamp(0.0, 0.05)
        } else {
            0.0
        };
        let mut cursor = 0.0f32;
        for &(value, paint) in segments {
            if !value.is_finite() || value <= 0.0 {
                continue;
            }
            let span = value / total;
            let start = cursor + gap / 2.0;
            let end = (cursor + span - gap / 2.0).max(start + 0.004);
            let _ = write!(
                s,
                r#"<path d="{d}" fill="none" {p} stroke-width="{w}" stroke-linecap="butt"/>"#,
                d = arc_path(c, c, r, start, end),
                p = paint.stroke(),
                w = fmt_num(stroke)
            );
            cursor += span;
        }
    }
    s.push_str("</svg>");
    s
}

// ============================================================================
// 24-hour day dial (calendar arcs + alarms + now hand)
// ============================================================================

#[derive(Clone, Debug, PartialEq)]
pub struct DialEvent {
    /// Minutes since local midnight.
    pub start_min: u32,
    pub end_min: u32,
    pub paint: Paint,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DialAlarm {
    pub minute_of_day: u32,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DayDial {
    pub size: f32,
    pub now_min: u32,
    pub events: Vec<DialEvent>,
    pub alarms: Vec<DialAlarm>,
    /// Ring track color.
    pub track: Paint,
    /// Night band (23:00 → 07:00) tint drawn on the track.
    pub night: Paint,
    /// Elapsed-day progress arc (inner).
    pub elapsed: Paint,
    pub tick: Paint,
    pub hand: Paint,
    pub alarm: Paint,
    pub face: Paint,
    /// Opaque surface behind the chart; every paint is flattened against it.
    pub matte: Paint,
}

const DAY_MIN: f32 = 1440.0;

/// Normalized `[start, end)` minutes for an event (inverted / empty → 30 min).
#[inline]
fn event_range(ev: &DialEvent) -> (u32, u32) {
    let start = ev.start_min.min(1439);
    let mut end = ev.end_min.min(1440);
    if end <= start {
        end = (start + 30).min(1440);
    }
    (start, end)
}

/// Lane assignment for overlapping events: `(lane_index, lanes_in_cluster)` per event, in
/// input order. Overlapping events (e.g. two 15:00 meetings) share the slot side by side
/// instead of hiding each other. At most 3 lanes; extra events reuse the last lane.
pub fn event_lanes(events: &[DialEvent]) -> Vec<(usize, usize)> {
    if events.is_empty() {
        return Vec::new();
    }
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by_key(|&i| event_range(&events[i]));
    let mut out = vec![(0usize, 1usize); events.len()];
    let mut cluster: Vec<usize> = Vec::with_capacity(events.len().min(8));
    let mut cluster_end = 0u32;
    let flush = |cluster: &mut Vec<usize>, out: &mut Vec<(usize, usize)>| {
        let lanes = cluster.len().clamp(1, 3);
        for (k, &i) in cluster.iter().enumerate() {
            out[i] = (k.min(lanes - 1), lanes);
        }
        cluster.clear();
    };
    for i in order {
        let (s, e) = event_range(&events[i]);
        if !cluster.is_empty() && s >= cluster_end {
            flush(&mut cluster, &mut out);
        }
        if cluster.is_empty() {
            cluster_end = e;
        } else {
            cluster_end = cluster_end.max(e);
        }
        cluster.push(i);
    }
    flush(&mut cluster, &mut out);
    out
}

#[inline]
fn day_turn(min: u32) -> f32 {
    (min.min(1440) as f32) / DAY_MIN
}

pub fn day_dial(spec: &DayDial) -> String {
    let m = spec.matte;
    let track = spec.track.over(m);
    let night = spec.night.over(m);
    let elapsed = spec.elapsed.over(m);
    let tick = spec.tick.over(m);
    let hand = spec.hand.over(m);
    let alarm_paint = spec.alarm.over(m);
    let face = spec.face.over(m);
    let minor_tick = tick.with_alpha(0.55).over(face);
    let disabled_alarm = alarm_paint.with_alpha(0.55).over(m);

    let size = spec.size.max(48.0);
    let c = size / 2.0;
    let ring_w = (size * 0.075).max(6.0);
    let alarm_r = c - 4.0;
    let ring_r = alarm_r - ring_w / 2.0 - 5.0;
    let inner_r = ring_r - ring_w / 2.0 - 6.0;
    let mut s = svg_open_with_capacity(
        size,
        size,
        2800 + (spec.events.len() + spec.alarms.len()) * 140,
    );

    // Face.
    let _ = write!(
        s,
        r#"<circle cx="{c}" cy="{c}" r="{r}" {f}/>"#,
        c = fmt_num(c),
        r = fmt_num(inner_r),
        f = face.fill()
    );
    // Track.
    let _ = write!(
        s,
        r#"<circle cx="{c}" cy="{c}" r="{r}" fill="none" {t} stroke-width="{w}"/>"#,
        c = fmt_num(c),
        r = fmt_num(ring_r),
        t = track.stroke(),
        w = fmt_num(ring_w)
    );
    // Night band 23:00 → 24:00 and 00:00 → 07:00.
    for (a, b) in [(23 * 60, 1440), (0, 7 * 60)] {
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" {p} stroke-width="{w}"/>"#,
            d = arc_path(c, c, ring_r, day_turn(a), day_turn(b)),
            p = night.stroke(),
            w = fmt_num(ring_w)
        );
    }
    // Hour ticks inside the ring.
    for hour in 0..24u32 {
        let major = hour % 6 == 0;
        let len = if major { ring_w * 0.9 } else { ring_w * 0.45 };
        let turn = hour as f32 / 24.0;
        let (x0, y0) = polar(c, c, inner_r - 2.0, turn);
        let (x1, y1) = polar(c, c, inner_r - 2.0 - len, turn);
        let _ = write!(
            s,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {} stroke-width="{}" stroke-linecap="round"/>"#,
            fmt_num(x0),
            fmt_num(y0),
            fmt_num(x1),
            fmt_num(y1),
            (if major { tick } else { minor_tick }).stroke(),
            fmt_num(if major { 1.6 } else { 1.0 })
        );
    }
    // Calendar event arcs (overlapping events split the ring width into lanes).
    let lanes = event_lanes(&spec.events);
    for (ev, (lane, count)) in spec.events.iter().zip(lanes) {
        let (start, end) = event_range(ev);
        // At least ~12 minutes so short events stay visible.
        let end = end.max((start + 12).min(1440));
        let lane_w = ring_w / count as f32;
        let r = ring_r + ring_w / 2.0 - lane_w * (lane as f32 + 0.5);
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" {p} stroke-width="{w}" stroke-linecap="{cap}"/>"#,
            d = arc_path(c, c, r, day_turn(start), day_turn(end)),
            p = ev.paint.over(m).stroke(),
            w = fmt_num(if count > 1 { lane_w - 0.8 } else { ring_w }),
            cap = if count > 1 { "butt" } else { "round" }
        );
    }
    // Elapsed progress (thin inner arc) + now hand.
    let now = spec.now_min.min(1439);
    if now > 0 {
        let _ = write!(
            s,
            r#"<path d="{d}" fill="none" {p} stroke-width="2" stroke-linecap="round"/>"#,
            d = arc_path(c, c, inner_r - ring_w - 6.0, 0.0, day_turn(now)),
            p = elapsed.stroke()
        );
    }
    let (hx0, hy0) = polar(c, c, inner_r * 0.62, day_turn(now));
    let (hx1, hy1) = polar(c, c, ring_r + ring_w / 2.0 + 1.0, day_turn(now));
    let _ = write!(
        s,
        r#"<line x1="{}" y1="{}" x2="{}" y2="{}" {} stroke-width="2.4" stroke-linecap="round"/>"#,
        fmt_num(hx0),
        fmt_num(hy0),
        fmt_num(hx1),
        fmt_num(hy1),
        hand.stroke()
    );
    let (nx, ny) = polar(c, c, ring_r, day_turn(now));
    let _ = write!(
        s,
        r#"<circle cx="{x}" cy="{y}" r="{halo}" {halo_p}/><circle cx="{x}" cy="{y}" r="{dot}" {p}/>"#,
        x = fmt_num(nx),
        y = fmt_num(ny),
        halo = fmt_num(ring_w * 0.95),
        halo_p = hand.with_alpha(0.22).over(track).fill(),
        dot = fmt_num(ring_w * 0.5),
        p = hand.fill()
    );
    // Alarm markers on the outer orbit.
    for alarm in &spec.alarms {
        let (ax, ay) = polar(c, c, alarm_r, day_turn(alarm.minute_of_day % 1440));
        if alarm.enabled {
            let _ = write!(
                s,
                r#"<circle cx="{}" cy="{}" r="3.2" {}/>"#,
                fmt_num(ax),
                fmt_num(ay),
                alarm_paint.fill()
            );
        } else {
            let _ = write!(
                s,
                r#"<circle cx="{}" cy="{}" r="2.6" fill="none" {} stroke-width="1.2"/>"#,
                fmt_num(ax),
                fmt_num(ay),
                disabled_alarm.stroke()
            );
        }
    }
    s.push_str("</svg>");
    s
}

// ============================================================================
// Horizontal day timeline (stretch-friendly: rectangles and lines only)
// ============================================================================

#[derive(Clone, Debug, PartialEq)]
pub struct DayTimeline {
    /// Design width; the UI stretches the raster to the available width.
    pub width: f32,
    pub height: f32,
    pub start_hour: u32,
    pub end_hour: u32,
    pub now_min: u32,
    pub events: Vec<DialEvent>,
    pub alarms: Vec<DialAlarm>,
    pub track: Paint,
    pub elapsed: Paint,
    pub tick: Paint,
    pub now: Paint,
    pub alarm: Paint,
    /// Opaque surface behind the chart; every paint is flattened against it.
    pub matte: Paint,
}

impl DayTimeline {
    #[inline]
    fn span(&self) -> (u32, u32) {
        let start = self.start_hour.min(23) * 60;
        let end = (self.end_hour.clamp(self.start_hour.min(23) + 1, 24)) * 60;
        (start, end)
    }

    /// X coordinate for a minute-of-day, clamped into the visible window.
    #[inline]
    pub fn x_for(&self, minute: u32) -> f32 {
        let (start, end) = self.span();
        let m = minute.clamp(start, end);
        let pad = 6.0;
        pad + (m - start) as f32 / (end - start) as f32 * (self.width - pad * 2.0)
    }
}

pub fn day_timeline(spec: &DayTimeline) -> String {
    let w = spec.width.max(120.0);
    let h = spec.height.max(24.0);
    let m = spec.matte;
    let track = spec.track.over(m);
    let elapsed = spec.elapsed.over(m);
    let tick = spec.tick.over(m);
    let minor_tick = tick.with_alpha(0.5).over(m);
    let now = spec.now.over(m);
    let alarm_paint = spec.alarm.over(m);
    let disabled_alarm = alarm_paint.with_alpha(0.4).over(m);

    let (start, end) = spec.span();
    let span_f = (end - start) as f32;
    let usable_w = w - 12.0;
    let x_for = |minute: u32| -> f32 {
        let m_clamped = minute.clamp(start, end);
        6.0 + (m_clamped - start) as f32 / span_f * usable_w
    };
    let track_h = (h * 0.34).max(8.0);
    let track_y = h * 0.42 - track_h / 2.0;
    let mut s = svg_open_with_capacity(
        w,
        h,
        2400 + (spec.events.len() + spec.alarms.len()) * 120,
    );
    // Track.
    let x_start = x_for(start);
    let x_end = x_for(end);
    let _ = write!(
        s,
        r#"<rect x="{x}" y="{y}" width="{tw}" height="{th}" rx="{rx}" {p}/>"#,
        x = fmt_num(x_start),
        y = fmt_num(track_y),
        tw = fmt_num(x_end - x_start),
        th = fmt_num(track_h),
        rx = fmt_num(track_h / 2.0),
        p = track.fill()
    );
    // Elapsed part of the day.
    if spec.now_min > start {
        let _ = write!(
            s,
            r#"<rect x="{x}" y="{y}" width="{tw}" height="{th}" rx="{rx}" {p}/>"#,
            x = fmt_num(x_start),
            y = fmt_num(track_y),
            tw = fmt_num(x_for(spec.now_min) - x_start),
            th = fmt_num(track_h),
            rx = fmt_num(track_h / 2.0),
            p = elapsed.fill()
        );
    }
    // Hour ticks below the track.
    let mut hour = start / 60;
    while hour * 60 <= end {
        let x = x_for(hour * 60);
        let major = hour % 3 == 0;
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="1" height="{}" {}/>"#,
            fmt_num(x - 0.5),
            fmt_num(track_y + track_h + 3.0),
            fmt_num(if major { 7.0 } else { 4.0 }),
            (if major { tick } else { minor_tick }).fill()
        );
        hour += 1;
    }
    // Event blocks (overlapping events stack into lanes within the block height).
    let lanes = event_lanes(&spec.events);
    for (ev, (lane, count)) in spec.events.iter().zip(lanes) {
        let (s_min, e_min) = event_range(ev);
        if e_min <= start || s_min >= end {
            continue;
        }
        let x0 = x_for(s_min);
        let x1 = x_for(e_min).max(x0 + 6.0);
        let full_h = track_h + 6.0;
        let lane_h = full_h / count as f32;
        let _ = write!(
            s,
            r#"<rect x="{x}" y="{y}" width="{bw}" height="{bh}" rx="{rx}" {p}/>"#,
            x = fmt_num(x0),
            y = fmt_num(track_y - 3.0 + lane_h * lane as f32),
            bw = fmt_num(x1 - x0),
            bh = fmt_num(if count > 1 { lane_h - 1.0 } else { full_h }),
            rx = fmt_num(if count > 1 { 2.0 } else { 3.0 }),
            p = ev.paint.over(m).fill()
        );
    }
    // Alarm pins above the track.
    for alarm in &spec.alarms {
        let minute = alarm.minute_of_day % 1440;
        if minute < start || minute > end {
            continue;
        }
        let x = x_for(minute);
        let paint = if alarm.enabled {
            alarm_paint
        } else {
            disabled_alarm
        };
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="3" height="{}" rx="1.5" {}/>"#,
            fmt_num(x - 1.5),
            fmt_num((track_y - 11.0).max(0.0)),
            fmt_num(7.0),
            paint.fill()
        );
    }
    // Now marker.
    if (start..=end).contains(&spec.now_min) {
        let x = x_for(spec.now_min);
        let _ = write!(
            s,
            r#"<rect x="{}" y="1" width="2.5" height="{}" rx="1.25" {}/>"#,
            fmt_num(x - 1.25),
            fmt_num(track_y + track_h + 4.0),
            now.fill()
        );
        let _ = write!(
            s,
            r#"<rect x="{}" y="0" width="9" height="5" rx="2.5" {}/>"#,
            fmt_num(x - 4.5),
            now.fill()
        );
    }
    s.push_str("</svg>");
    s
}

// ============================================================================
// Parcel progress classification
// ============================================================================

/// Maps a free-form parcel status onto the 4-node logistics stepper
/// 揽收 → 运输 → 到站/派送 → 签收, returning `(completed_nodes, total_nodes)`.
/// Order matters: "已到达【杭州转运中心】" is still in transit and "菜鸟驿站已揽收" is only
/// picked up, so transit and pickup keywords are checked before station keywords.
pub fn parcel_progress(status: &str) -> (usize, usize) {
    let s = status;
    let any = |keys: &[&str]| keys.iter().any(|k| s.contains(k));
    let done = if any(&["已签收", "已取件", "已取出", "已完成", "签收成功"]) {
        4
    } else if any(&["转运", "中转", "分拨", "发往", "在途", "运输", "干线"]) {
        1
    } else if any(&["揽收", "揽件", "已发货", "待揽"]) {
        0
    } else if any(&["待取", "到站", "驿站", "快递柜", "丰巢", "派送", "派件", "已到"]) {
        2
    } else {
        0
    };
    (done, 4)
}

/// Horizontal multi-node logistics progress stepper (`揽收 → 运输 → 到站/派送 → 签收`).
///
/// Nodes are placed at the centres of `total_steps` equal columns (`(i + 0.5) / total_steps`)
/// so a row of `Expanded(1.0)` centred labels underneath aligns at any card width.
/// Semi-transparent halo and track paints are pre-composited over `matte`.
pub fn logistics_stepper(
    width: f32,
    height: f32,
    done_steps: usize,
    total_steps: usize,
    done: Paint,
    active: Paint,
    pending: Paint,
    matte: Paint,
) -> String {
    let w = if width.is_finite() { width.max(60.0) } else { 60.0 };
    let h = if height.is_finite() { height.max(12.0) } else { 12.0 };
    let n = total_steps.max(1);
    let cy = h * 0.5;
    let r = (h * 0.28).clamp(3.5, 5.5);
    let halo_r = (r + 3.0).min(h * 0.48);
    let track_h = 3.0;

    let m = matte.with_alpha(1.0);
    let done_c = done.over(m);
    let active_c = active.over(m);
    let active_halo = active.with_alpha(0.24).over(m);
    let pending_stroke = pending.over(m);
    let track_c = pending.with_alpha(0.35).over(m);

    let mut s = svg_open_with_capacity(w, h, 256 + n * 220);
    for i in 0..n.saturating_sub(1) {
        let x0 = (i as f32 + 0.5) / n as f32 * w;
        let x1 = (i as f32 + 1.5) / n as f32 * w;
        let seg = if i < done_steps { done_c } else { track_c };
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1.5" {}/>"#,
            fmt_num(x0),
            fmt_num(cy - track_h * 0.5),
            fmt_num((x1 - x0).max(0.0)),
            fmt_num(track_h),
            seg.fill()
        );
    }
    for i in 0..n {
        let cx = (i as f32 + 0.5) / n as f32 * w;
        if i < done_steps {
            let _ = write!(
                s,
                r#"<circle cx="{cx}" cy="{cy}" r="{r}" {fill}/><circle cx="{cx}" cy="{cy}" r="{ir}" {inner}/>"#,
                cx = fmt_num(cx),
                cy = fmt_num(cy),
                r = fmt_num(r),
                ir = fmt_num(r * 0.42),
                fill = done_c.fill(),
                inner = m.fill()
            );
        } else if i == done_steps {
            let _ = write!(
                s,
                r#"<circle cx="{cx}" cy="{cy}" r="{hr}" {halo}/><circle cx="{cx}" cy="{cy}" r="{r}" {fill}/><circle cx="{cx}" cy="{cy}" r="{ir}" {inner}/>"#,
                cx = fmt_num(cx),
                cy = fmt_num(cy),
                hr = fmt_num(halo_r),
                r = fmt_num(r),
                ir = fmt_num(r * 0.38),
                halo = active_halo.fill(),
                fill = active_c.fill(),
                inner = m.fill()
            );
        } else {
            let _ = write!(
                s,
                r#"<circle cx="{}" cy="{}" r="{}" {} {} stroke-width="1.5"/>"#,
                fmt_num(cx),
                fmt_num(cy),
                fmt_num((r - 0.75).max(2.0)),
                m.fill(),
                pending_stroke.stroke()
            );
        }
    }
    s.push_str("</svg>");
    s
}

/// Horizontal multi-segment proportional spectrum capsule bar.
///
/// Used to visualize evidence source distribution (Notification / Calendar / Clipboard / Media /
/// Voice / Usage), safety audit breakdowns, or storage composition without SVG `<text>`.
/// All paints are pre-composited over `matte` so the SVG contains only opaque fills.
pub fn source_spectrum_bar(
    width: f32,
    height: f32,
    segments: &[(f32, Paint)],
    track: Paint,
    matte: Paint,
) -> String {
    let w = if width.is_finite() { width.max(48.0) } else { 48.0 };
    let h = if height.is_finite() { height.max(8.0) } else { 8.0 };
    let m = matte.with_alpha(1.0);
    let track_c = track.over(m);
    let bar_h = (h * 0.65).clamp(6.0, h);
    let bar_y = (h - bar_h) * 0.5;
    let rx = bar_h * 0.5;

    let mut s = svg_open_with_capacity(w, h, 256 + segments.len() * 110);
    let _ = write!(
        s,
        r#"<rect x="0" y="{}" width="{}" height="{}" rx="{}" {}/>"#,
        fmt_num(bar_y),
        fmt_num(w),
        fmt_num(bar_h),
        fmt_num(rx),
        track_c.fill()
    );

    let mut n = 0usize;
    let mut total = 0.0f32;
    for &(v, _) in segments {
        if v.is_finite() && v > 0.0 {
            n += 1;
            total += v;
        }
    }
    if total <= 0.0 || !total.is_finite() {
        s.push_str("</svg>");
        return s;
    }

    let gap = if n > 1 && w > (n as f32) * 8.0 { 2.5 } else { 0.0 };
    let usable_w = (w - gap * (n.saturating_sub(1) as f32)).max(1.0);
    let mut cursor_x = 0.0_f32;
    let mut idx = 0usize;
    for &(val, paint) in segments {
        if !val.is_finite() || val <= 0.0 {
            continue;
        }
        let seg_w = if idx + 1 == n {
            (w - cursor_x).max(2.0)
        } else {
            ((val / total) * usable_w).max(2.0)
        };
        let fill = paint.over(m);
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" {}/>"#,
            fmt_num(cursor_x),
            fmt_num(bar_y),
            fmt_num(seg_w.min(w - cursor_x)),
            fmt_num(bar_h),
            fmt_num(rx.min(seg_w * 0.5)),
            fill.fill()
        );
        cursor_x = (cursor_x + seg_w + gap).min(w);
        idx += 1;
    }

    s.push_str("</svg>");
    s
}

/// Horizontal multi-stage pipeline stepper with checkmarks, active pulse halo, and error cross.
///
/// Nodes are placed at `(i + 0.5) / total_stages * width` so a row of equal-width labels
/// beneath aligns cleanly. Semi-transparent paints are flattened over `matte`.
#[allow(clippy::too_many_arguments)]
pub fn pipeline_flow_stepper(
    width: f32,
    height: f32,
    active_stage: usize,
    total_stages: usize,
    failed: bool,
    done: Paint,
    active: Paint,
    danger: Paint,
    pending: Paint,
    matte: Paint,
) -> String {
    let w = if width.is_finite() { width.max(80.0) } else { 80.0 };
    let h = if height.is_finite() { height.max(16.0) } else { 16.0 };
    let n = total_stages.max(1);
    let cy = h * 0.5;
    let r = (h * 0.32).clamp(4.5, 7.5);
    let halo_r = (r + 3.5).min(h * 0.48);
    let track_h = 3.0;

    let m = matte.with_alpha(1.0);
    let done_c = done.over(m);
    let focus_base = if failed { danger } else { active };
    let active_c = focus_base.over(m);
    let active_halo = focus_base.with_alpha(0.24).over(m);
    let pending_stroke = pending.over(m);
    let track_c = pending.with_alpha(0.32).over(m);

    let mut s = svg_open_with_capacity(w, h, 256 + n * 240);
    for i in 0..n.saturating_sub(1) {
        let x0 = (i as f32 + 0.5) / n as f32 * w;
        let x1 = (i as f32 + 1.5) / n as f32 * w;
        let seg = if i < active_stage { done_c } else { track_c };
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="1.5" {}/>"#,
            fmt_num(x0),
            fmt_num(cy - track_h * 0.5),
            fmt_num((x1 - x0).max(0.0)),
            fmt_num(track_h),
            seg.fill()
        );
    }

    for i in 0..n {
        let cx = (i as f32 + 0.5) / n as f32 * w;
        if i < active_stage {
            let _ = write!(
                s,
                r#"<circle cx="{cx}" cy="{cy}" r="{r}" {fill}/><path d="M {x1} {y1} L {x2} {y2} L {x3} {y3}" fill="none" {stroke} stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
                cx = fmt_num(cx),
                cy = fmt_num(cy),
                r = fmt_num(r),
                fill = done_c.fill(),
                x1 = fmt_num(cx - r * 0.42),
                y1 = fmt_num(cy + r * 0.02),
                x2 = fmt_num(cx - r * 0.10),
                y2 = fmt_num(cy + r * 0.34),
                x3 = fmt_num(cx + r * 0.44),
                y3 = fmt_num(cy - r * 0.28),
                stroke = m.stroke(),
            );
        } else if i == active_stage {
            if failed {
                let d = r * 0.36;
                let _ = write!(
                    s,
                    r#"<circle cx="{cx}" cy="{cy}" r="{hr}" {halo}/><circle cx="{cx}" cy="{cy}" r="{r}" {fill}/><path d="M {x0} {y0} L {x1} {y1} M {x1} {y0} L {x0} {y1}" fill="none" {stroke} stroke-width="1.6" stroke-linecap="round"/>"#,
                    cx = fmt_num(cx),
                    cy = fmt_num(cy),
                    hr = fmt_num(halo_r),
                    r = fmt_num(r),
                    halo = active_halo.fill(),
                    fill = active_c.fill(),
                    x0 = fmt_num(cx - d),
                    y0 = fmt_num(cy - d),
                    x1 = fmt_num(cx + d),
                    y1 = fmt_num(cy + d),
                    stroke = m.stroke(),
                );
            } else {
                let _ = write!(
                    s,
                    r#"<circle cx="{cx}" cy="{cy}" r="{hr}" {halo}/><circle cx="{cx}" cy="{cy}" r="{r}" {fill}/><circle cx="{cx}" cy="{cy}" r="{ir}" {inner}/>"#,
                    cx = fmt_num(cx),
                    cy = fmt_num(cy),
                    hr = fmt_num(halo_r),
                    r = fmt_num(r),
                    ir = fmt_num(r * 0.38),
                    halo = active_halo.fill(),
                    fill = active_c.fill(),
                    inner = m.fill()
                );
            }
        } else {
            let _ = write!(
                s,
                r#"<circle cx="{}" cy="{}" r="{}" {} {} stroke-width="1.5"/>"#,
                fmt_num(cx),
                fmt_num(cy),
                fmt_num((r - 0.75).max(2.5)),
                m.fill(),
                pending_stroke.stroke()
            );
        }
    }
    s.push_str("</svg>");
    s
}

/// Minutes since midnight for a local timestamp in milliseconds; `None` when invalid.
pub fn minute_of_day_from_millis(ms: i64) -> Option<u32> {
    use chrono::{Local, TimeZone, Timelike};
    let dt = Local.timestamp_millis_opt(ms).single()?;
    Some(dt.hour() * 60 + dt.minute())
}

// ============================================================================
// Weekly 7-Day Activity Stacked Bar Chart (Work + Life + Issues)
// ============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WeeklyBarDay {
    pub work_count: u32,
    pub life_count: u32,
    pub issue_count: u32,
    pub selected: bool,
    pub today: bool,
}

/// Renders a 7-column stacked bar chart for the weekly summary on `Page::Home`.
/// Pure SVG without `<text>` nodes; all fills are flattened against `matte`.
pub fn weekly_activity_bars(
    width: f32,
    height: f32,
    days: &[WeeklyBarDay],
    work_paint: Paint,
    life_paint: Paint,
    issue_paint: Paint,
    track_paint: Paint,
    selected_paint: Paint,
    matte: Paint,
) -> String {
    let w = if width.is_finite() {
        width.max(84.0)
    } else {
        280.0
    };
    let h = if height.is_finite() {
        height.max(24.0)
    } else {
        52.0
    };
    let m = matte.with_alpha(1.0);
    let work_c = work_paint.over(m);
    let life_c = life_paint.over(m);
    let issue_c = issue_paint.over(m);
    let track_c = track_paint.over(m);
    let sel_c = selected_paint.over(m);

    let count = days.len().clamp(1, 14);
    let mut s = svg_open_with_capacity(w, h, 256 + count * 320);
    let col_w = w / count as f32;
    let bar_w = (col_w * 0.52).clamp(6.0, 28.0);
    let top_pad = 3.0f32;
    let bot_pad = 6.0f32;
    let usable_h = (h - top_pad - bot_pad).max(10.0);

    let max_total = days
        .iter()
        .take(count)
        .map(|d| d.work_count.saturating_add(d.life_count).saturating_add(d.issue_count))
        .max()
        .unwrap_or(0)
        .max(4) as f32;

    for i in 0..count {
        let day = days.get(i).copied().unwrap_or(WeeklyBarDay {
            work_count: 0,
            life_count: 0,
            issue_count: 0,
            selected: false,
            today: false,
        });
        let cx = (i as f32 + 0.5) * col_w;
        let bx = cx - bar_w * 0.5;

        // Track background pillar
        let _ = write!(
            s,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" {}/>"#,
            fmt_num(bx),
            fmt_num(top_pad),
            fmt_num(bar_w),
            fmt_num(usable_h),
            track_c.fill()
        );

        let total = day
            .work_count
            .saturating_add(day.life_count)
            .saturating_add(day.issue_count);
        if total > 0 {
            let total_h = ((total as f32 / max_total).clamp(0.14, 1.0)) * usable_h;
            let mut cursor_bottom = top_pad + usable_h;
            let segments = [
                (day.work_count, work_c),
                (day.life_count, life_c),
                (day.issue_count, issue_c),
            ];
            for (val, seg_paint) in segments {
                if val == 0 {
                    continue;
                }
                let seg_h = (total_h * (val as f32 / total as f32)).max(2.5);
                let y = (cursor_bottom - seg_h).max(top_pad);
                let actual_h = (cursor_bottom - y).max(1.5);
                let _ = write!(
                    s,
                    r#"<rect x="{}" y="{}" width="{}" height="{}" rx="2" {}/>"#,
                    fmt_num(bx),
                    fmt_num(y),
                    fmt_num(bar_w),
                    fmt_num(actual_h),
                    seg_paint.fill()
                );
                cursor_bottom = y;
            }
        }

        if day.selected || day.today {
            let ind_w = if day.selected {
                (bar_w * 0.95).max(8.0)
            } else {
                (bar_w * 0.5).max(4.0)
            };
            let ind_paint = if day.selected { sel_c } else { work_c };
            let _ = write!(
                s,
                r#"<rect x="{}" y="{}" width="{}" height="2.5" rx="1.2" {}/>"#,
                fmt_num(cx - ind_w * 0.5),
                fmt_num(h - 3.0),
                fmt_num(ind_w),
                ind_paint.fill()
            );
        }
    }

    s.push_str("</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paint() -> Paint {
        Paint::rgb(0x3B, 0x82, 0xF6)
    }

    fn assert_well_formed(svg: &str) {
        assert!(svg.starts_with("<svg"), "must start with <svg: {svg}");
        assert!(svg.ends_with("</svg>"));
        assert!(!svg.contains("NaN") && !svg.contains("inf"), "{svg}");
        assert!(!svg.contains("<text"), "labels are native, never SVG text");
        let mut reader = quick_xml::Reader::from_str(svg);
        let mut depth = 0i32;
        loop {
            match reader.read_event() {
                Ok(quick_xml::events::Event::Start(_)) => depth += 1,
                Ok(quick_xml::events::Event::End(_)) => depth -= 1,
                Ok(quick_xml::events::Event::Eof) => break,
                Ok(_) => {}
                Err(e) => panic!("malformed svg: {e}: {svg}"),
            }
        }
        assert_eq!(depth, 0, "unbalanced svg: {svg}");
    }

    #[test]
    fn number_formatting_is_compact_and_never_negative_zero() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(1.5), "1.5");
        assert_eq!(fmt_num(-0.0001), "0");
        assert_eq!(fmt_num(f32::NAN), "0");
        assert_eq!(fmt_num(2.345), "2.35");
    }

    #[test]
    fn ring_gauge_handles_boundaries() {
        for f in [0.0, 0.001, 0.5, 0.75, 1.0, 7.0, -3.0, f32::NAN] {
            let svg = ring_gauge(64.0, 7.0, f, paint().with_alpha(0.2), paint());
            assert_well_formed(&svg);
        }
        // Zero → track only; full → two half-arcs closed circle.
        assert!(!ring_gauge(64.0, 7.0, 0.0, paint(), paint()).contains("<path"));
        let full = ring_gauge(64.0, 7.0, 1.0, paint(), paint());
        assert_eq!(full.matches(" A ").count(), 2);
    }

    #[test]
    fn donut_ignores_invalid_segments_and_handles_empty() {
        assert_well_formed(&donut(96.0, 14.0, &[], paint(), 0.01));
        let svg = donut(
            96.0,
            14.0,
            &[
                (68.0, paint()),
                (-5.0, paint()),
                (f32::NAN, paint()),
                (32.0, Paint::rgb(1, 2, 3)),
            ],
            paint().with_alpha(0.1),
            0.01,
        );
        assert_well_formed(&svg);
        assert_eq!(svg.matches("<path").count(), 2);
        let single = donut(96.0, 14.0, &[(10.0, paint())], paint(), 0.02);
        assert_eq!(single.matches(" A ").count(), 2, "single segment is a full ring");
    }

    fn dial(now: u32, events: Vec<DialEvent>, alarms: Vec<DialAlarm>) -> DayDial {
        DayDial {
            size: 188.0,
            now_min: now,
            events,
            alarms,
            track: paint().with_alpha(0.1),
            night: paint().with_alpha(0.2),
            elapsed: paint().with_alpha(0.5),
            tick: paint(),
            hand: paint(),
            alarm: paint(),
            face: paint().with_alpha(0.04),
            matte: Paint::rgb(255, 255, 255),
        }
    }

    #[test]
    fn day_dial_renders_events_alarms_and_edge_times() {
        let events = vec![
            DialEvent {
                start_min: 9 * 60,
                end_min: 10 * 60,
                paint: paint(),
            },
            // Inverted / zero-length / beyond-midnight events are normalized.
            DialEvent {
                start_min: 15 * 60,
                end_min: 14 * 60,
                paint: paint(),
            },
            DialEvent {
                start_min: 1439,
                end_min: 5000,
                paint: paint(),
            },
        ];
        let alarms = vec![
            DialAlarm {
                minute_of_day: 7 * 60,
                enabled: true,
            },
            DialAlarm {
                minute_of_day: 3000,
                enabled: false,
            },
        ];
        for now in [0, 1, 720, 1439, 99999] {
            let svg = day_dial(&dial(now, events.clone(), alarms.clone()));
            assert_well_formed(&svg);
        }
        let svg = day_dial(&dial(600, events, alarms));
        // 2 night arcs + 3 events + elapsed arc.
        assert_eq!(svg.matches("<path").count(), 6);
        assert_well_formed(&day_dial(&dial(600, vec![], vec![])));
    }

    #[test]
    fn day_timeline_clips_events_outside_window() {
        let spec = DayTimeline {
            width: 960.0,
            height: 54.0,
            start_hour: 7,
            end_hour: 23,
            now_min: 18 * 60 + 13,
            events: vec![
                DialEvent {
                    start_min: 19 * 60,
                    end_min: 20 * 60,
                    paint: paint(),
                },
                DialEvent {
                    start_min: 2 * 60,
                    end_min: 3 * 60,
                    paint: paint(),
                },
            ],
            alarms: vec![
                DialAlarm {
                    minute_of_day: 8 * 60,
                    enabled: true,
                },
                DialAlarm {
                    minute_of_day: 60,
                    enabled: true,
                },
            ],
            track: paint().with_alpha(0.1),
            elapsed: paint().with_alpha(0.3),
            tick: paint(),
            now: paint(),
            alarm: paint(),
            matte: Paint::rgb(255, 255, 255),
        };
        let svg = day_timeline(&spec);
        assert_well_formed(&svg);
        assert!(spec.x_for(0) <= spec.x_for(7 * 60) + 0.01);
        assert!((spec.x_for(23 * 60) - (960.0 - 6.0)).abs() < 0.01);
        // Degenerate windows never divide by zero.
        let degenerate = DayTimeline {
            start_hour: 23,
            end_hour: 2,
            ..spec.clone()
        };
        assert_well_formed(&day_timeline(&degenerate));
        assert!(degenerate.x_for(23 * 60 + 30).is_finite());
    }

    #[test]
    fn parcel_progress_classifies_real_statuses() {
        assert_eq!(parcel_progress("待取件 · 已到站"), (2, 4));
        assert_eq!(parcel_progress("已到达【杭州转运中心】"), (1, 4));
        assert_eq!(parcel_progress("菜鸟驿站已揽收"), (0, 4));
        assert_eq!(parcel_progress("派送中 · 预计今日送达"), (2, 4));
        assert_eq!(parcel_progress("运输中 · 发往杭州"), (1, 4));
        assert_eq!(parcel_progress("已签收"), (4, 4));
        assert_eq!(parcel_progress("已揽收"), (0, 4));
        assert_eq!(parcel_progress(""), (0, 4));
        for done in 0..=5 {
            let svg = logistics_stepper(
                400.0,
                18.0,
                done,
                4,
                Paint::rgb(0x16, 0xA3, 0x4A),
                paint(),
                paint().with_alpha(0.45),
                Paint::rgb(255, 255, 255),
            );
            assert_well_formed(&svg);
            assert!(!svg.contains(r#"opacity="0."#), "stepper must be flattened: {svg}");
        }
        assert_well_formed(&logistics_stepper(
            f32::NAN,
            0.0,
            1,
            0,
            paint(),
            paint(),
            paint(),
            Paint::rgb(255, 255, 255),
        ));
    }

    #[test]
    fn charts_emit_only_opaque_colours_after_flattening() {
        let svg = day_dial(&dial(600, vec![], vec![DialAlarm { minute_of_day: 60, enabled: false }]));
        assert!(!svg.contains(r#"opacity="0."#), "dial must be fully flattened: {svg}");
        let over = Paint::rgb(0, 0, 0).with_alpha(0.25).over(Paint::rgb(255, 255, 255));
        assert_eq!((over.r, over.g, over.b, over.alpha), (191, 191, 191, 1.0));
    }

    #[test]
    fn overlapping_events_get_separate_lanes() {
        let ev = |s: u32, e: u32| DialEvent {
            start_min: s,
            end_min: e,
            paint: paint(),
        };
        // Two 15:00 meetings overlap; 18:00 is alone; input order is preserved.
        let events = vec![ev(18 * 60, 19 * 60), ev(900, 960), ev(900, 930)];
        let lanes = event_lanes(&events);
        assert_eq!(lanes[0], (0, 1));
        assert_eq!((lanes[1].1, lanes[2].1), (2, 2));
        assert_ne!(lanes[1].0, lanes[2].0, "overlapping events must not share a lane");
        // Chain overlap forms one cluster, capped at 3 lanes.
        let chain = vec![ev(0, 100), ev(50, 150), ev(120, 200), ev(140, 220)];
        let lanes = event_lanes(&chain);
        assert!(lanes.iter().all(|(i, n)| *n == 3 && *i < 3));
        assert!(event_lanes(&[]).is_empty());
        // Back-to-back events do not overlap.
        assert_eq!(event_lanes(&[ev(60, 120), ev(120, 180)]), vec![(0, 1), (0, 1)]);
    }

    #[test]
    fn paint_attributes_clamp_alpha() {
        assert_eq!(
            Paint::rgb(255, 0, 16).with_alpha(3.0).fill_attr(),
            r##"fill="#FF0010" fill-opacity="1""##
        );
        assert_eq!(
            Paint::rgb(0, 0, 0).with_alpha(f32::NAN).stroke_attr(),
            r##"stroke="#000000" stroke-opacity="0""##
        );
    }

    #[test]
    fn source_spectrum_bar_handles_empty_invalid_and_multi_segment_inputs() {
        let matte = Paint::rgb(255, 255, 255);
        let track = paint().with_alpha(0.12);
        let empty = source_spectrum_bar(320.0, 14.0, &[], track, matte);
        assert_well_formed(&empty);
        assert!(!empty.contains(r#"opacity="0."#));

        let multi = source_spectrum_bar(
            480.0,
            14.0,
            &[
                (12.0, Paint::rgb(0x25, 0x63, 0xEB)),
                (0.0, paint()),
                (-4.0, paint()),
                (f32::NAN, paint()),
                (8.0, Paint::rgb(0x16, 0xA3, 0x4A)),
                (5.0, Paint::rgb(0xF5, 0x9E, 0x0B)),
            ],
            track,
            matte,
        );
        assert_well_formed(&multi);
        assert!(!multi.contains(r#"opacity="0."#));
        assert_eq!(multi.matches("<rect").count(), 4);
    }

    #[test]
    fn pipeline_flow_stepper_renders_completed_active_failed_and_pending_nodes() {
        let matte = Paint::rgb(18, 22, 31);
        for stage in 0..=7 {
            for failed in [false, true] {
                let svg = pipeline_flow_stepper(
                    520.0,
                    24.0,
                    stage,
                    6,
                    failed,
                    Paint::rgb(0x16, 0xA3, 0x4A),
                    Paint::rgb(0x25, 0x63, 0xEB),
                    Paint::rgb(0xDC, 0x26, 0x26),
                    Paint::rgb(0x64, 0x74, 0x8B).with_alpha(0.5),
                    matte,
                );
                assert_well_formed(&svg);
                assert!(!svg.contains(r#"opacity="0."#));
            }
        }
    }

    #[test]
    fn weekly_activity_bars_handles_empty_and_7_day_stacks() {
        let matte = Paint::rgb(255, 255, 255);
        let empty = weekly_activity_bars(
            280.0,
            48.0,
            &[],
            paint(),
            Paint::rgb(0x10, 0xB9, 0x81),
            Paint::rgb(0xF5, 0x9E, 0x0B),
            paint().with_alpha(0.14),
            Paint::rgb(0x63, 0x66, 0xF1),
            matte,
        );
        assert_well_formed(&empty);
        assert!(!empty.contains(r#"opacity="0."#));

        let days = vec![
            WeeklyBarDay {
                work_count: 3,
                life_count: 1,
                issue_count: 0,
                selected: false,
                today: false,
            },
            WeeklyBarDay {
                work_count: 0,
                life_count: 0,
                issue_count: 0,
                selected: false,
                today: false,
            },
            WeeklyBarDay {
                work_count: 2,
                life_count: 2,
                issue_count: 1,
                selected: true,
                today: true,
            },
        ];
        let populated = weekly_activity_bars(
            280.0,
            52.0,
            &days,
            paint(),
            Paint::rgb(0x10, 0xB9, 0x81),
            Paint::rgb(0xF5, 0x9E, 0x0B),
            paint().with_alpha(0.14),
            Paint::rgb(0x63, 0x66, 0xF1),
            matte,
        );
        assert_well_formed(&populated);
        assert!(!populated.contains(r#"opacity="0."#));
        assert!(populated.matches("<rect").count() >= 9);
    }
}
