#![allow(unused_imports)]
use crate::ui::{
    amoo_ui_state::AmooUiAction, backscreen_state::BackScreenAction, icons::UiIcon,
    led_grid::LedGrid, theme::*, widgets::*, work_state::WorkAction, Action, Companion, Page,
    PushAction, ToolPage, F_ALARM_TIME, F_AMOO, F_AMOO_GUI_APP, F_BACK_MARQUEE, F_CALENDAR_TIME,
    F_HOME_COMMAND, F_OV_TEXT, F_PHONE_SCHEDULE_TITLE, F_PUSH_REPLY, F_SMS_BODY, F_SMS_TO,
    F_WORK_PROMPT,
};
use super::*;
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, Timelike};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor, ViewContext};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stepx_companion::{
    device::default_express_parcels,
    home_hub::{
        build_home_hub_report, build_selected_date_panorama, build_weekly_overview_report,
        quick_replies_for_message_with_context, resolve_top_app_display_name, DesktopDateEntry,
        HomeAiIntent, HomeAiRecommendation, HomeCardKind, HomeHubInput, HomeLifeDomain,
        SelectedDateEntryKind,
    },
    home_visuals::{
        day_dial, donut, parcel_progress, ring_gauge, DayDial, DayTimeline, DialAlarm, DialEvent,
        Paint, WeeklyBarDay,
    },
    push::{demo_push_snapshot, ImChannel, PushSnapshot},
};

/// These charts use their actual layout width. Fixed-width SVG rasters stretched
/// to a parent column also stretch end caps, strokes and bar corner radii.
pub(in crate::ui) enum HomePlotContent {
    Timeline(DayTimeline),
    Week {
        days: Vec<WeeklyBarDay>,
        fills: [ColorU; 3],
        track: ColorU,
        selected: ColorU,
    },
}

#[derive(Clone, Copy, Debug)]
pub(in crate::ui) struct HomePlotRect {
    pub(in crate::ui) x: f32,
    pub(in crate::ui) y: f32,
    pub(in crate::ui) width: f32,
    pub(in crate::ui) height: f32,
    pub(in crate::ui) radius: f32,
    pub(in crate::ui) fill: ColorU,
}

impl HomePlotContent {
    pub(in crate::ui) fn rectangles(&self, size: Vector2F) -> Vec<HomePlotRect> {
        let mut rects = Vec::new();
        let (w, h) = (size.x(), size.y());
        if !w.is_finite() || !h.is_finite() || w < 1. || h < 1. {
            return rects;
        }
        let mut push = |x: f32, y: f32, width: f32, height: f32, radius: f32, fill: ColorU| {
            let (left, top) = (x.max(0.), y.max(0.));
            let (right, bottom) = ((x + width).min(w), (y + height).min(h));
            if right > left && bottom > top {
                rects.push(HomePlotRect {
                    x: left,
                    y: top,
                    width: right - left,
                    height: bottom - top,
                    radius: radius.min((right - left) / 2.).min((bottom - top) / 2.),
                    fill,
                });
            }
        };
        match self {
            Self::Timeline(spec) => {
                let color = |paint: Paint| {
                    let p = paint.over(spec.matte);
                    rgb(p.r, p.g, p.b)
                };
                let start = spec.start_hour.min(23) * 60;
                let end = spec.end_hour.clamp(spec.start_hour.min(23) + 1, 24) * 60;
                let pad = 6f32.min(w / 2.);
                let x = |minute: u32| {
                    pad + (minute.clamp(start, end) - start) as f32 / (end - start) as f32
                        * (w - 2. * pad)
                };
                let track_h = (h * 0.34).max(8.).min(h);
                let track_y = h * 0.42 - track_h / 2.;
                push(
                    x(start),
                    track_y,
                    x(end) - x(start),
                    track_h,
                    track_h / 2.,
                    color(spec.track),
                );
                if spec.now_min > start {
                    push(
                        x(start),
                        track_y,
                        x(spec.now_min) - x(start),
                        track_h,
                        track_h / 2.,
                        color(spec.elapsed),
                    );
                }
                for hour in start / 60..=end / 60 {
                    let major = hour % 3 == 0;
                    let fill = color(if major {
                        spec.tick
                    } else {
                        spec.tick.with_alpha(0.5)
                    });
                    push(
                        x(hour * 60) - 0.5,
                        track_y + track_h + 3.,
                        1.,
                        if major { 7. } else { 4. },
                        0.,
                        fill,
                    );
                }
                let lanes = stepx_companion::home_visuals::event_lanes(&spec.events);
                for (event, (lane, count)) in spec.events.iter().zip(lanes) {
                    let from = event.start_min.min(1439);
                    let to = if event.end_min.min(1440) <= from {
                        (from + 30).min(1440)
                    } else {
                        event.end_min.min(1440)
                    };
                    if to <= start || from >= end {
                        continue;
                    }
                    let left = x(from);
                    let right = x(to).max(left + 6.).min(w - pad);
                    let lane_h = (track_h + 6.) / count as f32;
                    push(
                        left,
                        track_y - 3. + lane_h * lane as f32,
                        right - left,
                        if count > 1 {
                            (lane_h - 1.).max(1.)
                        } else {
                            track_h + 6.
                        },
                        if count > 1 { 2. } else { 3. },
                        color(event.paint),
                    );
                }
                for alarm in &spec.alarms {
                    let minute = alarm.minute_of_day % 1440;
                    if minute < start || minute > end {
                        continue;
                    }
                    push(
                        x(minute) - 1.5,
                        (track_y - 11.).max(0.),
                        3.,
                        7.,
                        1.5,
                        color(if alarm.enabled {
                            spec.alarm
                        } else {
                            spec.alarm.with_alpha(0.4)
                        }),
                    );
                }
                if (start..=end).contains(&spec.now_min) {
                    push(
                        x(spec.now_min) - 1.25,
                        1.,
                        2.5,
                        track_y + track_h + 4.,
                        1.25,
                        color(spec.now),
                    );
                    push(x(spec.now_min) - 4.5, 0., 9., 5., 2.5, color(spec.now));
                }
            }
            Self::Week {
                days,
                fills,
                track,
                selected,
            } => {
                let count = days.len().clamp(1, 14);
                let col = w / count as f32;
                let bar = (col * 0.52).clamp(1., 28.).min(col);
                let top = 3f32.min(h / 4.);
                let usable = (h - top - 6.).max(0.);
                let max_total = days
                    .iter()
                    .take(count)
                    .map(|day| {
                        day.work_count
                            .saturating_add(day.life_count)
                            .saturating_add(day.issue_count)
                    })
                    .max()
                    .unwrap_or(0)
                    .max(4) as f32;
                for (i, day) in days.iter().take(count).enumerate() {
                    let center = (i as f32 + 0.5) * col;
                    let left = center - bar / 2.;
                    push(left, top, bar, usable, 3., *track);
                    let mut bottom = top + usable;
                    for (value, fill) in [day.work_count, day.life_count, day.issue_count]
                        .into_iter()
                        .zip(*fills)
                    {
                        if value == 0 {
                            continue;
                        }
                        let height = (usable * value as f32 / max_total)
                            .max(1.4)
                            .min((bottom - top).max(0.));
                        bottom -= height;
                        push(left, bottom, bar, height, 2., fill);
                    }
                    if day.selected || day.today {
                        let marker = bar * if day.selected { 0.95 } else { 0.5 };
                        push(
                            center - marker / 2.,
                            h - 3.,
                            marker,
                            2.5,
                            1.2,
                            if day.selected { *selected } else { fills[0] },
                        );
                    }
                }
            }
        }
        rects
    }
}

pub(in crate::ui) struct HomePlot {
    content: HomePlotContent,
    preferred_height: f32,
    rects: Vec<HomePlotRect>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl facloud_ui::Element for HomePlot {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        _: &mut LayoutContext,
        _: &facloud_ui::AppContext,
    ) -> Vector2F {
        let width = if constraint.max.x().is_finite() {
            constraint.max.x()
        } else {
            constraint.min.x().max(480.)
        };
        let size = vec2f(width, self.preferred_height)
            .max(constraint.min)
            .min(constraint.max);
        self.rects = self.content.rectangles(size);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &facloud_ui::AppContext) {}
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, _: &facloud_ui::AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        ctx.scene.draw_rect_with_hit_recording(RectF::new(
            origin,
            self.size.unwrap_or_else(Vector2F::zero),
        ));
        for rect in &self.rects {
            ctx.scene
                .draw_rect_without_hit_recording(RectF::new(
                    origin + vec2f(rect.x, rect.y),
                    vec2f(rect.width, rect.height),
                ))
                .with_background(Fill::Solid(rect.fill))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(rect.radius)));
        }
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.origin
    }
    fn dispatch_event(
        &mut self,
        _: &facloud_ui::event::DispatchedEvent,
        _: &mut EventContext,
        _: &facloud_ui::AppContext,
    ) -> bool {
        false
    }
}

pub(in crate::ui) fn home_native_plot(id: &'static str, content: HomePlotContent, height: f32) -> El {
    let plot: El = Box::new(HomePlot {
        content,
        preferred_height: height,
        rects: vec![],
        size: None,
        origin: None,
    });
    home_geometry_probe(id, plot)
}

/// Read-only capture regions let the native review measure safe insets after
/// repeated window resizing. They have no hit actions or production overhead.
pub(in crate::ui) fn home_geometry_probe(id: impl Into<String>, element: El) -> El {
    #[cfg(test)]
    {
        SavePosition::new(element, &id.into()).finish()
    }
    #[cfg(all(not(test), feature = "ui-capture"))]
    {
        crate::ui::scenario_check::control(id.into(), element)
    }
    #[cfg(all(not(test), not(feature = "ui-capture")))]
    {
        let _ = id;
        element
    }
}


pub(in crate::ui) fn soft_tint(color: ColorU, dark: bool) -> ColorU {
    ColorU::new(color.r, color.g, color.b, if dark { 42 } else { 24 })
}

pub(in crate::ui) fn paint_of(c: ColorU) -> Paint {
    Paint {
        r: c.r,
        g: c.g,
        b: c.b,
        alpha: c.a as f32 / 255.0,
    }
}

/// Opaque chart colour: `c` at `alpha` composited over the surface `bg` the chart sits on.
pub(in crate::ui) fn solid(c: ColorU, alpha: f32, bg: ColorU) -> Paint {
    paint_of(c).with_alpha(alpha).over(paint_of(bg).with_alpha(1.0))
}

/// Distinct colours for calendar blocks on the dial / timeline (legend uses the same order).
pub(in crate::ui) fn event_colors(p: &Palette) -> [ColorU; 5] {
    [p.blue, p.indigo, p.pink, p.success_text, p.warning]
}

/// Icon + tint for any Home intent, shared by phone-AI and local suggestions.
pub(in crate::ui) fn intent_visual(intent: &HomeAiIntent, p: &Palette) -> (UiIcon, ColorU) {
    match intent {
        HomeAiIntent::AmooPrompt(_) => (UiIcon::Chat, p.blue),
        HomeAiIntent::QuickGuiScenario { .. } => (UiIcon::Smartphone, p.indigo),
        HomeAiIntent::WorkPrompt(_) => (UiIcon::File, p.blue),
        HomeAiIntent::BackScreenStatus(_) => (UiIcon::Matrix, p.warning),
        HomeAiIntent::QueryExpress(_) => (UiIcon::Package, p.warning),
        HomeAiIntent::AlipayCoupon => (UiIcon::Gift, p.pink),
        HomeAiIntent::CapCutHandoff => (UiIcon::Video, p.pink),
        HomeAiIntent::PullRemoteAgentFiles => (UiIcon::Download, p.blue),
        HomeAiIntent::StartPomodoro(_) => (UiIcon::Clock, p.indigo),
        HomeAiIntent::ToggleDnd(_) => (UiIcon::Mute, p.pink),
        HomeAiIntent::OpenPage(_) => (UiIcon::ChevronRight, p.text2),
        HomeAiIntent::CreatePhoneAlarm { .. } => (UiIcon::Clock, p.warning),
    }
}

/// "1h 05m" / "25 分钟".
pub(in crate::ui) fn fmt_minutes(mins: u32) -> String {
    if mins >= 60 {
        format!("{}h {:02}m", mins / 60, mins % 60)
    } else {
        format!("{mins} 分钟")
    }
}

pub(in crate::ui) fn home_updated_label(updated: i64) -> String {
    chrono::DateTime::from_timestamp(updated, 0)
        .map(|at| at.with_timezone(&Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "时间未记录".into())
}

/// Today's timed (non all-day) events as `(start_min, end_min, event)` sorted by start.
/// Events starting before `today` clamp to `00:00`, events continuing past midnight clamp to `24:00`,
/// and missing/inverted ends default to one hour.
pub(in crate::ui) fn today_event_spans(
    events: &[stepx_companion::toolbox::PhoneCalendarEvent],
    today: NaiveDate,
) -> Vec<(u32, u32, &stepx_companion::toolbox::PhoneCalendarEvent)> {
    let mut spans: Vec<_> = events
        .iter()
        .filter(|ev| !ev.all_day && ev.dtstart > 0 && !ev.is_e2e_probe())
        .filter_map(|ev| {
            let start_dt = chrono::TimeZone::timestamp_millis_opt(&Local, ev.dtstart).single()?;
            let start_date = start_dt.date_naive();
            if start_date > today {
                return None;
            }
            let effective_end_ms = if ev.dtend > ev.dtstart {
                ev.dtend
            } else {
                ev.dtstart + 3_600_000
            };
            let end_dt =
                chrono::TimeZone::timestamp_millis_opt(&Local, effective_end_ms).single()?;
            let end_date = end_dt.date_naive();
            let end_min_of_day = end_dt.hour() * 60 + end_dt.minute();
            if end_date < today || (end_date == today && start_date < today && end_min_of_day == 0)
            {
                return None;
            }
            let start = if start_date < today {
                0
            } else {
                start_dt.hour() * 60 + start_dt.minute()
            };
            let end = if end_date > today {
                1440
            } else {
                end_min_of_day.max(start + 1).min(1440)
            };
            Some((start, end.min(1440), ev))
        })
        .collect();
    spans.sort_by_key(|(s, e, _)| (*s, *e));
    spans
}

/// Stacks `layers` (each stretched to `w × h`) over `base`.
pub(in crate::ui) fn overlay(w: f32, h: f32, base: El, layers: Vec<El>) -> El {
    let mut stack = Stack::new().with_child(base);
    for layer in layers {
        stack = stack.with_child(
            ConstrainedBox::new(layer)
                .with_width(w)
                .with_height(h)
                .finish(),
        );
    }
    ConstrainedBox::new(stack.finish())
        .with_width(w)
        .with_height(h)
        .finish()
}

/// Constrains `left` with `Expanded(1.)` inside a `MainAxisSize::Max` row so `right` never
/// overflows outside its card border at narrow window widths (`1080px`–`1280px`).
pub(in crate::ui) fn row_between_expanded(left: El, right: El, spacing: f32) -> El {
    Flex::row()
        .with_main_axis_size(MainAxisSize::Max)
        .with_cross_axis_alignment(CrossAxisAlignment::Center)
        .with_spacing(spacing)
        .with_child(Expanded::new(1., left).finish())
        .with_child(right)
        .finish()
}

