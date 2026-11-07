//! Picture-led first connection guidance; authorization still happens on the phone.
use super::{icons::UiIcon, theme::*, widgets::*, *};

impl Companion {
    /// Real Android documentation screen, with Chinese instructions outside the
    /// image so the operating system's original controls are never fabricated.
    pub(super) fn connection_reference(&self, step: u8) -> El {
        let p = self.p();
        let (path, title, hint) = match step {
            0 => ("connection-guide/build-number.png", "连续点「版本号」7 次", "手机设置 → 关于手机 → 版本信息；出现“已处于开发者模式”即可。"),
            1 => ("connection-guide/usb-debugging.png", "打开「USB 调试」开关", "设置里搜索“开发者选项”，找到图中红框的位置。USB debugging 就是 USB 调试。"),
            2 => ("connection-guide/allow-usb.png", "插上数据线，在手机点「允许」", "只给你信任的电脑授权；图中的 OK 对应“允许／确定”。"),
            _ => ("connection-guide/wireless-setting.png", "打开「无线调试」，再选择配对方式", "手机和电脑连同一个 Wi-Fi。扫码选 QR code，六位码选 pairing code。"),
        };
        let photo = ConstrainedBox::new(
            facloud_ui::elements::Image::new(facloud_ui::assets::asset_cache::AssetSource::Bundled { path }, CacheOption::BySize)
                .contain().with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL))).finish(),
        ).with_height(if step >= 3 { 280. } else { 160. }).finish();
        self.vstack(S3, vec![
            Container::new(photo).with_uniform_padding(S3).with_background_color(rgb(255,255,255))
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL))).finish(),
            self.hstack(S2, vec![self.section_icon_badge(if step >= 3 { UiIcon::Wifi } else { UiIcon::Usb }, 16., 28.), self.text_w(title, SUBHEAD, p.text, facloud_ui::fonts::Weight::Semibold)]),
            self.text(hint, BODY, p.text2),
            self.text("Android 官方实机截图 · Google / CC BY 4.0 · 不同手机的菜单名称可能略有差异", MICRO, p.text3),
        ])
    }

    pub(super) fn adb_guide_modal(&self, step: u8) -> El {
        let p = self.p();
        let step = step.min(2);
        let connected = self.selected.is_some();
        let tabs = self.wrap(["找版本号", "开 USB 调试", "插线并允许"].iter().enumerate().map(|(i,label)| {
            self.button(format!("adb-guide-step-{i}"), format!("{}  {label}", i+1), Action::AdbGuide(Some(i as u8)), if step as usize == i { Btn::Primary } else { Btn::Secondary })
        }).collect());
        let mut content = vec![self.connection_reference(step)];
        if connected {
            content.push(self.hstack(S2, vec![self.content_icon(UiIcon::Check, 16., p.success), self.text("当前手机已连接，可以直接完成；也可切换步骤查看操作。", CAPTION, p.success)]));
        } else if step == 2 {
            let hint = if self.devices.iter().any(|d| d.state == "unauthorized") { "已找到手机，请在手机的授权弹窗点“允许”。" }
                else if self.devices.iter().any(|d| d.state == "offline") { "已找到手机但暂时离线，请重新插拔数据线。" }
                else if self.busy { "正在检查连接…" }
                else { "还没找到手机？试试换一根能传文件的数据线。" };
            content.push(self.text(hint, BODY, p.text2));
            if self.error { content.push(self.text(&self.status, CAPTION, p.danger)); }
            content.push(self.button_icon("adb-guide-detect", "重新检查", UiIcon::Refresh, Action::Refresh, Btn::Secondary));
        }
        if step == 0 { content.push(self.text("已经开启过？直接点上面的“开 USB 调试”。若手机要求密码，请在手机上自行验证。", CAPTION, p.text3)); }
        let mut actions = Vec::new();
        if step > 0 { actions.push(self.button("adb-guide-back", "上一步", Action::AdbGuide(Some(step - 1)), Btn::Secondary)); }
        if step < 2 { actions.push(self.button("adb-guide-next", "下一步", Action::AdbGuide(Some(step + 1)), Btn::Primary)); }
        else if connected { actions.push(self.button("adb-guide-companion", "继续安装伴侣", Action::AdbGuideContinue, Btn::Primary)); }
        else { actions.push(self.button("adb-guide-done", "返回连接页", Action::AdbGuide(None), Btn::Primary)); }
        let scroll = self.pane_scroll(self.adb_guide_scroll.clone(), self.vstack(S4, content));
        #[cfg(feature = "ui-capture")]
        let scroll = super::scenario_check::control("adb-guide-scroll".into(), scroll);
        let card = Flex::column().with_main_axis_size(MainAxisSize::Max).with_cross_axis_alignment(CrossAxisAlignment::Stretch).with_spacing(S3)
            .with_children(vec![
                self.between(self.section_heading(UiIcon::Usb, "第一次连接手机"), self.icon_action("adb-guide-close", "关闭连接教程", UiIcon::Close, vec![Action::AdbGuide(None)], false)),
                self.text("照着手机上的画面做，电脑不需要另外装 ADB。", CAPTION, p.text2),
                tabs, self.divider(), Expanded::new(1., scroll).finish(), self.divider(),
                self.between(self.button("adb-guide-skip", if connected { "完成" } else { "稍后设置" }, Action::AdbGuide(None), Btn::Ghost), self.wrap(actions)),
            ]).finish();
        let sheet = ConstrainedBox::new(popover_surface(Container::new(card).with_uniform_padding(S4).finish(), p))
            .with_max_width(760.).with_max_height(650.).finish();
        Container::new(Align::new(sheet).finish()).with_uniform_padding(S5).with_background_color(rgba(0,0,0,112)).finish()
    }
}

/// Keep the app visible under a modal without dispatching input to its controls.
pub(super) struct Inert(pub El);
impl Element for Inert {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        self.0.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.0.after_layout(ctx, app);
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.0.paint(origin, ctx, app);
    }
    fn size(&self) -> Option<Vector2F> {
        self.0.size()
    }
    fn origin(&self) -> Option<Point> {
        self.0.origin()
    }
    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
}
