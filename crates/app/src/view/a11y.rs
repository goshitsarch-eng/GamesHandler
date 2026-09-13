//! Keyboard focus and accessibility nodes for three toolkit widgets that ship
//! with neither.
//!
//! # What this module is for
//!
//! `docs/audit/COSMIC-UX.md` records three findings — UX-01, UX-02, UX-03 — and
//! all three are the same defect at three call sites: the widget the app draws
//! is not in the Tab ring and publishes nothing to assistive technology. The
//! toolkit is read at the pinned rev
//! `a401af8b1c54a8abd393b8c5b7c8809402f83850`
//! (`/home/gosh/.cargo/git/checkouts/libcosmic-41009aea1d72760b/a401af8`, the
//! rev `Cargo.lock` pins), and the three defects are:
//!
//! | Widget the app builds | `operate` | `a11y_nodes` |
//! |---|---|---|
//! | `cosmic::widget::Toggler` (`src/widget/toggler.rs:22`) | absent | absent |
//! | `cosmic::widget::TextInput` (`src/widget/text_input/input.rs:843-855`) | present, `focusable` + `text_input` | absent |
//! | `cosmic::widget::Dropdown` (`src/widget/dropdown/widget.rs:330`) | a commented-out body | commented out (`:375-384`) |
//!
//! The audit's UX-02 row cites iced's toggler
//! (`iced/widget/src/toggler.rs`, no `fn operate`, `a11y_nodes` at `:562-634`)
//! and reads libcosmic's `src/widget/toggler.rs:15-17` as a re-export of it.
//! **That is not what that line is.** It is
//! `pub use iced_widget::toggler::{Catalog, Style};`
//! (`src/widget/toggler.rs:15`) — the *style* types — and libcosmic does not use
//! iced's toggler at all: `src/widget/toggler.rs`
//! defines its own `Toggler` (`:22-32`) and `Toggler::new` (`:51`), and the whole
//! file contains zero occurrences of `operate`, of `a11y`, and of `keyboard`. So the
//! widget the app draws is *worse* off than the audit row says: it is not merely
//! unreachable, it publishes no node at all — not the `Role::Switch` the row
//! describes.
//!
//! # What is fixed here, and what is not
//!
//! [`Accessible`] is a forwarding wrapper: it holds the toolkit's own widget as
//! an [`Element`], forwards every `Widget` method to it, and adds the two hooks
//! the toolkit left out. The toolkit's own `update`/`draw`/`layout` stay the
//! ones that run, so nothing about how these controls look or behave under the
//! pointer changes.
//!
//! * **UX-02 (togglers)** is closed completely. [`toggler`] emits a
//!   `Role::Switch` node carrying the name the page labels the control with and
//!   the current on/off state, and it toggles on Enter or Space — publishing, on
//!   both, the *same message value* the pointer publishes, because it is handed
//!   the toolkit's own `on_toggle(!is_toggled)` result rather than a second copy
//!   of the logic.
//!
//!   The name is **not** always the string the toolkit paints, and that is a
//!   deliberate divergence rather than a slip. The toolkit's `Toggler` has one
//!   text slot and the pages here need two strings: `view/settings.rs:634-646`
//!   draws `CLOSE_ON_LAUNCH_LABEL` as the row's form label and passes
//!   `CLOSE_ON_LAUNCH_EXPLANATION` as the switch's own text, and the node is
//!   named after the **form label** — the string the user reads the control
//!   *as*, and the one `row` puts beside it. `view/form.rs:681-689` has the
//!   opposite arrangement: its switch has no form label of its own, so the node
//!   is named `"{row.name}: {row.subtitle}"` while the toolkit paints
//!   `row.subtitle` alone, the row's name being the only thing that keeps two
//!   rows' subtitles from colliding in this module's id table. Each site says
//!   which string it sends and why.
//! * **UX-03 (text inputs)** is closed for the inputs it is applied to: the
//!   node is a `Role::TextInput` carrying the field's label and the current
//!   value, so the field is named and its contents readable. The input was
//!   already focusable and already handles typing
//!   (`src/widget/text_input/input.rs:843-855` reports both `focusable` and
//!   `text_input`), so nothing else was missing. The name is the visible label
//!   at every site — the form's row label, the category field's placeholder, the
//!   search boxes' placeholders — except at the Library's search box, where the
//!   name is the placeholder and the *id* is the page's own constant; see
//!   [`input_with_id`].
//! * **UX-01 (dropdowns)** is closed for everything this repository can close.
//!   Every `cosmic::widget::Dropdown` the app builds is now wrapped — the eleven
//!   sites are listed on [`dropdown`] — so each is in the Tab ring and publishes
//!   a `Role::ComboBox` node carrying its name and its selected text, and Up and
//!   Down move the selection by publishing the dropdown's own `on_selected`
//!   message. The control *is* operable from the keyboard, and the message the
//!   keyboard path sends is the one the pointer path would send, because both
//!   come from the caller's own selection function rather than from a second
//!   copy of the rule.
//!
//!   What is **not** closed is the toolkit's popup, and it cannot be closed from
//!   here. Three facts, each measured against the pinned rev:
//!
//!   1. Opening the popup is gated on private fields of `dropdown::State`
//!      (`is_open`, `close_operation`, `open_operation` —
//!      `src/widget/dropdown/widget.rs:407-409`, in a struct declared at `:403`
//!      with no accessor), so no code outside that module can set them.
//!   2. No *operation* reaches that state either. `Dropdown::operate`'s body is
//!      commented out (`src/widget/dropdown/widget.rs:336-338`) and so are the
//!      two operations that would have opened and closed it
//!      (`src/widget/dropdown/mod.rs:59-67`), so an operation aimed at a
//!      dropdown moves nothing — the state it would have to reach is not
//!      consulted on any path this crate can take.
//!   3. Even a forced-open menu would be inert: the overlay has no keyboard arm
//!      at all — `grep -c keyboard src/widget/dropdown/menu/mod.rs` is **0** —
//!      so it could be neither navigated nor dismissed from the keyboard. Nor
//!      could the app dismiss it: `on_escape` is the `cosmic::Application`
//!      trait's own default (libcosmic `src/app/mod.rs:436-438`, `Task::none()`)
//!      and `crates/app` never overrides it, which is what libcosmic's Escape
//!      dispatch would call (`src/app/cosmic.rs:849`).
//!
//!   So the Up/Down affordance stands in for the popup rather than reproducing
//!   it, and Enter deliberately does nothing on a dropdown: opening a menu that
//!   cannot then be navigated or dismissed from the keyboard would be worse than
//!   not opening it. This is recorded rather than hidden; see [`dropdown`].
//!
//! # Why the accessible name is also the widget's id
//!
//! [`stable_id`] hands out one `Id` per name, allocated on first use. A
//! per-construction `Id::unique()` — what every widget in the toolkit does
//! (`src/widget/button/widget.rs:63`, `iced/widget/src/button.rs:130`,
//! `iced/widget/src/toggler.rs:137`) — would defeat both halves of the fix:
//!
//! * the node id is the widget id's *number*: `From<Id> for A11yId` keeps the
//!   `Internal::Unique` payload and `From<A11yId> for accesskit::NodeId` hands
//!   it straight to accesskit (`iced/accessibility/src/id.rs:29-34`, `:58-67`),
//!   so a new id each frame means a new node each frame and assistive technology
//!   cannot follow anything;
//! * an incoming action carries that number back as a plain `Unique` id —
//!   `conversion::a11y` builds `Id::from(u128::from(event.target_node.0) as u64)`
//!   (`iced/winit/src/conversion.rs:1670-1672`) — which can only match a widget
//!   whose id is that same number.
//!
//! **A name must therefore be unique within a window.** Two wrappers built with
//! the same name get the same id, and the second one's node overwrites the
//! first's in the accesskit tree with the same `NodeId`; the builders below take
//! the row key into the name for exactly this reason (a runner name, a field
//! name). A control whose id is fixed somewhere else does not take a
//! name-derived id at all: [`input_with_id`] is the one constructor that takes
//! its `Id` from the caller, for the Library's search box, which `Ctrl+F`
//! focuses by name.
//!
//! # The toolkit features this assumes
//!
//! `Widget::a11y_nodes` and `Event::A11y` do not exist in iced unless its `a11y`
//! feature is on (`iced/core/src/widget.rs:149-158`, `iced/core/src/event.rs:36-41`).
//! A crate cannot `cfg` on a *dependency's* feature, so this file names them
//! unconditionally; it compiles because `crates/app/Cargo.toml` turns the
//! feature on. `Cargo.toml` also names `iced_accessibility` directly, at the
//! same git rev as `libcosmic`, because `A11yTree` is not re-exported anywhere
//! in libcosmic's public API — `grep -rn 'pub use iced_accessibility' .` run at
//! the pinned checkout's root returns exactly one hit,
//! `iced/core/src/lib.rs:68`, and it re-exports the `id` module alone. (The same
//! grep restricted to `src/`, which is where libcosmic's own source lives,
//! returns **zero** — the one hit is in the *submodule*, not in `src/`. The path
//! in the command is what decides whether it finds anything, so it is written
//! out.) Without that dependency the `a11y_nodes` override cannot be written at
//! all, because its return type is unnameable.

use cosmic::Element;
use cosmic::iced::advanced::widget::operation::Focusable;
use cosmic::iced::advanced::widget::{Id, Operation, Tree, tree};
use cosmic::iced::advanced::{Clipboard, Layout, Shell, layout, mouse, overlay, renderer};
use cosmic::iced::core::id::IdEq;
use cosmic::iced::keyboard;
use cosmic::iced::{Event, Length, Rectangle, Size, Vector};
use iced_accessibility::accesskit::{Action, Node, Rect, Role};
use iced_accessibility::{A11yNode, A11yTree};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

/// One [`Id`] per name, allocated the first time the name is seen.
///
/// The map is process-global and never evicted. It is bounded by the number of
/// distinct accessible names the app builds — one per toggle row, one per
/// dropdown, one per labelled field — and each entry is a single `u64` behind a
/// `String` key. Two wrappers that ask for the same name share an id; see the
/// module docs.
fn stable_id(name: &str) -> Id {
    static IDS: LazyLock<Mutex<HashMap<String, Id>>> = LazyLock::new(Default::default);

    IDS.lock()
        .expect("the id table is only ever held for the length of a lookup")
        .entry(name.to_string())
        .or_insert_with(Id::unique)
        .clone()
}

/// Whether the keyboard is on this widget, as the framework last left it.
///
/// This lives in the widget [`Tree`]'s state and not in the widget itself,
/// because the application rebuilds every widget from `view` on every frame: a
/// field on the struct would be reset to `false` by the next frame's
/// construction, before any key could arrive. It is the same arrangement the
/// toolkit uses — `src/widget/button/widget.rs:1058-1061` implements `Focusable`
/// for the button's `State`, which its `tag`/`state` put in the tree.
#[derive(Debug, Default)]
struct Focus {
    focused: bool,
}

impl Focusable for Focus {
    fn is_focused(&self) -> bool {
        self.focused
    }

    fn focus(&mut self) {
        self.focused = true;
    }

    fn unfocus(&mut self) {
        self.focused = false;
    }
}

/// A toolkit widget plus the two hooks the toolkit left out.
///
/// See the module docs for which toggle is which. Constructed through
/// [`toggler`], [`input`] or [`dropdown`], which fix the role and the activation
/// for their widget kind; the setters below are shared by all three.
pub struct Accessible<'a, Message> {
    inner: Element<'a, Message>,
    id: Id,
    name: String,
    role: Role,
    value: Option<String>,
    checked: Option<bool>,
    /// What Enter and Space publish. Built by the constructor from the same
    /// value the toolkit's own pointer path publishes, so the two paths cannot
    /// disagree about what they send.
    activate: Option<Message>,
    /// What Up and Down publish, given a step of `-1` or `1`.
    ///
    /// Only the dropdown uses this, and it is what keeps the focus ring honest:
    /// a control Tab reaches and cannot change is not a fix, and the dropdown's
    /// popup is not reachable from outside `dropdown::widget` (module docs).
    on_step: Option<Box<dyn Fn(i32) -> Option<Message> + 'a>>,
    /// Whether *this* widget reports itself to `operation.focusable`, or leaves
    /// that to the widget it wraps.
    ///
    /// `false` for the text input and `true` for the other two, and the reason
    /// is measured rather than assumed: `cosmic::widget::TextInput` already
    /// reports itself (`src/widget/text_input/input.rs:853`), so a wrapper that
    /// reported *as well* would put one field in the Tab ring twice — a keyboard
    /// user would have to press Tab twice to leave it, and the first press would
    /// land on a widget that draws no focus ring — which is what
    /// `a_text_input_is_one_tab_stop_and_not_two` pins. The toggler and the
    /// dropdown report nothing at all, so there the wrapper is the only thing
    /// that can put them in the ring.
    own_focus: bool,
}

impl<'a, Message: Clone + 'a> Accessible<'a, Message> {
    fn wrap(inner: impl Into<Element<'a, Message>>, role: Role, name: String) -> Self {
        Self {
            inner: inner.into(),
            id: stable_id(&name),
            name,
            role,
            value: None,
            checked: None,
            activate: None,
            on_step: None,
            own_focus: true,
        }
    }

    /// The widget's current text, for a node that has one.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// The widget's on/off state, for a node that has one.
    #[must_use]
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = Some(checked);
        self
    }

    /// The message Enter and Space publish, and the one an assistive
    /// technology's `Action::Click` publishes.
    #[must_use]
    pub fn on_activate(mut self, message: Message) -> Self {
        self.activate = Some(message);
        self
    }

    /// The message Up and Down publish, given a step of `-1` or `1`.
    ///
    /// Returning `None` leaves the key uncaptured, so a control at the end of
    /// its list does not swallow the arrow.
    #[must_use]
    pub fn on_step(mut self, on_step: impl Fn(i32) -> Option<Message> + 'a) -> Self {
        self.on_step = Some(Box::new(on_step));
        self
    }
}

/// A [`cosmic::widget::Toggler`] a keyboard and a screen reader can both use.
///
/// `is_toggled` is the same value handed to the toolkit's `toggler`, and
/// `on_toggle_value` is what that toggler's own `on_toggle` closure returns for
/// the flipped state — i.e. `on_toggle(!is_toggled)`. It is a value rather than
/// a closure because the toolkit's `Toggler` owns its closure and this wrapper
/// never sees it, so the caller, which has the state, hands over the one message
/// the keyboard path needs.
///
/// # A disabled switch is not operable, and this is where that is decided
///
/// `on_toggle_value` is `Option` rather than a bare message because of a defect
/// this wrapper *introduced* and a red-team pass caught. `view::form` has rows
/// the reference disables on Linux (`enabled: !form.isLinux`), and
/// `toggle_control` expresses that by dropping the toolkit's `on_toggle` — the
/// **pointer** cannot flip them. But the wrapper was applied unconditionally, so
/// the **keyboard** still could: Enter and Space published a real
/// `FormToggleChanged`, the node advertised `Action::Click`, and eleven rows the
/// reference forbids became editable by anyone using a keyboard. Before the
/// wrapper existed those rows were simply unreachable, so this was a regression
/// rather than an unfixed gap.
///
/// Passing `None` therefore has to mean *the same thing the dropped `on_toggle`
/// means*: no activation path exists. It removes the `Action::Click` the node
/// would advertise and leaves the key uncaptured, so the switch reports its
/// state and cannot be changed — which is what an inert control is. `form.rs`
/// derives it from the same `live` flag as the pointer, so the two paths cannot
/// disagree again.
#[must_use]
pub fn toggler<'a, Message: Clone + 'static>(
    inner: cosmic::widget::Toggler<'a, Message>,
    label: impl Into<String>,
    is_toggled: bool,
    on_toggle_value: Option<Message>,
) -> Accessible<'a, Message> {
    let wrapper = Accessible::wrap(inner, Role::Switch, label.into()).checked(is_toggled);
    match on_toggle_value {
        Some(message) => wrapper.on_activate(message),
        None => wrapper,
    }
}

/// A [`cosmic::widget::TextInput`] that says what it is and what it holds.
///
/// It adds a node and nothing else. The input already reports itself focusable
/// and already handles typing, so [`Accessible::own_focus`] is turned **off**
/// here: with it on, one field would be two Tab stops.
///
/// # The input is given the id the node carries
///
/// The one thing this does to the wrapped widget. Left alone, the input reports
/// itself under its own `Id::unique()` (`src/widget/text_input/input.rs:238`,
/// reported at `:853`) while the node below carries the wrapper's stable,
/// name-derived id — so the accessibility tree would name an id that nothing in
/// the Tab ring reports, and assistive technology could see the field and not
/// reach it. `TextInput::id` (`:308`) is public, so the two are made the same
/// id instead, and the input also becomes addressable by its name. The id is
/// [`stable_id`]`(label)`, i.e. the same lookup [`Accessible::wrap`] is about to
/// derive from the same label; [`input_with_id`] is this function for a field
/// whose id is fixed outside this module.
///
/// **This was wrong first, and a page test found it.**
/// `a_text_input_is_one_tab_stop_and_not_two` originally asserted the opposite —
/// that the tab stop's id and the node's id *differ* — on the reading that the
/// tab stop "must be the widget that accepts typing". Both are the same widget
/// either way; what the mismatched ids also meant was that no focus report and
/// no node agreed on a name. The unit test could not see that, because it only
/// ever asked which ids were reported and never asked whether the page's node
/// ids were among them. `view/form.rs`'s
/// `every_wrapped_control_on_the_real_form_is_a_tab_stop_and_a_named_node` did,
/// and it failed on the category field with `Unique(22)` in the tree and
/// `Unique(21)` in the ring. The assertion here is inverted to match.
#[must_use]
pub fn input<'a, Message: Clone + 'static>(
    inner: cosmic::widget::TextInput<'a, Message>,
    label: impl Into<String>,
    value: impl Into<String>,
) -> Accessible<'a, Message> {
    let label = label.into();
    // The same `stable_id` lookup `Accessible::wrap` is about to make, so handing
    // it to the input gives both halves one id.
    let id = stable_id(&label);
    input_with_id(inner, label, value, id)
}

/// [`input`] for a field whose id is fixed somewhere other than its label.
///
/// The Library's search box is the caller, and the reason is `Ctrl+F`.
/// `view/library.rs`'s `SEARCH_INPUT_ID` is the id that box must be *reached*
/// by — `Shell::focus_library_search` focuses it by name from `main.rs` — so it
/// is a constant rather than anything derivable from `"Search games…"`, and the
/// name-derived id `input` would give it is an id no operation names. Naming the
/// node after the label while the input wears the constant is the one
/// arrangement that cannot work: the node would carry an id no focus report
/// mentions, which is the defect [`input`]'s doc records. So the caller names
/// one id and **both halves wear it**.
///
/// The module docs' uniqueness rule is a rule about *names*, and it does not
/// cover this: two of these handed the same `Id` would still collapse into one
/// node, and not handing over the same one twice is the caller's business.
#[must_use]
pub fn input_with_id<'a, Message: Clone + 'static>(
    inner: cosmic::widget::TextInput<'a, Message>,
    label: impl Into<String>,
    value: impl Into<String>,
    id: Id,
) -> Accessible<'a, Message> {
    let inner = inner.id(id.clone());
    let mut wrapper = Accessible::wrap(inner, Role::TextInput, label.into()).value(value);
    // The id `wrap` derived from the label is replaced, on both halves: the
    // wrapper is what publishes the node, the input is what reports itself to
    // `operation.focusable`.
    wrapper.id = id;
    wrapper.own_focus = false;
    wrapper
}

/// A [`cosmic::widget::Dropdown`] in the Tab ring and in the accessibility tree.
///
/// `step` receives `-1` or `1` and returns the message that moves the selection
/// by that much, or `None` at either end of the list. It is a parameter rather
/// than something this module computes because only the caller knows how its
/// selection is keyed — `crate::view::settings` selects a runner through a list
/// of choices, `crate::view::form` through a table of labels.
///
/// # The sites
///
/// Eleven, and each one takes its name from the string the page already draws
/// beside the control and its `step` from the same function the toolkit's
/// `on_selected` calls:
///
/// | Page | Controls | `step` counts from |
/// |---|---|---|
/// | [`crate::view::settings`] | colour scheme, layout, default runner | the index the selector shows, `None` at either end |
/// | [`crate::view::form`] | game type, category, runner | as above, off `kind_index` / `runner_index` |
/// | [`crate::view::library`] | category filter, sort order | `category_index` / `sort_index` |
/// | [`crate::view::installers`] | category filter, install runner | `selected_category` / `runner_index` |
/// | [`crate::view::runners`] | family | `family_index` |
///
/// What none of them can do is open the toolkit's popup; the module docs say
/// why, with the three measurements behind it.
#[must_use]
pub fn dropdown<'a, S, Message, AppMessage>(
    inner: cosmic::widget::Dropdown<'a, S, Message, AppMessage>,
    label: impl Into<String>,
    selected: Option<String>,
    step: impl Fn(i32) -> Option<Message> + 'a,
) -> Accessible<'a, Message>
where
    S: AsRef<str> + Send + Sync + Clone + 'static,
    Message: Clone + 'static,
    AppMessage: Clone + 'static,
    [S]: std::borrow::ToOwned,
{
    let wrapper = Accessible::wrap(inner, Role::ComboBox, label.into()).on_step(step);
    match selected {
        Some(selected) => wrapper.value(selected),
        None => wrapper,
    }
}

impl<'a, Message: Clone + 'a>
    cosmic::iced::advanced::Widget<Message, cosmic::Theme, cosmic::Renderer>
    for Accessible<'a, Message>
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Focus>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(Focus::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.inner)]
    }

    fn diff(&mut self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_mut(&mut self.inner));
    }

    fn size(&self) -> Size<Length> {
        self.inner.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &cosmic::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let node = self
            .inner
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        let size = node.size();
        // One level of indirection, so that every forwarded call below reaches
        // the inner widget through `layout.children().next()` and the two agree
        // about which rectangle is whose. This is `IdContainer`'s shape
        // (`src/widget/id_container.rs:66-77`).
        layout::Node::with_children(size, vec![node])
    }

    /// The hook `dropdown` and `toggler` do not have.
    ///
    /// `operation.focusable` is what puts the widget in the Tab order:
    /// libcosmic's Tab subscription resolves to `iced::widget::operation::
    /// focus_next()` (`src/app/cosmic.rs:842-849`, on `Named::Tab` from
    /// `src/keyboard_nav.rs:32-38`), and that operation walks the tree calling
    /// `Focusable::focus`/`unfocus` on every widget that reports itself
    /// (`iced/core/src/widget/operation/focusable.rs:170-208`). A widget that
    /// never calls `operation.focusable` is invisible to it — which is the
    /// whole of UX-01's first half and UX-02's.
    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &cosmic::Renderer,
        operation: &mut dyn Operation,
    ) {
        // The focus flag is this wrapper's, in the state slot `tag`/`state`
        // gave it, so it survives the frame that built the widget. The cast at
        // the end of the argument list is the same one iced's own text input
        // makes at `iced/widget/src/text_input.rs:706`.
        //
        // Skipped entirely when the wrapped widget reports itself — see
        // `own_focus`: two reports for one control is two Tab stops.
        if self.own_focus {
            operation.focusable(
                Some(&self.id),
                layout.bounds(),
                tree.state.downcast_mut::<Focus>(),
            );
        }
        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget_mut().operate(
            &mut tree.children[0],
            child_layout.with_virtual_offset(layout.virtual_offset()),
            renderer,
            operation,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &cosmic::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        // An assistive technology's activation is honoured whether or not the
        // framework currently has the keyboard here: an AT may activate a
        // control it has not focused, and iced's own button does the same
        // (`iced/widget/src/button.rs:409-419`). The keyboard arms below are
        // gated on focus, because a key belongs to whatever is focused.
        if self.on_a11y_action(event, shell)
            || (tree.state.downcast_ref::<Focus>().focused && self.on_keyboard(event, shell))
        {
            // Captured: the toolkit's own `update` never sees a key it would
            // have ignored anyway, and nothing downstream can act on the same
            // press a second time.
            return;
        }

        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget_mut().update(
            &mut tree.children[0],
            event,
            child_layout.with_virtual_offset(layout.virtual_offset()),
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &cosmic::Renderer,
    ) -> mouse::Interaction {
        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget().mouse_interaction(
            &tree.children[0],
            child_layout.with_virtual_offset(layout.virtual_offset()),
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut cosmic::Renderer,
        theme: &cosmic::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            child_layout.with_virtual_offset(layout.virtual_offset()),
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &cosmic::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, cosmic::Theme, cosmic::Renderer>> {
        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget_mut().overlay(
            &mut tree.children[0],
            child_layout.with_virtual_offset(layout.virtual_offset()),
            renderer,
            viewport,
            translation,
        )
    }

    fn drag_destinations(
        &self,
        state: &Tree,
        layout: Layout<'_>,
        renderer: &cosmic::Renderer,
        dnd_rectangles: &mut cosmic::iced::advanced::clipboard::DndDestinationRectangles,
    ) {
        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        self.inner.as_widget().drag_destinations(
            &state.children[0],
            child_layout.with_virtual_offset(layout.virtual_offset()),
            renderer,
            dnd_rectangles,
        );
    }

    /// **Not `Some(self.id)` — and this is the whole of the P0 fixed here.**
    ///
    /// The id in this slot is the *tree's* identity, and iced hands it to
    /// `Tree::diff` as well as to `set_id`. Reporting a `Custom` id from here
    /// puts the tree into the named-state branch of `Tree::diff`
    /// (`iced/core/src/widget/tree.rs:170-202`); the branch keys `NAMED` by the
    /// `Custom` string *alone* (`Internal::Custom`'s `Hash` and `PartialEq` are
    /// both the name — `iced/accessibility/src/id.rs:160-171`), and this wrapper
    /// reported the same name its own inner input wears, because `input_with_id`
    /// gives both halves one id. So the wrapper's arm consumed the single entry
    /// for that key and swapped a stripped root in; the input's own arm, run
    /// moments later by the `diff_children` below, found the already-empty
    /// `or_else` branch and left the input *stateless while its tag still
    /// claimed* `text_input::State`; and the input's `diff` downcast that
    /// (`tree.rs:505`, "Downcast on stateless state"). The GUI died on its
    /// second frame. Reproduced in
    /// `a_named_wrapper_survives_the_runtimes_named_state_handoff`.
    ///
    /// The toolkit's own `Named` widget draws this line in this exact place. It
    /// keeps the user's id in a field and reports [`None`] from `Widget::id`
    /// (`src/widget/named.rs:105-119`), which is why a `Named` wrapper never hit
    /// the crash — and why the id it wraps stays reachable by
    /// `operation::focus(id)` anyway: focusing by name matches on the `Custom`
    /// *string* through `IdEq`
    /// (`iced/core/src/widget/operation/focusable.rs:39-51`,
    /// `iced/accessibility/src/id.rs:188-205`), not on this slot. That is the
    /// path `Ctrl+F` takes (`main.rs`'s `focus_library_search`), and it is why
    /// the input — which does need its id in the tree — keeps it.
    fn id(&self) -> Option<Id> {
        None
    }

    fn set_id(&mut self, id: Id) {
        // Unreachable through the tree for the reason above, and left as the
        // plain setter so the field stays honest if it ever is called.
        self.id = id;
    }

    /// The hook `toggler` and `TextInput` do not have, and `dropdown` has only
    /// as a commented-out body (`src/widget/dropdown/widget.rs:375-384`).
    ///
    /// The inner widget's own tree is forwarded rather than discarded, so a
    /// toolkit widget that later grows an `a11y_nodes` of its own contributes
    /// its nodes underneath this one instead of losing them — the idiom is
    /// `iced`'s own `button`, which computes its content's tree
    /// (`iced/widget/src/button.rs:584-588`) and joins it to its own node
    /// (`:640-643`). Today all three forward the trait's default empty tree
    /// (`iced/core/src/widget.rs:149-158`), so the root here is this wrapper's
    /// node.
    fn a11y_nodes(&self, layout: Layout<'_>, state: &Tree, cursor: mouse::Cursor) -> A11yTree {
        let bounds = layout.bounds();
        let rect = Rect::new(
            f64::from(bounds.x),
            f64::from(bounds.y),
            f64::from(bounds.x + bounds.width),
            f64::from(bounds.y + bounds.height),
        );

        let mut node = Node::new(self.role);
        node.set_bounds(rect);
        node.set_label(self.name.clone());
        if let Some(value) = self.value.as_ref() {
            node.set_value(value.clone());
        }
        if let Some(checked) = self.checked {
            node.set_selected(checked);
        }
        // The same pair `iced`'s button, checkbox and toggler publish
        // (`iced/widget/src/button.rs:605-606`,
        // `iced/widget/src/checkbox.rs:556-557`,
        // `iced/widget/src/toggler.rs:590-591`). `Click` is the one this wrapper
        // can honour: see `on_a11y_action`.
        if self.activate.is_some() {
            node.add_action(Action::Click);
        }
        node.add_action(Action::Focus);

        let child_layout = layout
            .children()
            .next()
            .expect("`layout` puts the inner widget in exactly one child node");
        A11yTree::node_with_child_tree(
            A11yNode::new(node, self.id.clone()),
            self.inner.as_widget().a11y_nodes(
                child_layout.with_virtual_offset(layout.virtual_offset()),
                &state.children[0],
                cursor,
            ),
        )
    }
}

impl<Message: Clone> Accessible<'_, Message> {
    /// The keyboard arms this wrapper adds, and whether one of them fired.
    ///
    /// Only called when the framework has the keyboard on this widget.
    /// `capture_event` on a press that was acted on is what stops the same
    /// keystroke from also reaching the app's shortcut subscription: those are
    /// `cosmic::iced::keyboard::listen()`, which sees every key the runtime did
    /// not capture.
    fn on_keyboard(&self, event: &Event, shell: &mut Shell<'_, Message>) -> bool {
        let Event::Keyboard(keyboard::Event::KeyPressed { key, .. }) = event else {
            return false;
        };

        match key {
            // Space is not a `Named` key in this iced: `winit` has no
            // `NamedKey::Space`, so the bar arrives as `Key::Character`, and
            // `conversion::key` passes that straight through
            // (`iced/winit/src/conversion.rs:778`). `Code::Space` exists
            // (`iced/core/src/keyboard/key.rs:993`) but is the *physical* key,
            // which a `KeyPressed` carries in `physical_key` and which ignores
            // the layout — the logical key is the one to match, so that a
            // keyboard whose space bar is remapped still activates.
            keyboard::Key::Character(c) if c.as_str() == " " => self.activate(shell),
            keyboard::Key::Named(keyboard::key::Named::Enter) => self.activate(shell),
            keyboard::Key::Named(keyboard::key::Named::ArrowUp) => self.step(-1, shell),
            keyboard::Key::Named(keyboard::key::Named::ArrowDown) => self.step(1, shell),
            _ => false,
        }
    }

    fn activate(&self, shell: &mut Shell<'_, Message>) -> bool {
        let Some(message) = self.activate.as_ref() else {
            return false;
        };
        shell.publish(message.clone());
        shell.capture_event();
        true
    }

    fn step(&self, delta: i32, shell: &mut Shell<'_, Message>) -> bool {
        let Some(on_step) = self.on_step.as_ref() else {
            return false;
        };
        let Some(message) = on_step(delta) else {
            // The end of the list: the arrow is left to whoever else wants it
            // rather than being swallowed by a control that cannot move.
            return false;
        };
        shell.publish(message);
        shell.capture_event();
        true
    }

    /// An assistive technology's activation, matched to this widget.
    ///
    /// The comparison is [`IdEq`] and not `==`. The runtime turns an
    /// `ActionRequest` back into an id by taking the node's number —
    /// `Id::from(u128::from(target_node.0) as u64)`, a `Unique`
    /// (`iced/winit/src/conversion.rs:1670-1672`) — while this widget's id may be
    /// a `Custom`, because it came from `Id::from` on a `&str` rather than from
    /// [`stable_id`]'s `Id::unique()`: that is the Library's search box, the one
    /// [`input_with_id`] caller. `PartialEq` for `Id`'s internals only matches
    /// like with like and returns `false` for a `Unique` against a `Custom`
    /// (`iced/accessibility/src/id.rs:172-182`); `IdEq` is the comparison that
    /// reconciles exactly that pair (`:188-205`). `iced`'s own button compares
    /// with plain `==` (`iced/widget/src/button.rs:414`), which is sufficient
    /// there because both sides are `Unique`; libcosmic's button compares
    /// nothing at all and captures every `A11y` event that reaches it
    /// (`src/widget/button/widget.rs:847-858`), so a request aimed at a node
    /// deeper in its subtree is lost.
    ///
    /// A request for another node, or one asking for something other than
    /// activation, falls through to the inner widget rather than being captured,
    /// so a node deeper in this subtree can still be reached.
    fn on_a11y_action(&self, event: &Event, shell: &mut Shell<'_, Message>) -> bool {
        let Event::A11y(event_id, request) = event else {
            return false;
        };
        if request.action != Action::Click || !IdEq::eq(&self.id, event_id) {
            return false;
        }
        self.activate(shell)
    }
}

impl<'a, Message: Clone + 'a> From<Accessible<'a, Message>> for Element<'a, Message> {
    fn from(widget: Accessible<'a, Message>) -> Self {
        Element::new(widget)
    }
}

/// The measuring instrument: build an element, read what it reports.
///
/// # Why this is a module of its own and not `mod tests`'s private business
///
/// The tests in this file measure a *hand-built* wrapper, which is the right
/// unit for the wrapper's plumbing but says nothing about whether the pages that
/// use it are reachable. The page tests that answer that live beside the page
/// (`view/settings.rs`, `view/form.rs`), and they need this same instrument —
/// building the widget, running a real `Widget::operate`, reading a real
/// `Widget::a11y_nodes`.
///
/// `settings.rs:1000-1001` states this repo's rule for exactly that situation,
/// (measured against the working tree this file ships in, not against `HEAD`:
/// the accessibility work is what moved that comment down 99 lines, so a reader
/// checking it against `HEAD` will find it at `:901`),
/// written about its own copy of the drawn-strings walker: *"If a third caller
/// ever appears, the right move is a shared `#[cfg(test)]` helper, not a third
/// copy."* The page tests are the third caller. A second copy of the focus
/// walker would also be a second place for the `focus_next` chain loop
/// ([`tab_to`]) and the `A11yTree` shape ([`published`]) to be got wrong, and a
/// copy that got them wrong would fail *silently* — it would report an empty
/// list, which is the pre-fix state, and the test would still be red for the
/// right reason only by luck. Sharing it means one implementation, measured
/// once, used everywhere.
///
/// Everything here is generic over the message type on purpose: the page's
/// `Message` has no `PartialEq` (it holds a `ToastId`, a `Task` and a
/// `GameForm`), so the page tests compare messages by destructuring rather than
/// by `assert_eq!`. Nothing in this module needs to compare a message, so
/// nothing here constrains `Message`.
#[cfg(test)]
pub(crate) mod harness {
    use super::*;
    use cosmic::iced::advanced::widget::operation;
    use cosmic::iced::advanced::widget::operation::Focusable;
    use cosmic::iced::{Font, Pixels};

    /// A real renderer. `iced_tiny_skia` is pure software, so this needs no
    /// display and draws nothing — `layout` wants it only to ask the font stack
    /// how wide a string is.
    pub(crate) fn renderer() -> cosmic::Renderer {
        cosmic::Renderer::new(Font::default(), Pixels(16.0))
    }

    /// A built element: its tree, and the layout the framework computed for it.
    pub(crate) fn built<M: Clone + 'static>(el: &mut Element<'_, M>) -> (Tree, layout::Node) {
        let renderer = renderer();
        let mut tree = Tree::new(el.as_widget());
        let limits = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
        (tree, node)
    }

    /// Every widget that reported itself focusable, with the id it reported.
    pub(crate) fn focusables<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<Option<Id>> {
        #[derive(Default)]
        struct Reported(Vec<Option<Id>>);

        impl Operation for Reported {
            fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
                operate(self);
            }

            fn focusable(
                &mut self,
                id: Option<&Id>,
                _bounds: Rectangle,
                _state: &mut dyn Focusable,
            ) {
                self.0.push(id.cloned());
            }
        }

        let renderer = renderer();
        let (mut tree, node) = built(el);
        let mut reported = Reported::default();
        el.as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut reported);
        reported.0
    }

    /// One node of an [`A11yTree`], as the facts a reader of it would want.
    #[derive(Debug)]
    pub(crate) struct NodeFacts {
        pub(crate) id: iced_accessibility::A11yId,
        pub(crate) role: Role,
        pub(crate) label: Option<String>,
        pub(crate) value: Option<String>,
        pub(crate) selected: Option<bool>,
        pub(crate) bounds: Option<Rect>,
        pub(crate) click: bool,
        pub(crate) focus: bool,
    }

    pub(crate) fn facts(node: &A11yNode) -> NodeFacts {
        let raw = node.node();
        NodeFacts {
            id: node.id().clone(),
            role: raw.role(),
            label: raw.label().map(str::to_string),
            value: raw.value().map(str::to_string),
            selected: raw.is_selected(),
            bounds: raw.bounds(),
            click: raw.supports_action(Action::Click),
            focus: raw.supports_action(Action::Focus),
        }
    }

    /// The nodes a built element publishes, root first.
    ///
    /// `UserInterface::a11y_nodes` is the only entrance the runtime uses
    /// (`iced/runtime/src/user_interface.rs:606-616`), and it is exactly this
    /// call, so what this reads is what accesskit would be handed. The cursor is
    /// [`mouse::Cursor::Unavailable`], which is `#[default]`
    /// (`iced/core/src/mouse/cursor.rs:13`) — a node's shape does not depend on
    /// where the pointer is, and none of these widgets is hovered.
    pub(crate) fn published<M: Clone + 'static>(el: &mut Element<'_, M>) -> Vec<NodeFacts> {
        let (tree, node) = built(el);
        let a11y = el
            .as_widget()
            .a11y_nodes(Layout::new(&node), &tree, mouse::Cursor::Unavailable);
        a11y.root()
            .iter()
            .chain(a11y.children().iter())
            .map(facts)
            .collect()
    }

    /// The id a built control is addressed by: the one its **node** carries.
    ///
    /// # Why this reads the node and not `Widget::id()`
    ///
    /// It read `Widget::id()` until the P0 on [`Accessible::id`] was fixed, and
    /// the substitution is worth recording because the old form was a proxy that
    /// only happened to agree.
    ///
    /// `Widget::id()` is the id the framework stores in the widget *tree*, and
    /// the wrapper no longer reports one — for the reason documented on
    /// [`Accessible::id`], which is that a `Custom` id in that slot sends the
    /// tree through `Tree::diff`'s named-state branch and crashes the GUI. What
    /// the four tests below actually need is the id the control *answers to*:
    /// the one an accesskit `ActionRequest` is matched against
    /// (`Accessible::on_a11y_action`), the one `operation::focus(id)` reaches
    /// it by, and the one assistive technology reads off the node. That id
    /// survives the fix unchanged, and it is the one this helper now returns, so
    /// the tests assert the property they were written for rather than the
    /// storage location it used to coincide with.
    pub(crate) fn element_id<M: Clone + 'static>(el: &mut Element<'_, M>) -> Option<Id> {
        match published(el).into_iter().next()?.id {
            iced_accessibility::A11yId::Widget(id) => Some(id),
            iced_accessibility::A11yId::Window(_) => None,
        }
    }

    /// One node a page publishes that **no focus report carries**, filtered to
    /// the nodes that are controls of one of the three kinds this module wraps.
    ///
    /// # Why the filter is the three roles and not "every node"
    ///
    /// `Switch`, `ComboBox` and `TextInput` are the whole of UX-01, UX-02 and
    /// UX-03: a node with one of those roles is a control the app drew (the
    /// toolkit publishes none of the three — `the_toolkit_controls_the_app_used_
    /// to_build_are_invisible` measures that). Everything else a page publishes
    /// is a node the *toolkit* built for something that is not a control in this
    /// sense and never was in the Tab ring: a `text` widget publishes a
    /// `Paragraph` carrying the string it draws, an icon publishes an `Image`, a
    /// scrollable publishes its `ScrollView` and its `ScrollBar`. Asserting over
    /// those would be asserting that the drawn label beside a control is
    /// focusable, which it is not and should not be.
    ///
    /// # Why the caller must hand over one element and not two builds
    ///
    /// The toolkit's own widgets take `Id::unique()` at construction
    /// (`src/widget/button/widget.rs:63`, `iced/widget/src/toggler.rs:137`), so
    /// two builds of the same page disagree about their ids — a *toolkit* node
    /// read from one build and a *toolkit* focus report read from another would
    /// look unreachable when both are perfectly reachable. That difference is the
    /// module's own premise: the wrappers' ids are stable across frames
    /// ([`stable_id`]) and the toolkit's are not, which is the whole reason this
    /// module exists. So the caller builds **one** element and reads both halves
    /// out of it.
    ///
    /// # What this predicate is not
    ///
    /// It is a check on the *pair* "a control of one of these three roles" and
    /// "an id the ring reports", and it is weaker than "every control is
    /// reachable" in three measured ways a caller must not read past:
    ///
    /// * a focus report that carries `None` is dropped by `flatten()`, so a
    ///   focused widget that reports no id cannot match anything — the same
    ///   silence a genuinely unreachable node produces. The page tests each
    ///   assert separately that every stop names an id, which is what closes
    ///   that hole rather than this function pretending to;
    /// * **two controls sharing an id read as reachable.** One report under that
    ///   id is enough for both nodes. The duplicate-id loops in the four page
    ///   tests are what close *that* one;
    /// * two builds are not compared here at all — the caller hands over one
    ///   element and this builds from it twice (`published`, `focusables`), each
    ///   of which is a fresh [`Tree`] over the same element. That is safe only
    ///   because the wrappers' ids come from [`stable_id`] and the toolkit's
    ///   widgets it reads here report their ids from the element's own tree
    ///   rather than minting new ones; a widget whose id were allocated inside
    ///   `layout` would defeat it.
    ///
    /// The comparison is `IdEq` for the reason [`Accessible::on_a11y_action`]
    /// gives: the node's id may be a `Custom` (the Library's search box) where the
    /// report is a number, or the other way round.
    pub(crate) fn unreachable_controls<M: Clone + 'static>(
        el: &mut Element<'_, M>,
    ) -> Vec<(Role, Option<String>, iced_accessibility::A11yId)> {
        let nodes = published(el);
        let reported = focusables(el);
        nodes
            .into_iter()
            .filter(|node| matches!(node.role, Role::Switch | Role::ComboBox | Role::TextInput))
            .filter(|node| {
                !reported
                    .iter()
                    .flatten()
                    .any(|id| IdEq::eq(&iced_accessibility::A11yId::from(id.clone()), &node.id))
            })
            .map(|node| (node.role, node.label, node.id))
            .collect()
    }

    /// What a real `Widget::update` makes the element publish, and whether the
    /// event was captured.
    #[derive(Debug, Default)]
    pub(crate) struct Dispatched<M> {
        pub(crate) messages: Vec<M>,
        pub(crate) captured: bool,
    }

    pub(crate) fn dispatch<M: Clone + 'static>(
        el: &mut Element<'_, M>,
        tree: &mut Tree,
        node: &layout::Node,
        event: &Event,
        messages: &mut Vec<M>,
    ) -> Dispatched<M> {
        let renderer = renderer();
        let layout = Layout::new(node);
        let viewport = Rectangle::new(cosmic::iced::Point::ORIGIN, Size::new(4096.0, 4096.0));
        let before = messages.len();
        let mut clipboard = cosmic::iced::advanced::clipboard::Null;
        let mut shell = Shell::new(messages);
        el.as_widget_mut().update(
            tree,
            event,
            layout,
            mouse::Cursor::Unavailable,
            &renderer,
            &mut clipboard,
            &mut shell,
            &viewport,
        );
        // Read off the shell before the borrow of `messages` is released.
        let captured = shell.is_event_captured();
        Dispatched {
            messages: messages[before..].to_vec(),
            captured,
        }
    }

    /// Put the framework's keyboard focus on the element, through the operation
    /// libcosmic's Tab subscription runs (`src/app/cosmic.rs:842-849`).
    ///
    /// **Not by reaching into the tree state.** A test that set the focus flag
    /// itself would prove that the wrapper acts on a flag, which is not the
    /// claim — the claim is that Tab reaches it, and `focus_next` is what does
    /// that. This is the same operation the framework runs, so the two cannot
    /// disagree about whether the widget is in the Tab ring.
    ///
    /// # Why this is a loop and not one `operate` call
    ///
    /// `focus_next` is `operation::then(count(), …)`
    /// (`iced/core/src/widget/operation/focusable.rs:208`): the first pass only
    /// *counts* the focusables and `finish()` then hands back the operation that
    /// actually moves the focus (`Outcome::Chain`,
    /// `iced/core/src/widget/operation.rs:146-155`). Running it once would set
    /// nothing and this helper would silently measure nothing — which is why the
    /// loop below is the runtime's own, transcribed from the driver that
    /// dispatches a widget action (`iced/winit/src/lib.rs:2519-2532`, and the
    /// same four lines in iced's emulator, `iced/test/src/emulator.rs:194-207`).
    ///
    /// # Why the tree is passed in
    ///
    /// The focus this sets lives in the element's [`Tree`], so a helper that
    /// built its own tree and dropped it would leave the next `update` looking
    /// at a fresh, unfocused state — and every key test below would report "no
    /// effect" for a widget that works. That was the first version of this
    /// helper, measured: `enter_and_space_toggle_a_focused_toggler` published
    /// nothing until the tree was threaded through.
    pub(crate) fn tab_to<M: Clone + 'static>(
        el: &mut Element<'_, M>,
        tree: &mut Tree,
        node: &layout::Node,
    ) {
        let renderer = renderer();
        let mut current: Option<Box<dyn Operation<()>>> =
            Some(Box::new(operation::focusable::focus_next::<()>()));
        while let Some(mut op) = current.take() {
            el.as_widget_mut()
                .operate(tree, Layout::new(node), &renderer, op.as_mut());
            if let operation::Outcome::Chain(next) = op.finish() {
                current = Some(next);
            }
        }
    }

    /// A key press, built the way the runtime builds one.
    ///
    /// `physical_key` is pinned to `Code::Space` because a real keyboard event
    /// always carries a physical key and the widget under test must not be
    /// reading it: this wrapper keys off `key`/`modified_key` only, so a test
    /// that passed different physical keys would still pass, and pinning one
    /// fixed value is what makes that visible rather than accidental.
    ///
    /// # Space is `Character(" ")`, not `Named::Space`
    ///
    /// There is no `NamedKey::Space` in winit, so a real spacebar arrives as
    /// `keyboard::Key::Character(" ")` and `conversion::key` hands that straight
    /// through: the arm at `iced/winit/src/conversion.rs:778` is the generic
    /// `winit::keyboard::Key::Character(c) => keyboard::Key::Character(c)`, and
    /// it is the `Named` arm below it (`:779`) that would have had to carry a
    /// space-specific one. A test that pressed
    /// `Named::Space` would be pressing a key no keyboard produces, and the
    /// wrapper would look broken for a reason that cannot happen.
    pub(crate) fn pressed(key: keyboard::Key) -> Event {
        Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key.clone(),
            physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Space),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::default(),
            text: None,
            repeat: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::harness::*;
    use super::*;
    use cosmic::iced::keyboard::key;

    /// A message type of this module's own, so these tests measure the wrapper
    /// and not the app. The app's `Message` has no `PartialEq` — it holds a
    /// `ToastId`, a `Task` and a `GameForm` — so a comparison against it would
    /// have to destructure; a test-local type can just be compared, and what is
    /// under test here is the wrapper's plumbing rather than any app message.
    #[derive(Debug, Clone, PartialEq)]
    enum Msg {
        Toggled(bool),
        Picked(usize),
    }

    fn toggled_widget(is_toggled: bool) -> Element<'static, Msg> {
        toggler(
            cosmic::widget::toggler(is_toggled).label("Enable DXVK by default".to_string()),
            "Enable DXVK by default",
            is_toggled,
            Some(Msg::Toggled(!is_toggled)),
        )
        .into()
    }

    /// **A toggler is in the Tab ring and publishes a switch node carrying what
    /// it shows** — UX-02's two halves in one measurement.
    ///
    /// Both are asserted against the *built* element: the focus report comes out
    /// of a real `Widget::operate`, the node out of a real `Widget::a11y_nodes`,
    /// and the label the node carries is the same string the toolkit paints on
    /// the switch. The id handed to the traversal and the node's id are asserted
    /// to be the same id, because that identity is what lets an incoming
    /// `ActionRequest` be matched back to this widget at all (see
    /// `on_a11y_action`).
    #[test]
    fn a_toggler_is_focusable_and_publishes_a_switch() {
        let mut el = toggled_widget(true);
        let id = element_id(&mut el).expect("the wrapper reports an id of its own");

        assert_eq!(
            focusables(&mut el),
            vec![Some(id.clone())],
            "exactly one widget must report itself focusable: the wrapper. A bare \
             `cosmic::widget::Toggler` reports none (see \
             `the_toolkit_controls_the_app_used_to_build_are_invisible`), so this \
             is the assertion that fails if the wrapper is removed"
        );

        let nodes = published(&mut el);
        assert_eq!(nodes.len(), 1, "one root node, nothing else: {nodes:#?}");
        let node = &nodes[0];
        assert_eq!(node.id, iced_accessibility::A11yId::from(id));
        assert_eq!(node.role, Role::Switch);
        assert_eq!(node.label.as_deref(), Some("Enable DXVK by default"));
        assert_eq!(
            node.selected,
            Some(true),
            "the node must report the state the switch draws, not a default"
        );
        assert!(
            node.focus,
            "an assistive technology must be able to focus it"
        );
        assert!(
            node.click,
            "an assistive technology must be able to activate it"
        );
        assert!(
            node.bounds.is_some_and(|rect| rect.width() > 0.0),
            "a node with no rectangle cannot be pointed at: {:?}",
            node.bounds
        );
    }

    /// **The off state is reported as off** — the other half of the `selected`
    /// claim above, which a wrapper that always said `true` would pass.
    #[test]
    fn a_toggler_reports_off_as_off() {
        let mut el = toggled_widget(false);
        let nodes = published(&mut el);
        assert_eq!(nodes[0].selected, Some(false));
        assert_eq!(nodes[0].label.as_deref(), Some("Enable DXVK by default"));
    }

    /// **Enter and Space toggle a toggler Tab has reached, and the key is
    /// captured.**
    ///
    /// The focus comes from `focus_next` rather than from a flag set here (see
    /// [`tab_to`]), and the message is asserted to be the one the pointer path
    /// publishes — `on_toggle(!is_toggled)` — because a keyboard path that
    /// published `on_toggle(is_toggled)` would toggle nothing and would look
    /// identical from the widget tree.
    ///
    /// The captured flag is asserted because it is load-bearing: the app's
    /// shortcuts are `keyboard::listen()` subscriptions, and an uncaptured
    /// `Space` would reach them as well as this control.
    #[test]
    fn enter_and_space_toggle_a_focused_toggler() {
        for key in [
            keyboard::Key::Named(key::Named::Enter),
            keyboard::Key::Character(" ".into()),
        ] {
            let mut el = toggled_widget(true);
            let (mut tree, node) = built(&mut el);
            tab_to(&mut el, &mut tree, &node);

            let mut messages = Vec::new();
            let dispatched = dispatch(
                &mut el,
                &mut tree,
                &node,
                &pressed(key.clone()),
                &mut messages,
            );

            assert_eq!(
                dispatched.messages,
                vec![Msg::Toggled(false)],
                "{key:?} on a focused toggler must publish the flip exactly once"
            );
            assert!(
                dispatched.captured,
                "{key:?} must be captured, or the app's shortcut listeners see it too"
            );
        }
    }

    /// **A key that arrives while the toggler is not focused does nothing.**
    ///
    /// The other side of the assertion above: without this, a wrapper that
    /// toggled on Enter from anywhere would pass that test, and every Space
    /// typed into a form would flip every switch on the page.
    #[test]
    fn enter_does_nothing_without_focus() {
        let mut el = toggled_widget(true);
        let (mut tree, node) = built(&mut el);
        let mut messages = Vec::new();
        let dispatched = dispatch(
            &mut el,
            &mut tree,
            &node,
            &pressed(keyboard::Key::Named(key::Named::Enter)),
            &mut messages,
        );
        assert!(dispatched.messages.is_empty());
        assert!(!dispatched.captured);
    }

    /// **The toolkit controls the app used to build are invisible to both
    /// hooks** — the pre-fix state, kept as a guard rather than as history.
    ///
    /// This is what `docs/audit/COSMIC-UX.md`'s UX-01 and UX-02 rows describe,
    /// measured instead of read: `cosmic::widget::Toggler` has no `operate` at
    /// all and no `a11y_nodes` (`src/widget/toggler.rs` — zero occurrences of
    /// either, and no `pub use` of iced's, which is what the audit row mistook
    /// `:15` for), and `cosmic::widget::Dropdown`'s `operate` body and
    /// `a11y_nodes` are commented out (`src/widget/dropdown/widget.rs:330-337`,
    /// `:375-384`). So a page built from them reports no focusable widget and
    /// publishes no node.
    ///
    /// **If this test fails, upstream grew the hooks** — the wrapper is then
    /// redundant for that widget and this file's docs, the audit row and the
    /// call sites all need revisiting. It is written to fail loudly in that
    /// direction rather than to keep passing.
    #[test]
    fn the_toolkit_controls_the_app_used_to_build_are_invisible() {
        let mut bare_toggler: Element<'static, Msg> = cosmic::widget::toggler(true)
            .label("Enable DXVK".to_string())
            .into();
        assert_eq!(
            focusables(&mut bare_toggler),
            Vec::<Option<Id>>::new(),
            "`cosmic::widget::Toggler` has grown an `operate`; the wrapper and \
             the audit row are now stale"
        );
        let nodes = published(&mut bare_toggler);
        assert!(
            nodes.is_empty(),
            "`cosmic::widget::Toggler` has grown `a11y_nodes`; the wrapper and \
             the audit row are now stale. Nodes: {nodes:#?}"
        );

        let mut bare_dropdown: Element<'static, Msg> =
            cosmic::widget::dropdown(vec!["a".to_string(), "b".to_string()], Some(0), Msg::Picked)
                .into();
        assert_eq!(
            focusables(&mut bare_dropdown),
            Vec::<Option<Id>>::new(),
            "`cosmic::widget::Dropdown` has grown a working `operate`; the \
             wrapper and the audit row are now stale"
        );
        let nodes = published(&mut bare_dropdown);
        assert!(
            nodes.is_empty(),
            "`cosmic::widget::Dropdown` has grown `a11y_nodes`; the wrapper and \
             the audit row are now stale. Nodes: {nodes:#?}"
        );
    }

    /// A text input's node names the field and carries its contents.
    ///
    /// The input was already focusable before this wrapper existed — it reports
    /// itself at `src/widget/text_input/input.rs:853` — so the fix here is the
    /// node, and the focus report is a regression guard on the *count*: see
    /// [`a_text_input_is_one_tab_stop_and_not_two`] for why that count is one.
    /// Both are asserted so that a wrapper which dropped the inner widget's own
    /// report (by not forwarding `operate`) cannot pass.
    #[test]
    fn a_text_input_publishes_its_label_and_value() {
        let mut el: Element<'static, Msg> = input(
            cosmic::widget::text_input("Name", "Half-Life"),
            "Name",
            "Half-Life",
        )
        .into();
        let nodes = published(&mut el);
        assert_eq!(nodes.len(), 1, "{nodes:#?}");
        assert_eq!(nodes[0].role, Role::TextInput);
        assert_eq!(nodes[0].label.as_deref(), Some("Name"));
        assert_eq!(
            nodes[0].value.as_deref(),
            Some("Half-Life"),
            "the field's contents must be readable, not just its name"
        );
    }

    /// **One text field is one Tab stop, and it is the same id the node
    /// carries.**
    ///
    /// `cosmic::widget::TextInput` already reports itself focusable
    /// (`src/widget/text_input/input.rs:853`), so a wrapper that reported as
    /// well would put the field in the ring twice. This is not a hypothetical:
    /// the first version of [`input`] did exactly that, and it was this
    /// assertion — written as `vec![Some(id)]` over a traversal that returned
    /// **two** ids — that showed it.
    ///
    /// # The second half of this assertion was inverted, and that is a finding
    ///
    /// It used to read `assert_ne!`: the tab stop's id must **differ** from the
    /// node's id, on the reasoning that the tab stop has to be the widget that
    /// accepts typing rather than the wrapper. Both are the same widget — the
    /// traversal reaches the inner input either way — and the assertion was
    /// therefore pinning a *mismatch*: the inner input reported `Id::unique()`
    /// while the node carried the wrapper's name-derived id, so the id in the
    /// accessibility tree was one no focus report ever mentioned. A screen reader
    /// could see the field and not reach it, and this test called that correct.
    ///
    /// It was `view/form.rs`'s `every_wrapped_control_on_the_real_form_is_a_tab_
    /// stop_and_a_named_node` that failed on it — on the real page, where the
    /// category field's node id `Unique(22)` appeared in no focus report while
    /// `Unique(21)` did. The reason this unit test could not see it is the
    /// project's own named defect class in miniature: it inspected the *count* of
    /// focus reports and never asked whether the ids they carried were the ids
    /// the nodes carried. [`input`] now gives the input the node's id, and this
    /// asserts the identity instead of the difference.
    #[test]
    fn a_text_input_is_one_tab_stop_and_not_two() {
        let mut el: Element<'static, Msg> = input(
            cosmic::widget::text_input("Name", "Half-Life"),
            "Name",
            "Half-Life",
        )
        .into();

        let node_id = published(&mut el)[0].id.clone();
        let reported = focusables(&mut el);

        assert_eq!(
            reported.len(),
            1,
            "one field, one tab stop; got {reported:?}"
        );
        assert_eq!(
            reported[0]
                .as_ref()
                .map(|id| iced_accessibility::A11yId::from(id.clone())),
            Some(node_id),
            "the id Tab reaches and the id the node is published under must be the \
             same id. A `ne` here is not a fix: it means the accessibility tree \
             names a widget the Tab ring has never heard of, so the field can be \
             read and cannot be reached"
        );

        // The control: a toggler, where nothing else reports, so the wrapper is
        // the tab stop. Both halves together are what say the count above is a
        // property of the input and not of the traversal.
        let mut el = toggled_widget(true);
        let id = element_id(&mut el);
        assert_eq!(focusables(&mut el), vec![id]);
    }

    /// **A field whose id the caller fixes wears that id on both halves.**
    ///
    /// [`input`] derives the id from the label; this is the other case, and the
    /// one real caller is the Library's search box, which `Ctrl+F` focuses by the
    /// constant `view/library.rs` calls `SEARCH_INPUT_ID`. The identity asserted
    /// here is the one `a_text_input_is_one_tab_stop_and_not_two` asserts, over
    /// an id that is not name-derived: the wrapper publishes the node under it
    /// and the input reports *itself* to `operation.focusable` under it. A
    /// constructor that passed the id to only one of the two would leave the
    /// field either unreachable (`view/library.rs`'s shortcut would find no
    /// widget) or unnamed to assistive technology, and both are silent.
    #[test]
    fn a_caller_chosen_id_is_both_the_inputs_and_the_nodes() {
        let id: Id = "gamehandler.test.search".into();
        let mut el: Element<'static, Msg> = input_with_id(
            cosmic::widget::text_input("Search games…", "half-life"),
            "Search games…",
            "half-life",
            id.clone(),
        )
        .into();

        let nodes = published(&mut el);
        assert_eq!(nodes.len(), 1, "{nodes:#?}");
        assert_eq!(
            nodes[0].id,
            iced_accessibility::A11yId::from(id.clone()),
            "the node must be published under the caller's id, not one derived \
             from the label"
        );
        assert_eq!(nodes[0].label.as_deref(), Some("Search games…"));
        assert_eq!(nodes[0].value.as_deref(), Some("half-life"));
        assert_eq!(
            focusables(&mut el),
            vec![Some(id)],
            "one field, one Tab stop, and it must report the id the node is \
             published under — the id `Ctrl+F` focuses by"
        );
    }

    /// **Every node this module publishes is an id the Tab ring reports** —
    /// the invariant that ties UX-01 to UX-02/UX-03.
    ///
    /// A node and a focus report are two halves of one control, and it is
    /// possible to have each half working while the two name different things.
    /// That is not a hypothetical either: [`input`] shipped that way, the field's
    /// node under one id and its focus report under another, and the unit tests
    /// beside it were green because each half was measured on its own. See
    /// `a_text_input_is_one_tab_stop_and_not_two` for the inversion.
    ///
    /// Asserted over all three widget kinds rather than over the one that was
    /// wrong, because the property is the module's and not the input's: the
    /// toggler, the dropdown and the input each have to satisfy it, and a future
    /// fourth constructor would have to as well.
    ///
    /// The relation is `IdEq`, not `==`: it is the one that knows a `Unique` and
    /// a `Custom` id can name the same node
    /// (`iced/accessibility/src/id.rs:188-205`), and it is what accesskit's own
    /// `NodeId` conversion resolves to by number.
    #[test]
    fn every_published_node_is_an_id_the_tab_ring_reports() {
        let cases: Vec<(&str, Element<'static, Msg>)> = vec![
            ("toggler", toggled_widget(true)),
            (
                "text input",
                input(
                    cosmic::widget::text_input("Name", "Half-Life"),
                    "Name",
                    "Half-Life",
                )
                .into(),
            ),
            (
                "dropdown",
                dropdown(
                    cosmic::widget::dropdown(
                        vec!["dark".to_string(), "light".to_string()],
                        Some(0),
                        Msg::Picked,
                    ),
                    "Color scheme:",
                    Some("dark".to_string()),
                    |_| None,
                )
                .into(),
            ),
        ];

        for (what, mut el) in cases {
            let nodes = published(&mut el);
            let reported = focusables(&mut el);

            // The wrapper publishes exactly one node, and the toolkit's own
            // widgets underneath it contribute none of their own to the names
            // asserted here — this is the root the wrapper built.
            let node_id = nodes[0].id.clone();
            assert!(
                reported
                    .iter()
                    .flatten()
                    .any(|id| IdEq::eq(&iced_accessibility::A11yId::from(id.clone()), &node_id)),
                "the {what}'s node is published under {node_id:?}, which no focus \
                 report carries: assistive technology can see this control and \
                 cannot reach it. Reports: {reported:?}"
            );
        }
    }

    /// **The startup crash, as a test.**
    ///
    /// The application rebuilt this wrapper on the second frame and died with
    /// `Downcast on stateless state`, so this is the regression test for a P0
    /// that shipped in `8f7269e` and took the whole GUI down before it drew
    /// anything. It reproduces the runtime's own frame-to-frame sequence rather
    /// than a widget's, because that sequence is the thing that was wrong
    /// (`iced/runtime/src/user_interface.rs:106-121`):
    ///
    /// 1. the previous frame's tree hands its named subtrees to the thread-local
    ///    `NAMED` map, emptying each one's `state` in the process;
    /// 2. the map is cleared, so nothing can retrieve them;
    /// 3. the new frame's widget tree is diffed against the stripped tree.
    ///
    /// The wrapper reported an id that was stable *across* frames — the recorded
    /// text field reports `Id::new("gamehandler.library.search")` every frame —
    /// where iced's own state-stealing branch requires an id that is
    /// `Id::unique()`. `Id::unique()` is exactly what "this id names *this*
    /// widget in *this* frame, and nothing else" means, and a constant string is
    /// the opposite of that. On the second frame the id matched, the branch chose
    /// the `or_else` arm, and the arm swapped a `State::None` into a tree whose
    /// tag still claimed the input's state type; the input's `diff` then
    /// downcast it (`iced/core/src/widget/tree.rs:505`).
    ///
    /// The fix is the framework's precondition rather than a patch over it: the
    /// wrapper does not report an id at all. Ids on the wrappers were never
    /// needed for UX-01/UX-02 — the Tab ring is driven by `operate`, which hands
    /// the name to `operation.focusable` directly — and the toolkit's own
    /// `Named` widget removes its id from the public tree for the same reason.
    #[test]
    fn a_named_wrapper_survives_the_runtimes_named_state_handoff() {
        let renderer = renderer();
        let limits = layout::Limits::new(Size::ZERO, Size::new(f32::INFINITY, f32::INFINITY));
        let build = || -> Element<'static, Msg> {
            input_with_id(
                cosmic::widget::text_input("Search games…", "half-life"),
                "Search games…",
                "half-life",
                "gamehandler.test.handoff".into(),
            )
            .into()
        };

        // Frame one: laid out, so the input's state is in the tree.
        let mut first = build();
        let mut tree = Tree::new(first.as_widget());
        let _ = first.as_widget_mut().layout(&mut tree, &renderer, &limits);

        // The runtime's sequence, verbatim and in its order — take, diff, then
        // clear (`iced/runtime/src/user_interface.rs:106`, `:111`, `:119`).
        //
        // **The order is the whole test and my first version had it wrong.** The
        // first draft cleared the map before diffing, because a map that is
        // empty when the diff runs looks like the hostile case. It is not: the
        // bug is that the *lookup* empties the tree, and with the map cleared
        // first there is no lookup and no bug — the draft passed against the
        // broken wrapper, on its own mis-ordering, while `probe_02` on the real
        // sequence panicked. That is this audit's recurring defect in the test
        // written to catch it, so the sequence is spelled out against the
        // framework's line numbers rather than remembered.
        tree::NAMED.with(|named| {
            *named.borrow_mut() = tree.take_all_named();
        });

        // Frame two, against the stripped tree, while the map still holds what
        // frame one handed over.
        let mut second = build();
        tree.diff(second.as_widget_mut());
        let _ = second.as_widget_mut().layout(&mut tree, &renderer, &limits);

        tree::NAMED.with(|named| named.borrow_mut().clear());

        // And the control still works: one Tab stop, published under the name.
        assert_eq!(
            focusables(&mut second),
            vec![Some("gamehandler.test.handoff".into())]
        );
    }

    ///
    /// UX-01 in full, as far as it can be closed from here — and the test is
    /// built to say which part that is. The traversal reports the wrapper, the
    /// node carries the selected option's text, and `ArrowDown` from the
    /// selected index publishes what the dropdown itself would publish for the
    /// next option, because the step closure is handed the caller's own selection
    /// function. What no test here can assert is the toolkit popup: it is not
    /// reachable (module docs), and a test that only ever drives the wrapper
    /// would imply otherwise.
    ///
    /// At the end of the list the arrow is **not** consumed: the closure returns
    /// `None` and the wrapper leaves the key alone, which is the difference
    /// between a control that stops and one that swallows the key at its last
    /// entry.
    #[test]
    fn a_dropdown_is_focusable_named_and_steppable() {
        // A `const`, not a local: the step closure has to outlive the call that
        // hands it over (`'a` on `Accessible`), and a local the closure borrowed
        // would not.
        const SELECTIONS: [&str; 2] = ["dark", "light"];
        const SELECTED: usize = 0;
        let mut el: Element<'static, Msg> = dropdown(
            cosmic::widget::dropdown(SELECTIONS.to_vec(), Some(SELECTED), Msg::Picked),
            "Color scheme:",
            Some(SELECTIONS[SELECTED].to_string()),
            |delta| {
                let next = SELECTED.checked_add_signed(delta as isize)?;
                (next < SELECTIONS.len()).then_some(Msg::Picked(next))
            },
        )
        .into();

        let id = element_id(&mut el).expect("the wrapper reports an id of its own");
        assert_eq!(focusables(&mut el), vec![Some(id.clone())]);

        let nodes = published(&mut el);
        assert_eq!(nodes.len(), 1, "{nodes:#?}");
        assert_eq!(nodes[0].role, Role::ComboBox);
        assert_eq!(nodes[0].label.as_deref(), Some("Color scheme:"));
        assert_eq!(nodes[0].value.as_deref(), Some("dark"));

        let (mut tree, node) = built(&mut el);
        tab_to(&mut el, &mut tree, &node);
        let mut messages = Vec::new();
        let dispatched = dispatch(
            &mut el,
            &mut tree,
            &node,
            &pressed(keyboard::Key::Named(key::Named::ArrowDown)),
            &mut messages,
        );
        assert_eq!(
            dispatched.messages,
            vec![Msg::Picked(1)],
            "Down must select the next choice through the caller's own selection \
             function, or the keyboard path and the pointer path could disagree \
             about what a choice means"
        );
        assert!(dispatched.captured);

        // Up is the end of the list here: nothing to select, and nothing taken.
        let dispatched = dispatch(
            &mut el,
            &mut tree,
            &node,
            &pressed(keyboard::Key::Named(key::Named::ArrowUp)),
            &mut messages,
        );
        assert!(
            dispatched.messages.is_empty(),
            "a step past the end must publish nothing; otherwise the control \
             swallows an arrow key it cannot act on"
        );
        assert!(
            !dispatched.captured,
            "an arrow the dropdown cannot use must stay available to the rest of \
             the window"
        );
    }

    /// An assistive technology's `Action::Click` on the node activates the
    /// control, and one aimed at a different node does not.
    ///
    /// The id comparison is the point of the second half: `conversion::a11y`
    /// rebuilds the id from the node's *number*
    /// (`iced/winit/src/conversion.rs:1666-1674`), so the request that arrives
    /// carries a `Unique` — the wrapper has to match that back to itself, and
    /// must not answer for a node that is not its own.
    #[test]
    fn an_accesskit_click_activates_only_its_own_node() {
        let mut el = toggled_widget(true);
        let id = element_id(&mut el).expect("an id");
        let (mut tree, node) = built(&mut el);

        let request = |target: &Id| {
            Event::A11y(
                target.clone(),
                iced_accessibility::accesskit::ActionRequest {
                    action: Action::Click,
                    target_node: iced_accessibility::accesskit::NodeId(0),
                    target_tree: iced_accessibility::accesskit::TreeId::ROOT,
                    data: None,
                },
            )
        };

        let mut messages = Vec::new();
        let dispatched = dispatch(&mut el, &mut tree, &node, &request(&id), &mut messages);
        assert_eq!(dispatched.messages, vec![Msg::Toggled(false)]);
        assert!(dispatched.captured);

        let other = Id::unique();
        assert_ne!(other, id);
        let mut messages = Vec::new();
        let dispatched = dispatch(&mut el, &mut tree, &node, &request(&other), &mut messages);
        assert!(
            dispatched.messages.is_empty(),
            "a request for another node must fall through to the inner widget, \
             not be answered by this one"
        );
        assert!(!dispatched.captured);
    }

    /// The id a control is given is derived from its name, so it survives the
    /// frame that built it — and two names do not collide.
    ///
    /// [`stable_id`] is the reason: an `Id::unique()` per construction — what the
    /// toolkit does (`src/widget/button/widget.rs:63`) — would give the same
    /// control a new node id every frame, and an incoming `ActionRequest` (built
    /// from the node's number, see `an_accesskit_click_activates_only_its_own_node`)
    /// could never match it.
    #[test]
    fn a_controls_id_is_its_name_and_is_stable() {
        assert_eq!(stable_id("Name"), stable_id("Name"));
        assert_ne!(stable_id("Name"), stable_id("Runner"));

        let first = element_id(&mut toggled_widget(true));
        let second = element_id(&mut toggled_widget(false));
        assert_eq!(
            first, second,
            "two frames of the same control must report the same id, or \
             assistive technology sees a new node every frame"
        );
    }
}
