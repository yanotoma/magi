//! What a model can actually do, and what Magi does about it.
//!
//! Everything in this module is pure. The probing that produces a
//! [`Capabilities`] lives in [`crate::llm::preflight`] and touches the network;
//! deciding what a [`Capabilities`] *means* happens here and touches nothing, so
//! it can be tested exhaustively.
//!
//! That split exists because of how this code fails. A wrong tier raises nothing:
//! it hands a blind model the prompt that says a screen-capture tool is available,
//! and the model then offers to look at a screen it cannot see. There is no error
//! to catch, no log line, and the user's only clue is an assistant that behaves
//! strangely. So the decision is a total function over three booleans, and every
//! combination is asserted.

use serde::{Deserialize, Serialize};

/// What a model was observed to do, not what it claims.
///
/// Every field is the result of a probe that tried the thing. A model advertising
/// vision in its name or its `/v1/models` entry sets nothing here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    /// The endpoint answered at all, with this model and this key.
    pub reachable: bool,

    /// The model read a generated image and reported its contents correctly.
    ///
    /// False also covers the case that matters most: an endpoint that accepts an
    /// image payload without complaint and ignores it. Accepting is not seeing,
    /// and only asking about the contents can tell them apart.
    pub vision: bool,

    /// The model emitted a structurally valid tool call.
    ///
    /// Not "mentioned the tool" and not "produced output" — small local models
    /// routinely describe the call they would make in prose, which parses as
    /// nothing and would break an agentic loop.
    pub tools: bool,

    /// The model returned JSON matching a requested schema.
    ///
    /// Recorded but not used for tier assignment; see [`Tier`]. It is shown in
    /// Settings because it explains capabilities that arrive in later milestones.
    pub structured_output: bool,
}

/// How much of Magi works with a given model.
///
/// Assigned automatically and never configured by hand. The tier is not a score:
/// each one selects a different capture strategy and a different system prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tier {
    /// Vision and reliable tool-calling. The model decides when to look at the
    /// screen. This is the intended experience.
    Agentic,

    /// Vision without reliable tool-calling. Magi decides when to capture, using
    /// a local keyword heuristic over what the user said, and attaches the image
    /// before sending. The model is never told a tool exists.
    Heuristic,

    /// No vision. Capture is disabled and the model is told it cannot see the
    /// screen, so it stops offering to look.
    TextOnly,

    /// The endpoint could not be reached with this model and key. Distinct from
    /// [`Tier::TextOnly`] because nothing is known rather than something being
    /// absent — a text-only model works, an unreachable one does not, and telling
    /// the user "this model cannot see" when the real problem is a typo in the URL
    /// sends them to fix the wrong thing.
    Unreachable,
}

impl Tier {
    /// Whether Magi may capture the screen for this tier at all.
    pub fn allows_capture(self) -> bool {
        matches!(self, Tier::Agentic | Tier::Heuristic)
    }

    /// Whether the model is told that a capture tool exists.
    ///
    /// True for exactly one tier. [`Tier::Heuristic`] deliberately says nothing
    /// about tools even though it does send images: a model that malforms tool
    /// syntax will leak that syntax into prose if the concept is introduced, and
    /// the capture has already happened by the time the model sees the request.
    pub fn offers_capture_tool(self) -> bool {
        matches!(self, Tier::Agentic)
    }

    /// A short label for Settings and the tray tooltip.
    pub fn label(self) -> &'static str {
        match self {
            Tier::Agentic => "Agentic capture",
            Tier::Heuristic => "Assisted capture",
            Tier::TextOnly => "Text only",
            Tier::Unreachable => "Unreachable",
        }
    }

    /// Why the tier is what it is, in terms of what the user can act on.
    ///
    /// Settings shows this next to the capability matrix. A user who wonders why
    /// screen reading is off should not have to infer it from four checkmarks.
    pub fn explanation(self) -> &'static str {
        match self {
            Tier::Agentic => {
                "This model can see images and call tools reliably, so it decides \
                 for itself when to look at your screen."
            }
            Tier::Heuristic => {
                "This model can see images but does not call tools reliably, so Magi \
                 decides when to capture the screen and attaches it for you. \
                 Wording like \"this error\" or \"what is on my screen\" triggers it."
            }
            Tier::TextOnly => {
                "This model cannot see images, so screen capture is switched off. \
                 Pick a vision-capable model to turn it on."
            }
            Tier::Unreachable => {
                "Magi could not reach this model. Check the endpoint URL, the API \
                 key, and — for a local server — that it is running and the model \
                 is downloaded."
            }
        }
    }
}

/// Assigns a tier from observed capabilities.
///
/// Total by construction: every combination of the inputs maps to exactly one
/// tier, and `assigns_a_tier_for_every_combination` asserts that across all
/// sixteen. There is no fallback branch, because a fallback is where a
/// mis-assignment would hide.
///
/// Reachability is checked first and dominates. The other probes cannot produce a
/// meaningful negative when nothing answered — a vision probe against a wrong URL
/// fails for reasons that say nothing about the model.
///
/// `structured_output` deliberately does not affect the result. It is worth
/// showing and worth knowing, but nothing in v1's capture path depends on it, and
/// letting it move the tier would degrade capture for a model whose only weakness
/// is JSON-mode support.
pub fn assign(capabilities: &Capabilities) -> Tier {
    if !capabilities.reachable {
        return Tier::Unreachable;
    }

    match (capabilities.vision, capabilities.tools) {
        (true, true) => Tier::Agentic,
        (true, false) => Tier::Heuristic,
        // Tools without vision is still text only. Tool-calling is not the
        // capability the tiers are about: they describe how an image reaches the
        // model, and there is no image to route.
        (false, _) => Tier::TextOnly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(reachable: bool, vision: bool, tools: bool, structured: bool) -> Capabilities {
        Capabilities {
            reachable,
            vision,
            tools,
            structured_output: structured,
        }
    }

    /// The exhaustive table. Sixteen combinations, each with its expected tier.
    ///
    /// Written as data rather than as separate tests so that adding a capability
    /// field forces this list to be revisited: a new boolean doubles the rows, and
    /// a doubled table cannot be left half-filled without the count assertion
    /// failing.
    #[test]
    fn assigns_a_tier_for_every_combination() {
        let expected = [
            // reachable, vision, tools, structured -> tier
            (false, false, false, false, Tier::Unreachable),
            (false, false, false, true, Tier::Unreachable),
            (false, false, true, false, Tier::Unreachable),
            (false, false, true, true, Tier::Unreachable),
            (false, true, false, false, Tier::Unreachable),
            (false, true, false, true, Tier::Unreachable),
            (false, true, true, false, Tier::Unreachable),
            (false, true, true, true, Tier::Unreachable),
            (true, false, false, false, Tier::TextOnly),
            (true, false, false, true, Tier::TextOnly),
            (true, false, true, false, Tier::TextOnly),
            (true, false, true, true, Tier::TextOnly),
            (true, true, false, false, Tier::Heuristic),
            (true, true, false, true, Tier::Heuristic),
            (true, true, true, false, Tier::Agentic),
            (true, true, true, true, Tier::Agentic),
        ];

        assert_eq!(
            expected.len(),
            16,
            "four booleans have sixteen combinations; the table must cover all of them"
        );

        for (reachable, vision, tools, structured, tier) in expected {
            let capabilities = caps(reachable, vision, tools, structured);
            assert_eq!(
                assign(&capabilities),
                tier,
                "wrong tier for reachable={reachable} vision={vision} \
                 tools={tools} structured={structured}"
            );
        }
    }

    #[test]
    fn unreachable_dominates_every_other_probe() {
        // A vision probe against the wrong URL fails for a reason that says
        // nothing about the model, so a negative there must not be reported as
        // "cannot see".
        assert_eq!(assign(&caps(false, true, true, true)), Tier::Unreachable);
    }

    #[test]
    fn structured_output_never_changes_the_tier() {
        for reachable in [true, false] {
            for vision in [true, false] {
                for tools in [true, false] {
                    assert_eq!(
                        assign(&caps(reachable, vision, tools, false)),
                        assign(&caps(reachable, vision, tools, true)),
                        "structured output moved the tier for reachable={reachable} \
                         vision={vision} tools={tools}"
                    );
                }
            }
        }
    }

    #[test]
    fn tools_without_vision_is_still_text_only() {
        // The tiers describe how an image reaches the model. With no vision there
        // is no image to route, however good the tool-calling is.
        assert_eq!(assign(&caps(true, false, true, true)), Tier::TextOnly);
    }

    #[test]
    fn only_the_agentic_tier_is_told_about_the_capture_tool() {
        // The heuristic tier sends images without ever mentioning tools. That is
        // the whole point of it: the model malforms tool syntax, so introducing
        // the concept only invites that syntax into prose.
        assert!(Tier::Agentic.offers_capture_tool());
        assert!(!Tier::Heuristic.offers_capture_tool());
        assert!(!Tier::TextOnly.offers_capture_tool());
        assert!(!Tier::Unreachable.offers_capture_tool());
    }

    #[test]
    fn capture_is_allowed_for_both_vision_tiers() {
        assert!(Tier::Agentic.allows_capture());
        assert!(Tier::Heuristic.allows_capture());
        assert!(!Tier::TextOnly.allows_capture());
        assert!(!Tier::Unreachable.allows_capture());
    }

    #[test]
    fn default_capabilities_are_unreachable() {
        // `Default` is what a provider looks like before it has been probed, and
        // an unprobed model must not be treated as capable.
        assert_eq!(assign(&Capabilities::default()), Tier::Unreachable);
    }

    #[test]
    fn every_tier_explains_itself_in_actionable_terms() {
        for tier in [
            Tier::Agentic,
            Tier::Heuristic,
            Tier::TextOnly,
            Tier::Unreachable,
        ] {
            assert!(!tier.label().is_empty());
            let explanation = tier.explanation();
            assert!(
                explanation.len() > 40,
                "{tier:?} needs an explanation a user can act on, not a restatement"
            );
        }
    }

    #[test]
    fn tiers_round_trip_through_serde_as_kebab_case() {
        // The cache file and the Settings UI both carry these across a boundary,
        // and renaming a variant would silently invalidate every cached entry.
        let json = serde_json::to_string(&Tier::TextOnly).expect("serialisable");
        assert_eq!(json, "\"text-only\"");
        let back: Tier = serde_json::from_str(&json).expect("deserialisable");
        assert_eq!(back, Tier::TextOnly);
    }
}
