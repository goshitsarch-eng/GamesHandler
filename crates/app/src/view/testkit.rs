//! Test-only helpers shared by the view modules.
//!
//! **Why this module exists** (`ARCH-14`). Six modules grew their own
//! `drawn_strings` — a helper that lays a built widget tree out and walks it
//! with an [`Operation`] to read back every string the page actually hands the
//! framework — and two of those copies carried a comment saying a third copy
//! was the point at which the shared helper became warranted. The third copy
//! had already landed when the comment was written, and by the time this module
//! was created there were six: `view::library`, `view::settings`,
//! `view::credits`, `view::runners`, `view::installers` and `main`. The note
//! stated a rule whose own trigger had fired, which left the next reader free to
//! either trust the count and add a seventh or follow the rule and do a refactor
//! they had been told was unnecessary. Both readings were wrong, and the copies
//! had begun to drift — `view::credits` needs `Element<'static, Message>` and
//! `view::settings` needs its `mut` in a different place — so "the same 27
//! lines" was no longer quite true of all six.
//!
//! **Why the traversal records ids as well as text.** The six copies collected
//! only strings, because that is all their assertions wanted. Three things the
//! audit later needed to assert — that a control is *absent*, that a control is
//! *present but unlabelled*, and that a control sits inside a given parent —
//! cannot be said with strings alone, and the third needs the bounds. Rather
//! than add a seventh helper for those, the traversal reports everything the
//! framework offers and lets each test read the part it wants.
//!
//! Nothing here is compiled into the binary: the module is `#[cfg(test)]` at
//! its declaration in [`super`], so the release build has no test surface.

use cosmic::iced::advanced::Layout;
use cosmic::iced::advanced::layout::Limits;
use cosmic::iced::advanced::widget::Operation;
use cosmic::iced::advanced::widget::Tree;
use cosmic::iced::advanced::widget::operation::Focusable;
use cosmic::iced::{Font, Pixels, Rectangle, Size};
use cosmic::widget::Id;

/// One thing the framework reports about the widget tree.
///
/// Exactly one of `id` and `text` is set: the framework reports a container or
/// a focusable with its id, and a string with its text, and never both for the
/// same node.
#[derive(Debug, Clone)]
pub(crate) struct Seen {
    pub(crate) id: Option<Id>,
    pub(crate) bounds: Rectangle,
    pub(crate) text: Option<String>,
}

/// Collects what a traversal reports. `traverse` calls `operate(self)` so the
/// widgets keep descending — the contract [`Operation`] documents.
#[derive(Default)]
pub(crate) struct Collect(pub(crate) Vec<Seen>);

impl Operation for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.0.push(Seen {
            id: id.cloned(),
            bounds,
            text: None,
        });
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.0.push(Seen {
            id: None,
            bounds,
            text: Some(text.to_string()),
        });
    }

    /// Where a `cosmic::widget::button` reports its [`Id`].
    ///
    /// Not in [`Self::container`]: the button's `operate` calls
    /// `operation.container(None, layout.bounds())` — always `None` — and then
    /// `operation.focusable(Some(&self.id), …)`
    /// (`libcosmic src/widget/button/widget.rs:345` and `:359`). So a test that
    /// read only `container` would find a button's id nowhere and conclude the
    /// control had none, which is the "check that cannot see the thing it
    /// checks" shape. Recorded here rather than asserted against a button built
    /// in the test, so the id is read off the same traversal that reads the
    /// rest of the tree.
    fn focusable(&mut self, id: Option<&Id>, bounds: Rectangle, _state: &mut dyn Focusable) {
        self.0.push(Seen {
            id: id.cloned(),
            bounds,
            text: None,
        });
    }
}

/// A real renderer, for measuring text.
///
/// `iced_tiny_skia` is a pure-software backend, so this needs no display and
/// draws nothing — `layout` wants it only to ask the font stack how wide a
/// string is.
pub(crate) fn renderer() -> cosmic::Renderer {
    cosmic::Renderer::new(Font::default(), Pixels(16.0))
}

/// Lay `el` out and report everything the framework knows about its tree.
///
/// The element is taken by `&mut` and the caller keeps ownership, so a test can
/// traverse the same tree twice — once to assert, once to inspect — without
/// rebuilding it. `view::widgets` needs that for its width-dependent tests.
pub(crate) fn traversal<M: Clone + 'static>(el: &mut cosmic::Element<'_, M>) -> Vec<Seen> {
    traversal_with(el, Size::new(f32::INFINITY, f32::INFINITY))
}

/// The traversal itself, at the given size limit.
///
/// Private: the two public shapes above are the ones with meaning — `INFINITY`
/// ("lay it out as one line") and a real width ("lay it out as it will be
/// drawn") — and a caller reaching past them is a caller who wants a third
/// meaning that should be named.
fn traversal_with<M: Clone + 'static>(el: &mut cosmic::Element<'_, M>, max: Size) -> Vec<Seen> {
    let renderer = renderer();
    let mut tree = Tree::new(el.as_widget());
    let limits = Limits::new(Size::ZERO, max);
    // `Widget::layout` and `Widget::operate` both take the renderer by shared
    // reference — a widget measures text through it and does not draw — so
    // neither needs a `&mut` here and clippy says so.
    let node = el.as_widget_mut().layout(&mut tree, &renderer, &limits);
    let mut collect = Collect::default();
    el.as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
    collect.0
}

/// [`traversal`] at a bounded width, which is the only way a wrap is visible.
///
/// [`traversal`] passes `f32::INFINITY` as the limit, so nothing in it can
/// wrap: a name long enough to need two lines lays out as one long line and
/// every assertion about wrapping silently passes. This is the same traversal
/// with a real width, and the difference is exactly the difference between a
/// test that can see a truncated label and one that cannot — which is why it
/// lives beside `traversal` rather than in the one module that first needed it.
pub(crate) fn traversal_at_width<M: Clone + 'static>(
    el: &mut cosmic::Element<'_, M>,
    width: f32,
) -> Vec<Seen> {
    traversal_with(el, Size::new(width, f32::INFINITY))
}

/// [`traversal`] inside a real window — both axes bounded.
///
/// Neither public shape above can see the fold: an `INFINITY` height lets a
/// scrollable's content lay out at full length, so "below the fold" is a
/// position only a bounded height can produce. UX-30's assertion — the form's
/// action row inside the viewport while a mid-form control is beyond it —
/// needs exactly this.
pub(crate) fn traversal_at_size<M: Clone + 'static>(
    el: &mut cosmic::Element<'_, M>,
    max: Size,
) -> Vec<Seen> {
    traversal_with(el, max)
}

/// The ids the traversal reported, in the order it reported them.
pub(crate) fn ids(seen: &[Seen]) -> Vec<Id> {
    seen.iter().filter_map(|seen| seen.id.clone()).collect()
}

/// The strings the traversal was handed, in order.
pub(crate) fn texts(seen: &[Seen]) -> Vec<&str> {
    seen.iter()
        .filter_map(|seen| seen.text.as_deref())
        .collect()
}

/// Every string a built element hands the framework, in drawing order.
///
/// This is the one function the six former copies were each reimplementing.
/// Taking the element by value keeps the call sites that had `mut element` and
/// those that did not reading the same way.
pub(crate) fn drawn_strings<M: Clone + 'static>(
    mut element: cosmic::Element<'_, M>,
) -> Vec<String> {
    traversal(&mut element)
        .into_iter()
        .filter_map(|seen| seen.text)
        .collect()
}

/// The height `needle` is drawn at inside `el` — the level a heading renders
/// at, rather than the constructor a source scan would read.
///
/// UX-22's assertion shape: the finding was two heading *levels* doing one
/// job, and a test that greps the call sites for `title3`/`title4` cannot see
/// the level — only the measurement can. Compare against the height a bare
/// [`cosmic::widget::text`] of the wanted level gives the same string.
///
/// The traversal reports one entry per drawn string, so a `needle` drawn
/// twice answers its first bounds — fine for the headings this is for, and
/// the reason the `expect` names the string rather than the page.
pub(crate) fn text_height<M: Clone + 'static>(
    el: &mut cosmic::Element<'_, M>,
    needle: &str,
) -> f32 {
    traversal(el)
        .iter()
        .find(|seen| seen.text.as_deref() == Some(needle))
        .map(|seen| seen.bounds.height)
        .unwrap_or_else(|| panic!("`{needle}` is not drawn in this tree"))
}
