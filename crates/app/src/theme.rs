//! P-67: the persisted colour scheme, applied — and what libcosmic does and
//! does not provide for it.
//!
//! The reference applies the scheme at **two** sites and nowhere else:
//! `theme.py:85` `apply()` → `setPalette(build_palette(scheme))`, called from
//! `main.py:106` **at startup** with the persisted value and from
//! `bridge.py:201-202` **on change** from the Settings selector. This module is
//! the port's equivalent of those two call sites: [`theme_for`] is the mapping
//! and [`apply`] is the task both of them return.
//!
//! # There is no `theme()` hook to override, and T-24's row says there is
//!
//! The row (and `D-13`'s T-13 finding) read *"the port has no `Shell::theme()`
//! override"*, listing it as the thing to add. **Measured against the vendored
//! libcosmic at the pinned rev, there is no such hook and there cannot be one:**
//!
//! * `Cosmic::theme` (`src/app/cosmic.rs:697-704`) is libcosmic's own function
//!   and returns `crate::theme::active()`. `Application` has no `theme` method
//!   for an app to implement — the only `fn theme` in the whole crate is that
//!   one.
//! * `active()` reads `static THEME` (`src/theme/mod.rs:38-58`), which is
//!   `pub(crate)`, so an application cannot write it.
//! * The one *public* way to change it is
//!   `cosmic::command::set_theme::<M>(theme) -> Task<Action<M>>`
//!   (`src/command.rs:35-38`, behind the `winit` feature this app enables) —
//!   which is already the exact type `Shell::update` returns, so it needs no
//!   adaptation.
//!
//! A reader who trusted the row would go looking for a hook that does not exist.
//!
//! # The startup half, and why it is a task rather than a setting
//!
//! `app::Settings` has a `theme` field and libcosmic reads it
//! (`src/app/mod.rs:51`), but it is `pub(crate)` with **no builder** — the only
//! builder on that struct is `default_icon_theme`
//! (`src/app/settings.rs:71-75`). Its default is
//! `theme: crate::theme::system_preference()` (`:99`), which means **the app
//! already follows the desktop today**, before this module existed: that is
//! libcosmic's default rather than anything the port asked for, and it is why
//! the choice the Settings page offers had no effect — "dark", "light" and
//! "system" all landed on `system_preference()`.
//!
//! So the persisted scheme cannot reach `Settings`; it reaches the running app
//! through the same [`apply`] the change handler uses, returned from
//! `App::init`. The visible consequence is that the first frame may be drawn
//! with the system theme before the task lands — the reference has no such
//! window, because it sets the palette before `engine.load`.
//!
//! # What `"system"` means here, and the trap in it
//!
//! `system_preference()` (`src/theme/mod.rs:119-132`) has **two** early returns,
//! both `Theme::dark()`: one when the COSMIC `ThemeMode` config cannot be read,
//! one when `ThemeMode::is_dark` fails. So "system" on a desktop with no COSMIC
//! config *looks exactly like* "dark" — which is finding **#58**, and it is why
//! [`theme_for`] takes the system theme as a **parameter** instead of calling
//! `system_preference()` itself: with the value passed in, a test can hand it a
//! theme that is neither light nor dark and assert the scheme **defers**, which
//! is the property that distinguishes "asked the system" from "fell back to
//! dark". Called internally, that distinction is unobservable and the test would
//! agree with a port that ignored "system" entirely.
//!
//! # The palette itself: COSMIC's, not Breeze's
//!
//! The reference builds a thirteen-colour Breeze-flavoured `QPalette`
//! (`theme.py:20-30`) and, for `"system"`, restores the palette the platform
//! handed Qt at startup (`theme.py:78-88`). This port uses
//! `Theme::dark` / `Theme::light` / `system_preference` instead, i.e. the COSMIC
//! theme system's own palettes. That is a deliberate divergence and it is the
//! same one the rest of the migration makes: the reference's palette exists
//! because **Kirigami takes its colours from the platform palette**, which
//! `theme.py`'s own docstring says, and a COSMIC app that hardcoded Breeze's hex
//! values would be a second visual idiom fighting the desktop it now runs on.
//! The *feature* P-67 promises — three-way light / dark / system — is preserved
//! exactly; the colours are the toolkit's.
//!
//! [`Shell::update`]: crate::Shell

/// `"light"` (`settings.py:13`).
pub const LIGHT: &str = "light";

/// `"dark"` (`settings.py:13`).
pub const DARK: &str = "dark";

// The other two of the reference's three values are deliberately **not**
// constants here, and the asymmetry is the point rather than an oversight.
//
// `"system"` is covered by the catch-all arm of [`theme_for`], because
// `theme.py:79-88` gives it and an unrecognised value identical treatment —
// `build_palette` returns `None` for either and `apply` restores the platform
// palette. Naming it in the match would leave the fallback to be written
// separately, which is how two arms that must agree come to disagree.
//
// `"dark"` is both a scheme the user may choose and the value the load path
// folds an absent or invalid one to (`settings.rs:145-147`); the production code
// never needs to name that second role, and the tests below assert the
// relationship instead of restating it.

/// The theme a stored scheme asks for — `build_palette` and the choice around
/// it (`theme.py:79-88`), with the palette supplied by the toolkit.
///
/// `system` is whatever the caller passes as `system`, which is the whole point
/// of the parameter: see the module doc. Anything that is not `"light"` or
/// `"dark"` also defers, which is the reference's own fallback —
/// `build_palette` returns `None` for any scheme it does not recognise and
/// `apply` then restores the platform palette (`theme.py:79-88`), i.e. behaves
/// as `"system"`.
///
/// A stored value that is neither is unreachable in practice: the load path
/// folds it to `"dark"` (`settings.rs:145-147`) and the message path ignores
/// it (`main.rs`'s `SetColorScheme` arm). The arm is therefore a defence rather
/// than a behaviour — and it is written to match the reference's *fallback*
/// rather than to guess, because those two differ: folding an unknown value to
/// dark would let an unreadable settings file change the user's theme.
pub fn theme_for(scheme: &str, system: cosmic::Theme) -> cosmic::Theme {
    match scheme {
        LIGHT => cosmic::Theme::light(),
        DARK => cosmic::Theme::dark(),
        _ => system,
    }
}

/// Change a running app's theme to what `scheme` asks for.
///
/// `cosmic::command::set_theme` is the whole implementation — it is the only
/// public writer of libcosmic's theme static (see the module doc), and its
/// return type is already `Task<Action<M>>`, which is what `Shell::update`
/// returns, so nothing is adapted here.
///
/// **Both halves of P-67 call this one function**: `App::init` for the persisted
/// value at startup (`main.py:106`) and `Shell::update`'s `SetColorScheme` arm
/// for a change (`bridge.py:201-202`). The reference reaches one function,
/// `theme.py:85`, from both, and a second name here for the startup case would
/// be a second thing to keep in step.
///
/// # What this does not close — measured, not argued
///
/// `theme_for` is covered (see the tests: four mutations of its match, all
/// caught, including a `_ => Theme::dark()` fallback that a naive
/// `is_dark()` assertion would have accepted). **The `init` call site in
/// `main.rs` is not**, and it cannot be from here:
///
/// ```text
/// F  SetColorScheme's arm stores the value and returns no task  DEAD — U7's
///    `setting_the_color_scheme_returns_the_apply_task` pins the task's
///    existence (though not its content: which scheme the apply carries is as
///    unreadable as G, and the list does not pretend otherwise)
/// G  init applies a hardcoded "dark" instead of the stored scheme  SURVIVES
/// H  init returns no task at all, so the theme is never applied  SURVIVES
/// ```
///
/// The reason is the same wall `install_press`'s call site hits: the value that
/// flows into the plumbing is not readable back out of it. `set_theme` returns
/// an `iced::Task`, and a task is a value with no accessor — driving it needs an
/// executor and a window, and what it produces is an `Action` the framework
/// consumes rather than a message a test could match on.
///
/// So the honest statement of T-24's guarantees is: **the mapping is tested, the
/// plumbing is read.** P-67's acceptance criteria (a) "applied at startup" and
/// (b) "changing it changes the app" are Phase 3 items by construction — which is
/// what the task row's own criterion (e) already says, and why this paragraph
/// exists rather than three tests that would be satisfied by the very thing they
/// cannot distinguish.
pub fn apply<M: Send + 'static>(scheme: &str) -> cosmic::app::Task<M> {
    cosmic::command::set_theme(theme_for(scheme, cosmic::theme::system_preference()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::theme::ThemeType;

    /// Imported here rather than at the top of the file because the production
    /// code never names it: `theme_for` matches the three spellings directly, and
    /// the tuple is what *checks* them. Keeping the import beside the test that
    /// uses it is what makes that division visible.
    use gamehandler_core::settings::COLOR_SCHEMES;

    /// The reference's follow-the-desktop value (`settings.py:13`).
    ///
    /// Test-local, not a module constant, and that is deliberate rather than
    /// tidiness: production **cannot** name it without splitting [`theme_for`]'s
    /// catch-all in two, and the catch-all is load-bearing — `theme.py:79-88`
    /// treats `"system"` and an unrecognised value *identically*, so one arm must
    /// cover both or the two can drift. The tests are the only place that needs
    /// to say which value defers, so this is where it is defined.
    const SYSTEM_SCHEME: &str = "system";

    /// The scheme `Settings::default` opens with (`settings.py:21`), named here
    /// so the test below can assert it is one of `COLOR_SCHEMES` rather than
    /// repeating the literal in an assertion about itself.
    const DEFAULT_SCHEME: &str = "dark";

    /// A theme that is neither light nor dark, used as the `system` argument so
    /// "the scheme deferred" is distinguishable from "the scheme chose dark".
    ///
    /// High-contrast **light** rather than another dark theme: if the branch
    /// under test fell back to `Theme::dark()`, a dark sentinel would compare
    /// unequal and the test would still pass — by luck rather than by the
    /// property it claims.
    fn sentinel() -> cosmic::Theme {
        cosmic::Theme::light_hc()
    }

    /// The two forced schemes pick the toolkit's own light and dark, and the
    /// sentinel is left alone — so neither branch can be passing by deferring.
    #[test]
    fn light_and_dark_are_the_toolkits_own_and_not_the_system_theme() {
        let light = theme_for(LIGHT, sentinel());
        assert_eq!(light.theme_type, ThemeType::Light);
        assert!(!light.theme_type.is_dark());
        assert_ne!(light, sentinel(), "light must not defer to the system");

        let dark = theme_for(DARK, sentinel());
        assert_eq!(dark.theme_type, ThemeType::Dark);
        assert!(dark.theme_type.is_dark());
        assert_ne!(dark, sentinel(), "dark must not defer to the system");

        assert_ne!(light, dark, "the two forced schemes are not the same theme");
    }

    /// **`"system"` defers to the theme it is handed, whatever that is.**
    ///
    /// This is the acceptance criterion "`system` follows the desktop", stated
    /// as the one thing a test can actually hold. It cannot assert that the
    /// desktop was followed — `system_preference()` returns `Theme::dark()`
    /// whenever the COSMIC config is unreadable (#58), and in a test environment
    /// it always is — so asserting `theme_for("system", ..).is_dark()` would
    /// pass for a port that ignored "system" entirely. Handing it a sentinel and
    /// requiring it back is the difference between "asked the system" and "fell
    /// back to dark", and it is why the parameter exists.
    #[test]
    fn system_defers_to_the_theme_it_is_handed_and_does_not_fall_back_to_dark() {
        let system = sentinel();
        assert_eq!(
            theme_for(SYSTEM_SCHEME, system.clone()),
            system,
            "\"system\" must return the theme it was given, unchanged"
        );
        assert_eq!(
            theme_for(SYSTEM_SCHEME, system.clone()).theme_type,
            ThemeType::HighContrastLight,
            "the sentinel's own type, unreplaced — a `Theme::dark()` fallback \
             here is #58's defect wearing the right answer's clothes"
        );
    }

    /// An unrecognised scheme defers too, which is the reference's fallback
    /// (`theme.py:79-88` returns `None` and `apply` restores the platform
    /// palette) — **not** dark, which is the tempting reading.
    ///
    /// Unreachable from stored data, since the load path folds an unknown value
    /// to `"dark"` before it can get here; pinned so that if it ever becomes
    /// reachable the behaviour is the reference's rather than an invention.
    #[test]
    fn an_unrecognised_scheme_takes_the_references_fallback_and_not_dark() {
        for scheme in ["", "neon", "Dark", "SYSTEM", "system ", "lightt"] {
            assert_eq!(
                theme_for(scheme, sentinel()),
                sentinel(),
                "{scheme:?} is not a scheme the reference knows, so `apply` \
                 restores the platform palette (theme.py:79-88)"
            );
        }
    }

    /// Every name the settings file may store maps to a branch of its own —
    /// i.e. `COLOR_SCHEMES` and this module's match agree.
    ///
    /// The anti-drift check, and the reason the constant is imported rather than
    /// re-listed: if someone adds or renames a scheme in `core::settings` and
    /// not here, the new name would silently take the deferring arm, and a user
    /// selecting it would find it indistinguishable from "system" — which is
    /// precisely the state T-24 exists to end.
    #[test]
    fn every_scheme_the_settings_file_stores_has_a_branch_of_its_own() {
        let sentinel = sentinel();
        let mut seen: Vec<cosmic::Theme> = Vec::new();

        for scheme in COLOR_SCHEMES {
            let theme = theme_for(scheme, sentinel.clone());
            if scheme != SYSTEM_SCHEME {
                assert_ne!(
                    theme, sentinel,
                    "{scheme} deferred to the system theme, so the settings \
                     file offers a choice this module cannot act on"
                );
            }
            assert!(
                !seen.contains(&theme),
                "{scheme} maps to a theme another scheme already maps to"
            );
            seen.push(theme);
        }

        assert_eq!(seen.len(), COLOR_SCHEMES.len());
        assert_eq!(COLOR_SCHEMES.len(), 3, "the reference's three-way choice");
        assert!(
            COLOR_SCHEMES.contains(&DEFAULT_SCHEME),
            "the default the settings file writes must be one of its own schemes"
        );
        assert!(
            COLOR_SCHEMES.contains(&SYSTEM_SCHEME),
            "the follow-the-desktop value must be one the settings file accepts"
        );
    }

    /// The three names this module compares against are the reference's own
    /// spellings (`settings.py:13`) — not bare literals that could drift from
    /// the tuple the settings file is validated against.
    #[test]
    fn the_names_compared_here_are_the_references_own() {
        assert_eq!(LIGHT, "light");
        assert_eq!(DARK, "dark");
        assert_eq!(SYSTEM_SCHEME, "system");
        assert_eq!(COLOR_SCHEMES, [SYSTEM_SCHEME, LIGHT, DARK]);
    }
}
