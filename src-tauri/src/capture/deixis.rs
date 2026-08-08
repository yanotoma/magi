//! Does what the user said point at the screen?
//!
//! This is the [`Tier::Heuristic`] path: a model that can see but cannot be trusted to
//! call a tool never gets offered one, so Magi has to decide for itself whether to attach
//! a screenshot before sending. The decision is made here, from words alone — no model
//! call, no network, no screen access. It is deliberately dumb, and being dumb is what
//! makes it fast enough to run on every turn and simple enough to test exhaustively.
//!
//! The design doc settles the trade-off explicitly: *"It is allowed to over-trigger — a
//! spurious capture costs tokens, a missed one costs a wrong answer."* So this errs toward
//! capturing. A bare "this" is enough. What stops that from meaning "capture every turn"
//! is that a great many real questions contain no deictic at all — "what is a mutex", "how
//! do I center a div", "explain closures" — and those are the ones this must stay silent
//! on. The two M5 guards that bound the cost of over-triggering live elsewhere: a cap on
//! captures per turn, and an audit log the user can read.
//!
//! [`Tier::Heuristic`]: crate::llm::Tier::Heuristic

/// A phrase that means "look at my screen", in one language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Trigger {
    /// The words, in order, already lowercased so they compare equal to normalised input.
    ///
    /// Accents are significant. See [`normalise`] for why stripping them is not safe.
    words: &'static [&'static str],

    /// ISO 639-1 code of the language it belongs to.
    ///
    /// Carried for the audit log rather than for matching: every trigger is tested against
    /// every transcript regardless of language. Typed input has no detected language at
    /// all, and a bilingual user switches mid-sentence, so filtering by language would
    /// lose more than it saves. See [`asks_about_the_screen`] for why this is safe.
    language: &'static str,
}

/// What matched, for the audit log.
///
/// The user is entitled to know *why* their screen was read. "Captured because you said
/// 'this error'" is an answer; "captured by heuristic" is not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deixis {
    /// The matched phrase, lowercased, exactly as compared.
    pub phrase: String,

    /// ISO 639-1 code of the language the phrase belongs to.
    pub language: &'static str,
}

/// Phrases that point at the screen.
///
/// **English and Spanish only, and that is a known limitation rather than an oversight.**
/// Settings offers eleven languages for speech, so a user who selects German gets a Tier 2
/// model that never captures — the heuristic simply never fires. Writing German, Japanese
/// or Russian deixis without someone who speaks them to check it is how a plausible wrong
/// answer gets committed, and this file is the wrong place to guess. Adding a language is
/// adding rows here; `docs/TASKS.md` carries the task.
///
/// Ordered longest-phrase-first is *not* required — [`asks_about_the_screen`] prefers the
/// longest match on its own, so entries can be grouped by meaning instead.
const TRIGGERS: &[Trigger] = &[
    // Locatives. The clearest signal there is: a place, and the place is the screen.
    Trigger {
        words: &["here"],
        language: "en",
    },
    Trigger {
        words: &["aqui"],
        language: "es",
    },
    Trigger {
        words: &["aquí"],
        language: "es",
    },
    // Explicit references to the display itself.
    Trigger {
        words: &["this", "screen"],
        language: "en",
    },
    Trigger {
        words: &["my", "screen"],
        language: "en",
    },
    Trigger {
        words: &["the", "screen"],
        language: "en",
    },
    Trigger {
        words: &["on", "screen"],
        language: "en",
    },
    Trigger {
        words: &["esta", "pantalla"],
        language: "es",
    },
    Trigger {
        words: &["ésta", "pantalla"],
        language: "es",
    },
    Trigger {
        words: &["la", "pantalla"],
        language: "es",
    },
    Trigger {
        words: &["en", "pantalla"],
        language: "es",
    },
    Trigger {
        words: &["mi", "pantalla"],
        language: "es",
    },
    // Referring to the act of looking. Common in speech and unambiguous.
    Trigger {
        words: &["looking", "at"],
        language: "en",
    },
    Trigger {
        words: &["i", "am", "seeing"],
        language: "en",
    },
    Trigger {
        words: &["do", "you", "see"],
        language: "en",
    },
    Trigger {
        words: &["can", "you", "see"],
        language: "en",
    },
    Trigger {
        words: &["estoy", "viendo"],
        language: "es",
    },
    Trigger {
        words: &["ves"],
        language: "es",
    },
    Trigger {
        words: &["mira"],
        language: "es",
    },
    // Bare demonstratives. Broad on purpose — see the module docs.
    Trigger {
        words: &["this"],
        language: "en",
    },
    Trigger {
        words: &["these"],
        language: "en",
    },
    Trigger {
        words: &["those"],
        language: "en",
    },
    Trigger {
        words: &["esto"],
        language: "es",
    },
    Trigger {
        words: &["este"],
        language: "es",
    },
    Trigger {
        words: &["esta"],
        language: "es",
    },
    // Pre-2010 orthography still accented the pronouns, and speech-to-text reproduces
    // whatever its training data spelled. `ésta` is the pronoun; `está` is the verb "is"
    // and is deliberately absent.
    Trigger {
        words: &["ésta"],
        language: "es",
    },
    Trigger {
        words: &["éste"],
        language: "es",
    },
    Trigger {
        words: &["ésto"],
        language: "es",
    },
    Trigger {
        words: &["estos"],
        language: "es",
    },
    Trigger {
        words: &["estas"],
        language: "es",
    },
    Trigger {
        words: &["eso"],
        language: "es",
    },
    Trigger {
        words: &["ese"],
        language: "es",
    },
    Trigger {
        words: &["esa"],
        language: "es",
    },
    Trigger {
        words: &["esos"],
        language: "es",
    },
    Trigger {
        words: &["esas"],
        language: "es",
    },
];

/// Whether the transcript points at something on screen, and what said so.
///
/// Returns the **longest** matching phrase, so the audit log reads "this error" rather than
/// "this" when both matched. Ties go to whichever appears first in [`TRIGGERS`].
///
/// Every trigger is tested regardless of the transcript's language, which is safe here for
/// a specific reason worth stating: the English and Spanish phrases do not collide. No
/// Spanish word in [`TRIGGERS`] is also an English word, or vice versa. That is a property
/// of *these* entries, not a general truth about languages — adding a language means
/// checking its words against the existing ones, because a word that means "the" in one
/// language and "this" in another would fire on every sentence.
///
/// Two words that look like they belong here and do not: English `that`, which is a
/// conjunction and a relative pronoun far more often than a demonstrative, and Spanish
/// `está`, which is the verb "is". Both are argued in the tests that keep them out.
pub fn asks_about_the_screen(text: &str) -> Option<Deixis> {
    let tokens = normalise(text);
    if tokens.is_empty() {
        return None;
    }

    let mut best: Option<&Trigger> = None;

    for trigger in TRIGGERS {
        if !contains_phrase(&tokens, trigger.words) {
            continue;
        }
        // Longer wins. `>` rather than `>=` keeps the first of equal-length matches, which
        // makes the result depend on TRIGGERS order rather than on transcript order — the
        // former is stable and reviewable, the latter is not.
        if best.is_none_or(|current| trigger.words.len() > current.words.len()) {
            best = Some(trigger);
        }
    }

    best.map(|trigger| Deixis {
        phrase: trigger.words.join(" "),
        language: trigger.language,
    })
}

/// Splits text into comparable words.
///
/// Lowercases and treats every non-alphanumeric character as a separator. Tokenising rather
/// than searching the raw string is what makes `"this"` fail to match `"thistle"` — a
/// substring search would match it, and a hand-rolled word-boundary check gets punctuation
/// wrong sooner or later.
///
/// **Deliberately does not strip accents.** Folding `í` to `i` looks like a kindness to
/// someone typing "aqui" for "aquí", and in Spanish it silently merges two different words:
/// `está` ("is") becomes `esta` ("this"), so *"¿está funcionando?"* would ask for a
/// screenshot. `está` is among the most common words in the language, which makes that a
/// capture on a large share of ordinary questions. The accented and unaccented spellings are
/// listed separately in [`TRIGGERS`] instead — more rows, no collisions, and each row says
/// exactly what it matches. Note that the pronoun `ésta` and the verb `está` differ in
/// *which* vowel carries the accent, so listing both is unambiguous.
fn normalise(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether `phrase` appears in `tokens` as a run of consecutive whole words.
fn contains_phrase(tokens: &[String], phrase: &[&str]) -> bool {
    if phrase.is_empty() || phrase.len() > tokens.len() {
        return false;
    }

    tokens
        .windows(phrase.len())
        .any(|window| window.iter().zip(phrase).all(|(token, word)| token == word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fires(text: &str) -> bool {
        asks_about_the_screen(text).is_some()
    }

    #[test]
    fn the_design_docs_own_examples_fire() {
        // Straight from the design doc's Tier 2 row: "here", "this", "this error",
        // "this screen". If these ever stop matching, the documented behaviour is gone.
        assert!(fires("what's happening here?"));
        assert!(fires("what does this mean"));
        assert!(fires("what is this error"));
        assert!(fires("explain this screen"));
    }

    #[test]
    fn questions_with_no_referent_stay_silent() {
        // The whole reason this heuristic is worth having rather than capturing every
        // turn. Each of these is a complete question that needs no screen at all, and a
        // capture here costs vision tokens that stay in the thread for every later turn.
        assert!(!fires("what is a mutex"));
        assert!(!fires("how do I center a div"));
        assert!(!fires("explain closures in Rust"));
        assert!(!fires("write a haiku about compilers"));
        assert!(!fires("qué es un mutex"));
        assert!(!fires("cómo se declara una variable en Rust"));
    }

    #[test]
    fn spanish_deixis_fires_too() {
        // The gap M4 opened. Speech-to-text now accepts eleven languages, so an
        // English-only keyword list means a Spanish speaker on a Tier 2 model never gets
        // a capture — and nothing would report that, because "no match" is
        // indistinguishable from "nothing to look at".
        assert!(fires("qué es este error"));
        assert!(fires("qué significa esto"));
        assert!(fires("puedes ver esta pantalla"));
        assert!(fires("qué hay aquí"));
        assert!(fires("mira lo que estoy viendo"));
    }

    #[test]
    fn both_spellings_of_an_accented_trigger_fire() {
        // Speech-to-text writes "aquí"; a person typing the same question often writes
        // "aqui". Both have to work, or the feature is reliable only by voice.
        //
        // Listed as two rows rather than folded into one, so the reported phrase is the
        // spelling that actually matched — and, more importantly, so no *other* pair of
        // words gets merged as a side effect. See `normalise`.
        assert!(fires("qué hay aquí"));
        assert!(fires("que hay aqui"));
        assert_eq!(
            asks_about_the_screen("qué hay aquí").map(|d| d.language),
            Some("es")
        );
    }

    #[test]
    fn a_trigger_inside_a_longer_word_does_not_count() {
        // The bug a substring search would have. Every one of these contains a trigger's
        // letters inside a longer word, and none of them points at anything.
        assert!(!fires("the thistle is a flower")); // this
        assert!(!fires("esteban wrote the parser")); // este
        assert!(!fires("adhere to the style guide")); // here
        assert!(!fires("a thatched roof")); // that
        assert!(!fires("their theses were published")); // these
        assert!(!fires("el estado del servidor")); // esta
        assert!(!fires("the esophagus is a tube")); // eso
    }

    #[test]
    fn punctuation_does_not_hide_a_trigger() {
        // "(this)" and "this," must tokenise to the same word as "this".
        assert!(fires("what is this?"));
        assert!(fires("what is (this)"));
        assert!(fires("what,is.this"));
        assert!(fires("THIS"));
    }

    #[test]
    fn the_spanish_verb_esta_does_not_ask_for_a_screenshot() {
        // The bug an accent-stripping normaliser causes. `está` ("is") folds to `esta`
        // ("this"), and `está` is among the most common words in Spanish — so every
        // "¿está funcionando?" would have sent a screenshot. The accent is load-bearing.
        assert!(!fires("está funcionando"));
        assert!(!fires("el servidor está caído"));
        assert!(!fires("no sé si está bien"));

        // And the pronoun, which differs only in which vowel carries the accent, still
        // fires. If these two ever collapse into one token, the test above breaks.
        assert!(fires("ésta es la respuesta"));
        assert!(fires("esta es la respuesta"));
    }

    #[test]
    fn the_english_conjunction_that_does_not_ask_for_a_screenshot() {
        // `that` is a demonstrative perhaps a fifth of the time and a conjunction or
        // relative pronoun the rest. Including it would fire on a large share of ordinary
        // questions, which is the "capture every turn" cost the design doc rejects on
        // token grounds — so it is deliberately absent from TRIGGERS.
        assert!(!fires("explain that closures capture variables"));
        assert!(!fires("make sure that the tests pass"));
        assert!(!fires("I think that is wrong"));

        // And when a sentence contains both, the match is the demonstrative — so the
        // audit log names the word that actually pointed at something.
        let found = asks_about_the_screen("the crate that solves this").expect("fires");
        assert_eq!(found.phrase, "this");

        // The demonstratives that are unambiguous stay in.
        assert!(fires("what are those"));
        assert!(fires("what are these"));
    }

    #[test]
    fn the_longest_phrase_is_reported() {
        // Matters for the audit log, which shows the user why their screen was read.
        // "this screen" is a better explanation than "this", and both matched.
        let found = asks_about_the_screen("explain this screen to me").expect("fires");
        assert_eq!(found.phrase, "this screen");

        let found = asks_about_the_screen("puedes ver esta pantalla").expect("fires");
        assert_eq!(found.phrase, "esta pantalla");
    }

    #[test]
    fn the_language_is_reported_for_the_audit_log() {
        assert_eq!(
            asks_about_the_screen("what is this error").map(|d| d.language),
            Some("en")
        );
        assert_eq!(
            asks_about_the_screen("qué es este error").map(|d| d.language),
            Some("es")
        );
    }

    #[test]
    fn empty_and_whitespace_input_is_not_a_match() {
        assert!(!fires(""));
        assert!(!fires("   "));
        assert!(!fires("?!..."));
    }

    #[test]
    fn a_multi_word_phrase_needs_its_words_adjacent() {
        // "this" and "screen" both appear, but the phrase "this screen" does not. It
        // still fires on the bare "this" — what is asserted here is that the *reported*
        // phrase is the bare one, so the audit log does not claim a match that was not
        // there.
        let found = asks_about_the_screen("this is not about a screen").expect("fires");
        assert_eq!(found.phrase, "this");
    }

    #[test]
    fn no_english_trigger_is_also_a_spanish_trigger() {
        // The property that makes matching every language against every transcript safe.
        // If a future language contributes a word that means "the" in one language and
        // "this" in another, this test is where it gets caught.
        for a in TRIGGERS {
            for b in TRIGGERS {
                if a.language == b.language {
                    continue;
                }
                assert_ne!(
                    a.words, b.words,
                    "{:?} appears in both {} and {}",
                    a.words, a.language, b.language
                );
            }
        }
    }

    #[test]
    fn every_trigger_is_stored_already_normalised() {
        // A trigger written with a capital or an accent would never match, because input
        // is normalised and triggers are not. Silent, and invisible in review.
        for trigger in TRIGGERS {
            for word in trigger.words {
                let normalised = normalise(word);
                assert_eq!(
                    normalised,
                    vec![word.to_string()],
                    "trigger word {word:?} is not in normalised form"
                );
            }
        }
    }
}
