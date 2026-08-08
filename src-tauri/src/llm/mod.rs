//! Talking to language models.
//!
//! Organised as a trait with implementations rather than one client with
//! per-vendor branches. "OpenAI-compatible" is a family of endpoints, not a
//! specification, and Anthropic sits outside it — see
//! `.claude/skills/llm-providers/SKILL.md`.

pub mod anthropic;
pub mod cache;
pub mod capability;
pub mod discovery;
pub mod openai;
pub mod preflight;
pub mod probe_image;
pub mod prompt;
pub mod provider;
pub mod registry;
pub mod sse;
pub mod tools;
