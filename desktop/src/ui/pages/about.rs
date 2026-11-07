use super::super::{theme::*, widgets::*, Action, Companion};
use facloud_ui::{elements::*, fonts::Weight};

impl Companion {
    pub(in super::super) fn about_page(&self) -> El {
        let p = self.p();
        let logo = ConstrainedBox::new(
            Image::new(
                facloud_ui::assets::asset_cache::AssetSource::Bundled {
                    path: "stepx_companion_icon.png",
                },
                CacheOption::BySize,
            )
            .contain()
            .finish(),
        )
        .with_width(48.)
        .with_height(48.)
        .finish();
        self.panel(vec![
            self.between(
                self.hstack(S3, vec![logo, self.vstack(S1, vec![
                    self.line_w("StepX Companion", 26., p.text, Weight::Bold),
                    self.mono(format!("v{} · {}", stepx_companion::build_info::VERSION, stepx_companion::build_info::compiled_id()), CAPTION, p.text3),
                ])]),
                self.button(
                    "about-open-whats-new",
                    "图文更新说明",
                    Action::DesktopUpdate(super::super::desktop_update_state::DesktopUpdateAction::OpenWhatsNew),
                    Btn::Primary,
                ),
            ),
            self.text("手机与电脑之间，更顺手的工作台。", BODY, p.text2),
            self.divider(),
            self.personal_and_group_qr_cards("about"),
            self.between(
                self.line_w("微信 · 164245026   |   邮箱 · f@ab.cool", BODY, p.text, Weight::Medium),
                self.button("about-copy-wechat", "复制微信", Action::CopyText("164245026".into(), "微信号已复制".into()), Btn::Secondary),
            ),
            self.divider(),
            self.text("非官方开源项目，相关品牌与素材归原权利人所有。如有侵权，请联系我，我会及时核实并删除。", CAPTION, p.text3),
            self.hstack(S4, vec![
                self.link_action("about-repository", "项目主页", Action::StudioOpenUrl("https://github.com/xydzf520/stepx".into())),
                self.link_action("about-feedback", "反馈问题", Action::StudioOpenUrl("https://github.com/xydzf520/stepx/issues".into())),
                self.link_action("about-changelog", "完整更新日志", Action::DesktopUpdate(super::super::desktop_update_state::DesktopUpdateAction::OpenHistory)),
                self.link_action("about-license", "开源许可", Action::StudioOpenUrl("https://github.com/xydzf520/stepx/blob/main/THIRD_PARTY_NOTICES.md".into())),
            ]),
        ])
    }
}
