use super::*;

struct Harness {
    session: Session,
    env: Environment,
    stored: Option<Day>,
}
impl Harness {
    fn new(used: Option<u32>) -> Self {
        let stored = used.map(|used_ms| Day {
            day: "2026-09-10".into(),
            body: String::new(),
            used_ms,
            closed: false,
            closed_at: None,
            updated_at: 1789041600,
            revision: 1,
        });
        Self {
            session: Session::new("2026-09-10".into(), stored.clone()).unwrap(),
            stored,
            env: Environment {
                now_ms: 0.0,
                wall_ms: 1789041600000.0,
                visible: true,
                focused: true,
                online: true,
            },
        }
    }
    fn event(&mut self, event: Event) -> Output {
        self.session
            .dispatch(Input {
                event,
                environment: self.env,
            })
            .unwrap()
    }
    fn tick(&mut self, ms: f64) -> Output {
        self.env.now_ms += ms;
        self.env.wall_ms += ms;
        self.event(Event::Tick)
    }
    fn respond(&mut self, output: &Output) -> Output {
        let (id, command) = request(output);
        if let Some(Command {
            action:
                Action::Save {
                    day,
                    body,
                    used_ms,
                    close,
                    expected_revision,
                },
            ..
        }) = command
        {
            self.stored = Some(Day {
                day: day.clone(),
                body: body.clone(),
                used_ms: *used_ms,
                closed: *close,
                closed_at: close.then_some(1789041600),
                updated_at: 1789041600,
                revision: expected_revision + 1,
            });
        }
        self.event(Event::Response {
            id,
            snapshot: Snapshot {
                schema_epoch: CURRENT_SCHEMA_EPOCH,
                days: self.stored.clone().into_iter().collect(),
                emoji_usage: vec![],
            },
        })
    }
    fn start(&mut self) -> Output {
        let out = self.event(Event::Start);
        assert!(matches!(out.effects[0], Effect::AcquireLock));
        let out = self.event(Event::Lock { acquired: true });
        let out = self.respond(&out);
        if out
            .effects
            .iter()
            .any(|effect| matches!(effect, Effect::Request { .. }))
        {
            self.respond(&out)
        } else {
            out
        }
    }
    fn settle(&mut self, mut out: Output) -> Output {
        for _ in 0..3 {
            if out
                .effects
                .iter()
                .all(|effect| !matches!(effect, Effect::Request { .. }))
            {
                return out;
            }
            out = self.respond(&out);
        }
        panic!("unexpected request loop");
    }
}
fn request(output: &Output) -> (u32, &Option<Command>) {
    output
        .effects
        .iter()
        .find_map(|effect| match effect {
            Effect::Request { id, command, .. } => Some((*id, command)),
            _ => None,
        })
        .expect("request effect")
}

#[test]
fn start_and_resume_need_a_live_response_and_setup_spends_no_time() {
    for used in [None, Some(12000)] {
        let mut h = Harness::new(used);
        let out = h.event(Event::Start);
        assert!(!out.view.editor_visible);
        assert_eq!(out.view.start_label, "Starting…");
        let request = h.event(Event::Lock { acquired: true });
        h.tick(60000.0);
        assert_eq!(h.session.used, f64::from(used.unwrap_or(0)));
        let out = h.respond(&request);
        let out = h.settle(out);
        assert!(out.view.editor_visible);
        assert!(!out.view.read_only);
        assert_eq!(out.view.phase, "Writing");
        assert_eq!(h.session.used, f64::from(used.unwrap_or(0)));
    }
}

#[test]
fn start_offline_or_without_tab_lock_cannot_open_the_editor() {
    let mut h = Harness::new(None);
    h.event(Event::Start);
    let out = h.event(Event::Lock { acquired: false });
    assert!(!out.view.editor_visible);
    assert!(!out.view.start_disabled);
    h.env.online = false;
    h.event(Event::Start);
    let out = h.event(Event::Lock { acquired: true });
    assert!(!out.view.editor_visible);
    assert!(
        out.effects
            .iter()
            .any(|effect| matches!(effect, Effect::ReleaseLock))
    );
    assert!(h.session.pending.is_none());
}

#[test]
fn background_during_start_and_focus_without_interaction_are_untimed() {
    let mut h = Harness::new(Some(1000));
    h.event(Event::Start);
    let request = h.event(Event::Lock { acquired: true });
    h.env.visible = false;
    let out = h.respond(&request);
    assert!(out.view.read_only);
    assert_eq!(out.view.phase, "Paused");
    let out = h.tick(60000.0);
    h.settle(out);
    h.env.visible = true;
    h.env.focused = true;
    let out = h.event(Event::Refresh);
    h.settle(out);
    assert_eq!(h.session.used, 1000.0);
    assert!(!h.session.clock.active());
    h.event(Event::Interact);
    h.tick(100.0);
    assert_eq!(h.session.used, 1100.0);
}

#[test]
fn idle_and_blur_pause_immediately_and_idle_heartbeats_do_not_write() {
    let mut h = Harness::new(Some(0));
    h.start();
    for _ in 0..60 {
        let out = h.tick(100.0);
        h.settle(out);
    }
    assert_eq!(h.session.used, 5000.0);
    let revision = h.stored.as_ref().unwrap().revision;
    let out = h.tick(60000.0);
    h.settle(out);
    assert_eq!(h.stored.as_ref().unwrap().revision, revision);
    h.event(Event::Interact);
    h.tick(250.0);
    h.env.focused = false;
    let out = h.event(Event::Pause);
    h.settle(out);
    h.tick(60000.0);
    assert_eq!(h.session.used, 5250.0);
}

#[test]
fn autosave_debounces_text_and_acknowledgements_preserve_newer_input() {
    let mut h = Harness::new(Some(0));
    h.start();
    h.event(Event::Input {
        body: "  first words\n".into(),
    });
    assert!(h.tick(399.0).effects.is_empty());
    let first = h.tick(1.0);
    h.event(Event::Input {
        body: "  first words\nand more".into(),
    });
    let out = h.respond(&first);
    assert!(out.replace_body.is_none());
    assert_eq!(h.session.body, "  first words\nand more");
    assert_ne!(h.stored.as_ref().unwrap().body, h.session.body);
    let out = h.tick(400.0);
    h.settle(out);
    assert_eq!(h.stored.as_ref().unwrap().body, h.session.body);
}

#[test]
fn failed_autosave_retries_the_exact_command_and_never_charges_reconnection_time() {
    let mut h = Harness::new(Some(2000));
    h.start();
    h.event(Event::Input {
        body: "Keep this thought".into(),
    });
    let out = h.tick(400.0);
    let (id, command) = request(&out);
    let original = serde_json::to_string(command).unwrap();
    h.event(Event::Failed {
        id,
        reason: "offline".into(),
    });
    h.env.online = false;
    let out = h.tick(60000.0);
    assert!(out.view.read_only);
    assert!(out.view.warn_before_leave);
    assert_eq!(h.session.used, 2400.0);
    h.env.online = true;
    let retry = h.event(Event::Refresh);
    assert_eq!(serde_json::to_string(request(&retry).1).unwrap(), original);
    let out = h.respond(&retry);
    assert!(!out.view.warn_before_leave);
    assert!(!h.session.clock.active());
    h.event(Event::Interact);
    h.tick(100.0);
    assert_eq!(h.session.used, 2500.0);
}

#[test]
fn late_response_from_timed_out_request_cannot_acknowledge_a_newer_flight() {
    let mut h = Harness::new(Some(0));
    h.start();
    h.event(Event::Input {
        body: "draft".into(),
    });
    let first = h.tick(400.0);
    let old_id = request(&first).0;
    h.event(Event::Failed {
        id: old_id,
        reason: "offline".into(),
    });
    let retry = h.event(Event::Refresh);
    let out = h.event(Event::Response {
        id: old_id,
        snapshot: Snapshot {
            schema_epoch: CURRENT_SCHEMA_EPOCH,
            days: vec![],
            emoji_usage: vec![],
        },
    });
    assert!(out.view.warn_before_leave);
    assert_eq!(h.session.flight.as_ref().unwrap().id, request(&retry).0);
    h.respond(&retry);
    assert!(h.session.pending.is_none());
}

#[test]
fn finish_waits_for_an_existing_autosave_then_closes_and_expiry_keeps_the_final_words() {
    for (used, finish) in [(0, true), (899500, false)] {
        let mut h = Harness::new(Some(used));
        h.start();
        h.event(Event::Input {
            body: "final words".into(),
        });
        let save = h.tick(400.0);
        let closing = if finish {
            h.event(Event::Finish)
        } else {
            h.tick(200.0)
        };
        assert!(closing.view.read_only);
        assert!(!closing.view.closed);
        let out = h.respond(&save);
        let out = h.settle(out);
        assert!(out.view.closed);
        assert_eq!(out.view.day_status, "closed");
        assert_eq!(h.stored.as_ref().unwrap().body, "final words");
        assert!(h.stored.as_ref().unwrap().used_ms <= today::BUDGET_MS);
        let out = h.event(Event::Input {
            body: "cannot reopen".into(),
        });
        assert_eq!(out.replace_body.as_deref(), Some("final words"));
        assert!(out.effects.is_empty());
    }
}

#[test]
fn conflicts_and_epoch_changes_freeze_text_and_release_the_lock() {
    for reason in ["conflict", "update"] {
        let mut h = Harness::new(Some(0));
        h.start();
        h.event(Event::Input {
            body: "unsaved private text".into(),
        });
        let out = h.tick(400.0);
        let out = h.event(Event::Failed {
            id: request(&out).0,
            reason: reason.into(),
        });
        assert!(out.view.read_only);
        assert!(out.view.warn_before_leave);
        assert!(
            out.effects
                .iter()
                .any(|effect| matches!(effect, Effect::ReleaseLock))
        );
        assert!(h.event(Event::Refresh).effects.is_empty());
        assert_eq!(h.session.body, "unsaved private text");
    }
}

#[test]
fn mismatched_acknowledgement_cannot_discard_an_unsaved_draft() {
    let mut h = Harness::new(Some(0));
    h.start();
    h.event(Event::Input {
        body: "my draft".into(),
    });
    let out = h.tick(400.0);
    let out = h.event(Event::Response {
        id: request(&out).0,
        snapshot: Snapshot {
            schema_epoch: CURRENT_SCHEMA_EPOCH,
            days: h.stored.clone().into_iter().collect(),
            emoji_usage: vec![],
        },
    });
    assert!(out.view.read_only);
    assert!(out.view.warn_before_leave);
    assert_eq!(h.session.body, "my draft");
}

#[test]
fn day_rollover_saves_only_to_the_cutoff_without_awarding_closed_credit() {
    let mut h = Harness::new(Some(1000));
    h.env.wall_ms = h.session.day_end_ms - 1000.0;
    h.start();
    let out = h.tick(1500.0);
    let out = h.settle(out);
    assert_eq!(h.stored.as_ref().unwrap().used_ms, 2000);
    assert_eq!(out.view.day_status, "started");
    assert_eq!(out.view.phase, "Read only");
    assert!(out.view.read_only);
    assert!(!out.view.start_visible);
}
