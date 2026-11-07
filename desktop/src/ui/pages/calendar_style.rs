use crate::ui::{theme::*, widgets::El, Companion};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DayDomain {
    Work,
    Life,
}

impl DayDomain {
    fn label(self) -> &'static str {
        match self {
            Self::Work => "工作",
            Self::Life => "生活",
        }
    }

    pub(super) fn tint(self, palette: Palette) -> ColorU {
        match self {
            Self::Work => palette.blue,
            Self::Life => palette.teal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct DayMarks {
    work: bool,
    life: bool,
}

impl DayMarks {
    pub(super) fn include(&mut self, domain: DayDomain) {
        match domain {
            DayDomain::Work => self.work = true,
            DayDomain::Life => self.life = true,
        }
    }

    pub(super) fn domains(self) -> impl Iterator<Item = DayDomain> {
        [
            self.work.then_some(DayDomain::Work),
            self.life.then_some(DayDomain::Life),
        ]
        .into_iter()
        .flatten()
    }
}

pub(super) fn day_domain(source: &str, title: &str) -> Option<DayDomain> {
    use DayDomain::{Life, Work};

    for (domain, tags) in [
        (Work, ["【工作】", "[工作]", "工作：", "工作:"]),
        (Life, ["【生活】", "[生活]", "生活：", "生活:"]),
    ] {
        if tags.iter().any(|tag| title.trim_start().starts_with(tag)) {
            return Some(domain);
        }
    }
    let label = source.trim().to_ascii_lowercase();
    match label.as_str() {
        "工作" | "工作日历" | "公司日历" | "work" | "work calendar" => return Some(Work),
        "生活" | "生活日历" | "个人" | "个人日历" | "家庭日历" | "personal" | "family"
        | "personal calendar" | "family calendar" => return Some(Life),
        _ => {}
    }
    let work = [
        "会议",
        "开会",
        "周会",
        "晨会",
        "例会",
        "站会",
        "评审",
        "述职",
        "客户拜访",
        "公司团建",
        "上班",
        "办公",
        "交付",
        "面试",
        "项目复盘",
        "工作汇报",
    ]
    .iter()
    .any(|cue| title.contains(cue));
    let life = [
        "生日",
        "家人",
        "家庭聚餐",
        "聚餐",
        "接孩子",
        "送孩子",
        "牙医",
        "体检",
        "就诊",
        "健身",
        "瑜伽",
        "跑步",
        "旅行",
        "取快递",
        "快递取件",
        "买菜",
        "做饭",
        "起床",
        "午休",
        "服药",
        "电影",
        "遛狗",
    ]
    .iter()
    .any(|cue| title.contains(cue));
    match (work, life) {
        (true, false) => Some(Work),
        (false, true) => Some(Life),
        _ => None,
    }
}

impl Companion {
    pub(super) fn calendar_day_number(
        &self,
        day: u32,
        today: bool,
        selected: bool,
        in_month: bool,
    ) -> El {
        let p = self.p();
        ConstrainedBox::new(
            Container::new(
                Align::new(self.line_w(
                    day.to_string(),
                    BODY,
                    if today {
                        p.on_accent
                    } else if in_month {
                        p.text
                    } else {
                        p.text3
                    },
                    if today || selected {
                        Weight::Semibold
                    } else {
                        Weight::Normal
                    },
                ))
                .finish(),
            )
            .with_background_color(if today {
                p.accent
            } else {
                ColorU::transparent_black()
            })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish(),
        )
        .with_width(24.)
        .with_height(24.)
        .finish()
    }

    fn calendar_mark_dot(&self, domain: DayDomain) -> El {
        ConstrainedBox::new(
            Rect::new()
                .with_background_color(domain.tint(self.p()))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.5)))
                .finish(),
        )
        .with_width(5.)
        .with_height(5.)
        .finish()
    }

    pub(super) fn calendar_day_marks(&self, marks: DayMarks) -> El {
        ConstrainedBox::new(
            Align::new(
                self.hstack(
                    3.,
                    marks
                        .domains()
                        .map(|domain| self.calendar_mark_dot(domain))
                        .collect(),
                ),
            )
            .finish(),
        )
        .with_height(6.)
        .finish()
    }

    pub(super) fn calendar_mark_legend(&self) -> Vec<El> {
        [DayDomain::Work, DayDomain::Life]
            .into_iter()
            .map(|domain| {
                self.hstack(
                    5.,
                    vec![
                        self.calendar_mark_dot(domain),
                        self.line(domain.label(), MICRO, self.p().text2),
                    ],
                )
            })
            .collect()
    }

    pub(super) fn calendar_content_enter(&self, child: El, month: bool) -> El {
        let started = if month {
            self.back_screen.calendar_month_motion_started
        } else {
            self.back_screen.calendar_day_motion_started
        };
        match started {
            Some(at) => crate::ui::motion::sub_enter(child, at, !self.animations_disabled()),
            None => child,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markers_are_unique_and_use_work_then_life_order() {
        let mut day = DayMarks::default();
        assert_eq!(day.domains().count(), 0);
        day.include(DayDomain::Life);
        day.include(DayDomain::Life);
        assert_eq!(day.domains().collect::<Vec<_>>(), [DayDomain::Life]);
        day.include(DayDomain::Work);
        day.include(DayDomain::Work);
        assert_eq!(
            day.domains().collect::<Vec<_>>(),
            [DayDomain::Work, DayDomain::Life]
        );
    }

    #[test]
    fn calendar_domain_prefers_explicit_labels_and_keeps_unknowns_neutral() {
        use DayDomain::{Life, Work};
        for (source, title, domain) in [
            ("手机日历", "公司团建", Some(Work)),
            ("手机日历", "跑步", Some(Life)),
            ("个人日历", "项目会议", Some(Life)),
            ("手机日历", "【工作】会议和家庭聚餐", Some(Work)),
            ("手机日历", "test", None),
            ("手机日历", "会议和家庭聚餐", None),
            ("手机日历", "闹钟", None),
        ] {
            assert_eq!(day_domain(source, title), domain, "{source}: {title}");
        }
    }
}
