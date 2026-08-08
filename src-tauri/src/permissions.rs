//! What macOS has and has not granted.
//!
//! Every permission Magi needs fails the same way: no error, no log line, and for a
//! tray app with no window, nothing on screen at all. So the state is read and shown
//! rather than discovered when a feature quietly does not work.
//!
//! Reading is deliberately separate from asking. macOS shows its prompt the first time
//! a process actually touches the hardware, which is the behaviour the design wants —
//! ask at first genuine use, with the reason visible — so nothing here requests
//! anything. `cpal` opening a stream is what triggers the microphone prompt, and this
//! module only answers "what is the state now".

use serde::{Deserialize, Serialize};

/// Whether a permission has been granted, refused, or never asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Permission {
    /// Never asked. The prompt will appear at first use, which is the intended path
    /// rather than a problem to fix — so the UI says what will happen, not that
    /// something is wrong.
    NotAsked,
    Granted,
    /// Refused, or disabled by policy. Only the user can change it, and only in System
    /// Settings.
    Denied,
    /// Managed by a configuration profile — a work Mac. Distinct from `Denied` because
    /// the user cannot change it themselves, so telling them to open System Settings
    /// would send them somewhere that will not help.
    Restricted,
    /// The platform has no such concept. Every non-macOS build.
    NotApplicable,
}

impl Permission {
    pub fn is_usable(self) -> bool {
        matches!(self, Permission::Granted | Permission::NotApplicable)
    }

    /// What to tell the user, and what they can do about it.
    pub fn explanation(self, feature: &str) -> String {
        match self {
            Permission::Granted | Permission::NotApplicable => {
                format!("{feature} is available.")
            }
            Permission::NotAsked => format!(
                "macOS will ask for permission the first time you use {feature}. \
                 Nothing is recorded until you do."
            ),
            Permission::Denied => format!(
                "{feature} is blocked. Turn it on in System Settings › Privacy & \
                 Security, then quit and reopen Magi."
            ),
            Permission::Restricted => format!(
                "{feature} is blocked by a configuration profile on this Mac, which \
                 only whoever manages it can change."
            ),
        }
    }
}

/// The deep link to the right System Settings pane.
///
/// Named per permission rather than one generic URL, because sending someone to the
/// top of Privacy & Security and expecting them to find Microphone is most of the
/// difficulty of granting a permission.
pub fn settings_url(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Microphone => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone"
        }
        PermissionKind::Accessibility => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
        }
        PermissionKind::ScreenRecording => {
            "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture"
        }
    }
}

/// Which permission is being asked about.
/// `Deserialize` as well as `Serialize`: this crosses the IPC boundary in both
/// directions — out in a status view, and back in as the argument naming which System
/// Settings pane to open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PermissionKind {
    Microphone,
    /// Needed for the global hotkey. Has no usage-description key and no API to query,
    /// so it is inferred from whether registration succeeded.
    Accessibility,
    /// M5. Has no usage-description key either — macOS supplies its own string, which
    /// is exactly why the in-app explanation matters.
    ScreenRecording,
}

/// Reads microphone authorisation without prompting.
#[cfg(target_os = "macos")]
pub fn microphone() -> Permission {
    use objc2_av_foundation::{AVAuthorizationStatus, AVCaptureDevice, AVMediaTypeAudio};

    // Safe in the sense that matters: this is a pure query with no side effects and no
    // prompt. It is `unsafe` only because every Objective-C message send is.
    let status = unsafe {
        let Some(media_type) = AVMediaTypeAudio else {
            // The constant is weak-linked, so it can be absent on a system too old to
            // define it. Nothing is known then, which is not the same as denied.
            return Permission::NotAsked;
        };
        AVCaptureDevice::authorizationStatusForMediaType(media_type)
    };

    match status {
        AVAuthorizationStatus::Authorized => Permission::Granted,
        AVAuthorizationStatus::Denied => Permission::Denied,
        AVAuthorizationStatus::Restricted => Permission::Restricted,
        AVAuthorizationStatus::NotDetermined => Permission::NotAsked,
        // A status added in a future macOS. Treated as not-asked rather than denied:
        // the pessimistic reading would tell the user to fix something that may be
        // fine, and the optimistic one only costs a prompt they were going to see.
        _ => Permission::NotAsked,
    }
}

#[cfg(not(target_os = "macos"))]
pub fn microphone() -> Permission {
    Permission::NotApplicable
}

/// Reads screen-recording authorisation without prompting.
///
/// **Only ever two answers, unlike the microphone.** `CGPreflightScreenCaptureAccess`
/// returns a bare `bool`, and there is no screen-recording equivalent of
/// `AVAuthorizationStatus::NotDetermined` — "never asked" and "explicitly denied" both
/// come back `false`. That is not an omission in the binding: macOS never shows an
/// in-app prompt for screen recording the way it does for the microphone, it sends the
/// user to System Settings, so the two states would lead to the same instruction anyway.
///
/// [`Permission::Restricted`] is likewise unreachable here. Apple documents no managed
/// or profile-restricted state for this permission, and a `bool` could not express one.
///
/// The companion `CGRequestScreenCaptureAccess` is deliberately not called anywhere in
/// Magi: it opens System Settings as a side effect, which is a thing to do when a user
/// asks for it and not while drawing a settings pane.
#[cfg(target_os = "macos")]
pub fn screen_recording() -> Permission {
    // No `unsafe` block, unlike `microphone` above: this is a plain C function rather
    // than an Objective-C message send, and `objc2-core-graphics` binds it as safe. A
    // pure query — no prompt, no side effects.
    //
    // Not deprecated, unlike most of CGWindow.h — the two image-creating functions there
    // are marked obsoleted as of macOS 15 in favour of ScreenCaptureKit, but this one is
    // plain `API_AVAILABLE(macos(10.15))`, comfortably below Magi's 11.0 minimum.
    if objc2_core_graphics::CGPreflightScreenCaptureAccess() {
        Permission::Granted
    } else {
        Permission::Denied
    }
}

#[cfg(not(target_os = "macos"))]
pub fn screen_recording() -> Permission {
    Permission::NotApplicable
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reading_the_screen_recording_state_does_not_prompt_or_panic() {
        // Same contract as the microphone: asking must be safe, cheap and silent. A query
        // that prompted would hang a headless CI runner, and one that panicked would take
        // a background tray app down at startup with nothing on screen to show why.
        //
        // The value is whatever this machine reports and is deliberately not asserted —
        // a test that required the permission would be a test that requires a display.
        let state = screen_recording();
        assert!(matches!(
            state,
            Permission::Granted | Permission::Denied | Permission::NotApplicable
        ));
        assert!(
            !matches!(state, Permission::NotAsked | Permission::Restricted),
            "CGPreflightScreenCaptureAccess returns a bool; neither state is representable"
        );
    }

    #[test]
    fn reading_the_microphone_state_does_not_prompt_or_panic() {
        // Runs in CI, where the answer is whatever the runner reports. The assertion is
        // that asking is safe and cheap — a query that prompted would hang a headless
        // machine, and one that panicked would take the app down at startup.
        let state = microphone();
        assert!(matches!(
            state,
            Permission::NotAsked
                | Permission::Granted
                | Permission::Denied
                | Permission::Restricted
                | Permission::NotApplicable
        ));
    }

    #[test]
    fn reading_it_repeatedly_is_stable() {
        assert_eq!(microphone(), microphone());
    }

    #[test]
    fn only_granted_and_not_applicable_are_usable() {
        assert!(Permission::Granted.is_usable());
        assert!(Permission::NotApplicable.is_usable());
        assert!(!Permission::NotAsked.is_usable());
        assert!(!Permission::Denied.is_usable());
        assert!(!Permission::Restricted.is_usable());
    }

    #[test]
    fn not_asked_reads_as_normal_rather_than_broken() {
        // The first-run state, and the intended path. Wording it as a failure would
        // have every new user believing something is wrong before they have done
        // anything.
        let text = Permission::NotAsked.explanation("Voice input");
        assert!(text.contains("will ask"), "got: {text}");
        assert!(
            !text.to_lowercase().contains("blocked"),
            "the untouched state must not read as a failure: {text}"
        );
    }

    #[test]
    fn denied_says_where_to_go_and_that_a_restart_is_needed() {
        // macOS does not re-check a permission for a running process, so granting it
        // and carrying on looks like the grant did nothing.
        let text = Permission::Denied.explanation("Voice input");
        assert!(text.contains("System Settings"), "got: {text}");
        assert!(text.contains("reopen"), "got: {text}");
    }

    #[test]
    fn restricted_does_not_send_the_user_somewhere_useless() {
        // A managed Mac. Telling them to open System Settings would send them to a
        // toggle they cannot move.
        let text = Permission::Restricted.explanation("Voice input");
        assert!(text.contains("configuration profile"), "got: {text}");
        assert!(
            !text.contains("System Settings"),
            "restricted must not point at a pane that will not help: {text}"
        );
    }

    #[test]
    fn every_permission_has_its_own_settings_pane() {
        // Sending someone to the top of Privacy & Security and expecting them to find
        // Microphone is most of the difficulty of granting a permission.
        let urls = [
            settings_url(PermissionKind::Microphone),
            settings_url(PermissionKind::Accessibility),
            settings_url(PermissionKind::ScreenRecording),
        ];

        let distinct: std::collections::HashSet<_> = urls.iter().collect();
        assert_eq!(distinct.len(), 3, "two permissions share a pane link");

        for url in urls {
            assert!(url.starts_with("x-apple.systempreferences:"), "got: {url}");
            assert!(url.contains("Privacy_"), "got: {url}");
        }
    }
}
