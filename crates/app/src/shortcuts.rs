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
//! **Three of them are mapped here and the fourth is bound twice on purpose.**
//! `Ctrl+F` is libcosmic's own: `keyboard_nav::subscription()`
//! (`src/keyboard_nav.rs:50-55`) matches `Character("f")` with Control and
//! emits `Action::Search`, which `Cosmic::update` turns into
//! `Application::on_search()` (`src/app/cosmic.rs:850`). That subscription is
//! already composed into the framework's own (`cosmic.rs:674`), so
//! [`shortcut_for`] does not map `Ctrl+F` and must not. But the framework's
//! binding is gated on the event being `Ignored` (`src/keyboard_nav.rs:20-23`),
//! so it is silent in exactly the state `BUG-12` is about, and
//! [`search_the_framework_cannot_reach`] answers the key there and only there.
//! That arm is the one variant this module adds to `Message`
//! ([`Message::FocusLibrarySearch`]); the other three all reach `Shell::update`
//! as messages that already existed.
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
//! # Why the four fire while the user is typing, and why that is the reference
//!
//! `view/settings.rs` once recorded this as unanswerable — *"a focus-visibility
//! question the shell does not yet answer for any widget"* — and the first
//! answer the shell got was the wrong one. `keyboard::listen()`
//! (`iced/futures/src/keyboard.rs:9-19`) delivers an event **only** when its
//! status is `Ignored`, and this file used to argue that a focused
//! `text_input` captures only its editing bindings — `Ctrl+C/X/V/A`, the
//! arrows, Home/End, Delete, Enter, Escape (`iced/widget/src/text_input.rs:925-1290`)
//! — so that `Ctrl+N`, `Ctrl+,` and `Ctrl+Q` "pass through". T-19 disproved
//! that live, and the measurement is now in [`accelerator`]'s doc: a focused
//! `text_input` captures **every** key, including the ones it does nothing
//! with, because libcosmic's input ends its focused branch with
//! `shell.capture_event()` unconditionally (`src/widget/text_input/input.rs:2261-2262`).
//! The editing-bindings list above is the list of keys it *acts* on, which is a
//! different list, and reading one as the other is what made the divergence
//! look like a design.
//!
//! So the four accelerators are answered from a raw subscription, in every
//! status. That is the reference's behaviour rather than a liberty:
//! `Main.qml:122` sets `context: Qt.ApplicationShortcut`, whose documented
//! meaning is that the shortcut is active whenever a window of the application
//! is active, independent of which widget holds focus. Qt's own typed text is
//! not destroyed by this and neither is the port's: an accelerator always
//! carries Control, and insertion is guarded on Control
//! (`src/widget/text_input/input.rs:2095`).
//!
//! # The bound, stated rather than implied
//!
//! [`shortcut_for`] is a pure function and is tested below by driving real
//! `KeyPressed` values through it — eight mutations of it were run and seven
//! were caught (the guard dropped; the guard narrowed to `control()` alone; an
//! arm's body swapped for another's; releases accepted as presses; repeats
//! filtered out; every other `Ctrl` key sent to `Quit`; `Ctrl+F` bound here as
//! well). The one survivor is the `command()` branch, recorded above.
//!
//! The composition is tested too, through `messages_for`, which drives the
//! real runtime event through the subscription `App::subscription` returns and
//! reads the messages back — three tests, one of which measures the status a
//! focused `text_input` leaves instead of assuming it. What is still not
//! observable from here is a *window*: that winit delivers the key to the
//! runtime at all, and that `focus`'s operation lands on the search box, are
//! Phase 3 items under T-19, exactly as P-68's own acceptance says.

use cosmic::iced::Subscription;
use cosmic::iced::event::Status;
use cosmic::iced::keyboard::{Event, Key};
use cosmic::iced::window;

use crate::{Message, Page};

/// The runtime's own [`Event`](cosmic::iced::Event), which is what a
/// subscription receives, aliased because this module already spends the name
/// `Event` on the keyboard event [`shortcut_for`] reads.
use cosmic::iced::Event as RuntimeEvent;

/// The shell's keyboard subscription: **the reference's four accelerators,
/// from anywhere in the window.**
///
/// This is what `App::subscription` returns (`main.rs`), and it is a function
/// here rather than a body there because the composition *is* observable:
/// `into_recipes` and `Recipe::stream` are public
/// (`iced/src/advanced.rs`), so `messages_for` — the test-only driver at the
/// foot of this file — can drive a real runtime event
/// through exactly the subscription the shell runs. The module doc's "the
/// composition is not tested" paragraph is what used to say the opposite.
///
/// `listen_raw`, not `listen`: see [`accelerator`], which is the whole of the
/// fix for `BUG-12`.
pub fn subscription() -> Subscription<Message> {
    cosmic::iced::event::listen_raw(accelerator)
}

/// The handler [`subscription`] hands to `listen_raw` — and the whole of the
/// fix for `BUG-12`.
///
/// # Why this reads the status instead of filtering on it
///
/// `keyboard::listen()`, which this replaced, delivers an event **only** when
/// its status is `Ignored` (`iced/futures/src/keyboard.rs:9-19`), and a focused
/// `text_input` captures *every* key it is sent — not just the editing bindings
/// it acts on. libcosmic's input ends its focused branch with
/// `shell.capture_event()` unconditionally (`src/widget/text_input/input.rs:2261-2262`,
/// the tail of the `KeyPressed` arm), so the status that reached the old
/// subscription was `Captured` for all four accelerators.
///
/// Measured, not read, with the field's focus proved by a letter typed into the
/// same field in the same walk: a focused `text_input` leaves `Captured` on
/// `Ctrl+N`, `Ctrl+,`, `Ctrl+Q` and `Ctrl+F`, and publishes nothing for any of
/// them. None of the four therefore ever reached the shell. `Main.qml:122` sets
/// `context: Qt.ApplicationShortcut`, whose documented meaning is that the
/// shortcut is active whenever a window of the application is active, whatever
/// holds focus — so passing these through **is** the reference's behaviour and
/// swallowing them was the port's divergence, not a safety property.
///
/// # Why nothing recorded is lost by passing them through
///
/// The capture is unconditional; the *action* is not, and the same event that
/// is captured carries the reason. Character insertion is skipped whenever the
/// tracked modifiers are `command()` or the event's `control()` is set
/// (`src/widget/text_input/input.rs:2095`, the `if !state.keyboard_modifiers.command()
/// && !modifiers.control()` guard), so `Ctrl+N` over a field types no `n`: the
/// capture is the field acknowledging the key, not a claim on it. Nothing the
/// user typed is displaced by an accelerator, because an accelerator always
/// arrives with Control held.
///
/// # The bound
///
/// A `KeyPressed` is the only event this looks at, so the repaints and pointer
/// moves `listen_raw` also forwards — it does not drop `RedrawRequested`, the
/// way `listen_with` does (`iced/futures/src/event.rs:12-21`) — produce no
/// message and cannot loop. [`shortcut_for`]'s own guards (a modifier of
/// exactly Control, a press rather than a release, no repeats filtered) are
/// unchanged and are what keep a keystroke *typed into* a field from being
/// read as an accelerator; the test below drives each of them through this
/// handler rather than only through the pure function.
fn accelerator(event: RuntimeEvent, status: Status, _window: window::Id) -> Option<Message> {
    let RuntimeEvent::Keyboard(keyboard) = event else {
        return None;
    };
    shortcut_for(&keyboard).or_else(|| search_the_framework_cannot_reach(&keyboard, status))
}

/// `Ctrl+F`, for the one state the framework's own binding cannot reach.
///
/// `Ctrl+F` is libcosmic's, and that is not an omission: `keyboard_nav::subscription()`
/// matches it and `Cosmic::update` routes the `Action::Search` it emits to
/// `Application::on_search()` (`src/app/cosmic.rs:850`), which is `main.rs`'s
/// reply. But that subscription is gated exactly the way `keyboard::listen()`
/// was — `listen_raw(|event, status, _| if event::Status::Ignored != status {
/// return None })` (`src/keyboard_nav.rs:20-23`) — so the fourth accelerator
/// was dead in the same state the first three were, and the gate is in the
/// framework rather than in this crate.
///
/// So this is the same key in the one status where the framework stays silent.
/// The two are mutually exclusive by construction — the framework's arm runs
/// only on `Ignored` — which is what makes this a binding *beside* the
/// framework's rather than a second binding that would navigate twice for one
/// press.
///
/// The predicate is the framework's own (`src/keyboard_nav.rs:63-66`), Shift
/// and all, rather than [`shortcut_for`]'s exact-modifier guard: `Ctrl+Shift+F`
/// opens the search today, and making the same key behave differently depending
/// on where the focus is would be a stranger divergence than inheriting
/// libcosmic's. It is recorded as a divergence from `Qt::ExactMatch` in the
/// module doc, where it was already recorded before this arm existed.
fn search_the_framework_cannot_reach(event: &Event, status: Status) -> Option<Message> {
    if status == Status::Ignored {
        return None;
    }
    let Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if !matches!(key.as_ref(), Key::Character("f")) || !modifiers.control() {
        return None;
    }
    Some(Message::FocusLibrarySearch)
}

/// Drive `subscription` with one runtime event, as the runtime would, and
/// return every message it produces.
///
/// A [`Subscription`] is a list of recipes and a recipe is a function from the
/// runtime's event stream to a stream of messages
/// (`iced/src/advanced.rs`, `iced/futures/src/subscription.rs:436-448`), so the
/// whole composition is reachable from a test: hand it a one-event stream and
/// collect what comes out. This is the instrument the module doc used to say
/// did not exist — and the reason it exists is that the alternative, grepping
/// `main.rs` for `"keyboard"`, is a check that passes without inspecting what it
/// claims.
///
/// `#[cfg(test)]` and `pub(crate)`: `view/settings.rs`'s page guard drives the
/// same subscription, so the rows the page advertises are checked against the
/// shell's real behaviour rather than against a word in a file.
///
/// Generic over the message type, because there are two subscriptions in a
/// running window and the page's guard has to drive both: this one, and
/// libcosmic's `keyboard_nav::subscription()`, which produces `Action`s.
#[cfg(test)]
pub(crate) fn messages_for<T: 'static>(
    subscription: Subscription<T>,
    event: RuntimeEvent,
    status: Status,
) -> Vec<T> {
    use cosmic::iced::advanced::subscription::{
        Event as SubscriptionEvent, EventStream, into_recipes,
    };
    use cosmic::iced::futures::StreamExt as _;
    use cosmic::iced::futures::executor::block_on;

    let mut messages = Vec::new();
    for recipe in into_recipes(subscription) {
        // The window is `Id::NONE`: every handler here ignores it, and the
        // runtime's own is unavailable off a display.
        let runtime_event = SubscriptionEvent::Interaction {
            window: window::Id::NONE,
            event: event.clone(),
            status,
        };
        let input: EventStream = Box::pin(cosmic::iced::futures::stream::iter([runtime_event]));
        messages.extend(block_on(recipe.stream(input).collect::<Vec<_>>()));
    }
    messages
}

/// A key press as the runtime delivers it, for the tests that drive a
/// subscription rather than the pure mapping.
///
/// `text` and `repeat` are parameters because they are the two fields the
/// difference between a real keyboard and a hand-built event lives in: a real
/// press carries the character it would type, which is what a focused
/// `text_input` reads before deciding whether to insert, and repeats arrive with
/// the flag set. [`shortcut_for`] consults neither, and a fixture that could not
/// set them could not tell that apart.
#[cfg(test)]
pub(crate) fn pressed_event(
    character: &str,
    modifiers: cosmic::iced::keyboard::Modifiers,
    text: Option<&str>,
    repeat: bool,
) -> RuntimeEvent {
    use cosmic::iced::keyboard::{Location, key};

    RuntimeEvent::Keyboard(Event::KeyPressed {
        key: Key::Character(character.into()),
        modified_key: Key::Character(character.into()),
        physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
        location: Location::Standard,
        modifiers,
        text: text.map(Into::into),
        repeat,
    })
}

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
        let RuntimeEvent::Keyboard(event) = pressed_event(character, modifiers, None, false) else {
            unreachable!("`pressed_event` builds a keyboard event")
        };
        event
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

    /// The runtime event the shell's subscription receives when `character` is
    /// pressed with `modifiers`: [`press`]'s keyboard event, wrapped the way the
    /// runtime wraps it.
    fn key(character: &str, modifiers: Modifiers) -> RuntimeEvent {
        RuntimeEvent::Keyboard(press(character, modifiers))
    }

    /// [`key`] as a real keyboard sends it: `text` carries the character that
    /// would be typed, which is what a focused `text_input` reads to decide
    /// whether to insert. [`press`] leaves it `None`, which no key press from
    /// winit does.
    fn typed(character: &str, modifiers: Modifiers) -> RuntimeEvent {
        pressed_event(character, modifiers, Some(character), false)
    }

    /// The Library's search box, built the way `view/library.rs:668` builds it
    /// — the field `Ctrl+F` focuses, and the one T-19 had focused when it
    /// recorded this defect.
    fn search_box() -> cosmic::Element<'static, Message> {
        use crate::view::a11y;
        use crate::view::library::{SEARCH_INPUT_ID, SEARCH_PLACEHOLDER};

        a11y::input_with_id(
            cosmic::widget::text_input(SEARCH_PLACEHOLDER, "").on_input(Message::SetSearchText),
            SEARCH_PLACEHOLDER,
            String::new(),
            SEARCH_INPUT_ID.into(),
        )
        .into()
    }

    /// **The status the widget tree leaves on `event`, with the search box
    /// focused** — measured, and the messages the field itself published.
    ///
    /// The focus is taken through [`harness::tab_to`], the same operation
    /// libcosmic's Tab subscription runs, and the returned status is the one the
    /// runtime would publish for the event: `Shell::is_event_captured` is
    /// `event_status == Status::Captured` and `Status` has two variants, so this
    /// is that status rather than an approximation of it
    /// (`iced/core/src/shell.rs`, `iced/core/src/event.rs`).
    ///
    /// The caller is responsible for proving the focus took — a tree where Tab
    /// reached nothing would report `Ignored` and quietly make every assertion
    /// built on this vacuous. The regression test below does that by typing into
    /// the same box in the same walk.
    fn the_status_a_focused_search_box_leaves(event: &RuntimeEvent) -> (Status, Vec<Message>) {
        use crate::view::a11y::harness;

        let mut element = search_box();
        let (mut tree, node) = harness::built(&mut element);
        harness::tab_to(&mut element, &mut tree, &node);

        let mut messages = Vec::new();
        let out = harness::dispatch(&mut element, &mut tree, &node, event, &mut messages);
        let status = if out.captured {
            Status::Captured
        } else {
            Status::Ignored
        };
        (status, messages)
    }

    /// **The regression test for `BUG-12`: the accelerators arrive while a text
    /// field has the focus.**
    ///
    /// It measures the property end to end rather than asserting the code path:
    ///
    /// 1. it builds the Library's real search box, gives it the framework's
    ///    focus through the same operation Tab runs, and presses the key;
    /// 2. it reads the status the widget tree left behind — which is `Captured`,
    ///    the status `keyboard::listen()` filters out — and hands *that measured
    ///    status* to the subscription `App::subscription` returns;
    /// 3. it requires the reference's own message out of the other end.
    ///
    /// Step 2 is measured rather than assumed, because a test that wrote
    /// `Status::Captured` out of the air would pass against a shell whose fields
    /// leave `Ignored`: the fixture would be supplying a state no widget
    /// produces, which is this repository's own defect class. The control is the
    /// first assertion — a letter typed into the same box in the same walk must
    /// reach its `on_input` — so a tree that never took the focus cannot make
    /// the rest pass.
    #[test]
    fn an_accelerator_reaches_the_shell_while_a_text_field_has_focus() {
        let (status, typed_in) =
            the_status_a_focused_search_box_leaves(&typed("x", Modifiers::NONE));
        assert_eq!(
            status,
            Status::Captured,
            "the search box must take the focus Tab gives it, and a focused \
             `text_input` captures what is typed into it; it left {status:?}"
        );
        assert!(
            matches!(typed_in.as_slice(), [Message::SetSearchText(text)] if text == "x"),
            "typing `x` into the focused search box published {typed_in:?}, so \
             the field is not really focused and everything below would be \
             measuring an unfocused page"
        );

        // Each row is the reference's, from `Main.qml:121-143`, and the four
        // matchers are distinct so that a match whose arms were copy-pasted
        // cannot satisfy this.
        struct Case {
            character: &'static str,
            reference: &'static str,
            is_it: fn(&Message) -> bool,
        }

        let cases = [
            Case {
                character: "n",
                reference: "`root.openGameForm(\"\")` (Main.qml:121-125)",
                is_it: |message| matches!(message, Message::OpenNewGameForm),
            },
            Case {
                character: ",",
                reference: "`root.showPage(\"settings\")` (Main.qml:135-139)",
                is_it: |message| matches!(message, Message::NavigateTo(Page::Settings)),
            },
            Case {
                character: "q",
                reference: "`backend.quit()` (Main.qml:140-143)",
                is_it: |message| matches!(message, Message::Quit),
            },
            Case {
                character: "f",
                reference: "`root.showPage(\"library\"); focusSearch()` (Main.qml:126-134)",
                is_it: |message| matches!(message, Message::FocusLibrarySearch),
            },
        ];

        for Case {
            character,
            reference: what,
            is_it,
        } in cases
        {
            let (status, published) =
                the_status_a_focused_search_box_leaves(&key(character, Modifiers::CTRL));
            assert_eq!(
                status,
                Status::Captured,
                "a focused search box captures `Ctrl+{character}`, which is why \
                 `keyboard::listen()` never delivered it; got {status:?}"
            );
            assert!(
                published.is_empty(),
                "the field must do nothing with `Ctrl+{character}` — the capture \
                 is not a claim on the key — but it published {published:?}"
            );

            let produced = messages_for(subscription(), key(character, Modifiers::CTRL), status);
            assert!(
                matches!(produced.as_slice(), [only] if is_it(only)),
                "`Ctrl+{character}` — {what} — produced {produced:?} when the key \
                 arrived with the status a focused text field leaves. It must \
                 produce exactly the reference's message from anywhere in the \
                 window: `Main.qml:122` is a `Qt.ApplicationShortcut`, which is \
                 active whenever the window is, whatever holds focus"
            );
        }
    }

    /// **`Ctrl+F` is answered exactly once, whichever state it arrives in.**
    ///
    /// The framework answers it when the event is `Ignored`
    /// (`keyboard_nav::subscription`, `src/keyboard_nav.rs:20-23`); this module
    /// answers it when it is not. One press must produce one navigation, so the
    /// two states are asserted separately — and the `Ignored` half is the one
    /// that breaks if the new arm's gate is dropped, which is the shape the
    /// "bind `Ctrl+F` here too" fix would have taken.
    #[test]
    fn the_search_key_is_answered_once_in_each_state() {
        let ignored = messages_for(subscription(), key("f", Modifiers::CTRL), Status::Ignored);
        assert!(
            ignored.is_empty(),
            "`Ctrl+F` on an ignored event belongs to libcosmic's `keyboard_nav` \
             subscription, which routes it to `Application::on_search`; binding \
             it here as well would navigate twice for one key press, and this \
             module's whole reason for reading the status is that the two cases \
             are exclusive. Got {ignored:?}"
        );

        let captured = messages_for(subscription(), key("f", Modifiers::CTRL), Status::Captured);
        assert!(
            matches!(captured.as_slice(), [Message::FocusLibrarySearch]),
            "`Ctrl+F` on a captured event is the state the framework is silent \
             in — a focused text field — and it must reach the same navigation \
             from here; got {captured:?}"
        );
    }

    /// **The status was widened; the mapping was not.**
    ///
    /// Every keystroke typed into a field now reaches this handler, so the
    /// guards inside [`shortcut_for`] carry weight they did not carry when the
    /// field swallowed the event before it got here: a bare `n` is what a user
    /// types into a game's name, and `Ctrl+Shift+N` is not `Ctrl+N`.
    ///
    /// `Ctrl+Shift+F` is excluded from the extra-modifier loop and pinned
    /// separately: it is the recorded divergence from `Qt::ExactMatch`, inherited
    /// from libcosmic on purpose so that the key behaves the same whether or not
    /// a field has focus (see [`search_the_framework_cannot_reach`]).
    #[test]
    fn a_captured_keystroke_is_only_an_accelerator_with_exactly_control() {
        for character in ["n", ",", "q", "f"] {
            let bare = messages_for(
                subscription(),
                key(character, Modifiers::NONE),
                Status::Captured,
            );
            assert!(
                bare.is_empty(),
                "{character:?} with no modifier produced {bare:?} from a \
                 captured event. This is the case that matters most now: it is \
                 exactly the keystroke a user types into the search box or the \
                 game form's name field"
            );
        }

        for extra in [Modifiers::SHIFT, Modifiers::ALT, Modifiers::LOGO] {
            for character in ["n", ",", "q"] {
                let produced = messages_for(
                    subscription(),
                    key(character, Modifiers::CTRL | extra),
                    Status::Captured,
                );
                assert!(
                    produced.is_empty(),
                    "Ctrl+{extra:?}+{character:?} produced {produced:?}; Qt \
                     matches shortcuts with `Qt::ExactMatch`, so an extra \
                     modifier is not the accelerator"
                );
            }
        }

        let shifted_search = messages_for(
            subscription(),
            key("f", Modifiers::CTRL | Modifiers::SHIFT),
            Status::Captured,
        );
        assert!(
            matches!(shifted_search.as_slice(), [Message::FocusLibrarySearch]),
            "`Ctrl+Shift+F` reaches `on_search` today because libcosmic's match \
             tests Control and does not reject Shift; this arm mirrors that \
             predicate rather than [`shortcut_for`]'s exact-modifier guard so \
             that the key does not change meaning with the focus. If it is ever \
             tightened, tighten both and say so here. Got {shifted_search:?}"
        );

        // A release carries the same key as the press that already fired.
        let released = RuntimeEvent::Keyboard(Event::KeyReleased {
            key: Key::Character("n".into()),
            modified_key: Key::Character("n".into()),
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: Location::Standard,
            modifiers: Modifiers::CTRL,
        });
        let produced = messages_for(subscription(), released, Status::Captured);
        assert!(
            produced.is_empty(),
            "a release is not a press, and it now arrives with `Captured` too; \
             got {produced:?}"
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
