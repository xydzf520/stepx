//! Opt-in UI driver. Only dispatches pointer/keyboard events into painted controls.
//! No send, confirm, upload, or executor action shortcuts. State export is read-only.
//! Set `STEPX_SCENARIO_PAGE=home` (or pass `--page home`) for a live Home review:
//! preserve the app's real device/page selection and export counts, never chat contents.
use super::widgets::El;
use super::*;
use facloud_ui::{TypedActionView, WindowId};
thread_local! { static IDS:RefCell<std::collections::BTreeSet<String>>=RefCell::new(Default::default()); static LAST:RefCell<(u64,Option<Instant>,bool)>=RefCell::new((0,None,false)); }
pub(super) fn control(id: String, el: El) -> El {
    if std::env::var_os("STEPX_SCENARIO_DIR").is_none() {
        return el;
    }
    IDS.with(|s| s.borrow_mut().insert(id.clone()));
    SavePosition::new(el, &id).for_single_frame().finish()
}
fn events_for_window(ctx: &mut ViewContext<Companion>, id: WindowId, events: Vec<Event>) {
    // Dispatch after the current view update has finished. A static page may
    // reuse its cached frame, so on_frame_drawn is not a reliable event queue.
    thread_local! { static REGISTERED: Cell<bool> = const { Cell::new(false) }; }
    REGISTERED.with(|registered| {
        if !registered.replace(true) {
            ctx.add_global_action(
                "stepx-capture-pointer-events",
                |(id, events): &(WindowId, Vec<Event>), app| {
                    if let Some(w) = app.windows().platform_window(*id) {
                        for event in events {
                            (w.callbacks().event_callback)(event.clone(), app);
                        }
                    }
                    LAST.with(|cell| cell.borrow_mut().1 = None);
                },
            );
        }
    });
    ctx.dispatch_global_action("stepx-capture-pointer-events", (id, events));
}

fn events(ctx: &mut ViewContext<Companion>, events: Vec<Event>) {
    let id = ctx.window_id();
    events_for_window(ctx, id, events);
}
fn home_review_requested(page: Option<&str>, args: &[String]) -> bool {
    match page {
        Some(page) => page == "home",
        None => args
            .windows(2)
            .any(|pair| pair[0] == "--page" && pair[1] == "home"),
    }
}

/// Coordinates are window-local logical pixels, matching the exported control regions.
fn checked_position(
    x: Option<f64>,
    y: Option<f64>,
    width: f64,
    height: f64,
) -> Result<Vector2F, &'static str> {
    let (Some(x), Some(y)) = (x, y) else {
        return Err("x and y must be finite numbers");
    };
    if !x.is_finite()
        || !y.is_finite()
        || x < 0.
        || y < 0.
        || !width.is_finite()
        || !height.is_finite()
        || x >= width
        || y >= height
        || x > f32::MAX as f64
        || y > f32::MAX as f64
    {
        return Err("coordinates must be inside the current window");
    }
    Ok(Vector2F::new(x as f32, y as f32))
}

impl Companion {
    pub(super) fn scenario_tick(&mut self, ctx: &mut ViewContext<Self>) {
        let dir = PathBuf::from(std::env::var_os("STEPX_SCENARIO_DIR").unwrap());
        let requested_page = std::env::var("STEPX_SCENARIO_PAGE").ok();
        let home_review = home_review_requested(
            requested_page.as_deref(),
            &std::env::args().collect::<Vec<_>>(),
        );
        LAST.with(|cell| {
   let mut last=cell.borrow_mut();if last.1.is_some_and(|t|t.elapsed()<Duration::from_millis(80)){return;}last.1=Some(Instant::now());
   self.sync_desktop_popup_window(ctx);
   if !last.2 && !self.initializing && !self.busy {
     if let Ok(serial) = std::env::var("STEPX_WORKSPACE_TEST_SERIAL") {
       let serial = serial.trim();
       if !serial.is_empty() && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
         self.handle_action(&Action::Select(serial.to_string()), ctx);
         return;
       }
     } else if !home_review {
       let serial=std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit test phone");
       if self.selected.as_ref().is_none_or(|d|d.serial!=serial) {self.handle_action(&Action::Select(serial),ctx);return;}
     }
     if !home_review {
       self.handle_action(&Action::Page(Page::Amoo),ctx);
     }
     last.2=true;
   }
   let mut names=IDS.with(|s|s.borrow().clone());
   for name in ["page-header-actions", "page-header-title", "page-header-window-controls", "global-search-anchor"] {
     names.insert(name.into());
   }
   names.insert("context-surface".into());
   for i in 0..20 { names.insert(format!("context-choice-{i}")); }
   if !home_review {
   for name in ["composer-editor","composer-send","composer-surface","workspace-intent-anchor"]{names.insert(name.into());}
   }
   let regions=names.into_iter().filter_map(|id|{
     let r = if self.settings.hidden {
       self.desktop_popup.element_position_by_id(ctx, &id).or_else(|| ctx.element_position_by_id(id.clone()))
     } else {
       ctx.element_position_by_id(id.clone()).or_else(|| self.desktop_popup.element_position_by_id(ctx, &id))
     };
     r.map(|r|(id,[r.origin().x(),r.origin().y(),r.width(),r.height()]))
   }).collect::<std::collections::BTreeMap<_,_>>();
   let now_ms = chrono::Local::now().timestamp_millis();
   let active_popup = self.notification_active_popup();
   let desktop_popup_json = serde_json::json!({
     "window_created": self.desktop_popup.is_created(),
     "window_visible": self.desktop_popup.is_visible(),
     "main_window_active": self.desktop_popup.main_window_active,
     "main_native_focus": facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()).map(|w| w.has_focus()),
     "popup_native_focus": self.desktop_popup.window_id().and_then(|id| facloud_ui::windowing::winit::winit_window_for_id(ctx, id)).map(|w| w.has_focus()),
     "window_bounds": self.desktop_popup.bounds_array(),
     "main_window_hidden": self.settings.hidden,
     "settings_dirty": self.settings_dirty(),
     "review_open": self.interaction.review.is_some(),
     "alert_mode": format!("{:?}", self.notification_center.alert_mode),
     "drawer_open": self.notification_center.drawer_open,
     "unread_count": self.notification_unread_count(),
     "active_id": active_popup.map(|i| i.id.clone()),
     "active_title": active_popup.map(|i| i.title.clone()),
     "active_summary": active_popup.map(|i| i.summary.clone()),
     "active_source_badge": active_popup.map(|i| i.source_badge.clone()),
     "active_status_tag": active_popup.and_then(|i| i.status_tag.clone()),
     "priority": active_popup.map(|i| format!("{:?}", i.priority)),
     "widget_kind": active_popup.map(|i| i.popup_widget.kind_name()),
     "reply_draft_len": self.fields[F_TOAST_REPLY].text.chars().count(),
     "main_focus_field": self.focus,
     "reply_focused": self.desktop_popup.reply_focused,
     "reply_preedit_len": self.fields[F_TOAST_REPLY].preedit.chars().count(),
     "reply_draft_owner": self.notification_center.popup_reply_item_id,
     "queue_len": self.notification_center.popup_queue.len(),
     "queue_cursor": self.notification_center.popup_cursor,
     "queue_ids": self.notification_center.popup_queue.iter().cloned().collect::<Vec<_>>(),
     "remaining_ms": self.notification_center.popup_remaining_ms,
     "total_ms": self.notification_center.popup_total_ms,
     "snoozed": self.notification_center.is_currently_snoozed(),
     "last_copied_text": self.notification_center.last_copied_text,
     "feedback": self.notification_center.active_feedback(now_ms),
   });
   let release_story_json = serde_json::json!({
     "open": self.desktop_update.whats_new_open,
     "home": self.desktop_update.release_home,
     "history": self.desktop_update.release_history,
     "category": self.desktop_update.release_category,
     "video_playing": self.desktop_update.video_playing,
     "video_frame": self.desktop_update.video_frame,
     "video_ready": self.desktop_update.video_ready,
     "video_expanded": self.desktop_update.video_expanded,
     "video_error": self.desktop_update.video_error,
     "history_limit": self.desktop_update.release_history_limit,
     "history_total": stepx_companion::release_notes::history().len(),
     "scroll": self.desktop_update.whats_new_scroll.scroll_start().as_f32(),
   });
   let p_cur = self.p();
   let microcraft_json = serde_json::json!({
     "is_dark": p_cur.is_dark,
     "medium_optical_weight": p_cur.optical_weight(facloud_ui::fonts::Weight::Medium).to_css_weight(),
     "semibold_optical_weight": p_cur.optical_weight(facloud_ui::fonts::Weight::Semibold).to_css_weight(),
     "bloom_active": self.reply_motion.active(),
     "char_bloom_ms": crate::ui::reply_motion::CHAR_BLOOM_MS,
     "base_stagger_ms": crate::ui::reply_motion::BASE_STAGGER_MS,
     "concentric_list_row_r": theme::concentric_radius(theme::R_LG, 4.0),
     "concentric_nav_pill_r": theme::concentric_radius(theme::R_SHELL, 3.0),
     "bezier_snappy_quarter": crate::ui::motion::bezier_snappy_out(0.25),
   });
   let sms_state_json = serde_json::json!({
     "im_channel": format!("{:?}", self.push.im_channel),
     "selected_im_thread": self.push.selected_im_thread,
     "message_detail_key": self.push.selected_key,
     "selected_sms_thread": self.selected_thread,
     "push_revision": self.push.snapshot.revision,
     "screen_off_probes": self.push.snapshot.notifications.iter()
       .filter(|n| n.package_name == "com.stepx.context.verification" && n.text.starts_with("STEPX-VERIFY-"))
       .map(|n| serde_json::json!({"key":n.key,"post_time_ms":n.post_time_ms,"active":n.active,"text":n.text}))
       .collect::<Vec<_>>(),
     "push_loaded": self.push.loaded,
     "screen_interactive": self.push.snapshot.screen_interactive,
     "keyguard_locked": self.push.snapshot.keyguard_locked,
     "latest_wechat_received_ms": self.push.snapshot.threads.iter().filter(|t| t.package_name == "com.tencent.mm")
       .flat_map(|t| t.messages.iter()).filter(|m| !m.outgoing).map(|m| m.observed_at_ms).max(),
     "total_im_threads": self.push.snapshot.threads.len(),
     "matching_im_threads": self.push.snapshot.threads.iter().filter(|t| {
       self.push.im_channel != stepx_companion::push::ImChannel::Sms
         && !stepx_companion::push::is_excluded_im_package(&t.package_name)
         && self.push.im_channel.matches_package(&t.package_name)
     }).count(),
     "im_reply_text": self.fields[F_IM_REPLY].text,
     "sms_to_text": self.fields[F_SMS_TO].text,
     "sms_body_text": self.fields[F_SMS_BODY].text,
     "notif_total": self.push.snapshot.notifications.len(),
     "notif_unread": self.push.snapshot.notifications.iter().filter(|n| {
       !n.dismissed && !n.read && !n.is_ongoing && !stepx_companion::push::should_hide_notification(n)
     }).count(),
     "notif_category": self.push.notif_category,
     "missed_calls_unseen": self.call_logs.iter().filter(|r| {
       r.call_type == 3
         && !self.notification_center.read_ids.contains(&format!("missed-call:{}:{}", r.number.trim(), r.date_ms))
         && !self.notification_center.dismissed_ids.contains(&format!("missed-call:{}:{}", r.number.trim(), r.date_ms))
     }).count(),
     "gallery_items_count": self.back_screen.gallery_items.len(),
     "fresh_unread_sms_threads": self.fresh_unread_sms_thread_count(),
     "active_im_unread": self.active_im_unread_count(),
     "sidebar_msg_red_dot": (self.active_im_unread_count() + self.fresh_unread_sms_thread_count()) > 0
       || self.push.snapshot.threads.iter().any(|t| !Self::is_excluded_im_package(&t.package_name) && t.unread_count > 0),
     "reply_error": self.push.reply_error,
     "reply_in_flight": self.push.reply_in_flight,
     "reply_queue_len": self.push.reply_queue.len(),
     "active_reply_client_id": self.push.active_reply_job.as_ref().map(|j| j.client_msg_id.clone()),
     "selected_thread_outgoing": self.push.selected_im_thread.as_ref().and_then(|tid| {
       self.push.snapshot.threads.iter().find(|t| &t.thread_id == tid).map(|t| {
         t.messages.iter().filter(|m| m.outgoing).map(|m| serde_json::json!({
           "id": m.id,
           "text": m.text,
           "delivery_status": m.delivery_status,
           "via": m.via,
         })).collect::<Vec<_>>()
       })
     }).unwrap_or_default(),
     "selected_thread_messages_count": self.push.selected_im_thread.as_ref().and_then(|tid| {
       self.push.snapshot.threads.iter().find(|t| &t.thread_id == tid).map(|t| t.messages.len())
     }).unwrap_or(0),
     "pinned_targets": self.push.prefs.pinned_targets.iter().cloned().collect::<Vec<_>>(),
     "deleted_threads_count": self.push.prefs.deleted_threads.len(),
     "cleared_threads_count": self.push.prefs.cleared_threads.len(),
     "deleted_messages_count": self.push.prefs.deleted_messages.len(),
     "sms_threads_count": self.sms_threads().len(),
     "local_sent_sms_count": self.local_sent_sms.len(),
     "im_thread_ids": self.push.snapshot.threads.iter().map(|t| t.thread_id.clone()).collect::<Vec<_>>(),
     "im_thread_titles": self.push.snapshot.threads.iter().map(|t| t.title.clone()).collect::<Vec<_>>(),
     "sms_thread_addresses": self.sms_threads().iter().map(|t| t.address.clone()).collect::<Vec<_>>(),
     "express_parcels_count": self.express_parcels.len(),
     "dismissed_pickup_codes": self.push.prefs.dismissed_pickup_codes.iter().cloned().collect::<Vec<_>>(),
   });
   let desktop_native_json = serde_json::json!({
     "selected_app": self.selected_app,
     "has_app_details": self.selected_app_details.is_some(),
     "app_target_sdk": self.selected_app_details.as_ref().map(|d| d.target_sdk),
     "app_permissions_count": self.selected_app_details.as_ref().map(|d| d.permissions.len()).unwrap_or(0),
     "record_audio_granted": self.selected_app_details.as_ref().and_then(|d| {
       d.permissions.iter().find(|p| p.name == "android.permission.RECORD_AUDIO").map(|p| p.granted)
     }),
     "selected_file": self.selected_file,
     "selected_file_preview_path": self.selected_file_preview_path,
     "has_file_preview_text": self.selected_file_preview_text.as_ref().is_some_and(|t| !t.is_empty()),
     "contacts_count": self.contacts.len(),
     "top_contact_name": self.contacts.first().map(|c| c.name.clone()),
     "top_contact_number": self.contacts.first().map(|c| c.number.clone()),
     "workspace_panel": self.workspace.panel,
     "tool_page": self.tool_page.map(|t| format!("{t:?}")),
     "store_updatable_count": self.store_updatable_count(),
     "store_installed_count": self.store_installed_count(),
     "store_category": format!("{:?}", self.app_store.category),
     "store_selected_package": self.app_store.selected_package,
     "store_filtered_packages": self.filtered_store_apps().iter().map(|a| a.package.clone()).collect::<Vec<_>>(),
   });
   let connection_stability_json = serde_json::json!({
     "active_transport": self.selected.as_ref().map(|d| d.transport.clone()),
     "is_wireless": self.selected.as_ref().is_some_and(|d| is_wireless(&d.transport)),
     "backup_transports": self.selected.as_ref().map(|d| self.online_device_backups(d)).unwrap_or_default(),
     "known_backup_transports": self.selected.as_ref().map(|d| d.backup_transports()).unwrap_or_default(),
     "online_transports": self.selected.as_ref().map(|d| self.online_device_transports(d)).unwrap_or_default(),
     "connection_label": self.selected.as_ref().map(|d| self.device_connection_label(d)),
     "has_dual_standby": self.selected.as_ref().is_some_and(|d| self.device_has_online_usb_and_wireless(d)),
     "misses": self.connection_watch.misses,
     "user_paused": self.connection_watch.user_paused,
     "manage_open": self.connection_manage_open,
     "pairing_open": self.connection_pairing_open,
     "doctor_running": self.doctor_running,
     "doctor": self.doctor_report.as_ref().map(|report| serde_json::json!({
       "topic": report.topic, "checked_at_ms": report.checked_at_ms,
       "phone_ai_replied": report.phone_token_used, "model_label": report.model_label,
       "step_count": report.steps.len(),
     })),
     "rtt_ms": self.selected.as_ref().map(|d| stepx_companion::adb::transport_telemetry(&d.transport).avg_rtt_ms).unwrap_or(0),
   });
   let build_home_state = |app: &Self| {
     let (rep_key, rep_tid, rep_pkg, rep_title) = app.resolve_home_reply_target(&app.push.snapshot);
     let home_page_build_us = app.last_home_page_build_us();
     let module_sizes = pages::home::HomeModuleId::ALL
       .iter()
       .map(|m| {
         let w = app.home_modules.module_width(*m);
         let h = app.home_modules.module_height(*m);
         (
           m.slug().to_string(),
           serde_json::json!({
             "width": w.slug(),
             "height": h.slug(),
             "width_label": w.short_label(),
             "height_label": h.short_label(),
           }),
         )
       })
       .collect::<std::collections::BTreeMap<_, _>>();
     let module_value_profiles = pages::home::HomeModuleId::ALL
       .iter()
       .map(|m| {
         let w = app.home_modules.module_width(*m);
         let h = app.home_modules.module_height(*m);
         let prof = app.compute_home_module_value_profile(*m, w, h);
         (
           m.slug().to_string(),
           serde_json::json!({
             "density_tier": prof.density_tier,
             "effective_density": format!("{:?}", prof.effective_density),
             "resolved_height": prof.resolved_height,
             "max_items": prof.max_items,
             "primary_item_budget": prof.primary_item_budget,
             "columns": prof.columns,
             "internal_columns": prof.internal_columns,
             "char_budget": prof.char_budget,
             "show_side_detail_panel": prof.show_side_detail_panel,
             "show_extended_subcards": prof.show_extended_subcards,
             "unlocked_sections": prof.unlocked_sections,
           }),
         )
       })
       .collect::<std::collections::BTreeMap<_, _>>();
     let (drag_state, anim_frames, anim_phase, card_rects, drag_handle_rects, resize_handle_rects) = {
       let reg = app.home_pointer_registry.borrow();
       let ds = match &reg.active_op {
         None => serde_json::Value::Null,
         Some(pages::home::HomePointerOp::DragModule {
           source,
           hover_target,
           moved,
           ..
         }) => serde_json::json!({
           "kind": "reorder",
           "source": source.slug(),
           "hover_target": hover_target.map(|t| t.slug()),
           "moved": moved,
         }),
         Some(pages::home::HomePointerOp::ResizeModule {
           module,
           preview_width,
           preview_height,
           ..
         }) => serde_json::json!({
           "kind": "resize",
           "target": module.slug(),
           "preview_width": preview_width.slug(),
           "preview_height": preview_height.slug(),
         }),
       };
       let to_rect_map = |m: &std::collections::BTreeMap<pages::home::HomeModuleId, pathfinder_geometry::rect::RectF>| {
         m.iter()
           .map(|(k, r)| {
             (
               k.slug().to_string(),
               serde_json::json!({
                 "x": r.origin_x(),
                 "y": r.origin_y(),
                 "w": r.width(),
                 "h": r.height(),
               }),
             )
           })
           .collect::<std::collections::BTreeMap<_, _>>()
       };
       (
         ds,
         reg.anim_frame_count,
         reg.anim_last_phase,
         to_rect_map(&reg.card_rects),
         to_rect_map(&reg.drag_handle_rects),
         to_rect_map(&reg.resize_handle_rects),
       )
     };
     serde_json::json!({
       "preset": app.home_modules.preset.map(|p| format!("{p:?}")),
       "enabled_modules": app.home_modules.enabled.iter().map(|m| m.slug()).collect::<Vec<_>>(),
       "available_modules": app.home_modules.available_to_add().iter().map(|m| m.slug()).collect::<Vec<_>>(),
       "edit_mode": app.home_module_edit_mode,
       "pending_move_module": app.home_pending_move.map(|m| m.slug()),
       "drag_state": drag_state,
       "edit_mode_animating": (app.home_module_edit_mode || app.home_pending_move.is_some() || app.home_drag_active()) && anim_frames > 0,
       "edit_anim_frames": anim_frames,
       "edit_anim_phase": anim_phase,
       "card_rects": card_rects,
       "drag_handle_rects": drag_handle_rects,
       "resize_handle_rects": resize_handle_rects,
       "module_sizes": module_sizes,
       "module_value_profiles": module_value_profiles,
       "category_filter": app.home_module_filter.map(|c| format!("{c:?}")),
       "tools_expanded": app.home_tools_expanded,
       "context_focused": app.context_home_is_focused(),
       "selected_date": app.home_selected_date().format("%Y-%m-%d").to_string(),
       "schedules_count": app.back_screen.stepos_suite.calendar_events.len(),
       "schedules": app.back_screen.stepos_suite.calendar_events.iter().map(|e| serde_json::json!({
         "id": e.id, "title": e.title, "time": e.hm_label(), "date": format!("{:04}-{:02}-{:02}", e.date_ymd().0, e.date_ymd().1, e.date_ymd().2)
       })).collect::<Vec<_>>(),
       "alarms_count": app.back_screen.stepos_suite.alarms.len(),
       "alarms": app.back_screen.stepos_suite.alarms.iter().map(|a| serde_json::json!({
         "id": a.id, "time": format!("{:02}:{:02}", a.hour, a.minutes), "label": a.message, "enabled": a.enabled
       })).collect::<Vec<_>>(),
       "express_count": app.express_parcels.len(),
       "express_parcels": app.express_parcels.iter().map(|p| serde_json::json!({
         "tracking_no": p.tracking_no, "carrier": p.carrier, "status": p.status, "pickup_code": p.pickup_code
       })).collect::<Vec<_>>(),
       "context_item_titles": app.context.overview.items.iter().map(|i| i.title.clone()).collect::<Vec<_>>(),
       "reply_target": {
         "key": rep_key, "thread_id": rep_tid, "package_name": rep_pkg, "title": rep_title
       },
       "home_page_build_us": home_page_build_us,
       "home_build_count": app.home_build_count(),
       "home_build_stages_us": app.home_build_stages(),
     })
   };
   let top_bar_capsules_json = self.collect_top_bar_live_capsules().iter().map(|c| serde_json::json!({
     "id": c.id,
     "tag": c.tag,
     "kind": format!("{:?}", c.kind),
     "track": format!("{:?}", c.track),
     "domain_badge": c.domain_badge,
     "strategy_tag": c.strategy_tag,
     "source_tag": c.source_tag,
     "complete_label": c.complete_label,
     "metric": c.metric,
     "pill_text": c.pill_text,
     "title": c.title,
     "subtitle": c.subtitle,
   })).collect::<Vec<_>>();
   let build_reminder_gov_json = |app: &Self| {
     let auto_notices = stepx_companion::home_reminder_governance::recent_auto_expired_notices();
     let reg_reminder_keys: Vec<String> = app
       .home_pointer_registry
       .borrow()
       .reminder_rects
       .iter()
       .map(|(k, _, t)| format!("{} ({})", k, t.spec.primary_key))
       .collect();
     let ctx_entries: Vec<String> = app
       .context_menu
       .as_ref()
       .map(|m| app.context_entries(m).into_iter().map(|e| e.label).collect())
       .unwrap_or_default();
     serde_json::json!({
       "entries_count": app.reminder_governance.entries.len(),
       "entry_keys": app.reminder_governance.entries.keys().cloned().collect::<Vec<_>>(),
       "muted_sources_count": app.reminder_governance.muted_sources.len(),
       "auto_expired_count": auto_notices.len(),
       "auto_expired_titles": auto_notices.iter().map(|n| format!("{}: {} ({})", n.primary_key, n.title, n.reason)).collect::<Vec<_>>(),
       "drawer_open": app.home_reminder_drawer_open,
       "drawer_tab": app.home_reminder_drawer_tab,
       "context_menu_open": app.context_menu.is_some(),
       "context_menu_entries_count": ctx_entries.len(),
       "context_menu_entries": ctx_entries,
       "registered_reminder_keys": reg_reminder_keys,
     })
   };
   let nav_y_snapshot = serde_json::json!({
        "calendar": ctx.element_position_by_id("nav-calendar").map(|r| r.origin().y()),
        "alarms": ctx.element_position_by_id("nav-alarms").map(|r| r.origin().y()),
        "phone": ctx.element_position_by_id("nav-phone").map(|r| r.origin().y()),
        "sms": ctx.element_position_by_id("nav-sms").map(|r| r.origin().y()),
      });
      let build_calendar_alarms_v02103_json = |app: &Self| {
     use chrono::Datelike;
     let now_dt = chrono::Local::now();
     let now_ms = now_dt.timestamp_millis();
     let today_ymd = (now_dt.year(), now_dt.month(), now_dt.day());
     let cal_events = &app.back_screen.stepos_suite.calendar_events;
     let cal_today_total = cal_events
       .iter()
       .filter(|e| !e.is_e2e_probe() && e.date_ymd() == today_ymd)
       .count();
     let cal_today_valid = cal_events
       .iter()
       .filter(|e| {
         e.is_today_valid_unexpired_at(
           now_ms,
           &app.back_screen.completed_calendar_events,
           &app.reminder_governance,
         )
       })
       .count();
     let cal_upcoming_valid = cal_events
       .iter()
       .filter(|e| {
         e.is_valid_unexpired_at(
           now_ms,
           &app.back_screen.completed_calendar_events,
           &app.reminder_governance,
         )
       })
       .count();
     let alarms = &app.back_screen.stepos_suite.alarms;
     let alarms_total = alarms.iter().filter(|a| !a.is_e2e_probe()).count();
     let alarms_today_valid = alarms
       .iter()
       .filter(|a| a.is_today_valid_unexpired_at(now_ms, &app.reminder_governance))
       .count();
     let alarms_globally_valid = alarms
       .iter()
       .filter(|a| a.is_valid_future_or_today_unexpired_at(now_ms, &app.reminder_governance))
       .count();
     let alarm_statuses = alarms
       .iter()
       .filter(|a| !a.is_e2e_probe())
       .map(|a| {
         let (badge, today_ok) = a.validity_status_label_at(now_ms, &app.reminder_governance);
         let global_ok = a.is_valid_future_or_today_unexpired_at(now_ms, &app.reminder_governance);
         serde_json::json!({
           "id": a.id,
           "title": a.display_title(),
           "badge": badge,
           "today_unexpired": today_ok,
           "globally_valid": global_ok,
         })
       })
       .collect::<Vec<_>>();
     serde_json::json!({
       "page_title": app.page.title(),
       "nav_y": nav_y_snapshot.clone(),
       "calendar_today_total": cal_today_total,
       "calendar_today_valid_unexpired": cal_today_valid,
       "calendar_upcoming_valid_total": cal_upcoming_valid,
       "alarms_total": alarms_total,
       "alarms_today_valid_unexpired": alarms_today_valid,
       "alarms_globally_valid": alarms_globally_valid,
       "alarm_statuses": alarm_statuses,
     })
   };
   let mut state=if home_review {
     serde_json::json!({
       "pid": std::process::id(), "seq": last.0, "ready": last.2,
       "mode": "home_review", "page": format!("{:?}", self.page),
       "demo": self.demo_mode, "connected": self.selected.is_some(),
       "status": self.status, "busy": self.busy, "running": self.workspace_busy(),
       "error": self.error, "scroll": self.scroll.scroll_start().as_f32(),
       "context": {
         "items": self.context.overview.items.len(),
         "background_running": self.context.running(),
         "background": self.context.background_diagnostics(),
         "evidence_count": self.context.overview.evidence_count,
         "updated_ms": self.context.overview.updated_ms,
         "periods": self.context.overview.periods.iter().map(|p| serde_json::json!({
           "kind": p.kind, "evidence_count": p.evidence_count,
         })).collect::<Vec<_>>(),
       },
       "home_state": build_home_state(self),
       "agent": self.agent_diagnostics(),
       "calendar_marks": if matches!(self.page, Page::Home | Page::Calendar) { Some(self.calendar_mark_diagnostics()) } else { None },
       "reminder_governance": build_reminder_gov_json(self),
       "calendar_alarms_v02103": build_calendar_alarms_v02103_json(self),
       "productivity": self.productivity_diagnostics(),
       "auto_sync": self.auto_sync.to_json(self.selected.is_some()),
       "connection_stability": connection_stability_json,
       "desktop_popup": desktop_popup_json,
       "call_audio": self.call_audio_diagnostics(),
       "microcraft": microcraft_json,
       "voice_input": self.voice_input_diagnostics(),
       "voice_capability": self.voice_capability_diagnostics(),
       "sms_state": sms_state_json,
       "desktop_native": desktop_native_json,
       "active_countdowns": self.active_home_countdown_timers(&self.push.snapshot).len(),
       "countdown_titles": self.active_home_countdown_timers(&self.push.snapshot).iter().map(|t| format!("{} · {}", t.title, t.remaining_label)).collect::<Vec<_>>(),
       "capsule_motion": self.capsule_motion.borrow().diagnostics(),
       "live_capsule_popover_open": self.back_screen.live_capsule_popover_open,
       "live_capsule_filter": format!("{:?}", self.back_screen.live_capsule_filter),
       "top_bar_capsules_count": top_bar_capsules_json.len(),
       "top_bar_capsules": top_bar_capsules_json,
       "active_phone_capsule": self.back_screen.active_phone_capsule.as_ref().map(|(tag, pill, title, _)| serde_json::json!({"tag": tag, "pill": pill, "title": title})),
       "effective_notif_count": self.effective_push_notifications().len(),
       "effective_notif_unread": self.unread_notifications(),
       "recent_files":self.recent_files_diagnostics(),"search":self.search_diagnostics(),
       "regions": regions,
     })
   } else { serde_json::json!({"pid":std::process::id(),"seq":last.0,"ready":last.2,"connected":self.selected.is_some(),"status":self.status,"busy":self.busy,"running":self.workspace_busy(),"error":self.workspace.error,"draft":self.fields[F_AMOO].text,"attachments":self.amoo_attachments.iter().map(|a|a.local_path.as_ref().map(|p|p.to_string_lossy())).collect::<Vec<_>>(),"mode":format!("{:?}",self.push.shell_mode),"intent":format!("{:?}",self.workspace.intent),"proposal":self.workspace.proposal.as_ref().map(|p|&p.text),"task":self.workspace_task(),"panel":self.workspace.panel,"history":self.workspace.history,"stream_deltas":self.workspace.stream_delta_count,"selected_output":self.workspace.selected_output,"preview_loading":self.work.local_preview_loading,"preview_error":self.work.local_preview_error,"context_menu":self.context_menu.as_ref().map(|m|format!("{:?}",m.target)),"cursor":self.fields[F_AMOO].cursor,"preedit":self.fields[F_AMOO].preedit,"preview_images":self.work.image_previews,"preview_errors":self.work.preview_errors,"messages":self.amoo_sessions.get(self.amoo_session_idx).map(|s|s.items.iter().map(|i|format!("{i:?}")).collect::<Vec<_>>()),"tasks":self.workspace.journal.tasks.iter().filter(|t|self.workspace_visible(t)).map(|t|serde_json::json!({"id":t.id,"title":t.title,"draft":t.draft})).collect::<Vec<_>>(),"work_status":self.work.status,"lab_status":self.studio_lab.status,"viewport":[self.workspace.viewport.x(),self.workspace.viewport.y()],"auto_sync":self.auto_sync.to_json(self.selected.is_some()),"connection_stability":connection_stability_json,"desktop_popup":desktop_popup_json,"call_audio":self.call_audio_diagnostics(),"microcraft":microcraft_json,"voice_input":self.voice_input_diagnostics(),"voice_capability":self.voice_capability_diagnostics(),"sms_state":sms_state_json,"recent_files":self.recent_files_diagnostics(),"search":self.search_diagnostics(),"regions":regions}) };
   state["release_story"] = release_story_json;
   state["battery"] = serde_json::json!({"reading":self.battery_reading(),"hint":self.battery_hint(),"display_level":self.cached_device_metrics().battery_level});
   let _=std::fs::write(dir.join("state.tmp"),state.to_string());let _=std::fs::rename(dir.join("state.tmp"),dir.join("state.json"));
   let Ok(bytes)=std::fs::read(dir.join("command.json"))else{return;};let Ok(v)=serde_json::from_slice::<serde_json::Value>(&bytes)else{return;};let seq=v["seq"].as_u64().unwrap_or(0);if seq<=last.0{return;}last.0=seq;
   let mut error=None;
   match v["op"].as_str().unwrap_or("") {
    "preview_battery" if self.demo_mode => {
      match serde_json::from_value::<stepx_companion::battery::BatteryState>(v["battery"].clone()) {
        Ok(reading) => if let Some(serial) = self.selected.as_ref().map(|d| d.serial.clone()) {
          self.observe_battery(&serial, &reading);
        },
        Err(e) => error = Some(e.to_string()),
      }
      self.desktop_update.whats_new_open = false;
      ctx.notify();
    },
    "browse_files" => {
      if let Some(path)=v["path"].as_str() {
        self.handle_action(&Action::Page(Page::Files),ctx);
        self.handle_action(&Action::Browse(path.to_owned()),ctx);
      } else { error=Some("path required".into()); }
    },
    "search_query" => { self.handle_action(&Action::GlobalSearch(SearchAction::Open),ctx); self.handle_action(&Action::GlobalSearch(SearchAction::Query(v["query"].as_str().unwrap_or("").into())),ctx); },
    "preview_capsule_motion" => {
      self.capsule_motion.borrow_mut().preview(v["enabled"].as_bool().unwrap_or(true));
      self.desktop_update.whats_new_open = false;
      self.back_screen.live_capsule_popover_open = false;
      ctx.notify();
    },
    "preview_calendar_marks" if self.demo_mode => {
      use chrono::{Datelike, TimeZone};
      let today = chrono::Local::now().date_naive();
      let event = |id, day, hour, title: &str, source: &str| {
        let start = chrono::Local.with_ymd_and_hms(today.year(), today.month(), day, hour, 0, 0)
          .single().unwrap().timestamp_millis();
        stepx_companion::toolbox::PhoneCalendarEvent {
          id, title: title.into(), dtstart: start, dtend: start + 3_600_000,
          source: source.into(), ..Default::default()
        }
      };
      self.desktop_update.whats_new_open = false;
      self.workspace.journal.tasks.clear();
      self.work.artifacts.clear();
      self.context.overview.items.clear();
      self.back_screen.stepos_suite.alarms.clear();
      self.back_screen.stepos_suite.calendar_events = vec![
        event(9001, 10, 9, "项目评审", "工作日历"),
        event(9002, 10, 11, "周会", "手机日历"),
        event(9003, 10, 18, "和家人聚餐", "个人日历"),
        event(9004, 11, 9, "客户会议", "手机日历"),
        event(9005, 12, 18, "瑜伽", "手机日历"),
        event(9006, 13, 12, "待定事项", "手机日历"),
      ];
      self.handle_action(&Action::BackScreen(BackScreenAction::CalendarSelectDate(today.year(), today.month(), 10)), ctx);
      ctx.notify();
    },
    "preview_fulfillment_capsules" if self.demo_mode => {
      use stepx_companion::push::PushNotification;
      let now = chrono::Local::now().timestamp_millis();
      let delivery = v["delivery"].as_str().unwrap_or("arrived");
      let fixture = |key: &str, package: &str, title: &str, body: &str, age: i64| PushNotification {
        key: key.into(), package_name: package.into(), app_name: title.into(), title: title.into(),
        text: body.into(), post_time_ms: now - age, active: true, ..Default::default()
      };
      self.desktop_update.whats_new_open = false;
      self.sms.clear();
      self.express_parcels.clear();
      self.push.snapshot = Default::default();
      self.back_screen.stepos_suite.calendar_events.clear();
      self.back_screen.stepos_suite.alarms.clear();
      self.push.snapshot.notifications = vec![
        fixture("fixture-other", "com.yaya.zone", "叮咚买菜", "订单号 80020345 骑手正在配送", 1_000),
        fixture("fixture-out-update", "com.sf.activity", "顺丰速运", "您的快件 SF123456789012 正在派件", 2_000),
        fixture("fixture-in", "com.sf.activity", "顺丰速运", "快件 SF223456789012 正在派件，请您签收", 3_000),
        fixture("fixture-out", "com.sf.activity", "顺丰速运", "您寄出的快件 SF123456789012 已揽收", 20_000),
        fixture("fixture-dingdong", "com.yaya.zone", "叮咚买菜", "订单号 80010345 骑手正在配送", 30_000),
      ];
      if delivery == "arrived" || delivery == "expired" {
        self.push.snapshot.notifications.insert(0, fixture(
          "fixture-arrival", "com.yaya.zone", "叮咚买菜", "商品已送达，订单号 80010345",
          if delivery == "expired" { 601_000 } else { 4_000 },
        ));
      }
      self.sync_notification_center();
      ctx.notify();
    },
    "preview_popup_arrival" if self.demo_mode => {
      // In-memory arrivals exercise the real queue; no phone or recipient I/O.
      let now = chrono::Local::now().timestamp_millis();
      let id = v["id"].as_str().unwrap_or("latest");
      if v["kind"].as_str() == Some("otp") {
        let code = v["code"].as_str().unwrap_or("735291");
        self.push.snapshot.otp_banners.push(stepx_companion::push::OtpBanner {
          code: code.into(), source_app: "短信".into(), title: "登录验证码".into(),
          raw_text: format!("验证码{code}，5分钟内有效"), timestamp_ms: now, ..Default::default()
        });
      } else {
        let packet = v["kind"].as_str() == Some("red_packet");
        self.push.snapshot.threads.push(stepx_companion::push::ImThread {
          thread_id: format!("popup-arrival:{id}"), package_name: "com.tencent.mm".into(),
          app_name: "微信".into(), title: if packet { "小林 · 家人群" } else { "新到的消息" }.into(),
          last_sender: "小林".into(), last_message: if packet { "[微信红包]恭喜发财，大吉大利" } else { "刚刚收到的新内容" }.into(),
          last_timestamp_ms: now, unread_count: 1, ..Default::default()
        });
      }
      self.sync_notification_center();
      ctx.notify();
    },
    "preview_recent_files" if self.demo_mode => {
      self.desktop_update.whats_new_open = false;
      if let Some(root) = v["root"].as_str() { self.preview_recent_files(std::path::PathBuf::from(root)); }
      ctx.notify();
    },
    "preview_notification_layout" if self.demo_mode => {
      // Local render fixture only: never read the phone or send to a recipient.
      let body = v["body"].as_str().unwrap_or("收到，谢谢").chars().take(200).collect::<String>();
      let now = chrono::Local::now().timestamp_millis();
      self.push.prefs.popup_mode = if v["hidden"].as_bool().unwrap_or(false) {
        stepx_companion::desktop_notify::PopupMode::Banner
      } else { stepx_companion::desktop_notify::PopupMode::FullReply };
      self.push.prefs.app_popup_modes.clear();
      self.push.prefs.scope = stepx_companion::desktop_notify::AlertScope::All;
      self.settings.notify_draft = self.push.prefs.clone();
      let thread_count = v["threads"].as_i64().unwrap_or(3).clamp(0, 20);
      self.push.snapshot.threads = (0..thread_count).map(|i| stepx_companion::push::ImThread {
        thread_id: format!("layout:{i}"), package_name: "com.stepx.ui.preview".into(),
        app_name: "本机预览".into(), title: format!("消息排版预览 {}", i+1),
        last_sender: "预览联系人".into(), last_message: body.clone(),
        last_timestamp_ms: now-i, unread_count: 2,
        can_remote_reply: v["reply"].as_bool().unwrap_or(false),
        messages: vec![stepx_companion::push::ImMessage {
          id: format!("layout-message:{i}"), text: body.clone(), timestamp_ms: now-i,
          sender: "预览联系人".into(), outgoing: false, ..Default::default()
        }], ..Default::default()
      }).collect();
      self.push.snapshot.notifications.clear();
      self.notification_center.popup_menu_item_id = None;
      self.notification_center.drawer_open = false;
      self.notification_center.read_ids.clear();
      self.notification_center.dismissed_ids.clear();
      self.notification_center.snooze_until_ms = None;
      self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::PopupAndBadge;
      self.desktop_update.whats_new_open = false;
      self.sync_notification_center();
      // Keep unrelated demo reminders out of the badge-count geometry check.
      if v.get("threads").is_some() {
        let active_ids: std::collections::HashSet<_> = self.push.snapshot.threads.iter()
          .map(|thread| format!("im-thread:{}", thread.thread_id)).collect();
        for item in &mut self.notification_center.items {
          if !active_ids.contains(&item.id) {
            item.unread = false;
            self.notification_center.read_ids.insert(item.id.clone());
          }
        }
      }
      if thread_count > 0 && v["popup"].as_bool().unwrap_or(true) {
        self.apply_notification_center_action(crate::ui::notification_center_state::NotificationCenterAction::PreviewItemPopup("im-thread:layout:0".into()));
      }
      self.sync_desktop_popup_window(ctx);
      ctx.notify();
    },
    "click"=>{
      let raw_id=v["id"].as_str().unwrap_or("");
      let resolve_id = |ctx: &ViewContext<Companion>, id: &str| {
        ctx.element_position_by_id(id.to_owned()).or_else(|| {
          if id == "home-toggle-edit-mode" {
            ctx.element_position_by_id("home-mod-bar-edit-toggle".to_owned())
              .or_else(|| ctx.element_position_by_id("home-mod-studio-done".to_owned()))
          } else {
            id.strip_prefix("home-mod-move-")
              .or_else(|| id.strip_prefix("home-click-move-"))
              .and_then(|slug| {
                ctx.element_position_by_id(format!("home-click-move-{slug}"))
                  .or_else(|| ctx.element_position_by_id(format!("home-drop-target-{slug}")))
              })
          }
        })
      };
      let use_popup = self.settings.hidden || (v["window"].as_str() == Some("popup"));
      if use_popup {
        if let (Some(r), Some(pwid)) = (self.desktop_popup.element_position_by_id(ctx, raw_id), self.desktop_popup.window_id()) {
          let position=r.origin()+r.size()/2.;
          events_for_window(ctx,pwid,vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false},Event::LeftMouseDown{position,modifiers:Default::default(),click_count:1,is_first_mouse:false},Event::LeftMouseUp{position,modifiers:Default::default()}]);
          self.desktop_popup.notify(ctx);
        } else if let Some(r)=resolve_id(ctx, raw_id) {
          let position=r.origin()+r.size()/2.;
          events(ctx,vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false},Event::LeftMouseDown{position,modifiers:Default::default(),click_count:1,is_first_mouse:false},Event::LeftMouseUp{position,modifiers:Default::default()}]);
        } else {
          error=Some(format!("painted target missing: {raw_id}"));
        }
      } else if let Some(r)=resolve_id(ctx, raw_id) {
        let position=r.origin()+r.size()/2.;
        events(ctx,vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false},Event::LeftMouseDown{position,modifiers:Default::default(),click_count:1,is_first_mouse:false},Event::LeftMouseUp{position,modifiers:Default::default()}]);
      } else if let (Some(r), Some(pwid)) = (self.desktop_popup.element_position_by_id(ctx, raw_id), self.desktop_popup.window_id()) {
        let position=r.origin()+r.size()/2.;
        events_for_window(ctx,pwid,vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false},Event::LeftMouseDown{position,modifiers:Default::default(),click_count:1,is_first_mouse:false},Event::LeftMouseUp{position,modifiers:Default::default()}]);
        self.desktop_popup.notify(ctx);
      } else {
        error=Some(format!("painted target missing: {raw_id}"));
      }
    },
    "configure_popup_probe"=>{
      // UI-capture only; does not persist preferences or synthesize incoming data.
      self.push.prefs.scope = stepx_companion::desktop_notify::AlertScope::All;
      self.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::Banner;
      self.push.prefs.app_popup_modes.insert("com.stepx.context.verification".into(),
        if v["full_reply"].as_bool().unwrap_or(false) {
          stepx_companion::desktop_notify::PopupMode::FullReply
        } else { stepx_companion::desktop_notify::PopupMode::Banner });
      self.push.prefs.privacy_mask = false;
      // Probe preferences are not an unsaved user edit. Never persist this override.
      self.settings.notify_draft = self.push.prefs.clone();
      self.notification_center.drawer_open = v["drawer_open"].as_bool().unwrap_or(false);
      self.sync_notification_center();
      self.sync_desktop_popup_window(ctx);
    },
    "inspect_conversation_layout"=>{
      let tid = v["thread_id"].as_str().unwrap_or("").to_string();
      if self.push.snapshot.threads.iter().any(|t| t.thread_id == tid) {
        self.handle_action(&Action::Page(Page::Sms), ctx);
        self.handle_action(&Action::Push(PushAction::SelectImThread(tid.clone())), ctx);
        if v["detail"].as_bool() == Some(true) {
          if let Some(t) = self.push.snapshot.threads.iter().find(|t| t.thread_id == tid && !t.messages.is_empty()) {
            let key = pages::sms::im_detail_key(t, t.messages.len()-1);
            self.handle_action(&Action::Push(PushAction::SelectNotification(key)), ctx);
          }
        } else { self.push.selected_key = None; }
        ctx.notify();
      } else { error = Some("conversation unavailable".into()); }
    },
    "snooze_for_layout_check"=>{
      // In-memory only; production preferences are untouched on restart.
      self.notification_center.snooze_until_ms = Some(chrono::Local::now().timestamp_millis() + 600_000);
      self.sync_desktop_popup_window(ctx);
      ctx.notify();
    },
    "set_main_hidden"=>{
      let hidden = v["hidden"].as_bool().unwrap_or(false);
      self.settings.hidden = hidden;
      if hidden {
        ctx.windows().hide_window(ctx.window_id());
      } else {
        ctx.windows().show_window_and_focus_app(ctx.window_id());
      }
      self.sync_desktop_popup_window(ctx);
    },
    "set_theme"=>{
      let dark = v["dark"].as_bool().unwrap_or(false);
      self.theme_mode = if dark { theme::ThemeMode::Dark } else { theme::ThemeMode::Light };
      self.palette.set(theme::Palette::for_theme(dark));
      self.sync_desktop_popup_window(ctx);
    },
    "demo_stream_bloom"=>{
      self.page = Page::Amoo;
      self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
      self.notification_center.active_popup_id = None;
      self.notification_center.popup_queue.clear();
      self.sync_desktop_popup_window(ctx);
      let prefix = v["prefix"].as_str().unwrap_or(
        "已为您完成 Claude Desktop 级丝滑微交互改造：\n\n1. **暗色视网膜光渗字重补偿（460 / 560）**：白字黑底在视网膜光渗效应下会产生光晕发散，字重从 500 / 600 减至 460 / 560 后，深浅两个主题下看起来粗细完全一致。\n2. **正文绝不让一个字单独占一行**：排版引擎自动检测段末单字孤行，将倒数第二个词拉下来一起换行。"
      );
      let delta = v["delta"].as_str().unwrap_or(
        "\n3. **回答的字是一个个「晕开」出来的**：每个字从柔和虚化到清晰凝实只要 90ms，搭配三次贝塞尔过渡与同心圆角咬合，视觉极为丝滑。"
      );
      let full_text = format!("{prefix}{delta}");
      if self.workspace.active.is_none() || self.workspace.journal.tasks.is_empty() {
        let mut t = stepx_companion::amoo_workspace::Task::new(None, None);
        t.title = "Claude Desktop 微交互改造评估与实机演示".into();
        self.workspace.active = Some(t.id.clone());
        self.workspace.journal.tasks.push(t);
      }
      if let Some(task_id) = self.workspace.active.clone() {
        if let Some(task) = self.workspace.journal.tasks.iter_mut().find(|t| t.id == task_id) {
          task.title = "Claude Desktop 微交互改造评估与实机演示".into();
          task.session = None;
          task.turns.clear();
          task.turns.push(stepx_companion::amoo_workspace::Turn {
            at_ms: Some(chrono::Utc::now().timestamp_millis() - 1000),
            user: true,
            text: "请展示 Claude Desktop 前端微交互改造效果：1. 暗色 460/560 字重光渗补偿 2. 正文防孤字换行 3. 90ms 逐字柔雾晕开 4. 贝塞尔过渡与同心圆角咬合".into(),
          });
          task.turns.push(stepx_companion::amoo_workspace::Turn {
            at_ms: Some(chrono::Utc::now().timestamp_millis()),
            user: false,
            text: full_text.clone(),
          });
          let motion_key = format!("workspace-turn-{}-1", task.id);
          if let Some(elapsed_ms) = v["elapsed_ms"].as_f64() {
            self.reply_motion.sync_delta_with_elapsed(&motion_key, prefix, &full_text, elapsed_ms as f32);
          } else {
            self.reply_motion.sync_delta(&motion_key, prefix, &full_text, true);
          }
        }
      }
      if let Some(cap_name) = v["capture_name"].as_str() {
        let mut path = dir.join(cap_name);
        path.set_extension("png");
        if let Some(pw) = ctx.windows().platform_window(ctx.window_id()) {
          pw.as_ctx().request_frame_capture(Box::new(move|f|{let _=image::save_buffer(path,&f.data,f.width,f.height,image::ColorType::Rgba8);}));
        }
      }
      self.schedule_tick(ctx);
    },
    "demo_sms_wechat_setup"=>{
      self.page = Page::Sms;
      self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
      self.notification_center.active_popup_id = None;
      self.notification_center.popup_queue.clear();
      self.sync_desktop_popup_window(ctx);
      let now_ms = chrono::Utc::now().timestamp_millis();
      let wechat_threads = [
        ("Builder One • STEPX探索基地", true, "陈工", "下午 3 点评审新版桌面端通知与微信会话交互，大家准时参加。"),
        ("云啊", false, "云啊", "今天天气不错，周末去爬山吗？"),
        ("我是照相机先生 · step x neo", false, "我是照相机先生", "真机无线直连同步测试已经全部跑通了！"),
        ("朱凯凯", false, "朱凯凯", "收到，我把设计稿发你微信了。"),
        ("导入与导出", true, "林经理", "本周需要导出的表格已经整理完毕。"),
        ("软路由", true, "张工", "内网 ADB 端口转发稳定在 42041。"),
        ("微信支付", false, "微信支付", "微信支付收款 28.50 元（来自早餐铺）"),
        ("霸王茶姬", false, "霸王茶姬", "您的伯牙绝弦已制作完成，请凭取餐码 8291 取茶。"),
      ];
      self.push.snapshot.threads.clear();
      self.push.snapshot.otp_banners.clear();
      self.push.snapshot.notifications.clear();
      for (idx, (title, is_group, sender, msg)) in wechat_threads.iter().enumerate() {
        let tid = format!("com.tencent.mm::{title}");
        let ts = now_ms - (idx as i64 * 180_000);
        self.push.snapshot.threads.push(stepx_companion::push::ImThread {
          thread_id: tid,
          package_name: "com.tencent.mm".to_string(),
          app_name: "微信".to_string(),
          title: (*title).to_string(),
          is_group: *is_group,
          unread_count: 0,
          last_sender: (*sender).to_string(),
          last_message: (*msg).to_string(),
          last_timestamp_ms: ts,
          can_remote_reply: true,
          can_ui_reply: true,
          has_content_intent: true,
          notification_key: format!("wx-key-{idx}"),
          messages: vec![
            stepx_companion::push::ImMessage {
              id: format!("m-{idx}-1"),
              sender: (*sender).to_string(),
              text: (*msg).to_string(),
              timestamp_ms: ts - 30_000,
              observed_at_ms: ts - 30_000,
              time_known: Some(true),
              outgoing: false,
              delivery_status: "delivered".to_string(),
              ..Default::default()
            },
            stepx_companion::push::ImMessage {
              id: format!("m-{idx}-2"),
              sender: "我".to_string(),
              text: format!("好的，已收到「{title}」的消息。"),
              timestamp_ms: ts,
              observed_at_ms: ts,
              time_known: Some(true),
              outgoing: true,
              delivery_status: "delivered".to_string(),
              ..Default::default()
            },
          ],
          ..Default::default()
        });
      }
      self.sms = vec![
        stepx_companion::sms::Message {
          provider_id: Some(101),
          address: "+8613322988887".to_string(),
          body: "test".to_string(),
          date_ms: now_ms - 86_400_000,
          outgoing: false,
          kind: 1,
          read: true,
          subscription: Some(1),
        },
        stepx_companion::sms::Message {
          provider_id: Some(102),
          address: "+8613322988887".to_string(),
          body: "已收到您的测试消息，正在会议中稍后联系。".to_string(),
          date_ms: now_ms - 86_000_000,
          outgoing: true,
          kind: 2,
          read: true,
          subscription: Some(1),
        },
      ];
      self.sms_loaded = true;
      self.push.snapshot.notifications = vec![
        stepx_companion::push::PushNotification {
          key: "notif-wx-1".to_string(),
          seq_id: 1,
          package_name: "com.tencent.mm".to_string(),
          app_name: "微信".to_string(),
          category: "im".to_string(),
          title: "朱凯凯".to_string(),
          sender: "朱凯凯".to_string(),
          text: "收到，我把设计稿发你微信了。".to_string(),
          post_time_ms: now_ms - 60_000,
          is_clearable: true,
          active: true,
          read: false,
          can_reply: true,
          thread_id: "com.tencent.mm::朱凯凯".to_string(),
          ..Default::default()
        },
        stepx_companion::push::PushNotification {
          key: "notif-otp-2".to_string(),
          seq_id: 2,
          package_name: "com.android.mms".to_string(),
          app_name: "短信".to_string(),
          category: "system".to_string(),
          title: "10698000".to_string(),
          sender: "10698000".to_string(),
          text: "【StepX云】您的登录验证码为 684920，5分钟内有效，请勿泄露。".to_string(),
          post_time_ms: now_ms - 120_000,
          is_clearable: true,
          active: true,
          read: false,
          otp_code: Some("684920".to_string()),
          ..Default::default()
        },
        stepx_companion::push::PushNotification {
          key: "notif-express-3".to_string(),
          seq_id: 3,
          package_name: "com.cainiao.wireless".to_string(),
          app_name: "菜鸟".to_string(),
          category: "delivery_travel".to_string(),
          title: "菜鸟驿站取件通知".to_string(),
          sender: "菜鸟驿站".to_string(),
          text: "您的顺丰包裹已到达阳光花园驿站，请凭取件码 3-2-1088 前往领取。".to_string(),
          post_time_ms: now_ms - 300_000,
          is_clearable: true,
          active: true,
          read: false,
          ..Default::default()
        },
        stepx_companion::push::PushNotification {
          key: "notif-bill-4".to_string(),
          seq_id: 4,
          package_name: "com.eg.android.AlipayGphone".to_string(),
          app_name: "支付宝".to_string(),
          category: "finance".to_string(),
          title: "交易提醒".to_string(),
          sender: "支付宝".to_string(),
          text: "您有一笔 28.50 元的餐饮消费已完成支付。".to_string(),
          post_time_ms: now_ms - 900_000,
          is_clearable: true,
          active: true,
          read: true,
          ..Default::default()
        },
      ];
      self.push.im_channel = stepx_companion::push::ImChannel::All;
      self.push.selected_im_thread = Some("com.tencent.mm::Builder One • STEPX探索基地".to_string());
      self.selected_thread = None;
      self.schedule_tick(ctx);
    },
    "hover"=>{
      let id=v["id"].as_str().unwrap_or("");
      let popup = v["window"].as_str() == Some("popup");
      let rect = if popup { self.desktop_popup.element_position_by_id(ctx, id) } else { ctx.element_position_by_id(id.to_owned()) };
      if let Some(r)=rect {
        let position=r.origin()+Vector2F::new(r.width()*0.5, r.height()*0.5);
        let input = vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false}];
        if popup {
          if let Some(wid) = self.desktop_popup.window_id() { events_for_window(ctx, wid, input); }
          else { error=Some("popup window is not available".into()); }
        } else { events(ctx,input); }
      } else if popup && v["x"].is_number() && v["y"].is_number() {
        if let Some(wid) = self.desktop_popup.window_id() {
          let size = self.desktop_popup.bounds_array();
          match checked_position(v["x"].as_f64(), v["y"].as_f64(), size[2] as f64, size[3] as f64) {
            Ok(position) => events_for_window(ctx, wid, vec![Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false}]),
            Err(message) => error=Some(message.into()),
          }
        }
      } else {
        error=Some(format!("painted target missing: {id}"));
      }
    },
    "double_click"=>{
      let id=v["id"].as_str().unwrap_or("");
      if let Some(r)=ctx.element_position_by_id(id.to_owned()) {
        let position=r.origin()+r.size()/2.;
        events(ctx,vec![
          Event::MouseMoved{position,cmd:false,shift:false,is_synthetic:false},
          Event::LeftMouseDown{position,modifiers:Default::default(),click_count:2,is_first_mouse:false},
          Event::LeftMouseUp{position,modifiers:Default::default()},
        ]);
      } else {
        error=Some(format!("painted target missing: {id}"));
      }
    },
    "demo_desktop_native_action"=>{
      self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
      self.notification_center.active_popup_id = None;
      self.notification_center.popup_queue.clear();
      self.sync_desktop_popup_window(ctx);
      match v["step"].as_str().or_else(|| v["action"].as_str()).unwrap_or("") {
        "open_overview" => {
          self.handle_action(&Action::Page(Page::Overview), ctx);
        }
        "open_home" => {
          self.handle_action(&Action::Page(Page::Home), ctx);
        }
        "toggle_home_tools" => {
          self.handle_action(&Action::HomeToggleTools, ctx);
        }
        "open_sms" => {
          self.handle_action(&Action::Page(Page::Sms), ctx);
        }
        "open_sms_wechat" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::WeChat;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::WeChat);
          if let Some(hint) = v["thread_hint"].as_str() {
            if let Some(found_id) = self.push.snapshot.threads.iter()
              .find(|t| t.title.contains(hint) || t.last_message.contains(hint))
              .map(|t| t.thread_id.clone())
            {
              self.push.selected_im_thread = Some(found_id);
              self.selected_thread = None;
            } else if !hint.trim().is_empty() {
              let tid = format!("com.tencent.mm::{}", hint.trim());
              self.push.snapshot.threads.insert(0, stepx_companion::push::ImThread {
                thread_id: tid.clone(),
                package_name: "com.tencent.mm".to_string(),
                app_name: "微信".to_string(),
                title: hint.trim().to_string(),
                is_group: false,
                unread_count: 0,
                last_sender: hint.trim().to_string(),
                last_message: "StepX 桌面伴侣连接就绪".to_string(),
                last_timestamp_ms: 1793520000000,
                can_remote_reply: false,
                can_ui_reply: true,
                has_content_intent: false,
                notification_key: String::new(),
                messages: Vec::new(),
                ..Default::default()
              });
              self.push.selected_im_thread = Some(tid);
              self.selected_thread = None;
            }
          }
          if self.back_screen.gallery_items.is_empty() {
            self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
          }
        }
        "send_im_reply_text" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          if let Some(hint) = v["thread_hint"].as_str() {
            if let Some(found_id) = self.push.snapshot.threads.iter()
              .find(|t| t.title.contains(hint) || t.last_message.contains(hint))
              .map(|t| t.thread_id.clone())
            {
              self.push.selected_im_thread = Some(found_id);
              self.selected_thread = None;
            } else if !hint.trim().is_empty() {
              let tid = format!("com.tencent.mm::{}", hint.trim());
              self.push.snapshot.threads.insert(0, stepx_companion::push::ImThread {
                thread_id: tid.clone(),
                package_name: "com.tencent.mm".to_string(),
                app_name: "微信".to_string(),
                title: hint.trim().to_string(),
                is_group: false,
                unread_count: 0,
                last_sender: hint.trim().to_string(),
                last_message: "StepX 桌面伴侣连接就绪".to_string(),
                last_timestamp_ms: 1793520000000,
                can_remote_reply: false,
                can_ui_reply: true,
                has_content_intent: false,
                notification_key: String::new(),
                messages: Vec::new(),
                ..Default::default()
              });
              self.push.selected_im_thread = Some(tid);
              self.selected_thread = None;
            }
          }
          if let Some(text) = v["text"].as_str() {
            self.fields[F_IM_REPLY].set(text);
            self.handle_push_action(&PushAction::SendImReply, ctx);
          }
        }
        "trigger_quote_reply" => {
          let is_im = v["is_im"].as_bool().unwrap_or(true);
          let sender = v["sender"].as_str().unwrap_or("文件传输助手").to_string();
          let text = v["text"].as_str().unwrap_or("稍后同步进展").to_string();
          if let Some(draft) = v["draft"].as_str() {
            if is_im {
              self.fields[F_IM_REPLY].set(draft);
            } else {
              self.fields[F_SMS_BODY].set(draft);
            }
          }
          self.handle_push_action(&PushAction::QuoteReply { is_im, sender, text }, ctx);
        }
        "open_notifications_tab" => {
          self.page = Page::Notifications;
          self.push.tab = crate::ui::push_state::PushCenterTab::Notifications;
          self.push.notif_category = v["category"].as_u64().unwrap_or(0) as u8;
          let target_key = "0|com.tencent.mm|4099|filehelper|10220".to_string();
          if !self.push.snapshot.notifications.iter().any(|n| n.key == target_key) {
            self.push.snapshot.notifications.insert(
              0,
              stepx_companion::push::PushNotification {
                key: target_key.clone(),
                seq_id: 999,
                package_name: "com.tencent.mm".to_string(),
                app_name: "微信".to_string(),
                category: "im".to_string(),
                title: "文件传输助手".to_string(),
                sender: "文件传输助手".to_string(),
                text: "跨设备文件与后台消息同步通道已就绪，可直接在此快捷回复".to_string(),
                sub_text: "刚刚 · 微信桌面互联".to_string(),
                post_time_ms: 1793526000000,
                is_ongoing: false,
                is_clearable: true,
                active: true,
                removed_time_ms: 0,
                is_group: false,
                dismissed: false,
                read: false,
                can_reply: true,
                thread_id: "com.tencent.mm::文件传输助手".to_string(),
                otp_code: None,
                action_labels: vec!["回复".to_string(), "标为已读".to_string()],
                url_hint: None,
                ..Default::default()
              },
            );
          }
          self.push.selected_key = Some(target_key.clone());
          self.push.replying_key = Some(target_key);
          if let Some(draft) = v["draft"].as_str() {
            self.fields[F_PUSH_REPLY].set(draft);
          }
        }
        "open_sms_direct_channel" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::Sms;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::Sms);
          if let Some(first_addr) = self.sms_threads().first().map(|t| t.address.clone()) {
            self.selected_thread = Some(first_addr);
            self.push.selected_im_thread = None;
          }
          if let Some(draft) = v["draft"].as_str() {
            self.fields[F_SMS_BODY].set(draft);
          }
        }
        "prepare_v0297_wechat_threads" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::WeChat;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::WeChat);
          let now_ms = chrono::Local::now().timestamp_millis();
          let ensure_thread = |threads: &mut Vec<stepx_companion::push::ImThread>, tid: &str, title: &str, is_group: bool, unread: usize, ts: i64, msgs: Vec<stepx_companion::push::ImMessage>| {
            if let Some(existing) = threads.iter_mut().find(|t| t.thread_id == tid || t.title == title) {
              existing.thread_id = tid.to_string();
              existing.title = title.to_string();
              existing.unread_count = unread;
              existing.last_timestamp_ms = ts;
              existing.messages = msgs;
              stepx_companion::push::recompute_thread_summary(existing);
            } else {
              let mut t = stepx_companion::push::ImThread {
                thread_id: tid.to_string(),
                package_name: "com.tencent.mm".to_string(),
                app_name: "微信".to_string(),
                title: title.to_string(),
                is_group,
                unread_count: unread,
                last_timestamp_ms: ts,
                can_remote_reply: true,
                can_ui_reply: true,
                messages: msgs,
                ..Default::default()
              };
              stepx_companion::push::recompute_thread_summary(&mut t);
              threads.push(t);
            }
          };
          ensure_thread(
            &mut self.push.snapshot.threads,
            "com.tencent.mm::产品研发群",
            "产品研发群",
            true,
            2,
            now_ms - 60_000,
            vec![
              stepx_companion::push::ImMessage {
                id: "rd-1".into(),
                sender: "邱-stepx".into(),
                text: "v0.2.97 消息与通知的置顶、删除对话和已发消息同步今天一起合入。".into(),
                timestamp_ms: now_ms - 120_000,
                observed_at_ms: now_ms - 120_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
              stepx_companion::push::ImMessage {
                id: "rd-2".into(),
                sender: "窝-StepX".into(),
                text: "收到，测试完文件传输助手和短信通道就发版！".into(),
                timestamp_ms: now_ms - 60_000,
                observed_at_ms: now_ms - 60_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
          );
          ensure_thread(
            &mut self.push.snapshot.threads,
            "com.tencent.mm::文件传输助手",
            "文件传输助手",
            false,
            0,
            now_ms - 20_000,
            vec![
              stepx_companion::push::ImMessage {
                id: "fh-1".into(),
                sender: "文件传输助手".into(),
                text: "跨设备文件与后台消息同步通道已就绪，请核对最新发送记录。".into(),
                timestamp_ms: now_ms - 300_000,
                observed_at_ms: now_ms - 300_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
              stepx_companion::push::ImMessage {
                id: "fh-2".into(),
                sender: "文件传输助手".into(),
                text: "这是一条待清理的旧测试草稿记录（可单条删除）。".into(),
                timestamp_ms: now_ms - 180_000,
                observed_at_ms: now_ms - 180_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
              stepx_companion::push::ImMessage {
                id: "fh-out-1".into(),
                sender: "我".into(),
                text: "我刚在桌面端发送的回复消息，已实时显示并持久化保存！".into(),
                timestamp_ms: now_ms - 20_000,
                observed_at_ms: now_ms - 20_000,
                time_known: Some(true),
                outgoing: true,
                delivery_status: "sent".into(),
                via: "remote_input".into(),
                client_msg_id: "stepx-out-fh-1".into(),
                ..Default::default()
              },
            ],
          );
          ensure_thread(
            &mut self.push.snapshot.threads,
            "com.tencent.mm::已删除旧联系人",
            "已删除旧联系人",
            false,
            1,
            now_ms - 240_000,
            vec![
              stepx_companion::push::ImMessage {
                id: "del-user-1".into(),
                sender: "已删除旧联系人".into(),
                text: "这条已删除用户的残留会话即将被一键删除并同步清理。".into(),
                timestamp_ms: now_ms - 240_000,
                observed_at_ms: now_ms - 240_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
          );
          crate::ui::push_state::apply_push_preferences_to_snapshot(
            &mut self.push.snapshot,
            &mut self.push.prefs,
          );
          if v["select_deleted_candidate"].as_bool() == Some(true) {
            self.push.selected_im_thread = Some("com.tencent.mm::已删除旧联系人".into());
          } else {
            self.push.selected_im_thread = Some("com.tencent.mm::文件传输助手".into());
          }
          self.selected_thread = None;
          self.fields[F_IM_REPLY].set("继续同步跟进文件传输助手");
        }
        "simulate_v0297_post_delete_sync" => {
          let now_ms = chrono::Local::now().timestamp_millis();
          let mut incoming_snapshot = self.push.snapshot.clone();
          incoming_snapshot.threads.push(stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::已删除旧联系人".to_string(),
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            title: "已删除旧联系人".to_string(),
            is_group: false,
            unread_count: 1,
            last_timestamp_ms: now_ms - 240_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "del-user-1".into(),
                sender: "已删除旧联系人".into(),
                text: "这条已删除用户的残留会话即将被一键删除并同步清理。".into(),
                timestamp_ms: now_ms - 240_000,
                observed_at_ms: now_ms - 240_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          });
          stepx_companion::push::merge_local_replies(
            &mut incoming_snapshot,
            &self.push.snapshot,
          );
          crate::ui::push_state::apply_push_preferences_to_snapshot(
            &mut incoming_snapshot,
            &mut self.push.prefs,
          );
          self.push.snapshot = incoming_snapshot;
          self.push.selected_im_thread = Some("com.tencent.mm::文件传输助手".into());
        }
        "prepare_v0297_sms_outgoing" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::Sms;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::Sms);
          let target_addr = self.sms_threads().first().map(|t| t.address.clone()).unwrap_or_else(|| "1065599763".into());
          let now_ms = chrono::Local::now().timestamp_millis();
          if !self.sms.iter().any(|m| m.address == target_addr && !m.outgoing) {
            self.sms.push(stepx_companion::sms::Message {
              provider_id: Some(999_000),
              address: target_addr.clone(),
              body: "【中国联通】尊敬的客户，您本月的 5G 畅爽流量包可免费升级，回复短信即可直接确认办理。".into(),
              date_ms: now_ms - 120_000,
              outgoing: false,
              kind: 1,
              read: true,
              subscription: None,
            });
          }
          let sent_msg = stepx_companion::sms::Message {
            provider_id: Some(999_001),
            address: target_addr.clone(),
            body: "已收到套餐通知，确认办理流量包（桌面端直接回复并持久显示）".into(),
            date_ms: now_ms,
            outgoing: true,
            kind: 2,
            read: true,
            subscription: None,
          };
          self.local_sent_sms.retain(|m| m.provider_id != Some(999_001));
          self.local_sent_sms.push(sent_msg.clone());
          self.sms.retain(|m| m.provider_id != Some(999_001));
          self.sms.insert(0, sent_msg);
          self.sms_loaded = true;
          self.selected_thread = Some(target_addr.clone());
          self.push.selected_im_thread = Some(String::new());
          let pin_key = stepx_companion::desktop_notify::NotifyPreferences::sms_target_key(&target_addr);
          self.push.prefs.pinned_targets.insert(pin_key);
          self.fields[F_SMS_BODY].set("好的，已知悉详情");
        }
        "open_sms_all_with_unread" => {
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::All;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::All);
          self.read_sms_addresses.clear();
          let now_ms = chrono::Local::now().timestamp_millis();
          if let Some(first_sms) = self.sms.first_mut() {
            first_sms.read = false;
            first_sms.kind = 1;
            if first_sms.date_ms <= 0 || now_ms - first_sms.date_ms > 3600_000 {
              first_sms.date_ms = now_ms - 180_000;
            }
          }
          if let Some(first_addr) = self.sms_threads().first().map(|t| t.address.clone()) {
            self.selected_thread = Some(first_addr);
            self.push.selected_im_thread = None;
          }
        }
        "prepare_v0298_active_pickup_and_ghost_sources" => {
          self.demo_mode = false;
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.call_logs.clear();
          self.home_modules.enabled = vec![
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleId::ContextLedgerHub,
            pages::home::HomeModuleId::CalendarPanorama,
          ];
          self.home_modules.preset = None;
          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();
          self.push.prefs.dismissed_pickup_codes.clear();
          self.push.prefs.deleted_threads.clear();
          self.push.prefs.cleared_threads.clear();
          self.push.prefs.deleted_messages.clear();
          self.read_sms_addresses.clear();
          self.local_sent_sms.clear();
          self.back_screen.stepos_suite.phone_clipboard.clear();
          self.back_screen.stepos_suite.alarms.clear();

          self.sms = vec![
            stepx_companion::sms::Message {
              provider_id: Some(888_101),
              address: "106881517503453".into(),
              body: "【丰巢】凭取件码79983898至金隅万科城丰巢快递柜取包裹。".into(),
              date_ms: now_ms - 120_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(888_102),
              address: "95338".into(),
              body: "【顺丰速运】您的快件SF1628890123456已签收，感谢使用顺丰。".into(),
              date_ms: now_ms - 180_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(888_103),
              address: "10086".into(),
              body: "【中国移动】尊敬的客户，您本月5G套餐余量充足，账单正常。".into(),
              date_ms: now_ms - 600_000,
              outgoing: false,
              kind: 1,
              read: true,
              subscription: None,
            },
          ];
          self.sms_loaded = true;

          let mut wsm_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::王司马".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "王司马".into(),
            is_group: false,
            unread_count: 1,
            last_timestamp_ms: now_ms - 90_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "wsm-1".into(),
                sender: "王司马".into(),
                text: "下午3点前请确认丰巢包裹和接口联调排期？".into(),
                timestamp_ms: now_ms - 90_000,
                observed_at_ms: now_ms - 90_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut wsm_thread);

          let mut rd_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::产品研发群".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "产品研发群".into(),
            is_group: true,
            unread_count: 1,
            last_timestamp_ms: now_ms - 60_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "rd-v0298-1".into(),
                sender: "邱-stepx".into(),
                text: "今天完成 v0.2.98 桌面与手机状态同步验收，准备发版。".into(),
                timestamp_ms: now_ms - 60_000,
                observed_at_ms: now_ms - 60_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut rd_thread);

          self.push.snapshot.threads = vec![rd_thread, wsm_thread];
          self.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
              key: "notif-fc-79983898".into(),
              package_name: "com.android.mms".into(),
              app_name: "短信".into(),
              title: "106881517503453".into(),
              text: "【丰巢】凭取件码79983898至金隅万科城丰巢快递柜取包裹。".into(),
              post_time_ms: now_ms - 120_000,
              read: false,
              dismissed: false,
              ..Default::default()
            },
          ];
          self.push.snapshot.otp_banners = vec![
            stepx_companion::push::OtpBanner {
              notification_key: "notif-fc-79983898".into(),
              code: "79983898".into(),
              source_app: "丰巢".into(),
              title: "106881517503453".into(),
              raw_text: "【丰巢】凭取件码79983898至金隅万科城丰巢快递柜取包裹。".into(),
              timestamp_ms: now_ms - 120_000,
            },
          ];

          self.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 80,
              title: "10月27日历史复盘周会".into(),
              dtstart: 1_761_530_400_000, // 2026-10-27 10:00 CST (past date!)
              dtend: 1_761_534_000_000,
              location: "3F 会议室".into(),
              description: "已过期的上周历史日程，不应进入桌面顶栏实时胶囊".into(),
              all_day: false,
              source: String::new(),
            },
          ];

          let ledger_dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("stepx-v0298-ledger");
          let device_serial = self.selected.as_ref().map(|d| d.serial.clone()).unwrap_or_else(|| "STEPX_V0298_DEVICE".into());
          let capture = stepx_companion::context_ledger::Capture {
            device: device_serial,
            account: None,
            connected: true,
            now_ms,
            utc_offset_seconds: now.offset().local_minus_utc(),
            tasks: vec![],
            push: Some(self.push.snapshot.clone()),
            sms: Some(self.sms.clone()),
            calendar: Some(self.back_screen.stepos_suite.calendar_events.clone()),
            ..Default::default()
          };
          if let Ok(overview) = stepx_companion::context_ledger::process(&ledger_dir, &capture) {
            self.context.overview = overview;
          }

          self.sync_demo_auto_sync_domains();
          self.expansion.snapshot.error = false;
          self.amoo_ui.set_account_scope(Some("stepx-user".into()));
          self.status = "StepX v0.2.98 · 手机与桌面状态实时同步正常".into();
          self.error = false;
          self.sync_live_express_parcels();
          self.sync_notification_center();

          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::All;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::All);
          self.selected_thread = Some("106881517503453".into());
          self.push.selected_im_thread = None;
        }
        "open_v0298_home_page" => {
          self.page = Page::Home;
          self.scroll = Default::default();
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.sync_demo_auto_sync_domains();
          self.amoo_ui.set_account_scope(Some("stepx-user".into()));
          self.status = "StepX v0.2.98 · 手机与桌面状态实时同步正常".into();
          self.error = false;
          self.sync_live_express_parcels();
          self.sync_notification_center();
        }
        "simulate_v0298_phone_deleted_and_dismissed_sync" => {
          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();

          let ledger_dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("stepx-v0298-ledger");
          let device_serial = self.selected.as_ref().map(|d| d.serial.clone()).unwrap_or_else(|| "STEPX_V0298_DEVICE".into());

          // First record a today calendar event "王司马接口联调评审会" in ContextLedger before phone deletion:
          let pre_delete_cal = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 80,
              title: "10月27日历史复盘周会".into(),
              dtstart: 1_761_530_400_000,
              dtend: 1_761_534_000_000,
              location: "3F 会议室".into(),
              description: "已过期的上周历史日程".into(),
              all_day: false,
              source: String::new(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 298,
              title: "王司马接口联调评审会".into(),
              dtstart: now_ms + 10_800_000,
              dtend: now_ms + 14_400_000,
              location: "线上会议".into(),
              description: "与王司马确认接口排期".into(),
              all_day: false,
              source: String::new(),
            },
          ];
          let _ = stepx_companion::context_ledger::process(
            &ledger_dir,
            &stepx_companion::context_ledger::Capture {
              device: device_serial.clone(),
              account: None,
              connected: true,
              now_ms: now_ms + 500,
              utc_offset_seconds: now.offset().local_minus_utc(),
              tasks: vec![],
              push: Some(self.push.snapshot.clone()),
              sms: Some(self.sms.clone()),
              calendar: Some(pre_delete_cal),
              ..Default::default()
            },
          );

          // 1. User deleted the 丰巢 pickup SMS & completed SF SMS on phone; only regular 10086 remains.
          self.sms.retain(|m| m.address == "10086");

          // 2. User deleted "王司马" thread on phone (recorded in deleted_threads tombstones).
          let wsm_key1 = stepx_companion::desktop_notify::NotifyPreferences::im_target_key(
            "com.tencent.mm::王司马",
          );
          let wsm_key2 = stepx_companion::desktop_notify::NotifyPreferences::im_target_key(
            "王司马",
          );
          self.push.prefs.deleted_threads.insert(wsm_key1, now_ms);
          self.push.prefs.deleted_threads.insert(wsm_key2, now_ms);

          // 3. Incoming snapshot from phone: notification is dismissed, otp_banners is empty,
          //    and a legacy zombie `com.android.mms::106881517503453` + stale `王司马` thread attempt to sneak in.
          let mut incoming_snapshot = self.push.snapshot.clone();
          for n in &mut incoming_snapshot.notifications {
            if n.key == "notif-fc-79983898" || n.text.contains("79983898") {
              n.dismissed = true;
              n.read = true;
            }
          }
          incoming_snapshot.otp_banners.clear();
          incoming_snapshot.threads.push(stepx_companion::push::ImThread {
            thread_id: "com.android.mms::106881517503453".into(),
            package_name: "com.android.mms".into(),
            app_name: "短信".into(),
            title: "106881517503453".into(),
            is_group: false,
            unread_count: 1,
            last_timestamp_ms: now_ms - 120_000,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "zombie-mms-1".into(),
                sender: "106881517503453".into(),
                text: "【丰巢】凭取件码79983898至金隅万科城丰巢快递柜取包裹。".into(),
                timestamp_ms: now_ms - 120_000,
                observed_at_ms: now_ms - 120_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          });

          stepx_companion::push::merge_local_replies(
            &mut incoming_snapshot,
            &self.push.snapshot,
          );
          crate::ui::push_state::apply_push_preferences_to_snapshot(
            &mut incoming_snapshot,
            &mut self.push.prefs,
          );
          self.push.snapshot = incoming_snapshot;

          // 4. Phone calendar event "王司马接口联调评审会" & expired event deleted on phone.
          self.back_screen.stepos_suite.calendar_events.clear();

          // 5. Re-run ContextLedger & live parcel extraction & NotificationCenter sync.
          let capture = stepx_companion::context_ledger::Capture {
            device: device_serial,
            account: None,
            connected: true,
            now_ms: now_ms + 1_000,
            utc_offset_seconds: now.offset().local_minus_utc(),
            tasks: vec![],
            push: Some(self.push.snapshot.clone()),
            sms: Some(self.sms.clone()),
            calendar: Some(self.back_screen.stepos_suite.calendar_events.clone()),
            ..Default::default()
          };
          if let Ok(overview) = stepx_companion::context_ledger::process(&ledger_dir, &capture) {
            self.context.overview = overview;
          }

          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.sync_demo_auto_sync_domains();
          self.amoo_ui.set_account_scope(Some("stepx-user".into()));
          self.status = "StepX v0.2.98 · 手机与桌面状态实时同步正常".into();
          self.error = false;
          self.sync_live_express_parcels();
          self.sync_notification_center();

          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::All;
          self.apply_select_im_channel(stepx_companion::push::ImChannel::All);
          self.selected_thread = None;
          self.push.selected_im_thread = Some("com.tencent.mm::产品研发群".into());
        }
        "open_phone_tab" => {
          self.handle_action(&Action::Page(Page::Phone), ctx);
        }
        "open_connect" => {
          self.handle_action(&Action::Page(Page::Connect), ctx);
        }
        "open_apps" => {
          self.handle_action(&Action::Page(Page::Apps), ctx);
        }
        "open_app_store" => {
          self.handle_action(&Action::Page(Page::AppStore), ctx);
        }
        "open_app_store_updatable" => {
          self.handle_action(&Action::Page(Page::AppStore), ctx);
          self.handle_action(&Action::AppStore(crate::ui::app_store_state::AppStoreAction::SelectCategory(stepx_companion::app_store::StoreCategory::Updatable)), ctx);
        }
        "open_app_store_installed" => {
          self.handle_action(&Action::Page(Page::AppStore), ctx);
          self.handle_action(&Action::AppStore(crate::ui::app_store_state::AppStoreAction::SelectCategory(stepx_companion::app_store::StoreCategory::Installed)), ctx);
        }
        "open_app_store_detail" => {
          let pkg = v["package"].as_str().unwrap_or("com.google.android.youtube").to_string();
          self.handle_action(&Action::Page(Page::AppStore), ctx);
          self.handle_action(&Action::AppStore(crate::ui::app_store_state::AppStoreAction::SelectApp(pkg)), ctx);
        }
        "open_backscreen" => {
          self.handle_action(&Action::Page(Page::BackScreen), ctx);
        }
        "open_files" => {
          self.handle_action(&Action::Page(Page::Files), ctx);
          if let Some(idx) = v["file_index"].as_u64() {
            self.handle_action(&Action::SelectFile(idx as usize), ctx);
          }
        }
        "open_phone_prefill_contact" => {
          self.handle_action(&Action::Page(Page::Phone), ctx);
          self.fields[F_PHONE_CONTACT_NAME].set(v["name"].as_str().unwrap_or("李工 · 研发部"));
          self.fields[F_PHONE_DIAL].set(v["number"].as_str().unwrap_or("135 8888 9999"));
        }
        "trigger_mcp_key" => {
          let key = v["key"].as_str().unwrap_or("open_app_manager").to_string();
          self.handle_action(&Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(key)), ctx);
        }
        "trigger_amoo_memory" => {
          self.handle_action(&Action::AmooOpenMemory, ctx);
        }
        "prepare_v0299_home_stage1_drag_reorder" => {
          self.demo_mode = true;
          self.init_demo_push();
          self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
          self.notification_center.active_popup_id = None;
          self.notification_center.popup_queue.clear();
          self.sync_desktop_popup_window(ctx);
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.home_modules.enabled = vec![
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleId::ProductivityHub,
            pages::home::HomeModuleId::PulseAndKpi,
            pages::home::HomeModuleId::CalendarPanorama,
          ];
          self.home_modules.sizes.clear();
          for m in [
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
          ] {
            self.home_modules.set_module_size(
              m,
              pages::home::HomeModuleWidth::OneThird,
              pages::home::HomeModuleHeight::Standard,
            );
          }
          self.status = "StepX v0.2.99 · 首页自由拖拽、点击换位与自适应价值密度已就绪".into();
          self.error = false;
        }
        "prepare_v0299_home_stage3_edit_mode" => {
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.home_module_edit_mode = true;
          self.home_edit_anim_started = Instant::now();
          self.status = "模块编辑模式：可拖拽 ⠿ 手柄换位、拖拽 ⤡ 手柄缩放宽高，或点击预设/尺寸按钮".into();
          self.error = false;
        }
        "prepare_v0299_home_stage4_compact_vs_half_standard" => {
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.home_modules.enabled = vec![
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleId::ProductivityHub,
            pages::home::HomeModuleId::PulseAndKpi,
          ];
          self.home_modules.sizes.clear();
          for m in [
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
          ] {
            self.home_modules.set_module_size(
              m,
              pages::home::HomeModuleWidth::OneThird,
              pages::home::HomeModuleHeight::Compact,
            );
          }
          for m in [
            pages::home::HomeModuleId::ProductivityHub,
            pages::home::HomeModuleId::PulseAndKpi,
          ] {
            self.home_modules.set_module_size(
              m,
              pages::home::HomeModuleWidth::OneHalf,
              pages::home::HomeModuleHeight::Standard,
            );
          }
          self.status = "尺寸即价值对比：第一行 1/3×紧凑（HeadlineTop1） · 第二行 1/2×标准（BalancedWorkbench）".into();
          self.error = false;
        }
        "prepare_v0299_home_stage5_tall_panorama" => {
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.home_modules.enabled = vec![
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::PulseAndKpi,
            pages::home::HomeModuleId::CalendarPanorama,
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleId::ProductivityHub,
          ];
          self.home_modules.sizes.clear();
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleWidth::TwoThirds,
            pages::home::HomeModuleHeight::Tall,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::PulseAndKpi,
            pages::home::HomeModuleWidth::OneThird,
            pages::home::HomeModuleHeight::Tall,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::CalendarPanorama,
            pages::home::HomeModuleWidth::Full,
            pages::home::HomeModuleHeight::Tall,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Tall,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Tall,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::ProductivityHub,
            pages::home::HomeModuleWidth::Full,
            pages::home::HomeModuleHeight::Tall,
          );
          self.status = "尺寸即价值全景：2/3×舒展双栏工作台 与 全宽×舒展（DeepPanorama）全景工作流".into();
          self.error = false;
        }
        "restore_v0299_default_home_layout" => {
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.home_modules.reset_default();
          self.save_home_modules_config();
          self.status = "已恢复默认模块布局".into();
          self.error = false;
        }
        "prepare_v02100_strategy_ai_capsules" => {
          self.demo_mode = false;
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_pointer_registry.borrow_mut().active_op = None;
          self.back_screen.live_capsule_popover_open = false;
          self.back_screen.live_capsule_filter = crate::ui::backscreen_state::LiveCapsuleFilter::All;
          self.back_screen.dismissed_capsule_ids.clear();
          self.push.prefs.dismissed_pickup_codes.clear();
          self.push.prefs.deleted_threads.clear();
          self.push.prefs.cleared_threads.clear();
          self.push.prefs.deleted_messages.clear();
          self.read_sms_addresses.clear();
          self.local_sent_sms.clear();
          self.back_screen.stepos_suite.phone_clipboard.clear();
          self.back_screen.stepos_suite.alarms.clear();

          // Active Pomodoro on LiveActivity track
          self.back_screen.pomodoro_active = true;
          self.back_screen.pomodoro_duration_secs = 25 * 60;
          self.back_screen.pomodoro_started_at = Some(std::time::Instant::now());

          // Configure Home modules for crisp Dual-Track + Life/Work priority card layout
          self.home_modules.enabled = vec![
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleId::CalendarPanorama,
          ];
          self.home_modules.sizes.clear();
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::LifeModule,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Standard,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::WorkModule,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Standard,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::IssuesAndReplyModule,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Standard,
          );
          self.home_modules.set_module_size(
            pages::home::HomeModuleId::CalendarPanorama,
            pages::home::HomeModuleWidth::OneHalf,
            pages::home::HomeModuleHeight::Standard,
          );
          self.home_modules.preset = Some(pages::home::HomeModulePreset::DefaultAll);

          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();

          // Seed SMS: DingDong grocery arrived + Cainiao pickup code + SF Express in-delivery
          self.sms = vec![
            stepx_companion::sms::Message {
              provider_id: Some(910_001),
              address: "106839909638768323".into(),
              body: "【叮咚买菜】您尾号8350的订单已代收（家门口），如果您有疑问，请随时联系骑手".into(),
              date_ms: now_ms - 45_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(910_002),
              address: "106881517503453".into(),
              body: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。".into(),
              date_ms: now_ms - 120_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(910_003),
              address: "95338".into(),
              body: "【顺丰速运】您的快件 SF1438890216543 正在派件中，派件员王师傅正在为您派送，请留意接听电话。".into(),
              date_ms: now_ms - 180_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
          ];
          self.sms_loaded = true;

          // Seed IM threads: WeChat direct @ mention + BOSS直聘 HR message
          let mut harbor_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::STEPX星辰停泊港".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "STEPX星辰停泊港".into(),
            is_group: true,
            unread_count: 2,
            last_timestamp_ms: now_ms - 30_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "harbor-msg-1".into(),
                sender: "窝-StepX".into(),
                text: "窝-StepX: @倒影颠倒\u{2005}老大最近有啥新科研吗".into(),
                timestamp_ms: now_ms - 30_000,
                observed_at_ms: now_ms - 30_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut harbor_thread);

          let mut boss_thread = stepx_companion::push::ImThread {
            thread_id: "com.hpbr.bosszhipin::张经理 · 字节跳动 HR".into(),
            package_name: "com.hpbr.bosszhipin".into(),
            app_name: "BOSS直聘".into(),
            title: "张经理 · 字节跳动 HR".into(),
            is_group: false,
            unread_count: 1,
            last_timestamp_ms: now_ms - 75_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "boss-msg-1".into(),
                sender: "张经理".into(),
                text: "请问今天下午方便聊一下架构师岗位吗？".into(),
                timestamp_ms: now_ms - 75_000,
                observed_at_ms: now_ms - 75_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut boss_thread);

          self.push.snapshot.threads = vec![harbor_thread, boss_thread];

          // Seed ongoing rider delivery notification (LiveActivity track) + BOSS/WeChat notifications
          self.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
              key: "notif-meituan-rider-live".into(),
              package_name: "com.sankuai.meituan.takeoutnew".into(),
              app_name: "美团外卖".into(),
              title: "美团外卖 · 骑手正在配送，距您 350m".into(),
              text: "专送骑手李师傅正在飞速赶来，距您 350m".into(),
              post_time_ms: now_ms - 25_000,
              is_ongoing: true,
              progress_current: 68,
              progress_max: 100,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
            stepx_companion::push::PushNotification {
              key: "notif-cainiao-4-2-1086".into(),
              package_name: "com.android.mms".into(),
              app_name: "短信".into(),
              title: "106881517503453".into(),
              text: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。".into(),
              post_time_ms: now_ms - 120_000,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
          ];
          self.push.snapshot.otp_banners.clear();

          // Seed Today's upcoming schedule
          self.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 2100,
              title: "13:04 第四季度产品与桌面协同评审会".into(),
              dtstart: now_ms + 22 * 60_000,
              dtend: now_ms + 82 * 60_000,
              location: "StepX 3F 星辰会议室".into(),
              description: "评审策略引擎 + AI 双支柱胶囊中枢与跨端同步体验".into(),
              all_day: false,
              source: "StepX Calendar".into(),
            },
          ];

          self.sync_live_express_parcels();
          self.sync_notification_center();

          let ledger_dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join("stepx-v02100-ledger");
          let device_serial = self.selected.as_ref().map(|d| d.serial.clone()).unwrap_or_else(|| "STEPX_V02100_DEVICE".into());
          let capture = stepx_companion::context_ledger::Capture {
            device: device_serial,
            account: None,
            connected: true,
            now_ms,
            utc_offset_seconds: now.offset().local_minus_utc(),
            tasks: vec![],
            push: Some(self.push.snapshot.clone()),
            sms: Some(self.sms.clone()),
            calendar: Some(self.back_screen.stepos_suite.calendar_events.clone()),
            ..Default::default()
          };
          if let Ok(overview) = stepx_companion::context_ledger::process(&ledger_dir, &capture) {
            self.context.overview = overview;
          }

          self.sync_demo_auto_sync_domains();
          self.status = "策略+AI 双支柱胶囊中枢就绪：手机实况 + 重要消息双轨并进".into();
          self.error = false;
        }
        "prepare_v02101_popup_and_notifications_scenarios" => {
          self.demo_mode = true;
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.page = Page::Sms;
          self.push.tab = crate::ui::push_state::PushCenterTab::ImChat;
          self.push.im_channel = stepx_companion::push::ImChannel::All;
          self.push.collapse_ongoing = false;
          self.push.notif_category = 0;
          self.push.category = stepx_companion::push::PushCategory::All;
          self.push.app_filter = None;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.back_screen.live_capsule_popover_open = false;
          self.back_screen.dismissed_capsule_ids.clear();
          self.push.prefs.dismissed_pickup_codes.clear();
          self.push.prefs.deleted_threads.clear();
          self.push.prefs.cleared_threads.clear();
          self.push.prefs.deleted_messages.clear();
          self.read_sms_addresses.clear();
          self.local_sent_sms.clear();
          self.push.active_toasts.clear();

          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();

          self.notification_center.alert_mode =
            crate::ui::notification_center_state::NotificationAlertMode::PopupAndBadge;
          self.notification_center.snooze_until_ms = None;
          self.notification_center.drawer_open = false;
          self.notification_center.popup_shown_ids.clear();
          self.notification_center.read_ids.clear();
          self.notification_center.dismissed_ids.clear();
          self.notification_center.initial_sync_seeded = false;
          self.notification_center.popup_session_started_ms = now_ms - 60_000;

          self.sms = vec![
            stepx_companion::sms::Message {
              provider_id: Some(920_001),
              address: "106839909638768323".into(),
              body: "【叮咚买菜】您尾号8350的订单已代收（家门口），如果您有疑问，请随时联系骑手".into(),
              date_ms: now_ms - 12_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(920_002),
              address: "106881517503453".into(),
              body: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。".into(),
              date_ms: now_ms - 25_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(920_003),
              address: "95338".into(),
              body: "【顺丰速运】您的快件 SF1438890216543 正在派件中，派件员王师傅正在为您派送，请留意接听电话。".into(),
              date_ms: now_ms - 40_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
          ];
          self.sms_loaded = true;

          let mut harbor_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::STEPX星辰停泊港".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "STEPX星辰停泊港".into(),
            is_group: true,
            unread_count: 2,
            last_timestamp_ms: now_ms - 8_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "harbor-msg-v101".into(),
                sender: "窝-StepX".into(),
                text: "窝-StepX: @倒影颠倒\u{2005}老大最近有啥新科研吗".into(),
                timestamp_ms: now_ms - 8_000,
                observed_at_ms: now_ms - 8_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut harbor_thread);

          let mut boss_thread = stepx_companion::push::ImThread {
            thread_id: "com.hpbr.bosszhipin::张经理 · 字节跳动 HR".into(),
            package_name: "com.hpbr.bosszhipin".into(),
            app_name: "BOSS直聘".into(),
            title: "张经理 · 字节跳动 HR".into(),
            is_group: false,
            unread_count: 1,
            last_timestamp_ms: now_ms - 18_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "boss-msg-v101".into(),
                sender: "张经理".into(),
                text: "请问今天下午方便聊一下架构师岗位吗？".into(),
                timestamp_ms: now_ms - 18_000,
                observed_at_ms: now_ms - 18_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut boss_thread);

          self.push.snapshot.threads = vec![harbor_thread, boss_thread];
          self.push.snapshot.listener_connected = true;
          self.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
              key: "notif-meituan-rider-live".into(),
              package_name: "com.sankuai.meituan.takeoutnew".into(),
              app_name: "美团外卖".into(),
              category: "delivery_travel".into(),
              title: "美团外卖 · 骑手正在配送，距您 350m".into(),
              text: "专送骑手李师傅正在飞速赶来，距您 350m，预计 3 分钟送达".into(),
              sub_text: "实时配送进度 68%".into(),
              post_time_ms: now_ms - 15_000,
              is_ongoing: true,
              progress_current: 68,
              progress_max: 100,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
            stepx_companion::push::PushNotification {
              key: "notif-cmb-otp-839201".into(),
              package_name: "cmb.pb".into(),
              app_name: "招商银行".into(),
              category: "system".into(),
              title: "招商银行安全验证".into(),
              text: "【招商银行】您的登录验证码为 839201，5分钟内有效，请勿泄露给他人。".into(),
              sub_text: "刚刚 · 银行安全".into(),
              post_time_ms: now_ms - 5_000,
              otp_code: Some("839201".into()),
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
          ];
          self.push.snapshot.otp_banners = vec![
            stepx_companion::push::OtpBanner {
              notification_key: "notif-cmb-otp-839201".into(),
              code: "839201".into(),
              source_app: "招商银行".into(),
              title: "招商银行".into(),
              raw_text: "【招商银行】您的登录验证码为 839201，5分钟内有效，请勿泄露给他人。".into(),
              timestamp_ms: now_ms - 5_000,
            },
          ];

          self.apply_select_im_channel(stepx_companion::push::ImChannel::All);
          self.selected_thread = None;
          self.push.selected_im_thread = Some("com.tencent.mm::STEPX星辰停泊港".into());
          self.back_screen.stepos_suite.phone_clipboard = "Github 送的 Copilot Max".into();
          self.sync_live_express_parcels();
          self.sync_notification_center();
          self.apply_notification_center_action(
            crate::ui::notification_center_state::NotificationCenterAction::ReplayLatestPopups,
          );
          self.sync_desktop_popup_window(ctx);
          self.sync_demo_auto_sync_domains();
          self.back_screen.stepos_suite.phone_clipboard = "Github 送的 Copilot Max".into();
          self.status = "StepX v0.2.107 · 488px大尺寸弹窗与桌面消息通知全同步已就绪".into();
          self.error = false;
        }
        "prepare_v02102_home_reminder_governance" => {
          self.demo_mode = true;
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.page = Page::Home;
          self.scroll = Default::default();
          self.home_module_edit_mode = false;
          self.home_pending_move = None;
          self.home_reminder_drawer_open = false;
          self.home_reminder_drawer_tab = 0;
          self.context_menu = None;
          self.reminder_governance.clear_all_suppressed();
          self.reminder_governance.muted_sources.clear();
          self.reminder_governance.category_ttl_mins.clear();
          self.reminder_governance.save();
          self.back_screen.live_capsule_popover_open = false;
          self.back_screen.dismissed_capsule_ids.clear();
          self.back_screen.completed_calendar_events.clear();
          self.push.prefs.dismissed_pickup_codes.clear();
          self.push.active_toasts.clear();
          self.notification_center.alert_mode =
            crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
          self.notification_center.active_popup_id = None;
          self.notification_center.popup_queue.clear();
          self.sync_desktop_popup_window(ctx);

          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();

          self.sms = vec![
            stepx_companion::sms::Message {
              provider_id: Some(930_001),
              address: "106839909638768323".into(),
              body: "【叮咚买菜】您尾号8350的订单已代收（家门口），如果您有疑问，请随时联系骑手".into(),
              date_ms: now_ms - 10_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(930_002),
              address: "106881517503453".into(),
              body: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。".into(),
              date_ms: now_ms - 25_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            // Already-read arrived order from 75 minutes ago -> should auto-expire after 30m post-read grace!
            stepx_companion::sms::Message {
              provider_id: Some(930_003),
              address: "106901234567".into(),
              body: "【盒马鲜生】您尾号6612的订单已送达前台代收点，请及时拿取。".into(),
              date_ms: now_ms - 75 * 60_000,
              outgoing: false,
              kind: 1,
              read: true,
              subscription: None,
            },
          ];
          self.sms_loaded = true;

          let mut harbor_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::STEPX星辰停泊港".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "STEPX星辰停泊港".into(),
            is_group: true,
            unread_count: 2,
            last_timestamp_ms: now_ms - 8_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "harbor-msg-v102".into(),
                sender: "窝-StepX".into(),
                text: "窝-StepX: @倒影颠倒\u{2005}老大最近有啥新科研吗".into(),
                timestamp_ms: now_ms - 8_000,
                observed_at_ms: now_ms - 8_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut harbor_thread);
          self.push.snapshot.threads = vec![harbor_thread];
          self.push.selected_im_thread = Some("com.tencent.mm::STEPX星辰停泊港".into());
          self.selected_thread = Some("com.tencent.mm::STEPX星辰停泊港".into());
          self.push.snapshot.listener_connected = true;
          self.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
              key: "notif-meituan-rider-live".into(),
              package_name: "com.sankuai.meituan.takeoutnew".into(),
              app_name: "美团外卖".into(),
              category: "delivery_travel".into(),
              title: "美团外卖 · 骑手正在配送，距您 350m".into(),
              text: "专送骑手李师傅正在飞速赶来，距您 350m，预计 3 分钟送达".into(),
              sub_text: "实时配送进度 68%".into(),
              post_time_ms: now_ms - 15_000,
              is_ongoing: true,
              progress_current: 68,
              progress_max: 100,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
          ];
          self.push.snapshot.otp_banners.clear();

          // Seed 1 upcoming meeting today + 1 already-ended meeting today (which auto-expires!)
          self.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 2102,
              title: "跨端桌面与首页提醒策略评审会".into(),
              dtstart: now_ms + 28 * 60_000,
              dtend: now_ms + 88 * 60_000,
              location: "StepX 3F 星辰会议室".into(),
              description: "评审首页自动过期、手动过期与全局右键菜单体验".into(),
              all_day: false,
              source: "StepX Calendar".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 2101,
              title: "早间研发站会（已结束超过1小时）".into(),
              dtstart: now_ms - 130 * 60_000,
              dtend: now_ms - 95 * 60_000,
              location: "StepX 2F 敏捷区".into(),
              description: "已结束的早间会议应自动从优先提醒与胶囊收起".into(),
              all_day: false,
              source: "StepX Calendar".into(),
            },
          ];

          self.sync_live_express_parcels();
          self.sync_notification_center();
          self.sync_demo_auto_sync_domains();
          self.status = "StepX v0.2.102 · 首页提醒自动过期、手动不再提醒与全局右键菜单已就绪".into();
          self.error = false;
        }
        "prepare_v02103_calendar_and_alarms" => {
          use chrono::{Datelike, Timelike};
          self.demo_mode = true;
          self.phone_link = None;
          self.expansion.worker = None;
          self.expansion.snapshot.error = false;
          self.page = Page::Calendar;
          self.scroll = Default::default();
          self.context_menu = None;
          self.reminder_governance.clear_all_suppressed();
          self.reminder_governance.muted_sources.clear();
          self.reminder_governance.category_ttl_mins.clear();
          self.reminder_governance.save();
          self.back_screen.live_capsule_popover_open = false;
          self.back_screen.dismissed_capsule_ids.clear();
          self.back_screen.completed_calendar_events.clear();
          self.notification_center.alert_mode =
            crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
          self.notification_center.active_popup_id = None;
          self.notification_center.popup_queue.clear();
          self.sync_desktop_popup_window(ctx);

          let now = chrono::Local::now();
          let now_ms = now.timestamp_millis();
          let today = now.date_naive();
          self.back_screen.calendar_year = today.year();
          self.back_screen.calendar_month = today.month();
          self.back_screen.selected_calendar_day = today.day();
          self.back_screen.completed_calendar_events.insert(21034);

          self.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21031,
              title: "StepX 跨端日历与闹钟状态架构同步".into(),
              dtstart: now_ms - 15 * 60_000,
              dtend: now_ms + 35 * 60_000,
              location: "StepX 3F 星辰会议室".into(),
              description: "进行中的有效日程：统一今日有效未过期与已过期状态口径".into(),
              all_day: false,
              source: "手机日历".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21032,
              title: "桌面端日历右键菜单精简体验评审".into(),
              dtstart: now_ms + 50 * 60_000,
              dtend: now_ms + 110 * 60_000,
              location: "StepX 2F 设计工坊".into(),
              description: "待开始的有效未过期日程 #1".into(),
              all_day: false,
              source: "手机日历".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21033,
              title: "移动端与桌面端闹钟未过期状态联调".into(),
              dtstart: now_ms + 140 * 60_000,
              dtend: now_ms + 185 * 60_000,
              location: "飞书线上会议室".into(),
              description: "待开始的有效未过期日程 #2".into(),
              all_day: false,
              source: "系统日历".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21034,
              title: "早间核心模块需求对齐（已完成）".into(),
              dtstart: now_ms - 180 * 60_000,
              dtend: now_ms - 140 * 60_000,
              location: "StepX 2F 敏捷区".into(),
              description: "已标记完成的日程不计入今日有效未过期数量".into(),
              all_day: false,
              source: "手机日历".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21035,
              title: "上午构建流水线巡检（已过结束时间）".into(),
              dtstart: now_ms - 110 * 60_000,
              dtend: now_ms - 65 * 60_000,
              location: "研发中心 A 区".into(),
              description: "已结束的日程显示今日已过期状态".into(),
              all_day: false,
              source: "手机日历".into(),
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
              recurring: Some(false),
              id: 21036,
              title: "明日版本发布全链路回归演练".into(),
              dtstart: now_ms + 24 * 3_600_000,
              dtend: now_ms + 25 * 3_600_000,
              location: "StepX 4F 路演厅".into(),
              description: "明日有效待办日程".into(),
              all_day: false,
              source: "手机日历".into(),
            },
          ];

          let now_min = now.hour() * 60 + now.minute();
          let up1_min = (now_min + 25).min(23 * 60 + 52);
          let up2_min = (now_min + 75).min(23 * 60 + 58);
          let past1_min = now_min.saturating_sub(90);
          let past2_min = now_min.saturating_sub(45);

          self.back_screen.stepos_suite.alarms = vec![
            stepx_companion::toolbox::PhoneAlarmItem {
              id: 3101,
              hour: (up1_min / 60) as u8,
              minutes: (up1_min % 60) as u8,
              enabled: true,
              days_of_week: 127,
              vibrate: true,
              message: "设计走查与专注冲刺提醒".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
              id: 3102,
              hour: (up2_min / 60) as u8,
              minutes: (up2_min % 60) as u8,
              enabled: true,
              days_of_week: 127,
              vibrate: true,
              message: "跨端版本构建封板提醒".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
              id: 3103,
              hour: (past1_min / 60) as u8,
              minutes: (past1_min % 60) as u8,
              enabled: true,
              days_of_week: 127,
              vibrate: true,
              message: "早间晨会与站会固定闹钟".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
              id: 3104,
              hour: (past2_min / 60) as u8,
              minutes: (past2_min % 60) as u8,
              enabled: true,
              days_of_week: 0,
              vibrate: false,
              message: "上午快递取件临时单次提醒".into(),
            },
            stepx_companion::toolbox::PhoneAlarmItem {
              id: 3105,
              hour: 22,
              minutes: 30,
              enabled: false,
              days_of_week: 96,
              vibrate: false,
              message: "周末晚间阅读休息闹钟".into(),
            },
          ];

          self.call_logs = super::default_demo_call_logs();
          self.contacts = super::default_demo_contacts();
          self.phone_loaded = true;
          self.push.call_filter = 0;

          self.sms = vec![
            stepx_companion::sms::Message {
              provider_id: Some(940_001),
              address: "106839909638768323".into(),
              body: "【叮咚买菜】您尾号8350的订单已代收（家门口），如果您有疑问，请随时联系骑手".into(),
              date_ms: now_ms - 10_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
            stepx_companion::sms::Message {
              provider_id: Some(940_002),
              address: "106881517503453".into(),
              body: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。".into(),
              date_ms: now_ms - 25_000,
              outgoing: false,
              kind: 1,
              read: false,
              subscription: None,
            },
          ];
          self.sms_loaded = true;

          let mut harbor_thread = stepx_companion::push::ImThread {
            thread_id: "com.tencent.mm::STEPX星辰停泊港".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "STEPX星辰停泊港".into(),
            is_group: true,
            unread_count: 2,
            last_timestamp_ms: now_ms - 8_000,
            can_remote_reply: true,
            can_ui_reply: true,
            messages: vec![
              stepx_companion::push::ImMessage {
                id: "harbor-msg-v104".into(),
                sender: "窝-StepX".into(),
                text: "窝-StepX: @倒影颠倒\u{2005}消息模块与独立电话模块已重构完毕".into(),
                timestamp_ms: now_ms - 8_000,
                observed_at_ms: now_ms - 8_000,
                time_known: Some(true),
                outgoing: false,
                delivery_status: "received".into(),
                via: "notification".into(),
                ..Default::default()
              },
            ],
            ..Default::default()
          };
          stepx_companion::push::recompute_thread_summary(&mut harbor_thread);
          self.push.snapshot.threads = vec![harbor_thread];
          self.push.selected_im_thread = Some("com.tencent.mm::STEPX星辰停泊港".into());
          self.selected_thread = Some("com.tencent.mm::STEPX星辰停泊港".into());
          self.push.snapshot.listener_connected = true;
          self.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
              key: "notif-meituan-rider-live".into(),
              package_name: "com.sankuai.meituan.takeoutnew".into(),
              app_name: "美团外卖".into(),
              category: "delivery_travel".into(),
              title: "美团外卖 · 骑手正在配送，距您 350m".into(),
              text: "专送骑手李师傅正在飞速赶来，距您 350m，预计 3 分钟送达".into(),
              sub_text: "实时配送进度 68%".into(),
              post_time_ms: now_ms - 15_000,
              is_ongoing: true,
              progress_current: 68,
              progress_max: 100,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
            stepx_companion::push::PushNotification {
              key: "notif-feishu-doc-mention".into(),
              package_name: "com.ss.android.lark".into(),
              app_name: "飞书".into(),
              category: "im_social".into(),
              title: "飞书文档 · 张晓楠 在《StepX v0.2.104 通信导航架构》中提及了你".into(),
              text: "@正法 请确认「日历、闹钟、电话、消息」侧边栏排序与独立电话工作台交互".into(),
              sub_text: "刚刚 · 研发协同".into(),
              post_time_ms: now_ms - 35_000,
              is_ongoing: false,
              read: false,
              dismissed: false,
              active: true,
              ..Default::default()
            },
          ];

          self.sync_live_express_parcels();
          self.sync_notification_center();
          self.sync_demo_auto_sync_domains();
          self.status = "StepX v0.2.104 · 日历、闹钟、电话、消息四模块架构已就绪".into();
          self.error = false;
        }
        _ => {}
      }
    },
    "demo_interaction_craft"=>{
      self.page = Page::Amoo;
      self.notification_center.alert_mode = crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly;
      self.notification_center.active_popup_id = None;
      self.notification_center.popup_queue.clear();
      self.sync_desktop_popup_window(ctx);
      self.workspace.active = None;
      self.amoo_attachments.clear();
      self.workspace.journal.tasks.clear();
      let sample_tasks = [
        ("task-pin-1", "上海天气与周末出行规划", true, "继续对话 · 已置顶"),
        ("task-2", "解释 Google Antigravity 多 Agent 架构", false, "手机会话"),
        ("task-3", "请帮我汇总最近的顺丰快递物流状态", false, "有草稿"),
        ("task-4", "生成一个赛博朋克风格背屏像素动画", false, "继续对话"),
        ("task-5", "设置明天早上 07:30 晨会闹钟", false, "手机会话"),
        ("task-6", "周四产品评审会 PPT 演示文稿大纲", false, "继续对话"),
        ("task-7", "社区读书会活动筹备方案讨论", false, "手机会话"),
      ];
      for (idx, (id, title, pinned, draft)) in sample_tasks.iter().enumerate() {
        let mut t = stepx_companion::amoo_workspace::Task::new(None, None);
        t.id = (*id).to_string();
        t.title = (*title).to_string();
        t.pinned = *pinned;
        t.updated = 1_762_000_000_i64.saturating_sub(idx as i64 * 60);
        if *draft == "有草稿" {
          t.draft = "帮我查一下最近的快递单号".to_string();
        }
        t.turns.push(stepx_companion::amoo_workspace::Turn {
          at_ms: Some(chrono::Utc::now().timestamp_millis() - 60_000),
          user: true,
          text: (*title).to_string(),
        });
        t.turns.push(stepx_companion::amoo_workspace::Turn {
          at_ms: Some(chrono::Utc::now().timestamp_millis() - 55_000),
          user: false,
          text: format!("已为您整理「{}」的详细内容。", title),
        });
        self.workspace.journal.tasks.push(t);
      }
      if v["open_context_menu"].as_bool() == Some(true) {
        let target_id = v["target_task_id"].as_str().unwrap_or("task-2").to_string();
        let target_title = self.workspace.journal.tasks.iter()
          .find(|t| t.id == target_id)
          .map(|t| t.title.clone())
          .unwrap_or_else(|| "解释 Google Antigravity 多 Agent 架构".to_string());
        self.handle_action(
          &Action::ContextOpen(
            crate::ui::context_menu::Target::ChatSession {
              workspace_task_id: Some(target_id),
              phone_session_idx: None,
              title: target_title,
            },
            Vector2F::new(235., 182.),
          ),
          ctx,
        );
      } else {
        self.context_menu = None;
        events(ctx, vec![Event::MouseMoved { position: Vector2F::new(920., 180.), cmd: false, shift: false, is_synthetic: false }]);
      }
      self.schedule_tick(ctx);
    },
    "click_at" | "scroll_at" => {
      if let Some(w) = facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()) {
        let size = w.inner_size().to_logical::<f64>(w.scale_factor());
        match checked_position(v["x"].as_f64(), v["y"].as_f64(), size.width, size.height) {
          Ok(position) if v["op"] == "click_at" => events(ctx, vec![
            Event::MouseMoved { position, cmd: false, shift: false, is_synthetic: false },
            Event::LeftMouseDown { position, modifiers: Default::default(), click_count: 1, is_first_mouse: false },
            Event::LeftMouseUp { position, modifiers: Default::default() },
          ]),
          Ok(position) => {
            if let Some(delta) = v["delta"].as_f64().filter(|d| d.is_finite() && d.abs() <= f32::MAX as f64) {
              events(ctx, vec![Event::ScrollWheel { position, delta: Vector2F::new(0., delta as f32), precise: true, modifiers: Default::default() }]);
            } else { error = Some("delta must be a finite number".into()); }
          }
          Err(message) => error = Some(message.into()),
        }
      } else { error = Some("current window is unavailable".into()); }
    },
    "drag" => {
      let resolve_id_center = |id: &str| -> Option<Vector2F> {
        if let Some(r) = ctx.element_position_by_id(id.to_owned()) {
          return Some(r.origin() + r.size() / 2.);
        }
        let reg = self.home_pointer_registry.borrow();
        let find_slug = |s: &str| pages::home::HomeModuleId::ALL.iter().copied().find(|m| m.slug() == s);
        if let Some(slug) = id.strip_prefix("home-mod-drag-").or_else(|| id.strip_prefix("home-drag-handle-")) {
          if let Some(m) = find_slug(slug) {
            return reg.drag_handle_rects.get(&m).map(|r| r.origin() + r.size() / 2.);
          }
        }
        if let Some(slug) = id.strip_prefix("home-mod-resize-").or_else(|| id.strip_prefix("home-resize-handle-")) {
          if let Some(m) = find_slug(slug) {
            return reg.resize_handle_rects.get(&m).map(|r| r.origin() + r.size() / 2.);
          }
        }
        if let Some(slug) = id.strip_prefix("home-module-card-").or_else(|| id.strip_prefix("home-card-")) {
          if let Some(m) = find_slug(slug) {
            return reg.card_rects.get(&m).map(|r| r.origin() + r.size() / 2.);
          }
        }
        None
      };
      let from_opt = if let Some(fid) = v["from_id"].as_str() {
        resolve_id_center(fid)
      } else if let (Some(fx), Some(fy)) = (v["from_x"].as_f64(), v["from_y"].as_f64()) {
        Some(Vector2F::new(fx as f32, fy as f32))
      } else {
        None
      };
      let to_opt = if let Some(tid) = v["to_id"].as_str() {
        resolve_id_center(tid)
      } else if let Some(from) = from_opt {
        if v["dx"].is_number() || v["dy"].is_number() {
          let dx = v["dx"].as_f64().unwrap_or(0.) as f32;
          let dy = v["dy"].as_f64().unwrap_or(0.) as f32;
          Some(from + Vector2F::new(dx, dy))
        } else if let (Some(tx), Some(ty)) = (v["to_x"].as_f64(), v["to_y"].as_f64()) {
          Some(Vector2F::new(tx as f32, ty as f32))
        } else {
          None
        }
      } else {
        None
      };
      if let (Some(from), Some(to)) = (from_opt, to_opt) {
        let steps = v["steps"].as_u64().unwrap_or(8).clamp(2, 32) as usize;
        let mut evs = Vec::with_capacity(steps * 2 + 4);
        evs.push(Event::MouseMoved { position: from, cmd: false, shift: false, is_synthetic: false });
        evs.push(Event::LeftMouseDown { position: from, modifiers: Default::default(), click_count: 1, is_first_mouse: false });
        for step in 1..=steps {
          let t = step as f32 / steps as f32;
          let pos = Vector2F::new(from.x() + (to.x() - from.x()) * t, from.y() + (to.y() - from.y()) * t);
          evs.push(Event::LeftMouseDragged { position: pos, modifiers: Default::default() });
          evs.push(Event::MouseMoved { position: pos, cmd: false, shift: false, is_synthetic: false });
        }
        if v["hold"].as_bool() != Some(true) {
          evs.push(Event::LeftMouseUp { position: to, modifiers: Default::default() });
        }
        events(ctx, evs);
      } else {
        error = Some("drag requires valid from_id/from_x,from_y and to_id/dx,dy/to_x,to_y".into());
      }
    },
    "pointer_down" | "pointer_move" | "pointer_up" => {
      let pos_opt = if let Some(id) = v["id"].as_str().or_else(|| v["target_id"].as_str()) {
        ctx.element_position_by_id(id.to_owned()).map(|r| r.origin() + r.size() / 2.).or_else(|| {
          let reg = self.home_pointer_registry.borrow();
          let find_slug = |s: &str| pages::home::HomeModuleId::ALL.iter().copied().find(|m| m.slug() == s);
          if let Some(slug) = id.strip_prefix("home-mod-drag-").or_else(|| id.strip_prefix("home-drag-handle-")) {
            find_slug(slug).and_then(|m| reg.drag_handle_rects.get(&m).map(|r| r.origin() + r.size() / 2.))
          } else if let Some(slug) = id.strip_prefix("home-mod-resize-").or_else(|| id.strip_prefix("home-resize-handle-")) {
            find_slug(slug).and_then(|m| reg.resize_handle_rects.get(&m).map(|r| r.origin() + r.size() / 2.))
          } else if let Some(slug) = id.strip_prefix("home-module-card-").or_else(|| id.strip_prefix("home-card-")) {
            find_slug(slug).and_then(|m| reg.card_rects.get(&m).map(|r| r.origin() + r.size() / 2.))
          } else {
            None
          }
        })
      } else if let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) {
        Some(Vector2F::new(x as f32, y as f32))
      } else {
        None
      };
      if let Some(position) = pos_opt {
        match v["op"].as_str().unwrap_or("") {
    "search_query" => { self.handle_action(&Action::GlobalSearch(SearchAction::Open),ctx); self.handle_action(&Action::GlobalSearch(SearchAction::Query(v["query"].as_str().unwrap_or("").into())),ctx); },
          "pointer_down" => events(ctx, vec![
            Event::MouseMoved { position, cmd: false, shift: false, is_synthetic: false },
            Event::LeftMouseDown { position, modifiers: Default::default(), click_count: 1, is_first_mouse: false },
          ]),
          "pointer_move" => events(ctx, vec![
            Event::LeftMouseDragged { position, modifiers: Default::default() },
            Event::MouseMoved { position, cmd: false, shift: false, is_synthetic: false },
          ]),
          _ => events(ctx, vec![
            Event::LeftMouseUp { position, modifiers: Default::default() },
          ]),
        }
      } else {
        error = Some("pointer command requires valid id or x,y".into());
      }
    },
    "right_click" => {
      let pos_opt = if let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) {
        Some(Vector2F::new(x as f32, y as f32))
      } else {
        let id = v["id"].as_str().unwrap_or("composer-editor");
        ctx.element_position_by_id(id.to_owned())
          .map(|r| r.origin() + r.size() / 2.)
          .or_else(|| {
            let reg = self.home_pointer_registry.borrow();
            let stripped = id.strip_prefix("home-reminder-").unwrap_or(id);
            if let Some((_, rect, _)) = reg
              .reminder_rects
              .iter()
              .find(|(k, _, t)| k == id || k == stripped || t.spec.primary_key == id || t.spec.primary_key == stripped)
            {
              return Some(rect.origin() + rect.size() / 2.);
            }
            let find_slug = |s: &str| pages::home::HomeModuleId::ALL.iter().copied().find(|m| m.slug() == s);
            if let Some(slug) = id.strip_prefix("home-module-card-").or_else(|| id.strip_prefix("home-card-")) {
              if let Some(m) = find_slug(slug) {
                return reg.card_rects.get(&m).map(|r| r.origin() + Vector2F::new(56., 18.));
              }
            }
            None
          })
      };
      if let Some(position) = pos_opt {
        events(
          ctx,
          vec![
            Event::MouseMoved { position, cmd: false, shift: false, is_synthetic: false },
            Event::RightMouseDown { position, cmd: false, shift: false, click_count: 1 },
          ],
        );
      } else {
        let id = v["id"].as_str().unwrap_or("composer-editor");
        error = Some(format!("painted target missing: {id}"));
      }
    },
    "ime" | "ime_clear" => {
      let input = if v["op"] == "ime_clear" { vec![Event::ClearMarkedText] }
        else { vec![Event::SetMarkedText{marked_text:v["text"].as_str().unwrap_or("").into(),selected_range:0..0}] };
      if v["window"].as_str() == Some("popup") {
        if let Some(id) = self.desktop_popup.window_id() { events_for_window(ctx, id, input); }
        else { error = Some("popup window is not available".into()); }
      } else { events(ctx, input); }
    },
    "scroll"=>{let id=v["id"].as_str().unwrap_or("composer-editor");if let Some(r)=ctx.element_position_by_id(id.to_owned()){events(ctx,vec![Event::ScrollWheel{position:r.origin()+r.size()/2.,delta:Vector2F::new(0.,v["delta"].as_f64().unwrap_or(-120.)as f32),precise:true,modifiers:Default::default()}]);}else{error=Some(format!("painted target missing: {id}"));}},
    "drop"=>events(ctx,vec![Event::DragAndDropFiles { paths:v["paths"].as_array().unwrap().iter().map(|p|p.as_str().unwrap().to_owned()).collect(),location:ctx.element_position_by_id("composer-editor".to_owned()).unwrap().origin()+Vector2F::new(20.,20.) }]),
    "type"=>{
      let input = vec![Event::TypedCharacters{chars:v["text"].as_str().unwrap_or("").into()}];
      if v["window"].as_str() == Some("popup") {
        if let Some(id) = self.desktop_popup.window_id() { events_for_window(ctx, id, input); }
        else { error = Some("popup window is not available".into()); }
      } else { events(ctx, input); }
    },
    "key" => {
      let input = vec![Event::KeyDown{keystroke:facloud_ui::keymap::Keystroke{key:v["key"].as_str().unwrap_or("enter").into(),shift:v["shift"].as_bool().unwrap_or(false),ctrl:v["ctrl"].as_bool().unwrap_or(false),..Default::default()},chars:String::new(),details:Default::default(),is_composing:false}];
      if v["window"].as_str() == Some("popup") {
        if let Some(id) = self.desktop_popup.window_id() { events_for_window(ctx, id, input); }
        else { error = Some("popup window is not available".into()); }
      } else { events(ctx, input); }
    },
    "resize"=>{let w=facloud_ui::windowing::winit::winit_window_for_id(ctx,ctx.window_id()).unwrap();w.set_maximized(false);let _=w.request_inner_size(winit::dpi::LogicalSize::new(v["w"].as_f64().unwrap_or(1100.),v["h"].as_f64().unwrap_or(760.)));},
    "capture"=>{
      let mut path=dir.join(v["name"].as_str().unwrap_or("ui.png"));
      path.set_extension("png");
      let target_wid = if v["target"].as_str() == Some("popup") {
        self.desktop_popup.window_id().unwrap_or_else(|| ctx.window_id())
      } else {
        ctx.window_id()
      };
      if let Some(pw) = ctx.windows().platform_window(target_wid) {
        pw.as_ctx().request_frame_capture(Box::new(move|f|{
          if let Err(e) = image::save_buffer(&path,&f.data,f.width,f.height,image::ColorType::Rgba8) {
            eprintln!("[scenario capture] save_buffer failed for {}: {e}", path.display());
          }
        }));
        pw.as_ctx().request_redraw();
        if let Some(w) = facloud_ui::windowing::winit::winit_window_for_id(ctx, target_wid) {
          w.request_redraw();
        }
        if Some(target_wid) == self.desktop_popup.window_id() {
          self.desktop_popup.notify(ctx);
        } else {
          ctx.notify();
        }
      } else {
        error = Some("capture target window unavailable".into());
      }
    },
    "quit" => {
      // Closing invalidates the native window. Acknowledge first and do not
      // query window/popup state or request another redraw after teardown.
      let _ = std::fs::write(dir.join("ack.json"), serde_json::json!({"seq":seq,"error":null}).to_string());
      self.handle_action(&Action::QuitApplication, ctx);
      return;
    },
    _=>error=Some("unknown UI command".into()),
   }
   last.1=None;
   let p_post = Palette::for_theme(self.theme_mode.is_dark(ctx.system_theme()));
   self.palette.set(p_post);
   let post_mc = serde_json::json!({
     "is_dark": p_post.is_dark,
     "medium_optical_weight": p_post.optical_weight(facloud_ui::fonts::Weight::Medium).to_css_weight(),
     "semibold_optical_weight": p_post.optical_weight(facloud_ui::fonts::Weight::Semibold).to_css_weight(),
     "bloom_active": self.reply_motion.active(),
     "char_bloom_ms": crate::ui::reply_motion::CHAR_BLOOM_MS,
     "base_stagger_ms": crate::ui::reply_motion::BASE_STAGGER_MS,
     "concentric_list_row_r": theme::concentric_radius(theme::R_LG, 4.0),
     "concentric_nav_pill_r": theme::concentric_radius(theme::R_SHELL, 3.0),
     "bezier_snappy_quarter": crate::ui::motion::bezier_snappy_out(0.25),
   });
   let post_sms = serde_json::json!({
     "im_channel": format!("{:?}", self.push.im_channel),
     "selected_im_thread": self.push.selected_im_thread,
     "message_detail_key": self.push.selected_key,
     "selected_sms_thread": self.selected_thread,
     "total_im_threads": self.push.snapshot.threads.len(),
     "matching_im_threads": self.push.snapshot.threads.iter().filter(|t| {
       self.push.im_channel != stepx_companion::push::ImChannel::Sms
         && !stepx_companion::push::is_excluded_im_package(&t.package_name)
         && self.push.im_channel.matches_package(&t.package_name)
     }).count(),
     "im_reply_text": self.fields[F_IM_REPLY].text,
     "sms_to_text": self.fields[F_SMS_TO].text,
     "sms_body_text": self.fields[F_SMS_BODY].text,
     "notif_total": self.push.snapshot.notifications.len(),
     "notif_unread": self.push.snapshot.notifications.iter().filter(|n| {
       !n.dismissed && !n.read && !n.is_ongoing && !stepx_companion::push::should_hide_notification(n)
     }).count(),
     "notif_category": self.push.notif_category,
     "missed_calls_unseen": self.call_logs.iter().filter(|r| {
       r.call_type == 3
         && !self.notification_center.read_ids.contains(&format!("missed-call:{}:{}", r.number.trim(), r.date_ms))
         && !self.notification_center.dismissed_ids.contains(&format!("missed-call:{}:{}", r.number.trim(), r.date_ms))
     }).count(),
     "gallery_items_count": self.back_screen.gallery_items.len(),
     "fresh_unread_sms_threads": self.fresh_unread_sms_thread_count(),
     "active_im_unread": self.active_im_unread_count(),
     "sidebar_msg_red_dot": (self.active_im_unread_count() + self.fresh_unread_sms_thread_count()) > 0
       || self.push.snapshot.threads.iter().any(|t| !Self::is_excluded_im_package(&t.package_name) && t.unread_count > 0),
     "reply_error": self.push.reply_error,
     "reply_in_flight": self.push.reply_in_flight,
     "reply_queue_len": self.push.reply_queue.len(),
     "active_reply_client_id": self.push.active_reply_job.as_ref().map(|j| j.client_msg_id.clone()),
     "selected_thread_outgoing": self.push.selected_im_thread.as_ref().and_then(|tid| {
       self.push.snapshot.threads.iter().find(|t| &t.thread_id == tid).map(|t| {
         t.messages.iter().filter(|m| m.outgoing).map(|m| serde_json::json!({
           "id": m.id,
           "text": m.text,
           "delivery_status": m.delivery_status,
           "via": m.via,
         })).collect::<Vec<_>>()
       })
     }).unwrap_or_default(),
     "selected_thread_messages_count": self.push.selected_im_thread.as_ref().and_then(|tid| {
       self.push.snapshot.threads.iter().find(|t| &t.thread_id == tid).map(|t| t.messages.len())
     }).unwrap_or(0),
     "pinned_targets": self.push.prefs.pinned_targets.iter().cloned().collect::<Vec<_>>(),
     "deleted_threads_count": self.push.prefs.deleted_threads.len(),
     "cleared_threads_count": self.push.prefs.cleared_threads.len(),
     "deleted_messages_count": self.push.prefs.deleted_messages.len(),
     "sms_threads_count": self.sms_threads().len(),
     "local_sent_sms_count": self.local_sent_sms.len(),
     "im_thread_ids": self.push.snapshot.threads.iter().map(|t| t.thread_id.clone()).collect::<Vec<_>>(),
     "im_thread_titles": self.push.snapshot.threads.iter().map(|t| t.title.clone()).collect::<Vec<_>>(),
     "sms_thread_addresses": self.sms_threads().iter().map(|t| t.address.clone()).collect::<Vec<_>>(),
     "express_parcels_count": self.express_parcels.len(),
     "dismissed_pickup_codes": self.push.prefs.dismissed_pickup_codes.iter().cloned().collect::<Vec<_>>(),
   });
   let post_dn = serde_json::json!({
     "selected_app": self.selected_app,
     "has_app_details": self.selected_app_details.is_some(),
     "app_target_sdk": self.selected_app_details.as_ref().map(|d| d.target_sdk),
     "app_permissions_count": self.selected_app_details.as_ref().map(|d| d.permissions.len()).unwrap_or(0),
     "record_audio_granted": self.selected_app_details.as_ref().and_then(|d| {
       d.permissions.iter().find(|p| p.name == "android.permission.RECORD_AUDIO").map(|p| p.granted)
     }),
     "selected_file": self.selected_file,
     "selected_file_preview_path": self.selected_file_preview_path,
     "has_file_preview_text": self.selected_file_preview_text.as_ref().is_some_and(|t| !t.is_empty()),
     "contacts_count": self.contacts.len(),
     "top_contact_name": self.contacts.first().map(|c| c.name.clone()),
     "top_contact_number": self.contacts.first().map(|c| c.number.clone()),
     "workspace_panel": self.workspace.panel,
     "tool_page": self.tool_page.map(|t| format!("{t:?}")),
     "store_updatable_count": self.store_updatable_count(),
     "store_installed_count": self.store_installed_count(),
     "store_category": format!("{:?}", self.app_store.category),
     "store_selected_package": self.app_store.selected_package,
     "store_filtered_packages": self.filtered_store_apps().iter().map(|a| a.package.clone()).collect::<Vec<_>>(),
   });
   let post_capsules = self.collect_top_bar_live_capsules().iter().map(|c| serde_json::json!({
     "id": c.id,
     "tag": c.tag,
     "kind": format!("{:?}", c.kind),
     "track": format!("{:?}", c.track),
     "domain_badge": c.domain_badge,
     "strategy_tag": c.strategy_tag,
     "source_tag": c.source_tag,
     "complete_label": c.complete_label,
     "metric": c.metric,
     "pill_text": c.pill_text,
     "title": c.title,
     "subtitle": c.subtitle,
   })).collect::<Vec<_>>();
   self.sync_desktop_popup_window(ctx);
   let post_active_popup = self.notification_active_popup();
   let post_popup = serde_json::json!({
     "window_created": self.desktop_popup.is_created(),
     "window_visible": self.desktop_popup.is_visible(),
     "main_window_active": self.desktop_popup.main_window_active,
     "main_native_focus": facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()).map(|w| w.has_focus()),
     "popup_native_focus": self.desktop_popup.window_id().and_then(|id| facloud_ui::windowing::winit::winit_window_for_id(ctx, id)).map(|w| w.has_focus()),
     "window_bounds": self.desktop_popup.bounds_array(),
     "main_window_hidden": self.settings.hidden,
     "alert_mode": format!("{:?}", self.notification_center.alert_mode),
     "drawer_open": self.notification_center.drawer_open,
     "active_id": post_active_popup.map(|i| i.id.clone()),
     "active_title": post_active_popup.map(|i| i.title.clone()),
     "active_summary": post_active_popup.map(|i| i.summary.clone()),
     "active_source_badge": post_active_popup.map(|i| i.source_badge.clone()),
     "active_status_tag": post_active_popup.and_then(|i| i.status_tag.clone()),
     "priority": post_active_popup.map(|i| format!("{:?}", i.priority)),
     "widget_kind": post_active_popup.map(|i| i.popup_widget.kind_name()),
     "reply_draft_len": self.fields[F_TOAST_REPLY].text.chars().count(),
     "main_focus_field": self.focus,
     "reply_focused": self.desktop_popup.reply_focused,
     "reply_preedit_len": self.fields[F_TOAST_REPLY].preedit.chars().count(),
     "reply_draft_owner": self.notification_center.popup_reply_item_id,
     "queue_len": self.notification_center.popup_queue.len(),
     "queue_cursor": self.notification_center.popup_cursor,
     "queue_ids": self.notification_center.popup_queue.iter().cloned().collect::<Vec<_>>(),
     "remaining_ms": self.notification_center.popup_remaining_ms,
     "total_ms": self.notification_center.popup_total_ms,
     "snoozed": self.notification_center.is_currently_snoozed(),
     "last_copied_text": self.notification_center.last_copied_text,
     "feedback": self.notification_center.active_feedback(now_ms),
   });
   let mut state_post = state;
   state_post["productivity"] = self.productivity_diagnostics();
   state_post["page"] = serde_json::json!(format!("{:?}", self.page));
   state_post["status"] = serde_json::json!(self.status);
   state_post["voice_input"] = self.voice_input_diagnostics();
   state_post["desktop_popup"] = post_popup;
   state_post["release_story"] = serde_json::json!({"open":self.desktop_update.whats_new_open,"history":self.desktop_update.release_history,"history_limit":self.desktop_update.release_history_limit,"history_total":stepx_companion::release_notes::history().len(),"scroll":self.desktop_update.whats_new_scroll.scroll_start().as_f32()});
   state_post["effective_notif_count"] = serde_json::json!(self.effective_push_notifications().len());
   state_post["effective_notif_unread"] = serde_json::json!(self.unread_notifications());
   state_post["reminder_governance"] = build_reminder_gov_json(self);
   state_post["calendar_alarms_v02103"] = build_calendar_alarms_v02103_json(self);
   state_post["live_capsule_popover_open"] = serde_json::json!(self.back_screen.live_capsule_popover_open);
   state_post["live_capsule_filter"] = serde_json::json!(format!("{:?}", self.back_screen.live_capsule_filter));
   state_post["top_bar_capsules_count"] = serde_json::json!(post_capsules.len());
   state_post["top_bar_capsules"] = serde_json::json!(post_capsules);
   state_post["active_phone_capsule"] = serde_json::json!(self.back_screen.active_phone_capsule.as_ref().map(|(tag, pill, title, _)| serde_json::json!({"tag": tag, "pill": pill, "title": title})));
   state_post["microcraft"] = post_mc;
   state_post["sms_state"] = post_sms;
   state_post["desktop_native"] = post_dn;
   state_post["connection_stability"] = serde_json::json!({
     "active_transport": self.selected.as_ref().map(|d| d.transport.clone()),
     "is_wireless": self.selected.as_ref().is_some_and(|d| is_wireless(&d.transport)),
     "backup_transports": self.selected.as_ref().map(|d| self.online_device_backups(d)).unwrap_or_default(),
     "known_backup_transports": self.selected.as_ref().map(|d| d.backup_transports()).unwrap_or_default(),
     "online_transports": self.selected.as_ref().map(|d| self.online_device_transports(d)).unwrap_or_default(),
     "connection_label": self.selected.as_ref().map(|d| self.device_connection_label(d)),
     "has_dual_standby": self.selected.as_ref().is_some_and(|d| self.device_has_online_usb_and_wireless(d)),
     "misses": self.connection_watch.misses,
     "user_paused": self.connection_watch.user_paused,
     "manage_open": self.connection_manage_open,
     "pairing_open": self.connection_pairing_open,
     "doctor_running": self.doctor_running,
     "doctor": self.doctor_report.as_ref().map(|report| serde_json::json!({
       "topic": report.topic, "checked_at_ms": report.checked_at_ms,
       "phone_ai_replied": report.phone_token_used, "model_label": report.model_label,
       "step_count": report.steps.len(),
     })),
     "rtt_ms": self.selected.as_ref().map(|d| stepx_companion::adb::transport_telemetry(&d.transport).avg_rtt_ms).unwrap_or(0),
   });
   if home_review {
     state_post["home_state"] = build_home_state(self);
     state_post["agent"] = self.agent_diagnostics();
     state_post["calendar_marks"] = if matches!(self.page, Page::Home | Page::Calendar) { self.calendar_mark_diagnostics() } else { serde_json::Value::Null };
   }
   let _=std::fs::write(dir.join("state.tmp"),state_post.to_string());let _=std::fs::rename(dir.join("state.tmp"),dir.join("state.json"));
   let _=std::fs::write(dir.join("ack.json"),serde_json::json!({"seq":seq,"error":error}).to_string());ctx.notify();
  });
    }
}

#[cfg(test)]
mod tests {
    use super::{checked_position, home_review_requested};

    #[test]
    fn coordinate_events_reject_invalid_and_outside_points() {
        assert!(checked_position(Some(0.), Some(0.), 1100., 760.).is_ok());
        assert!(checked_position(Some(1099.5), Some(759.5), 1100., 760.).is_ok());
        for (x, y) in [
            (None, Some(10.)),
            (Some(f64::NAN), Some(10.)),
            (Some(f64::INFINITY), Some(10.)),
            (Some(-1.), Some(10.)),
            (Some(10.), Some(-1.)),
            (Some(1100.), Some(10.)),
            (Some(10.), Some(760.)),
        ] {
            assert!(checked_position(x, y, 1100., 760.).is_err());
        }
    }

    #[test]
    fn home_review_is_explicit_and_preserves_workspace_default() {
        let no_args = vec!["stepx-companion".to_string()];
        let home_args = vec!["stepx-companion".into(), "--page".into(), "home".into()];
        assert!(!home_review_requested(None, &no_args));
        assert!(home_review_requested(Some("home"), &no_args));
        assert!(home_review_requested(None, &home_args));
        assert!(!home_review_requested(Some("workspace"), &home_args));
    }
}
