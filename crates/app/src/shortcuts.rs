//! P-68: the accelerators `Main.qml:121-143` binds, and where each one is
//! implemented.
//!
//! The reference binds four:
//!
//! ```text
//! Ctrl+N   root.openGameForm("")                              Main.qml:121-125
//! Ctrl+F   root.showPage("library"); focusSearch()             Main.qml:126-134
//! Ctrl+,   root.showPage("settings")                           Main.qml:135-139
//! Ctrl+Q   backend.quit()                                      Main.qml:140-143
//! ```
//!
//! **Only three of them are here, and the fourth is not an omission.**
//! `Ctrl+F` is already bound by libcosmic itself: `keyboard_nav::subscription()`
//! (`src/keyboard_nav.rs:50-55`) matches `Character("f")` with Control and
//! emits `Action::Search`, which `Cosmic::update` turns into
//! `Application::on_search()` (`src/app/cosmic.rs:850`). That subscription is
//! already composed into the framework's own (`cosmic.rs:674`), so a second
//! binding here would be a duplicate of a key the toolkit already owns — and
//! the reply to it is the `on_search` override in `main.rs`, not a message from
//! this module. T-24's row asked for a `Shell::theme()` hook that did not exist;
//! this is the same check run the other way, and it *found* one that does.
//!
//! The remaining three all reach `Shell::update` as messages that already
//! existed — [`Message::OpenNewGameForm`], [`Message::NavigateTo`] and
//! [`Message::Quit`] — so this task adds no variant to `Message` and no arm to
//! `Shell::update`, and `only_the_written_handlers_change_anything` is
//! untouched by it.
//!
//! # Exact modifiers, and why the match is lowercase
//!
//! `KeyPressed.key` is winit's `key_without_modifiers`
//! (`iced/winit/src/conversion.rs:344-352`), so it arrives **already stripped**:
//! `Ctrl+Shift+N` gives `Character("n")`, not `"N"`. Two consequences, and both
//! are why this function does not simply test `modifiers.control()`:
//!
//! * the match is on the lowercase spelling, which is therefore shift-proof;
//! * the extra-modifier guard below is *separate* work rather than something the
//!   key value already did.
//!
//! The guard rejects Shift, Alt and Logo because Qt's `Shortcut` matches with
//! `Qt::ExactMatch` semantics, so the reference's `Ctrl+N` does not fire on
//! `Ctrl+Shift+N`. **That is reasoned from Qt's documented matching rather than
//! measured here: PySide6 is not installed in this environment, so the
//! reference could not be driven to confirm it.** It is recorded as a
//! divergence risk rather than presented as a measurement — the port is stricter
//! than the framework, which is the safe direction, but a user who expects
//! `Ctrl+Shift+N` to do something will find that it does not.
//!
//! `Control` specifically, not `command()`: Qt's `"Ctrl+"` means the Control
//! key on every platform, while winit's `command()` is Logo on macOS. Mapping
//! `Ctrl+Q` onto Command-Q there would be a different accelerator than the one
//! the reference lists — and on macOS, Command-Q is the platform's own quit.
//!
//! **That last rule is unobservable in a test on Linux, and the mutation run
//! says so.** Rewriting the guard to use `command()` instead of `control()`
//! leaves every test below green, because `Modifiers::COMMAND` is `Self::CTRL`
//! off macOS (`iced/core/src/keyboard/modifiers.rs:41-46`) and the two
//! accessors read the same bit. The rule is kept because it is right on the
//! platform where it differs, and it is recorded here rather than left as an
//! implied guarantee — `logo_alone_is_not_control` below pins the
//! platform-independent half (Logo is not accepted) and cannot do more than
//! that. This is the same shape as `theme.rs`'s survivors: a covered mapping,
//! with the one branch a Linux test cannot reach named rather than assumed.
//!
//! # Held keys repeat, and that is the reference's behaviour
//!
//! There is no `repeat` filter. `QShortcut::autoRepeat` defaults to `true`, so a
//! held `Ctrl+N` opens the form repeatedly in the reference too, and
//! `keyboard::listen()` delivers repeats unchanged. Filtering them would be a
//! silent divergence invented here; if it is ever wanted it should be a decision
//! with its own note, not a line that looks like tidiness.
//!
//! # Why "it must not fire while the user is typing" needed no code
//!
//! This was the hazard `view/settings.rs` recorded as unanswerable — *"a
//! focus-visibility question the shell does not yet answer for any widget"* —
//! and the framework answers it. `keyboard::listen()`
//! (`iced/futures/src/keyboard.rs:9-19`) delivers an event **only** when its
//! status is `Ignored`, and the status comes from the widget-tree traversal
//! (`iced/winit/src/lib.rs:1488-1494` broadcasts each event with the status it
//! was given). A focused `text_input` captures its editing bindings —
//! `Ctrl+C/X/V/A`, the arrows, Home/End, Delete, Enter, Escape
//! (`iced/widget/src/text_input.rs:925-1290`) — and nothing else.
//!
//! `Ctrl+N`, `Ctrl+,` and `Ctrl+Q` are not in that set — and the sentence this
//! paragraph used to end with, that the three therefore reach this function
//! **even while a text field has focus**, was disproven live and is kept here
//! crossed out rather than silently deleted. T-19 typed into the Add-game
//! form's Name field and pressed `Ctrl+N`: the form did not reset, and the
//! same holds with the library search focused. A focused `text_input` swallows
//! these keys despite not binding them — the status that reaches
//! `keyboard::listen()` is evidently not `Ignored` for them, whatever the
//! `text_input.rs:925-1290` reading above suggests. That is a real divergence
//! from the reference: `Main.qml:122` sets
//! `context: Qt.ApplicationShortcut`, whose documented meaning is that the
//! shortcut is active whenever a window of the application is active —
//! independent of which widget holds focus. Unfocused, all four accelerators
//! fire (walked live); focused, the user must first leave the field. Whether a
//! raw-event subscription can recover the reference behaviour is unverified —
//! a change to the shell's event plumbing either way, and not to this mapping —
//! so this stands as a recorded divergence, not an attempted fix, under T-19.
//!
//! # The bound, stated rather than implied
//!
//! [`shortcut_for`] is a pure function and is tested below by driving real
//! `KeyPressed` values through it — eight mutations of it were run and seven
//! were caught (the guard dropped; the guard narrowed to `control()` alone; an
//! arm's body swapped for another's; releases accepted as presses; repeats
//! filtered out; every other `Ctrl` key sent to `Quit`; `Ctrl+F` bound here as
//! well). The one survivor is the `command()` branch, recorded above. **The
//! composition is not tested**: the
//! `Subscription` that carries it has no accessor, so nothing can assert that
//! `main.rs`'s `subscription()` actually calls this — that half is *read*, and
//! `view/settings.rs`'s `the_page_does_not_claim_a_shortcut_the_shell_does_not_implement`
//! is the text-level check that the shell listens for keys at all. Neither the
//! delivery of a real key event nor `Ctrl+F`'s focus landing is observable
//! without a window, so both are Phase 3 items under T-19, exactly as P-68's
//! own acceptance says.

use cosmic::iced::keyboard::{Event, Key};

use crate::{Message, Page};

/// The message `event` activates, or `None` when it is not one of the three
/// accelerators this module owns.
///
/// Pure, and takes the event by reference rather than owning it, so a test can
/// drive the same event through several cases. See the module doc for `Ctrl+F`
/// (not here), for the modifier guard, and for why a repeat is not filtered.
pub fn shortcut_for(event: &Event) -> Option<Message> {
    let Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };

    // `Control` and nothing else. `Modifiers` is a bitset, so this is the exact
    // test and not a substring of it: a version written as
    // `modifiers.contains(Modifiers::CTRL)` would accept `Ctrl+Shift+N`, which
    // Qt does not match against a `Ctrl+N` shortcut.
    if !modifiers.control() || modifiers.shift() || modifiers.alt() || modifiers.logo() {
        return None;
    }

    match key.as_ref() {
        // `newGameTemplate()` + push, `bridge.py:382-399`.
        Key::Character("n") => Some(Message::OpenNewGameForm),
        // `root.showPage("settings")`. The comment on `Shell::update`'s
        // `NavigateTo` arm already named "a shortcut" as one of its routes.
        Key::Character(",") => Some(Message::NavigateTo(Page::Settings)),
        // `backend.quit()`. `Message::Quit` is answered in `App::update`, which
        // is also where the window that ends the process lives.
        Key::Character("q") => Some(Message::Quit),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::keyboard::{Location, Modifiers, key};

    /// A `KeyPressed` for `character`, carrying `modifiers` and no text — which
    /// is what winit reports for a Ctrl-modified key. (An earlier revision of
    /// this comment claimed that was the reason a focused `text_input` does
    /// not swallow these; T-19 disproved it live — see the module doc. The
    /// fixture is unchanged because the mapping is unchanged; only the claim
    /// about delivery was wrong.)
    ///
    /// `key` and `modified_key` are given the same value because that is the
    /// case under test: `key` is the modifier-stripped one and it is the only
    /// one [`shortcut_for`] reads, so a test that varied `modified_key` would be
    /// asserting about a field the function does not consult.
    fn press(character: &str, modifiers: Modifiers) -> Event {
        Event::KeyPressed {
            key: Key::Character(character.into()),
            modified_key: Key::Character(character.into()),
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    /// The three keys this module owns, each with the message the reference's
    /// handler sends — and each distinct from the others.
    ///
    /// Distinctness is the point rather than decoration: a match whose arms had
    /// been copy-pasted would satisfy three separate `is_some()` assertions
    /// while sending every key to the same place, which is the #43 shape.
    #[test]
    fn each_own_accelerator_produces_the_reference_s_own_message() {
        assert!(
            matches!(
                shortcut_for(&press("n", Modifiers::CTRL)),
                Some(Message::OpenNewGameForm)
            ),
            "Ctrl+N is `root.openGameForm(\"\")` (Main.qml:121-125)"
        );
        assert!(
            matches!(
                shortcut_for(&press(",", Modifiers::CTRL)),
                Some(Message::NavigateTo(Page::Settings))
            ),
            "Ctrl+, is `root.showPage(\"settings\")` (Main.qml:135-139)"
        );
        assert!(
            matches!(
                shortcut_for(&press("q", Modifiers::CTRL)),
                Some(Message::Quit)
            ),
            "Ctrl+Q is `backend.quit()` (Main.qml:140-143)"
        );
    }

    /// **A bare keystroke activates nothing.**
    ///
    /// This is the assertion the "while typing" hazard needs: these four
    /// characters are exactly what a user types into the Library search box or
    /// the game form, and if the guard were dropped — a `match` on the key with
    /// no modifier test at all, which is the natural first draft — every `n`
    /// typed into the search box would open the Add-game form.
    #[test]
    fn a_bare_key_activates_nothing() {
        for character in ["n", ",", "q", "f"] {
            assert!(
                shortcut_for(&press(character, Modifiers::NONE)).is_none(),
                "{character:?} with no modifier must do nothing, or typing \
                 would trigger it"
            );
        }
    }

    /// An extra modifier stops the match, so `Ctrl+Shift+N` is not `Ctrl+N` —
    /// Qt's `Qt::ExactMatch`, which is the semantics `Shortcut` uses.
    ///
    /// Enumerated per modifier rather than tested with one, because the guard is
    /// four separate calls and a version that omitted, say, only `shift()` would
    /// pass a test that tried shift alone... which is the reason this loops.
    #[test]
    fn an_extra_modifier_stops_the_match() {
        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::LOGO] {
            for character in ["n", ",", "q"] {
                assert!(
                    shortcut_for(&press(character, Modifiers::CTRL | extra)).is_none(),
                    "Ctrl+{extra:?}+{character:?} is not the reference's \
                     accelerator; Qt matches shortcuts exactly"
                );
            }
        }
    }

    /// Only `Control` — not `command()`.
    ///
    /// On Linux these are the same bit, so this test cannot distinguish them
    /// here and does not pretend to: it pins that the Logo modifier alone is not
    /// enough, which is the platform-independent half of the rule. The macOS
    /// half is a reading, recorded in the module doc.
    #[test]
    fn logo_alone_is_not_control() {
        for character in ["n", ",", "q"] {
            assert!(
                shortcut_for(&press(character, Modifiers::LOGO)).is_none(),
                "Logo+{character:?} is not Ctrl+{character:?} (the guard rejects \
                 Logo, and on macOS `command()` is Logo rather than Control)"
            );
        }
    }

    /// Keys this module must not handle, and the list is two kinds of key.
    ///
    /// `Ctrl+S` and `Ctrl+P` are conventional enough that a reader might expect
    /// them — they are not in `Main.qml`, so adding one is a deliberate edit
    /// rather than a match arm that happened to already work. And the uppercase
    /// spellings are unreachable by construction (`key` is modifier-stripped),
    /// which is worth pinning: a future "fix" that matched `"N"` as well would
    /// be defending against a case that cannot arrive.
    ///
    /// **`"f"` is the important one.** `Ctrl+F` is the fourth accelerator and it
    /// is *not handled here*: libcosmic's own `keyboard_nav` subscription binds
    /// it and calls `Application::on_search` (see the module doc). A `Character("f")`
    /// arm added here would be a second binding for the same key, and the user
    /// would get the navigation twice — once from the framework and once from
    /// this function. This assertion is what makes that mistake fail rather than
    /// ship.
    #[test]
    fn the_keys_this_module_does_not_own_do_nothing() {
        for character in ["s", "p", "w", "1", "N", "F", "Q", ""] {
            assert!(
                shortcut_for(&press(character, Modifiers::CTRL)).is_none(),
                "Ctrl+{character:?} is not one of the reference's accelerators"
            );
        }
        assert!(
            shortcut_for(&press("f", Modifiers::CTRL)).is_none(),
            "Ctrl+F belongs to libcosmic's `keyboard_nav` subscription, which \
             routes it to `Application::on_search`; binding it here too would \
             navigate twice for one key press"
        );
    }

    /// Only a key **press** activates: the release and the modifier-changed
    /// events carry the same key and must not re-fire it.
    ///
    /// A release reaching the match would double every shortcut — once on press
    /// and once on release — and `listen()` delivers both.
    #[test]
    fn only_a_key_press_activates() {
        assert!(
            shortcut_for(&Event::KeyReleased {
                key: Key::Character("n".into()),
                modified_key: Key::Character("n".into()),
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                location: Location::Standard,
                modifiers: Modifiers::CTRL,
            })
            .is_none(),
            "a release is not a press"
        );
        assert!(
            shortcut_for(&Event::ModifiersChanged(Modifiers::CTRL)).is_none(),
            "pressing Control alone is not an accelerator"
        );
    }

    /// A held key repeats and keeps activating, which is `QShortcut`'s default
    /// `autoRepeat` — so the absence of a `repeat` filter is pinned rather than
    /// merely absent.
    #[test]
    fn a_repeated_press_still_activates() {
        let Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            ..
        } = press("n", Modifiers::CTRL)
        else {
            unreachable!("`press` builds a KeyPressed")
        };

        let repeated = Event::KeyPressed {
            key,
            modified_key,
            physical_key,
            location,
            modifiers,
            text,
            repeat: true,
        };

        assert!(
            matches!(shortcut_for(&repeated), Some(Message::OpenNewGameForm)),
            "the reference's shortcuts auto-repeat (QShortcut::autoRepeat \
             defaults to true), so a repeat is not filtered"
        );
    }
}
