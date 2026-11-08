//! Native layout and pointer regressions for the home task composer.
use super::*;
use facloud_ui::{AppContext, Entity, Event, Presenter, TypedActionView, View, ViewHandle, WindowInvalidation};

struct QuickStartOnly(ViewHandle<Companion>);
impl Entity for QuickStartOnly { type Event = (); }
impl TypedActionView for QuickStartOnly { type Action = (); }
impl View for QuickStartOnly {
    fn ui_name() -> &'static str { "home_quick_start_test" }
    fn render(&self, ctx: &AppContext) -> El {
        self.0.read(ctx, |companion, _| {
            Stack::new().with_child(Align::new(SavePosition::new(
                companion.render_home_new_request(), "quick-start-panel",
            ).finish()).top_left().finish()).finish()
        })
    }
}

fn inside(child: RectF, parent: RectF) -> bool {
    child.min_x() >= parent.min_x() - 0.5 && child.min_y() >= parent.min_y() - 0.5
        && child.max_x() <= parent.max_x() + 0.5 && child.max_y() <= parent.max_y() + 0.5
}

fn overlap(a: RectF, b: RectF) -> bool {
    a.min_x() < b.max_x() - 0.5 && b.min_x() < a.max_x() - 0.5
        && a.min_y() < b.max_y() - 0.5 && b.min_y() < a.max_y() - 0.5
}

// Exercise the actual painted hit region, not a fabricated command sequence.
fn click_actions(presenter: &mut Presenter, ctx: &AppContext, id: &str) -> Vec<Action> {
    let bounds = presenter.position_cache().get_position(id).expect(id);
    let position = bounds.origin() + bounds.size() * 0.5;
    let mut actions = Vec::new();
    for event in [
        Event::MouseMoved { position, cmd: false, shift: false, is_synthetic: false },
        Event::LeftMouseDown { position, modifiers: Default::default(), click_count: 1, is_first_mouse: false },
        Event::LeftMouseUp { position, modifiers: Default::default() },
        Event::MouseMoved { position: vec2f(-10., -10.), cmd: false, shift: false, is_synthetic: false },
    ] {
        for dispatched in presenter.dispatch_event(event, ctx).actions.into_iter().rev() {
            if let facloud_ui::presenter::DispatchedActionKind::Typed(action) = dispatched.kind {
                if let Some(action) = action.as_any().downcast_ref::<Action>() {
                    actions.push(action.clone());
                }
            }
        }
    }
    actions
}

#[test]
fn quick_start_controls_fit_and_template_clicks_preserve_user_input() {
    // Existing ui-capture probes wrap the real controls without changing layout.
    let previous_capture_dir = std::env::var_os("STEPX_SCENARIO_DIR");
    std::env::set_var("STEPX_SCENARIO_DIR", "/tmp/stepx-quick-start-layout-test");
    facloud_ui::App::test((), |mut app| async move {
        for width in [260., 390., 720., 1100.] {
            for height in [HomeModuleHeight::Compact, HomeModuleHeight::Standard, HomeModuleHeight::Tall] {
                let (window, view) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |ctx| {
                    QuickStartOnly(ctx.add_typed_action_view(|_| {
                        let mut companion = Companion::new_for_test();
                        companion.page = Page::Home;
                        companion.workspace.viewport = vec2f(width + 278., 900.);
                        companion.home_modules.enabled = vec![HomeModuleId::QuickStartRequest];
                        companion.home_modules.set_module_size(HomeModuleId::QuickStartRequest, HomeModuleWidth::Full, height);
                        companion.fields[F_HOME_COMMAND].clear();
                        companion.settings.value.reduced_motion = true;
                        companion.desktop_update.open = false;
                        companion.desktop_update.whats_new_open = false;
                        companion
                    }))
                });
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let companion = view.read(ctx, |view, _| view.0.clone());
                    let mut presenter = Presenter::new(window);
                    let size = vec2f(width, 2000.);
                    let invalidate = |presenter: &mut Presenter, ctx: &mut AppContext| {
                        presenter.invalidate(WindowInvalidation {
                            updated: std::collections::HashSet::from([root]), ..Default::default()
                        }, ctx);
                    };
                    invalidate(&mut presenter, ctx);
                    presenter.build_scene(size, 1., None, ctx);
                    let panel = presenter.position_cache().get_position("quick-start-panel").unwrap();
                    assert!(panel.max_x() <= width + 0.5, "panel overflows at {width:?}/{height:?}: {panel:?}");
                    let mut ids = vec![format!("input:{F_HOME_COMMAND}"), "home-omnibox-submit".into(),
                        "home-new-gui".into(), "home-new-work".into(), "home-preset-review".into()];
                    for id in ["plan", "summary", "reply", "weekly", "express", "voice", "archive"] {
                        let id = format!("home-preset-{id}");
                        if presenter.position_cache().get_position(&id).is_some() { ids.push(id); }
                    }
                    assert_eq!(ids.len(), if height == HomeModuleHeight::Tall { 12 } else { 9 },
                        "density must retain the documented scenes and the daily review action");
                    let bounds: Vec<_> = ids.iter().map(|id| (id, presenter.position_cache().get_position(id).unwrap())).collect();
                    for (index, (id, rect)) in bounds.iter().enumerate() {
                        assert!(inside(*rect, panel), "{id} escapes {width:?}/{height:?}: {rect:?} / {panel:?}");
                        for (other_id, other_rect) in &bounds[index + 1..] {
                            assert!(!overlap(*rect, *other_rect), "{id} overlaps {other_id} at {width:?}/{height:?}");
                        }
                    }

                    let plan = "写一份活动方案并整理执行清单";
                    let summary = "总结今天的进展与下一步待办";
                    for (id, expected) in [("home-preset-plan", plan), ("home-preset-summary", summary)] {
                        let actions = click_actions(&mut presenter, ctx, id);
                        assert_eq!(actions.len(), 1, "one atomic template action must fill the draft exactly once");
                        assert!(actions.iter().all(|a| matches!(a, Action::HomeFillDraft(_))),
                            "choosing a scenario must not submit work or navigate");
                        companion.update(ctx, |companion, ctx| {
                            for action in actions { companion.handle_action(&action, ctx); }
                            assert_eq!(companion.fields[F_HOME_COMMAND].text, expected);
                            assert_eq!(companion.focus, Some(F_HOME_COMMAND));
                            assert_eq!(companion.page, Page::Home);
                        });
                        invalidate(&mut presenter, ctx);
                        let scene = presenter.build_scene(size, 1., None, ctx);
                        let selected = presenter.position_cache().get_position(id).unwrap();
                        let accent = companion.read(ctx, |c, _| c.p().accent_soft);
                        assert!(scene.layers().flat_map(|layer| &layer.rects).any(|rect|
                            rect.background == Fill::Solid(accent) && inside(rect.bounds, selected)
                                && rect.bounds.size().x() > selected.size().x() - 3.),
                            "selected template needs visible feedback at {width:?}/{height:?}: {selected:?}, color {accent:?}; nearby {:?}",
                            scene.layers().flat_map(|layer| &layer.rects).filter(|rect| overlap(rect.bounds, selected))
                                .map(|rect| (rect.bounds, rect.background)).collect::<Vec<_>>());
                    }
                    companion.update(ctx, |companion, ctx| {
                        companion.handle_action(&Action::ClearField(F_HOME_COMMAND), ctx);
                        companion.handle_action(&Action::Insert("下周三团建，预算 3000 元".into()), ctx);
                    });
                    for (id, prompt) in [("home-preset-plan", plan), ("home-preset-plan", plan),
                        ("home-preset-weekly", "汇总本周关键成果并生成结构化周报")] {
                        invalidate(&mut presenter, ctx);
                        presenter.build_scene(size, 1., None, ctx);
                        let actions = click_actions(&mut presenter, ctx, id);
                        assert_eq!(actions.len(), 1);
                        companion.update(ctx, |companion, ctx| {
                            for action in actions { companion.handle_action(&action, ctx); }
                            assert_eq!(companion.fields[F_HOME_COMMAND].text, format!("下周三团建，预算 3000 元\n\n{prompt}"),
                                "scene changes/repeated clicks must retain only the custom draft and current template");
                            assert_eq!(companion.page, Page::Home);
                        });
                    }
                    // Long drafts and appended template paragraphs must not
                    // paint the caret outside the field or move its IME anchor
                    // over a neighbouring control.
                    companion.update(ctx, |companion, _| {
                        companion.fields[F_HOME_COMMAND].set(&format!("{}\n\n{plan}",
                            "下周三团建，预算 3000 元 🎉 ".repeat(8)));
                    });
                    invalidate(&mut presenter, ctx);
                    presenter.build_scene(size, 1., None, ctx);
                    let field = presenter.position_cache().get_position(&format!("input:{F_HOME_COMMAND}")).unwrap();
                    let caret = presenter.position_cache().get_position(&format!("input-caret:{F_HOME_COMMAND}")).unwrap();
                    assert!(inside(caret, field), "long-draft caret escapes {width:?}/{height:?}: {caret:?} / {field:?}");
                });
            }
        }
    });
    if let Some(value) = previous_capture_dir { std::env::set_var("STEPX_SCENARIO_DIR", value); }
    else { std::env::remove_var("STEPX_SCENARIO_DIR"); }
}
