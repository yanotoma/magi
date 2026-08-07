//! Talking to language models.
//!
//! Organised as a trait with implementations rather than one client with
//! per-vendor branches. "OpenAI-compatible" is a family of endpoints, not a
//! specification, and Anthropic sits outside it — see
//! `.claude/skills/llm-providers/SKILL.md`.

pub mod anthropic;
pub mod discovery;
pub mod openai;
pub mod provider;
pub mod registry;
pub mod sse;
