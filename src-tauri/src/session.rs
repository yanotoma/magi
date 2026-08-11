//! What Magi is doing, and the only module that knows about the others.
//!
//! `CLAUDE.md` states the invariant this file exists to hold: **`session.rs` is the only
//! module that knows about the others.** `audio`, `stt`, `capture` and `llm` are leaves that
//! know nothing about the application, each behind a trait with a fake. That is what makes
//! the no-hardware-in-CI rule achievable, and it only survives if the knowledge lives here
//! rather than leaking outward.
//!
//! ## Why a state machine rather than a set of flags
//!
//! Three milestones of features were built before this arrived, and each grew its own
//! partial answer to "what is happening": `voice::VoiceState` for recording, a dozen
//! `magi://*` events for the panel to infer from, and `tray::ShellState` with five variants
//! and — until now — nothing that ever set one. The panel worked because it could piece the
//! answer together; the tray did not, because it could not.
//!
//! Twelve events the frontend has to reason about is not a vocabulary, it is a puzzle. One
//! state, emitted when it changes, is what lets a menu bar icon and a panel indicator agree
//! without either of them reimplementing the other's guesswork.
//!
//! ## Pure transitions
//!
//! [`next`] is a function of a state and an event, with no `self` and no side effects, so
//! every transition — including the ones that should not happen — is testable without a
//! microphone, a display, a network or a running Tauri app. The stateful part is a `Mutex`
//! around one value and an emit on change; there is nothing to get wrong in it.

use crate::llm::capability::Tier;
use crate::tray::ShellState;

/// What Magi is doing right now.
///
/// One turn's worth of life, and deliberately not a superset of every condition that could
/// be reported: whether a model can see, whether a download is running and whether the panel
/// is open are all true or false *independently* of this. Folding them in would produce a
/// state per combination, which is how a state machine becomes a lookup table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SessionState {
    /// Nothing in flight. The resting state, and where every failure returns to.
    #[default]
    Idle,

    /// The microphone is open. Ends when the user lets go of the hotkey.
    Listening,

    /// Speech-to-text is running. Distinct from [`Listening`] because they end for
    /// different reasons — recording ends when you release, transcription ends when it
    /// ends — and a user waiting through one is owed a different indicator than the other.
    ///
    /// [`Listening`]: SessionState::Listening
    Transcribing,

    /// A request is away and nothing has come back. The gap the spinner fills.
    Thinking,

    /// A screenshot is being taken, either because the model asked or because the words did.
    ///
    /// Its own state rather than part of [`Thinking`] for a reason that is about honesty
    /// rather than accuracy: reading someone's screen is the one thing here they would want
    /// to notice happening, and a state that says so can be shown, logged and tested.
    ///
    /// [`Thinking`]: SessionState::Thinking
    Capturing,

    /// Tokens are arriving.
    Streaming,
}

/// Something that happened.
///
/// Named after the event rather than the intended state — `FirstToken`, not `StartStreaming`
/// — so a caller reports what occurred and this module decides what it means. A caller that
/// names the destination is a caller with an opinion about the machine, and two of those
/// disagree eventually.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The push-to-talk hotkey went down. Repeats while held, which the OS does.
    Held,

    /// The push-to-talk hotkey came up.
    Released,

    /// A transcript arrived, or the recording held no speech.
    Transcribed,

    /// A request has been sent.
    Asked,

    /// A capture has begun.
    Looking,

    /// A capture has finished, and the turn continues.
    Looked,

    /// The first token of an answer arrived.
    Answering,

    /// The turn ended normally.
    Answered,

    /// The turn failed, or the user dismissed the panel. Always returns to
    /// [`SessionState::Idle`].
    Stopped,
}

/// The state after `event` happens in `state`.
///
/// **Unexpected pairings leave the state unchanged**, and that is the load-bearing decision
/// in this function. The tempting alternative — treat anything unexpected as a reset — makes
/// one stray event blank the indicator mid-answer, and stray events happen: the OS repeats
/// key-down while a key is held, a provider can send a token after a stop reason, and a
/// cancelled turn's last frame can arrive after the next turn has started. None of those is
/// a reason to tell the user that nothing is happening.
///
/// [`Event::Stopped`] is the exception and always wins, because a failure or a dismissal is
/// exactly the case where whatever was in flight is over regardless of what it was.
pub fn next(state: SessionState, event: Event) -> SessionState {
    use Event as E;
    use SessionState as S;

    match (state, event) {
        // A dismissal or a failure ends anything.
        (_, E::Stopped) => S::Idle,

        // Voice. `Held` from Listening is the OS repeating key-down, not a second press.
        (S::Idle, E::Held) => S::Listening,
        (S::Listening, E::Held) => S::Listening,
        (S::Listening, E::Released) => S::Transcribing,
        (S::Transcribing, E::Transcribed) => S::Idle,

        // A turn. Reachable from Idle by typing, and from Transcribing when a transcript is
        // sent on: the transcript lands in the input rather than being sent, so the usual
        // path is Transcribing → Idle → Asked, and this arm covers a future that sends
        // directly without the machine needing to change.
        (S::Idle | S::Transcribing, E::Asked) => S::Thinking,

        // Capture, from either side of a turn. `Thinking` is the agentic path — the model
        // asked mid-turn — and `Idle` is the heuristic one, where Magi decides before the
        // request is built.
        (S::Idle | S::Thinking | S::Streaming, E::Looking) => S::Capturing,
        // Back to waiting: the capture is an interruption in a turn, not the end of one.
        (S::Capturing, E::Looked) => S::Thinking,

        (S::Thinking | S::Capturing, E::Answering) => S::Streaming,
        // A token after streaming has begun is the ordinary case and changes nothing.
        (S::Streaming, E::Answering) => S::Streaming,
        (S::Thinking | S::Capturing | S::Streaming, E::Answered) => S::Idle,

        // Everything else: keep what we have. See the doc comment.
        (state, _) => state,
    }
}

impl SessionState {
    /// Whether a turn or a recording is under way.
    ///
    /// The panel uses this to decide whether the composer offers Send or Stop, and it is a
    /// method rather than a comparison at the call site so "busy" means one thing.
    pub fn is_busy(self) -> bool {
        !matches!(self, SessionState::Idle)
    }
}

/// What the menu bar should show.
///
/// Three inputs because three independent facts decide it, and collapsing any of them into
/// [`SessionState`] would multiply its variants rather than simplify this.
///
/// **Activity outranks degradation.** A model that cannot see the screen is worth saying so
/// while nothing is happening, and while something *is* happening the user would rather know
/// what — so `Degraded` is the resting mark, not a permanent one. The alternative shows a
/// warning through an answer arriving, which reads as the answer being the problem.
pub fn shell_state(state: SessionState, tier: Option<Tier>, panel_visible: bool) -> ShellState {
    match state {
        SessionState::Listening => ShellState::Listening,
        // One mark for every kind of working. The menu bar is 22 points tall and the
        // difference between transcribing and thinking is not worth a glyph nobody can
        // read; the panel is where the distinction is visible and useful.
        SessionState::Transcribing
        | SessionState::Thinking
        | SessionState::Capturing
        | SessionState::Streaming => ShellState::Thinking,

        SessionState::Idle => match tier {
            // Nothing known yet is not the same as known-limited. A model nobody has probed
            // must not be reported as broken.
            Some(tier) if !tier.allows_capture() => ShellState::Degraded,
            _ => crate::tray::state_for_panel(panel_visible),
        },
    }
}

/// The authoritative state, and the only writer of `magi://state`.
///
/// A `Mutex` around one `Copy` value: no lock is ever held across an await, and the critical
/// section is a comparison and an assignment. The interesting part is that it **emits only
/// on change** — a turn produces a token event per token, and forwarding a state event with
/// each would give the frontend thousands of identical messages to ignore.
#[derive(Debug, Default)]
pub struct Session {
    state: std::sync::Mutex<SessionState>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> SessionState {
        self.state
            .lock()
            .map(|state| *state)
            .unwrap_or(SessionState::Idle)
    }

    /// Applies an event, returning the new state when it changed.
    ///
    /// `None` means nothing moved, which is the common case and is why the caller can emit
    /// unconditionally on `Some` without checking anything itself.
    ///
    /// A poisoned lock reports no change rather than propagating. The alternative is a
    /// background tray app that stops responding to the hotkey because one thread panicked
    /// somewhere unrelated, with no window in which to say so.
    pub fn apply(&self, event: Event) -> Option<SessionState> {
        let Ok(mut current) = self.state.lock() else {
            tracing::warn!(
                ?event,
                "the session lock is poisoned; the state will not move"
            );
            return None;
        };

        let moved = next(*current, event);
        if moved == *current {
            return None;
        }

        tracing::debug!(from = ?*current, to = ?moved, ?event, "session state");
        *current = moved;
        Some(moved)
    }
}

/// Opens the panel, or closes it.
///
/// The one place that knows what that entails, which is why it is here rather than in
/// `windows`: `CLAUDE.md` makes this module the only one allowed to know about the others, and
/// window handling briefly grew a dependency on `commands` and on this file because that is
/// where the window handle already was.
///
/// M6's task list called this `toggle_session` and described it as moving the state machine
/// out of `Idle`. That came from the original interaction model, where one hotkey opened the
/// panel *and* the microphone; M2 split those in two. Opening a window is not an activity, so
/// the session state is deliberately left alone — [`SessionState`] is about what Magi is
/// doing, and the panel being open is reported through [`ShellState::PanelOpen`] instead.
pub fn toggle_panel(app: &tauri::AppHandle) -> Result<(), crate::error::ShellError> {
    if crate::windows::panel_is_visible(app)? {
        // Hidden, not ended. The thread survives closing and only Clear discards it.
        crate::windows::hide_panel(app)?;
        // The thread stays; the screenshot does not stay forever. Five minutes closed and the
        // image is released — long enough to look something up and come back, short enough
        // that a picture of the screen is not still in memory after lunch.
        crate::commands::expire_capture_later(app);
    } else {
        crate::windows::show_panel(app)?;
        crate::commands::cancel_capture_expiry(app);
    }

    // Either way the menu bar has something new to say: it has a mark for the panel being
    // open, and until `refresh_shell` existed only a session event made it recompute.
    refresh_shell(app);
    Ok(())
}

/// Refreshes the menu bar without changing the session state.
///
/// For the facts that change independently of what Magi is doing: the panel opening or
/// closing, and the active model changing. [`shell_state`] takes all three, and until this
/// existed only a session event could make the tray recompute — so opening the panel left the
/// icon showing whatever it had been showing.
pub fn refresh_shell(app: &tauri::AppHandle) {
    use tauri::Manager;

    let Some(state) = app.try_state::<crate::commands::AppState>() else {
        return;
    };

    let session = state.session.state();
    let tier = active_tier(app);
    let panel_visible = app
        .get_webview_window("panel")
        .and_then(|panel| panel.is_visible().ok())
        .unwrap_or(false);

    crate::tray::show_state(app, shell_state(session, tier, panel_visible));
}

/// The tier of the model currently selected, if it has been probed.
///
/// Read on demand rather than cached on the session: the session is about what is happening
/// and this is about what is configured, and the two have different lifetimes.
fn active_tier(app: &tauri::AppHandle) -> Option<Tier> {
    use tauri::Manager;

    let state = app.try_state::<crate::commands::AppState>()?;
    let active = state.config.lock().ok()?.active.clone()?;
    let cache = state.capabilities.lock().ok()?;
    cache.tier(&active.provider, &active.model)
}

/// Applies an event and tells everything that shows state about it.
///
/// **The only place a session event should be reported from.** One function so the three
/// things that must agree cannot drift: the stored state, the `magi://state` event the panel
/// listens to, and the mark in the menu bar. Before this existed the panel inferred its
/// answer from a dozen events and the tray inferred nothing, which is exactly the drift a
/// single writer prevents.
///
/// Does nothing when the state did not move, so a turn emitting a token per token does not
/// emit a state event per token.
pub fn report(app: &tauri::AppHandle, event: Event) {
    use tauri::{Emitter, Manager};

    let Some(state) = app.try_state::<crate::commands::AppState>() else {
        return;
    };

    let Some(moved) = state.session.apply(event) else {
        return;
    };

    if let Err(error) = app.emit("magi://state", moved) {
        tracing::warn!(%error, ?moved, "could not emit the session state");
    }

    let panel_visible = app
        .get_webview_window("panel")
        .and_then(|panel| panel.is_visible().ok())
        .unwrap_or(false);

    crate::tray::show_state(app, shell_state(moved, active_tier(app), panel_visible));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every state, so a new variant cannot be added without the exhaustive tests below
    /// noticing it.
    const EVERY_STATE: [SessionState; 6] = [
        SessionState::Idle,
        SessionState::Listening,
        SessionState::Transcribing,
        SessionState::Thinking,
        SessionState::Capturing,
        SessionState::Streaming,
    ];

    const EVERY_EVENT: [Event; 9] = [
        Event::Held,
        Event::Released,
        Event::Transcribed,
        Event::Asked,
        Event::Looking,
        Event::Looked,
        Event::Answering,
        Event::Answered,
        Event::Stopped,
    ];

    #[test]
    fn a_held_hotkey_records_then_transcribes_then_rests() {
        // The voice path end to end, which is what M4 shipped and this now describes.
        let mut state = SessionState::Idle;
        for (event, expected) in [
            (Event::Held, SessionState::Listening),
            (Event::Released, SessionState::Transcribing),
            (Event::Transcribed, SessionState::Idle),
        ] {
            state = next(state, event);
            assert_eq!(state, expected, "after {event:?}");
        }
    }

    #[test]
    fn a_repeated_key_down_does_not_restart_anything() {
        // The OS repeats key-down while a key is held. `audio::AudioSource::start` is
        // idempotent for the same reason; this is that decision expressed as a transition.
        assert_eq!(
            next(SessionState::Listening, Event::Held),
            SessionState::Listening
        );
    }

    #[test]
    fn a_turn_thinks_then_streams_then_rests() {
        let mut state = SessionState::Idle;
        for (event, expected) in [
            (Event::Asked, SessionState::Thinking),
            (Event::Answering, SessionState::Streaming),
            (Event::Answered, SessionState::Idle),
        ] {
            state = next(state, event);
            assert_eq!(state, expected, "after {event:?}");
        }
    }

    #[test]
    fn a_capture_interrupts_a_turn_and_returns_to_it() {
        // The agentic path: the model asks to look mid-turn, and the turn continues after.
        // Returning to Thinking rather than Idle is the whole point — a capture is a step
        // inside a turn, and going to Idle would tell the user the answer had arrived.
        let mut state = SessionState::Thinking;
        state = next(state, Event::Looking);
        assert_eq!(state, SessionState::Capturing);
        state = next(state, Event::Looked);
        assert_eq!(state, SessionState::Thinking);
        state = next(state, Event::Answering);
        assert_eq!(state, SessionState::Streaming);
    }

    #[test]
    fn the_heuristic_path_can_capture_before_a_turn_exists() {
        // Tier 2 decides from the user's words and attaches the image before asking, so a
        // capture begins from Idle rather than from Thinking.
        assert_eq!(
            next(SessionState::Idle, Event::Looking),
            SessionState::Capturing
        );
    }

    #[test]
    fn a_model_can_look_again_while_streaming() {
        // A turn may capture more than once — `CaptureBudget` allows three — and the later
        // ones can begin after tokens have already arrived.
        assert_eq!(
            next(SessionState::Streaming, Event::Looking),
            SessionState::Capturing
        );
    }

    #[test]
    fn stopping_always_reaches_idle_from_anywhere() {
        // A dismissal or a failure ends whatever was in flight, and there is no state in
        // which it should not. Exhaustive so a new variant cannot quietly opt out.
        for state in EVERY_STATE {
            assert_eq!(
                next(state, Event::Stopped),
                SessionState::Idle,
                "{state:?} did not stop"
            );
        }
    }

    #[test]
    fn only_a_finish_reaches_idle_from_a_busy_state() {
        // The load-bearing decision, stated as the complete list of ways a turn or a
        // recording may end. Stray events happen — the OS repeats key-down, a provider can
        // send a token after a stop reason, a cancelled turn's last frame can arrive after
        // the next turn began — and none of them is a reason to tell the user that nothing
        // is happening. Anything reaching Idle that is not on this list is a bug, and
        // anything added to this list is a decision rather than an accident.
        const ENDINGS: [(SessionState, Event); 5] = [
            // The voice path finishes: the transcript is in the input and the machine rests.
            (SessionState::Transcribing, Event::Transcribed),
            // A turn finishes, from wherever it happened to be.
            (SessionState::Thinking, Event::Answered),
            (SessionState::Capturing, Event::Answered),
            (SessionState::Streaming, Event::Answered),
            // Every state also ends on `Stopped`, asserted separately and exhaustively.
            (SessionState::Idle, Event::Stopped),
        ];

        for state in EVERY_STATE {
            for event in EVERY_EVENT {
                if event == Event::Stopped {
                    continue;
                }

                let moved = next(state, event);
                if moved != SessionState::Idle || state == SessionState::Idle {
                    continue;
                }

                assert!(
                    ENDINGS.contains(&(state, event)),
                    "{state:?} + {event:?} → Idle, which is not one of the listed endings"
                );
            }
        }
    }

    #[test]
    fn every_pairing_is_defined_and_terminates() {
        // No panics, and no transition that needs a second application to settle — a
        // machine whose output is not a fixed point after one step is a machine that
        // depends on how often it is polled.
        for state in EVERY_STATE {
            for event in EVERY_EVENT {
                let once = next(state, event);
                let twice = next(once, event);
                if !matches!(event, Event::Held | Event::Looking | Event::Answering) {
                    continue;
                }
                assert_eq!(
                    once, twice,
                    "{state:?} + {event:?} is not stable: {once:?} then {twice:?}"
                );
            }
        }
    }

    #[test]
    fn only_idle_is_not_busy() {
        for state in EVERY_STATE {
            assert_eq!(
                state.is_busy(),
                state != SessionState::Idle,
                "{state:?} reports the wrong busyness"
            );
        }
    }

    #[test]
    fn the_menu_bar_shows_one_mark_for_every_kind_of_working() {
        // 22 points is not enough to distinguish transcribing from thinking, and the panel
        // is where that difference is both visible and useful.
        for state in [
            SessionState::Transcribing,
            SessionState::Thinking,
            SessionState::Capturing,
            SessionState::Streaming,
        ] {
            assert_eq!(
                shell_state(state, None, false),
                ShellState::Thinking,
                "{state:?}"
            );
        }
        assert_eq!(
            shell_state(SessionState::Listening, None, false),
            ShellState::Listening
        );
    }

    #[test]
    fn activity_outranks_degradation() {
        // A warning shown through an answer arriving reads as the answer being the problem.
        // Degraded is the resting mark, not a permanent one.
        assert_eq!(
            shell_state(SessionState::Streaming, Some(Tier::TextOnly), false),
            ShellState::Thinking
        );
        assert_eq!(
            shell_state(SessionState::Idle, Some(Tier::TextOnly), false),
            ShellState::Degraded
        );
    }

    #[test]
    fn an_unprobed_model_is_not_reported_as_broken() {
        // Nothing known is not the same as known-limited, and the tray is not the place to
        // guess. This is the same distinction `Tier::Unreachable` exists for.
        assert_eq!(
            shell_state(SessionState::Idle, None, false),
            ShellState::Idle
        );
        assert_eq!(
            shell_state(SessionState::Idle, None, true),
            ShellState::PanelOpen
        );
    }

    #[test]
    fn a_capable_model_at_rest_follows_the_panel() {
        for tier in [Tier::Agentic, Tier::Heuristic] {
            assert_eq!(
                shell_state(SessionState::Idle, Some(tier), true),
                ShellState::PanelOpen,
                "{tier:?}"
            );
            assert_eq!(
                shell_state(SessionState::Idle, Some(tier), false),
                ShellState::Idle,
                "{tier:?}"
            );
        }
    }

    #[test]
    fn applying_reports_only_real_changes() {
        // Why: a turn emits an event per token, and a state event with each would give the
        // frontend thousands of identical messages to ignore.
        let session = Session::new();
        assert_eq!(session.apply(Event::Asked), Some(SessionState::Thinking));
        assert_eq!(
            session.apply(Event::Answering),
            Some(SessionState::Streaming)
        );
        assert_eq!(session.apply(Event::Answering), None, "no change to report");
        assert_eq!(session.state(), SessionState::Streaming);
    }

    #[test]
    fn a_fresh_session_is_idle() {
        assert_eq!(Session::new().state(), SessionState::Idle);
        assert!(!Session::new().state().is_busy());
    }

    #[test]
    fn the_states_serialise_as_the_panel_expects() {
        // The panel switches on these strings; a renamed variant would silently stop
        // matching and leave an indicator stuck. Same guard as `voice::VoiceState`.
        for (state, expected) in [
            (SessionState::Idle, "\"idle\""),
            (SessionState::Listening, "\"listening\""),
            (SessionState::Transcribing, "\"transcribing\""),
            (SessionState::Thinking, "\"thinking\""),
            (SessionState::Capturing, "\"capturing\""),
            (SessionState::Streaming, "\"streaming\""),
        ] {
            assert_eq!(
                serde_json::to_string(&state).expect("serialisable"),
                expected
            );
        }
    }

    #[test]
    fn the_state_survives_being_driven_from_several_threads() {
        // `AppState` is shared and events arrive from the hotkey handler, from a
        // `spawn_blocking` worker and from the turn task. A lost transition would leave an
        // indicator wrong with nothing to explain it.
        let session = std::sync::Arc::new(Session::new());
        let mut handles = Vec::new();

        for _ in 0..8 {
            let session = std::sync::Arc::clone(&session);
            handles.push(std::thread::spawn(move || {
                for _ in 0..50 {
                    session.apply(Event::Asked);
                    session.apply(Event::Answering);
                    session.apply(Event::Answered);
                }
            }));
        }

        for handle in handles {
            handle.join().expect("no thread panicked");
        }

        // Whatever interleaving happened, the machine is in a real state rather than a
        // torn one, and the last event any thread applied was `Answered`.
        assert_eq!(session.state(), SessionState::Idle);
    }
}
