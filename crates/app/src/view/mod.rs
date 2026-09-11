//! The interface: pure layout over the model, with no state of its own.
//!
//! # What this module is allowed to know
//!
//! Nothing here reads [`State`], and nothing here emits a [`Message`]. The
//! widget functions are generic over the message type —
//! `fn card<M: Clone + 'static>(…) -> Element<'_, M>` — so the caller fixes
//! `M` when it binds them, and this layer never learns what `M` is. A widget
//! that only lays out produces no messages, which is what makes the whole layer
//! testable before the state contract exists and bindable afterwards without a
//! rewrite.
//!
//! # The split, and why it is worth keeping
//!
//! Each visible decision is a **pure function over data** in one of the small
//! modules below, and the widget builder that calls it is thin:
//!
//! | Decision | Module |
//! |---|---|
//! | which cover to draw, and how it fits | [`cover`] |
//! | the text under a game's name | [`meta`] |
//! | how big a tile and its cover are | [`metrics`] |
//!
//! That is deliberate, and it is the project's own rule rather than a
//! preference: `main.rs` records that "every non-trivial decision lives in
//! `gamehandler-core`, where it is tested without a display". These functions
//! are view-shaped so they cannot live there, but they are written to the same
//! standard — no renderer, no display, no fixture that can only be built by
//! running the app.
//!
//! # What the tests here are for
//!
//! The tests assert the *data* — the chosen cover source, the subtitle string,
//! the arithmetic — not the widget tree. A test that rendered a widget and
//! looked for text would need a display, a font stack and an event loop, and
//! would still be asserting the same strings.
//!
//! [`State`]: crate::App
//! [`Message`]: crate::Message

pub mod badge;
pub mod cover;
pub mod installers;
pub mod library;
pub mod meta;
pub mod metrics;
pub mod runners;
pub mod settings;
pub mod widgets;
