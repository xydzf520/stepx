//! Phone-side speech entry points. No recorder/model is enabled before a real
//! offline phone inference adapter has been verified for the current device.
use super::*;
use super::{icons::UiIcon, theme::*, widgets::El};
use facloud_ui::platform::Cursor;

#[derive(Clone, Debug)]
pub(super) enum VoiceInputAction {
    Inspect(usize),
}

#[derive(Default)]
pub(super) struct VoiceInputUi {
    inspected_field: Option<usize>,
}

pub(super) fn accepts_dictation(field: usize) -> bool {
    matches!(
        field,
        F_SMS_BODY
            | F_AMOO
            | F_SMS_SEARCH
            | F_FILE_SEARCH
            | F_APP_SEARCH
            | F_OV_TEXT
            | F_BACK_PROMPT
            | F_BACK_MARQUEE
            | F_STUDIO_NAME
            | F_CONTACT_SEARCH
            | F_AMOO_SEARCH
            | F_WORK_PROMPT
            | F_WORK_SEARCH
            | F_WORK_CAP_SEARCH
            | F_WORK_AGENT_NAME
            | F_AMOO_GUI_APP
            | F_STUDIO_SEARCH
            | F_LAB_PROMPT
            | F_PUSH_SEARCH
            | F_PUSH_REPLY
            | F_IM_REPLY
            | F_TOAST_REPLY
            | F_PHONE_SCHEDULE_TITLE
            | F_AMOO_MEMORY_INPUT
            | F_GALLERY_SEARCH
            | F_STORE_SEARCH
            | F_ALARM_LABEL
            | F_PHONE_CONTACT_NAME
            | F_HOME_COMMAND
            | F_PRODUCTIVITY_NOTE
            | F_GLOBAL_SEARCH
    )
}

impl Companion {
    #[cfg(feature = "ui-capture")]
    pub(super) fn voice_input_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "phase": "Unavailable",
            "location": "手机端侧",
            "field": self.voice_input.inspected_field,
            "active": false,
            "available": false,
            "transcript_chars": 0,
        })
    }

    pub(super) fn handle_voice_input(
        &mut self,
        action: &VoiceInputAction,
        ctx: &mut ViewContext<Self>,
    ) {
        let VoiceInputAction::Inspect(index) = *action;
        if index >= self.fields.len() || !accepts_dictation(index) {
            return;
        }
        self.voice_input.inspected_field = Some(index);
        // Inspect in place. A capability check must not navigate away from a
        // draft, change its selection, open a recorder, or synthesize speech.
        self.status = self.phone_dictation_reason();
        self.error = false;
        self.refresh_voice_capability(ctx);
        ctx.notify();
    }

    /// A reserved SVG target keeps text and adjacent controls in the same place.
    /// An unavailable microphone opens an explanation, not a fallback recorder.
    pub(super) fn voice_input_button(&self, index: usize) -> El {
        let p = self.p();
        let reason = self.phone_dictation_reason();
        let button = Hoverable::new(
            self.mouse_state(format!("voice-input:{index}")),
            move |mouse| {
                let tile = ConstrainedBox::new(
                    Container::new(Align::new(self.ui_icon(UiIcon::Mic, 16., p.text3)).finish())
                        .with_background_color(if mouse.is_mouse_over_element() {
                            p.hover
                        } else {
                            facloud_ui::color::ColorU::transparent_black()
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                )
                .with_width(28.)
                .with_height(24.)
                .finish();
                if !mouse.is_mouse_over_element() {
                    return tile;
                }
                let tip = Container::new(self.fixed_w(
                    280.,
                    self.vstack(
                        S1,
                        vec![
                            self.line("手机端侧语音尚不可用，点击检查", MICRO, p.text),
                            self.text(&reason, MICRO, p.text2),
                            self.line("设置 → 通用可查看详情", MICRO, p.text3),
                        ],
                    ),
                ))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S1)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                Stack::new()
                    .with_child(tile)
                    .with_positioned_child(
                        tip,
                        OffsetPositioning::offset_from_parent(
                            vec2f(0., S1),
                            ParentOffsetBounds::WindowByPosition,
                            ParentAnchor::BottomRight,
                            ChildAnchor::TopRight,
                        ),
                    )
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::VoiceInput(VoiceInputAction::Inspect(index)))
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let button = super::scenario_check::control(format!("voice-input:{index}"), button);
        button
    }
}
