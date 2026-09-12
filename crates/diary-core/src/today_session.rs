//! Pure daily-writing session. The browser supplies events and clock/focus
//! facts, executes effects, and paints a projection. All start, timing,
//! autosave, replay, conflict, and closure decisions stay here.
use serde::{Deserialize, Serialize};

use crate::{
    contract::CURRENT_SCHEMA_EPOCH,
    today::{self, ActiveClock, Day, Snapshot},
    today_store::{Action, Command},
};

pub const HEARTBEAT_MS: u32 = 1000;
pub const CONNECTION_MS: u32 = 5000;
const AUTOSAVE_MS: u32 = 400;

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub now_ms: f64,
    pub wall_ms: f64,
    pub visible: bool,
    pub focused: bool,
    pub online: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Event {
    Refresh,
    Start,
    Lock { acquired: bool },
    Interact,
    Input { body: String },
    Tick,
    Pause,
    Offline,
    Finish,
    Response { id: u32, snapshot: Snapshot },
    Failed { id: u32, reason: String },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub event: Event,
    pub environment: Environment,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Effect {
    AcquireLock,
    ReleaseLock,
    FocusEditor,
    Request {
        id: u32,
        day: String,
        command: Option<Command>,
        timeout_ms: u32,
    },
}

#[derive(Debug, Serialize)]
pub struct Output {
    pub view: View,
    pub effects: Vec<Effect>,
    /// Only replace the textarea on a load or rejected edit. An acknowledgement
    /// of older text never moves the caret or erases newer input.
    pub replace_body: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct View {
    pub time: String,
    pub phase: &'static str,
    pub status: &'static str,
    pub editor_visible: bool,
    pub read_only: bool,
    pub prompts_visible: bool,
    pub start_visible: bool,
    pub start_disabled: bool,
    pub start_label: &'static str,
    pub finish_visible: bool,
    pub finish_disabled: bool,
    pub warn_before_leave: bool,
    pub closed: bool,
    pub day_status: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Ready,
    Acquiring,
    Starting,
    Writing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Problem {
    Offline,
    Auth,
    Update,
    Conflict,
    Busy,
}

impl Problem {
    fn from_reason(reason: &str) -> Self {
        match reason {
            "auth" => Self::Auth,
            "update" => Self::Update,
            "conflict" => Self::Conflict,
            _ => Self::Offline,
        }
    }
    fn terminal(self) -> bool {
        matches!(self, Self::Conflict | Self::Update)
    }
}

#[derive(Clone, Debug)]
struct Flight {
    id: u32,
    command: Option<Command>,
}

pub struct Session {
    day: String,
    day_end_ms: f64,
    row: Option<Day>,
    body: String,
    used: f64,
    clock: ActiveClock,
    mode: Mode,
    closing: bool,
    focus_paused: bool,
    connected: bool,
    problem: Option<Problem>,
    last_response: f64,
    last_attempt: f64,
    last_input: Option<f64>,
    force_request: bool,
    /// Retained byte-for-byte after a lost response, until acknowledged.
    pending: Option<Command>,
    flight: Option<Flight>,
    next_request: u32,
    lock_held: bool,
}

impl Session {
    pub fn new(day: String, row: Option<Day>) -> Result<Self, String> {
        let day_end_ms = today::day_end(&day).ok_or("invalid diary day")? as f64 * 1000.0;
        if let Some(row) = &row {
            row.validate()?;
            if row.day != day {
                return Err("reflection belongs to another day".into());
            }
        }
        Ok(Self {
            day,
            day_end_ms,
            body: row.as_ref().map(|row| row.body.clone()).unwrap_or_default(),
            used: row
                .as_ref()
                .map(|row| f64::from(row.used_ms))
                .unwrap_or_default(),
            row,
            clock: ActiveClock::default(),
            mode: Mode::Ready,
            closing: false,
            focus_paused: false,
            connected: false,
            problem: None,
            last_response: 0.0,
            last_attempt: f64::NEG_INFINITY,
            last_input: None,
            force_request: false,
            pending: None,
            flight: None,
            next_request: 0,
            lock_held: false,
        })
    }

    pub fn dispatch(&mut self, input: Input) -> Result<Output, String> {
        let env = input.environment;
        if !env.now_ms.is_finite()
            || env.now_ms < 0.0
            || !env.wall_ms.is_finite()
            || today::day_at((env.wall_ms / 1000.0).floor() as i64).is_none()
        {
            return Err("invalid browser clock".into());
        }
        let mut effects = Vec::new();
        let mut replace_body = None;
        let could_write = self.can_interact(env);
        self.advance(env, &mut effects);
        match input.event {
            Event::Refresh => {
                if self.mode != Mode::Acquiring {
                    self.force_request = true;
                }
            }
            Event::Start => {
                if self.mode == Mode::Ready
                    && self.current(env)
                    && !self.closed()
                    && !self.terminal()
                {
                    self.mode = Mode::Acquiring;
                    self.problem = None;
                    effects.push(Effect::AcquireLock);
                }
            }
            Event::Lock { acquired } => {
                if self.mode != Mode::Acquiring {
                    if acquired {
                        effects.push(Effect::ReleaseLock);
                    }
                } else if acquired {
                    self.lock_held = true;
                    self.mode = Mode::Starting;
                    self.force_request = true;
                } else {
                    self.mode = Mode::Ready;
                    self.problem = Some(Problem::Busy);
                }
            }
            Event::Interact => self.interact(env),
            Event::Input { body } => {
                if could_write && self.mode == Mode::Writing && !self.closed() && !self.terminal() {
                    self.body = body.replace("\r\n", "\n").replace('\r', "\n");
                    self.last_input = Some(env.now_ms);
                    self.interact(env);
                } else {
                    replace_body = Some(self.body.clone());
                }
            }
            Event::Tick => {}
            Event::Pause => {
                self.pause(env);
                if self.changed() {
                    self.force_request = true;
                }
            }
            Event::Offline => self.fail(Problem::Offline, env, &mut effects),
            Event::Finish => {
                if self.mode == Mode::Writing
                    && !self.closed()
                    && !self.terminal()
                    && self.current(env)
                {
                    self.pause(env);
                    self.closing = true;
                    self.force_request = true;
                }
            }
            Event::Response { id, snapshot } => {
                if self.flight.as_ref().is_some_and(|flight| flight.id == id) {
                    let flight = self.flight.take().expect("matching flight");
                    match self.accept(snapshot, &flight, env, &mut effects, &mut replace_body) {
                        Ok(()) => {}
                        Err(problem) => self.fail(problem, env, &mut effects),
                    }
                }
            }
            Event::Failed { id, reason } => {
                if self.flight.as_ref().is_some_and(|flight| flight.id == id) {
                    self.flight = None;
                    self.fail(Problem::from_reason(&reason), env, &mut effects);
                }
            }
        }
        self.pump(env, &mut effects);
        Ok(Output {
            view: self.view(env),
            effects,
            replace_body,
        })
    }

    fn current(&self, env: Environment) -> bool {
        today::day_at((env.wall_ms / 1000.0).floor() as i64).as_deref() == Some(self.day.as_str())
    }
    fn stamp(&self, env: Environment) -> f64 {
        env.now_ms - (env.wall_ms - self.day_end_ms).max(0.0)
    }
    fn closed(&self) -> bool {
        self.row.as_ref().is_some_and(|row| row.closed)
    }
    fn terminal(&self) -> bool {
        self.problem.is_some_and(Problem::terminal)
    }
    fn can_interact(&self, env: Environment) -> bool {
        self.mode == Mode::Writing
            && !self.closing
            && !self.closed()
            && !self.terminal()
            && self.connected
            && env.online
            && env.visible
            && env.focused
            && self.current(env)
            && env.now_ms - self.last_response < f64::from(CONNECTION_MS)
            && self.used < f64::from(today::BUDGET_MS)
    }
    fn changed(&self) -> bool {
        self.mode == Mode::Writing
            && !self.closed()
            && self.row.as_ref().is_some_and(|row| {
                self.body != row.body || self.used.floor() as u32 != row.used_ms || self.closing
            })
    }
    fn debit(&mut self, delta: f64, env: Environment) {
        self.used = (self.used + delta).min(f64::from(today::BUDGET_MS));
        if self.used >= f64::from(today::BUDGET_MS) && !self.closing {
            self.closing = true;
            self.clock.pause(self.stamp(env));
            self.force_request = true;
        }
    }
    fn pause(&mut self, env: Environment) {
        let debit = self.clock.pause(self.stamp(env));
        self.debit(debit, env);
        self.focus_paused = true;
    }
    fn interact(&mut self, env: Environment) {
        if self.can_interact(env) {
            self.focus_paused = false;
            let debit = self.clock.interact(self.stamp(env));
            self.debit(debit, env);
        }
    }
    fn release_lock(&mut self, effects: &mut Vec<Effect>) {
        if self.lock_held {
            self.lock_held = false;
            effects.push(Effect::ReleaseLock);
        }
    }
    fn advance(&mut self, env: Environment, effects: &mut Vec<Effect>) {
        if self.mode != Mode::Writing {
            return;
        }
        let was_active = self.clock.active();
        let debit = self.clock.advance(self.stamp(env));
        self.debit(debit, env);
        if !env.visible || !env.focused {
            self.pause(env);
        }
        if self.connected
            && (!env.online || env.now_ms - self.last_response >= f64::from(CONNECTION_MS))
        {
            self.fail(Problem::Offline, env, effects);
        }
        if was_active && !self.clock.active() {
            self.force_request = true;
        }
        if !self.current(env) {
            self.pause(env);
            self.release_lock(effects);
            if self.changed() {
                self.force_request = true;
            } else if self.pending.is_none() && self.flight.is_none() {
                self.mode = Mode::Ready;
            }
        }
    }
    fn fail(&mut self, problem: Problem, env: Environment, effects: &mut Vec<Effect>) {
        self.pause(env);
        self.connected = false;
        self.problem = Some(problem);
        self.force_request = false;
        if self.mode == Mode::Starting {
            self.mode = Mode::Ready;
            self.release_lock(effects);
        } else if problem.terminal() {
            self.release_lock(effects);
        }
    }

    fn accept(
        &mut self,
        snapshot: Snapshot,
        flight: &Flight,
        env: Environment,
        effects: &mut Vec<Effect>,
        replace_body: &mut Option<String>,
    ) -> Result<(), Problem> {
        if snapshot.schema_epoch != CURRENT_SCHEMA_EPOCH {
            return Err(Problem::Update);
        }
        if snapshot.days.len() > 1
            || snapshot
                .days
                .iter()
                .any(|row| row.day != self.day || row.validate().is_err())
        {
            return Err(Problem::Offline);
        }
        let row = snapshot.days.into_iter().next();
        if let Some(Command {
            action:
                Action::Save {
                    body,
                    used_ms,
                    close,
                    expected_revision,
                    ..
                },
            ..
        }) = &flight.command
        {
            if !row.as_ref().is_some_and(|row| {
                row.revision == expected_revision + 1
                    && row.body == *body
                    && row.used_ms == *used_ms
                    && row.closed == *close
            }) {
                return Err(Problem::Conflict);
            }
            self.pending = None;
        } else if self.mode == Mode::Writing
            && row.as_ref().map(|row| row.revision) != self.row.as_ref().map(|row| row.revision)
        {
            return Err(Problem::Conflict);
        }
        self.row = row;
        if self.mode != Mode::Writing {
            self.body = self
                .row
                .as_ref()
                .map(|row| row.body.clone())
                .unwrap_or_default();
            self.used = self
                .row
                .as_ref()
                .map(|row| f64::from(row.used_ms))
                .unwrap_or_default();
            *replace_body = Some(self.body.clone());
        }
        self.connected = env.online;
        self.last_response = env.now_ms;
        self.problem = if env.online {
            None
        } else {
            Some(Problem::Offline)
        };
        if self.closed() {
            self.mode = Mode::Ready;
            self.pause(env);
            self.force_request = false;
            self.release_lock(effects);
        } else if !self.current(env) {
            if !self.changed() {
                self.mode = Mode::Ready;
            }
            self.release_lock(effects);
        } else if self.mode == Mode::Starting {
            if self.row.is_none() {
                self.pending = Some(self.save_command());
                self.force_request = true;
            } else {
                self.mode = Mode::Writing;
                self.force_request = false;
                self.interact(env);
                if self.can_interact(env) {
                    effects.push(Effect::FocusEditor);
                }
            }
        }
        Ok(())
    }
    fn save_command(&self) -> Command {
        Command::new(Action::Save {
            day: self.day.clone(),
            body: self.body.clone(),
            used_ms: self.used.floor() as u32,
            close: self.closing,
            expected_revision: self.row.as_ref().map(|row| row.revision).unwrap_or(0),
        })
    }
    fn pump(&mut self, env: Environment, effects: &mut Vec<Effect>) {
        if self.flight.is_some() || self.terminal() || self.mode == Mode::Acquiring {
            return;
        }
        if !env.online {
            if self.force_request {
                self.fail(Problem::Offline, env, effects);
            }
            return;
        }
        let heartbeat = self.mode == Mode::Writing
            && env.visible
            && env.now_ms - self.last_attempt >= f64::from(HEARTBEAT_MS);
        let input_due = self.changed()
            && self
                .last_input
                .is_some_and(|at| env.now_ms - at >= f64::from(AUTOSAVE_MS))
            && env.now_ms - self.last_attempt >= f64::from(AUTOSAVE_MS);
        if !self.force_request && !heartbeat && !input_due {
            return;
        }
        if self.pending.is_none() && self.changed() {
            self.pending = Some(self.save_command());
        }
        self.force_request = false;
        self.last_attempt = env.now_ms;
        self.next_request = self.next_request.wrapping_add(1);
        let flight = Flight {
            id: self.next_request,
            command: self.pending.clone(),
        };
        effects.push(Effect::Request {
            id: flight.id,
            day: self.day.clone(),
            command: flight.command.clone(),
            timeout_ms: CONNECTION_MS,
        });
        self.flight = Some(flight);
    }

    fn view(&self, env: Environment) -> View {
        let closed = self.closed();
        let past = !self.current(env);
        let starting = matches!(self.mode, Mode::Starting | Mode::Acquiring);
        let writing = self.mode == Mode::Writing && !closed && !past;
        let seconds = ((f64::from(today::BUDGET_MS) - self.used).max(0.0) / 1000.0).ceil() as u32;
        let status = match self.problem {
            Some(Problem::Conflict) => {
                "This reflection changed or closed elsewhere. Your unsaved text is still here to copy. Reload to see the saved version."
            }
            Some(Problem::Update) => "A diary update needs a reload. Copy any unsaved text first.",
            Some(Problem::Auth) => {
                "Sign in again to save. Keep this page open; any unsaved text is still here."
            }
            Some(Problem::Offline) if self.mode == Mode::Writing => {
                "Connection lost. Writing is paused. Keep this page open to save any unsaved changes."
            }
            Some(Problem::Offline) => {
                "Connect to start Today. Your saved reflection is on the server."
            }
            Some(Problem::Busy) => {
                "Today is open for writing in another tab. Close that tab to continue here."
            }
            None if closed => "Saved and closed. You can read this whenever you like.",
            None if starting => "Starting…",
            None if self.closing => "Saving and closing…",
            None if self.changed() || self.pending.is_some() => "Saving…",
            None if past => "This day is available to read.",
            None if writing => "Saved.",
            None if !self.connected => "Loading your saved writing time…",
            None if self.row.is_some() => "Press Resume to continue with your saved writing time.",
            None => "Press Start to open today’s reflection and begin your writing time.",
        };
        View {
            time: format!("{}:{:02}", seconds / 60, seconds % 60),
            phase: if closed {
                "Closed"
            } else if past {
                "Read only"
            } else if starting {
                "Starting…"
            } else if self.clock.active() {
                "Writing"
            } else if writing {
                "Paused"
            } else {
                "Ready"
            },
            status,
            editor_visible: writing || closed || past,
            read_only: !self.can_interact(env) || self.focus_paused,
            prompts_visible: writing,
            start_visible: !writing && !closed && !past,
            start_disabled: starting || self.terminal(),
            start_label: if starting {
                "Starting…"
            } else if self.row.is_some() {
                "Resume"
            } else {
                "Start"
            },
            finish_visible: writing,
            finish_disabled: self.closing || !self.connected || self.terminal(),
            warn_before_leave: self.changed() || self.pending.is_some(),
            closed,
            day_status: self.row.as_ref().map(Day::status).unwrap_or("empty"),
        }
    }
}

#[cfg(test)]
mod tests;
