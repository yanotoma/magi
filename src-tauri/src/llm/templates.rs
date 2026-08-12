//! Questions worth one click.
//!
//! Pre-written **user turns**, not instructions — which is the distinction that keeps this out
//! of `llm::prompt`. The system prompt tells a model what it is; these are things a person
//! might type, offered so they do not have to.
//!
//! ## Why the backend chooses them
//!
//! "Summarise what is on my screen" offered to a model that cannot see is the failure this
//! project keeps finding: a capability promised that the code cannot deliver. A model told it
//! can see answers confidently about a screen it never looked at, and a *user* offered a button
//! that cannot work learns to distrust the buttons.
//!
//! The panel does not know the active model's tier and should not have to. The backend does, so
//! filtering happens here and the panel renders whatever it is given.

use crate::llm::capability::Tier;

/// A question offered as a shortcut.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Template {
    /// What the button says. Short enough for a row of them in a 520-point panel.
    pub label: String,

    /// What lands in the input. A complete question, because half a question in the box is
    /// worse than an empty one — the user has to work out what was meant before editing it.
    pub prompt: String,
}

/// Everything on offer, with what each needs to work.
///
/// Hard-coded rather than configurable, deliberately for now. A user-editable list is a real
/// feature and a bigger one — it needs a Settings pane, a config schema and a migration — and
/// it is not worth adding before anyone has said which questions they actually want. Four that
/// earn their place beat twenty that need managing.
const TEMPLATES: &[(&str, &str, bool)] = &[
    (
        "Explain this error",
        "Explain the error on my screen: what causes it, and what should I do about it?",
        // Needs vision, obviously. Listed rather than assumed so the filter below is a rule
        // rather than a special case.
        true,
    ),
    (
        "What's on screen?",
        "Summarise what is on my screen right now.",
        true,
    ),
    (
        "What should I do next?",
        "Looking at what I have open, what is the next thing I should do?",
        true,
    ),
    (
        // The one that works everywhere, and the reason the list is never empty: a text-only
        // model still has a use, and an empty row of shortcuts reads as a broken feature.
        "Explain this",
        "Explain the following, briefly:\n\n",
        false,
    ),
];

/// The templates the active model can actually honour.
///
/// A tier that cannot capture gets only the ones needing no screen. [`Tier::Unreachable`] and an
/// unprobed model are treated the same way — nothing is known, so nothing is promised.
pub fn for_tier(tier: Option<Tier>) -> Vec<Template> {
    let can_see = tier.is_some_and(Tier::allows_capture);

    TEMPLATES
        .iter()
        .filter(|(_, _, needs_screen)| can_see || !needs_screen)
        .map(|(label, prompt, _)| Template {
            label: (*label).to_string(),
            prompt: (*prompt).to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_that_cannot_see_is_not_offered_the_screen() {
        // The failure this module exists to avoid. A button that cannot work teaches the user
        // to distrust the buttons, which costs more than the button was worth.
        for tier in [Tier::TextOnly, Tier::Unreachable] {
            let offered = for_tier(Some(tier));
            assert!(
                offered.iter().all(|template| !mentions_screen(template)),
                "{tier:?} was offered a screen question: {offered:?}"
            );
        }
    }

    #[test]
    fn an_unprobed_model_is_treated_as_unable_rather_than_able() {
        // Nothing known is not the same as known-capable, and the optimistic reading is the
        // one that produces a button that fails. Same rule as the tray's `Degraded` mark.
        let offered = for_tier(None);
        assert!(offered.iter().all(|template| !mentions_screen(template)));
    }

    #[test]
    fn both_seeing_tiers_are_offered_everything() {
        for tier in [Tier::Agentic, Tier::Heuristic] {
            assert_eq!(
                for_tier(Some(tier)).len(),
                TEMPLATES.len(),
                "{tier:?} lost a template"
            );
        }
    }

    #[test]
    fn the_list_is_never_empty() {
        // An empty row of shortcuts reads as a broken feature rather than as an unsupported
        // one, so at least one template must need nothing.
        for tier in [
            None,
            Some(Tier::TextOnly),
            Some(Tier::Unreachable),
            Some(Tier::Agentic),
            Some(Tier::Heuristic),
        ] {
            assert!(!for_tier(tier).is_empty(), "{tier:?} got nothing");
        }
    }

    #[test]
    fn every_prompt_is_a_whole_question() {
        // Half a question in the input is worse than an empty box: the user has to work out
        // what was meant before they can edit it. The one that ends open does so on purpose,
        // inviting a paste, and says so by ending in a newline rather than mid-sentence.
        for (label, prompt, _) in TEMPLATES {
            assert!(prompt.len() > label.len(), "{label}: {prompt}");
            // Terminal punctuation of any kind, not a question mark: an imperative — "summarise
            // what is on my screen" — is a whole thought and ends in a full stop. The concern is
            // a prompt cut off mid-sentence, not its grammatical mood.
            //
            // A trailing newline counts too, and marks the one template that deliberately ends
            // open, inviting something to be pasted after it.
            assert!(
                prompt.ends_with(['?', '.', '!', '\n']),
                "{label} ends mid-thought: {prompt:?}"
            );
        }
    }

    #[test]
    fn labels_are_short_enough_for_a_row_of_them() {
        // The panel is 520 points wide and these sit in one row. A label that wraps turns a
        // shortcut into a paragraph.
        for (label, _, _) in TEMPLATES {
            assert!(label.len() <= 24, "{label:?} is too long for a chip");
        }
    }

    fn mentions_screen(template: &Template) -> bool {
        let text = template.prompt.to_lowercase();
        text.contains("screen") || text.contains("have open")
    }
}
