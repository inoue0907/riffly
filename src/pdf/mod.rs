//! A PDF tab: the pages as they are printed, read-only, with comments
//! pinned to places on them.
//!
//! The PDF itself is never written. Comments are kept beside it in
//! `<file>.pdf.riffly-comments.json`, saved as they change, so a review
//! travels with the file it is about.

pub mod render;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Image, ImageFormat, InteractiveElement as _, IntoElement, KeyBinding, MouseButton,
    MouseDownEvent, ParentElement as _, Pixels, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, actions, canvas, div, img,
    px,
};
use serde::{Deserialize, Serialize};

use crate::editor::actions as editor_actions;
use crate::editor::theme::{ActiveEditorTheme, ZoomIn, ZoomOut, ZoomReset};
use crate::editor::ui;

pub const CONTEXT: &str = "PdfView";

actions!(
    pdf,
    [NextPage, PreviousPage, FirstPage, LastPage, CancelPdf, PlaceComment]
);

/// Keys of the PDF tab. The comment keys are the editor's own actions, so
/// a comment is added, walked, resolved and deleted with the same keys in
/// a document and in a PDF.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("pagedown", NextPage, Some(CONTEXT)),
        KeyBinding::new("space", NextPage, Some(CONTEXT)),
        KeyBinding::new("pageup", PreviousPage, Some(CONTEXT)),
        KeyBinding::new("shift-space", PreviousPage, Some(CONTEXT)),
        KeyBinding::new("home", FirstPage, Some(CONTEXT)),
        KeyBinding::new("end", LastPage, Some(CONTEXT)),
        KeyBinding::new("escape", CancelPdf, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-m", PlaceComment, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-m", PlaceComment, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-n", editor_actions::NextComment, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-p", editor_actions::PreviousComment, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-r", editor_actions::ResolveComment, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-shift-m", editor_actions::DeleteComment, Some(CONTEXT)),
    ]);
}

/// Width of a page at 100%, in logical pixels.
const PAGE_WIDTH: f32 = 820.;
const ZOOM_STEPS: [f32; 9] = [0.5, 0.67, 0.8, 0.9, 1.0, 1.1, 1.25, 1.5, 2.0];
const AUTHOR: &str = "You";

/// Emitted when the comments change, so the shell can redraw its lists.
pub struct CommentsChanged;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub author: String,
    pub body: String,
}

/// A comment pinned to a point on a page; `x` and `y` are fractions of the
/// page's width and height, so the pin stays put at any zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PdfComment {
    pub id: u64,
    pub page: usize,
    pub x: f32,
    pub y: f32,
    #[serde(default)]
    pub notes: Vec<Note>,
    #[serde(default)]
    pub resolved: bool,
}

#[derive(Default, Serialize, Deserialize)]
struct Sidecar {
    #[serde(default)]
    comments: Vec<PdfComment>,
}

/// Where the comments on `pdf` are kept.
pub fn sidecar_path(pdf: &Path) -> PathBuf {
    let name = pdf
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "document.pdf".into());
    pdf.with_file_name(format!("{name}.riffly-comments.json"))
}

pub fn load_comments(pdf: &Path) -> Vec<PdfComment> {
    std::fs::read_to_string(sidecar_path(pdf))
        .ok()
        .and_then(|text| serde_json::from_str::<Sidecar>(&text).ok())
        .map(|sidecar| sidecar.comments)
        .unwrap_or_default()
}

/// Write the comments beside the PDF; with none left, the file goes away.
pub fn save_comments(pdf: &Path, comments: &[PdfComment]) -> std::io::Result<()> {
    let path = sidecar_path(pdf);
    let kept: Vec<PdfComment> = comments.iter().filter(|c| !c.notes.is_empty()).cloned().collect();
    if kept.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    let text = serde_json::to_string_pretty(&Sidecar { comments: kept })?;
    std::fs::write(path, text)
}

struct Page {
    /// Width and height; only the shape is used.
    size: (f32, f32),
    image: Option<Arc<Image>>,
    /// A drawing of this page has been asked for and not dropped since.
    requested: bool,
}

/// Pages drawn either side of the ones on screen, so scrolling finds them
/// ready; pages further away than `KEEP_PAGES` let go of their pictures.
const AHEAD_PAGES: usize = 2;
const KEEP_PAGES: usize = 6;

struct Draft {
    comment: u64,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct PdfView {
    path: PathBuf,
    pages: Vec<Page>,
    loading: bool,
    error: Option<SharedString>,
    zoom: f32,
    scroll: ScrollHandle,
    focus_handle: FocusHandle,
    comments: Vec<PdfComment>,
    next_id: u64,
    /// The next press on a page drops a comment pin there.
    placing: bool,
    open_comment: Option<u64>,
    draft: Option<Draft>,
    /// Where each page landed on screen, refreshed every frame.
    page_bounds: Vec<Option<Bounds<Pixels>>>,
    save_error: Option<SharedString>,
    document: Option<render::Document>,
}

impl EventEmitter<CommentsChanged> for PdfView {}

impl Focusable for PdfView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PdfView {
    /// Open `path` and start drawing its pages in the background.
    pub fn open(path: PathBuf, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let comments = load_comments(&path);
        let next_id = comments.iter().map(|c| c.id).max().unwrap_or(0) + 1;
        let this = Self {
            path: path.clone(),
            pages: Vec::new(),
            loading: true,
            error: None,
            zoom: 1.0,
            scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            comments,
            next_id,
            placing: false,
            open_comment: None,
            draft: None,
            page_bounds: Vec::new(),
            save_error: None,
            document: None,
        };

        cx.spawn_in(window, async move |this, cx| {
            let opened = cx
                .background_spawn({
                    let path = path.clone();
                    async move { render::open(&path) }
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match opened {
                    Ok((document, sizes)) => {
                        this.pages = sizes
                            .into_iter()
                            .map(|size| Page { size, image: None, requested: false })
                            .collect();
                        this.document = Some(document);
                        this.request_visible(cx);
                    }
                    Err(error) => this.error = Some(format!("{error:#}").into()),
                }
                cx.notify();
            });
        })
        .detach();

        this
    }

    /// Ask for drawings of the pages on screen and the few around them,
    /// and let go of the pictures of pages far away.
    fn request_visible(&mut self, cx: &mut Context<Self>) {
        let Some(document) = self.document.clone() else { return };
        if self.pages.is_empty() {
            return;
        }
        let viewport = self.scroll.bounds();
        let visible: Vec<usize> = self
            .page_bounds
            .iter()
            .enumerate()
            .filter_map(|(ix, bounds)| bounds.filter(|b| b.intersects(&viewport)).map(|_| ix))
            .collect();
        let (first, last) = match (visible.first(), visible.last()) {
            (Some(first), Some(last)) => (*first, *last),
            // Nothing laid out yet: the first pages.
            _ => (0, 0),
        };
        let from = first.saturating_sub(AHEAD_PAGES);
        let to = (last + AHEAD_PAGES).min(self.pages.len() - 1);

        for (ix, page) in self.pages.iter_mut().enumerate() {
            let far = ix + KEEP_PAGES < first || ix > last + KEEP_PAGES;
            if far && page.requested {
                page.image = None;
                page.requested = false;
            }
        }
        for (ix, page) in self.pages.iter_mut().enumerate().take(to + 1).skip(from) {
            if page.requested {
                continue;
            }
            page.requested = true;
            let document = document.clone();
            cx.spawn(async move |this, cx| {
                let png = cx
                    .background_spawn(async move { document.render(ix, render::RENDER_WIDTH) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    let Some(page) = this.pages.get_mut(ix) else { return };
                    if !page.requested {
                        return; // scrolled far away meanwhile
                    }
                    match png {
                        Ok(bytes) => {
                            page.image = Some(Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
                        }
                        Err(error) => {
                            this.error = Some(format!("Page {}: {error:#}", ix + 1).into())
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    // ------------------------------------------------------------- queries

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    pub fn comments(&self) -> &[PdfComment] {
        &self.comments
    }

    pub fn open_comment(&self) -> Option<u64> {
        self.open_comment
    }

    pub fn is_placing(&self) -> bool {
        self.placing
    }

    /// The page nearest the top of the view.
    pub fn current_page(&self) -> usize {
        let viewport = self.scroll.bounds();
        self.page_bounds
            .iter()
            .enumerate()
            .filter_map(|(ix, bounds)| bounds.map(|b| (ix, b)))
            .find(|(_, bounds)| bounds.bottom() > viewport.top() + px(40.))
            .map(|(ix, _)| ix)
            .unwrap_or(0)
    }

    /// Comments in reading order: by page, then top to bottom.
    fn ordered(&self) -> Vec<&PdfComment> {
        let mut list: Vec<&PdfComment> =
            self.comments.iter().filter(|c| !c.resolved && !c.notes.is_empty()).collect();
        list.sort_by(|a, b| {
            (a.page, a.y, a.x)
                .partial_cmp(&(b.page, b.y, b.x))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        list
    }

    /// Open comments with the number each pin shows.
    pub fn numbered_comments(&self) -> Vec<(usize, PdfComment)> {
        self.ordered().into_iter().cloned().enumerate().map(|(ix, c)| (ix + 1, c)).collect()
    }

    // ---------------------------------------------------------- navigation

    pub fn scroll_to_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if page < self.pages.len() {
            self.scroll.scroll_to_top_of_item(page);
            cx.notify();
        }
    }

    fn step_page(&mut self, delta: isize, cx: &mut Context<Self>) {
        if self.pages.is_empty() {
            return;
        }
        let target = (self.current_page() as isize + delta).clamp(0, self.pages.len() as isize - 1);
        self.scroll_to_page(target as usize, cx);
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: f32, cx: &mut Context<Self>) {
        self.zoom = zoom.clamp(ZOOM_STEPS[0], ZOOM_STEPS[ZOOM_STEPS.len() - 1]);
        cx.notify();
    }

    fn step_zoom(&mut self, step: isize, cx: &mut Context<Self>) {
        let nearest = ZOOM_STEPS
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| (*a - self.zoom).abs().total_cmp(&(*b - self.zoom).abs()))
            .map(|(ix, _)| ix as isize)
            .unwrap_or(4);
        let next = (nearest + step).clamp(0, ZOOM_STEPS.len() as isize - 1) as usize;
        self.set_zoom(ZOOM_STEPS[next], cx);
    }

    // ------------------------------------------------------------ comments

    fn persist(&mut self, cx: &mut Context<Self>) {
        self.save_error = save_comments(&self.path, &self.comments)
            .err()
            .map(|error| format!("Comments could not be saved: {error}").into());
        cx.emit(CommentsChanged);
        cx.notify();
    }

    /// Arm comment placing: the next press on a page drops a pin there.
    pub fn start_placing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_comment(window, cx);
        self.placing = true;
        cx.notify();
    }

    /// Drop a pin at a point on a page (fractions of its size) and open it.
    pub fn add_comment_at(
        &mut self,
        page: usize,
        x: f32,
        y: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        self.close_comment(window, cx);
        self.placing = false;
        let id = self.next_id;
        self.next_id += 1;
        self.comments.push(PdfComment {
            id,
            page,
            x: x.clamp(0., 1.),
            y: y.clamp(0., 1.),
            notes: Vec::new(),
            resolved: false,
        });
        self.show_comment(id, window, cx);
        id
    }

    /// Show a comment's thread with the keyboard in its reply box.
    pub fn show_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(comment) = self.comments.iter().find(|c| c.id == id) else { return };
        let empty = comment.notes.is_empty();
        let page = comment.page;
        if self.open_comment.is_some_and(|open| open != id) {
            self.close_comment(window, cx);
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(if empty { "Add a comment…" } else { "Reply…" })
        });
        let subscription = cx.subscribe_in(&input, window, |this: &mut Self, _, event, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.post(window, cx);
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.open_comment = Some(id);
        self.draft = Some(Draft { comment: id, input, _subscription: subscription });
        // Bring the pin's page into view when it is off screen.
        let on_screen = self
            .page_bounds
            .get(page)
            .copied()
            .flatten()
            .is_some_and(|bounds| bounds.intersects(&self.scroll.bounds()));
        if !on_screen {
            self.scroll.scroll_to_top_of_item(page);
        }
        cx.notify();
    }

    /// Post what is in the reply box.
    pub fn post(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(draft) = self.draft.as_ref() else { return };
        let body = draft.input.read(cx).value().trim().to_string();
        if body.is_empty() {
            return;
        }
        let id = draft.comment;
        let input = draft.input.clone();
        if let Some(comment) = self.comments.iter_mut().find(|c| c.id == id) {
            comment.notes.push(Note { author: AUTHOR.into(), body });
        }
        input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder("Reply…", window, cx);
        });
        self.persist(cx);
    }

    /// Close the open thread; a pin nobody wrote on is dropped.
    pub fn close_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.open_comment.take() else { return };
        self.draft = None;
        let before = self.comments.len();
        self.comments.retain(|c| c.id != id || !c.notes.is_empty());
        if self.comments.len() != before {
            cx.emit(CommentsChanged);
        }
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    /// The comment in view: the open one.
    fn current(&self) -> Option<u64> {
        self.open_comment
    }

    pub fn resolve_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(comment) = self.comments.iter_mut().find(|c| c.id == id) {
            comment.resolved = true;
        }
        if self.open_comment == Some(id) {
            self.open_comment = None;
            self.draft = None;
            window.focus(&self.focus_handle, cx);
        }
        self.persist(cx);
    }

    pub fn delete_comment(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        self.comments.retain(|c| c.id != id);
        if self.open_comment == Some(id) {
            self.open_comment = None;
            self.draft = None;
            window.focus(&self.focus_handle, cx);
        }
        self.persist(cx);
    }

    /// Open the next (or previous) comment in reading order, wrapping.
    pub fn step_comment(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let order: Vec<u64> = self.ordered().iter().map(|c| c.id).collect();
        if order.is_empty() {
            return;
        }
        let at = self.open_comment.and_then(|id| order.iter().position(|o| *o == id));
        let next = match (at, forward) {
            (Some(ix), true) => (ix + 1) % order.len(),
            (Some(ix), false) => (ix + order.len() - 1) % order.len(),
            (None, true) => 0,
            (None, false) => order.len() - 1,
        };
        self.show_comment(order[next], window, cx);
    }

    fn on_page_press(
        &mut self,
        page: usize,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        // A double-click comments right there; so does a press while placing.
        if !(self.placing || event.click_count >= 2) {
            if self.open_comment.is_some() {
                self.close_comment(window, cx);
            }
            return;
        }
        let Some(bounds) = self.page_bounds.get(page).copied().flatten() else { return };
        let x = f32::from(event.position.x - bounds.left()) / f32::from(bounds.size.width);
        let y = f32::from(event.position.y - bounds.top()) / f32::from(bounds.size.height);
        self.add_comment_at(page, x, y, window, cx);
        cx.stop_propagation();
    }

    // -------------------------------------------------------------- render

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.editor_theme().clone();
        let colors = cx.theme().clone();
        let pages = if self.pages.is_empty() {
            "".to_string()
        } else {
            format!("{} pages", self.pages.len())
        };
        h_flex()
            .flex_none()
            .h(theme.rems(2.25))
            .px(theme.rems(0.75))
            .gap(theme.rems(0.5))
            .border_b_1()
            .border_color(colors.border)
            .bg(colors.background)
            .text_size(theme.ui_text_size)
            .text_color(colors.muted_foreground)
            .child(div().child(pages))
            .child(div().flex_1())
            .children(self.save_error.clone().map(|e| div().text_color(colors.danger).child(e)))
            .when(self.placing, |this| {
                this.child(div().text_color(colors.foreground).child("Click a page to place the comment · Esc to cancel"))
            })
            .child(
                Button::new("pdf-comment")
                    .ghost()
                    .small()
                    .label("Comment")
                    .on_click(cx.listener(|this, _, window, cx| this.start_placing(window, cx))),
            )
            .child(
                Button::new("pdf-zoom-out")
                    .ghost()
                    .small()
                    .label("−")
                    .on_click(cx.listener(|this, _, _, cx| this.step_zoom(-1, cx))),
            )
            .child(div().min_w(theme.rems(3.)).flex().justify_center().child(format!("{}%", (self.zoom * 100.).round())))
            .child(
                Button::new("pdf-zoom-in")
                    .ghost()
                    .small()
                    .label("+")
                    .on_click(cx.listener(|this, _, _, cx| this.step_zoom(1, cx))),
            )
    }

    fn render_thread(&self, comment: &PdfComment, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.draft.as_ref().filter(|d| d.comment == comment.id)?;
        let theme = cx.editor_theme().clone();
        let id = comment.id;
        let notes = comment.notes.iter().map(|note| {
            v_flex()
                .gap(theme.rems(0.125))
                .child(
                    div()
                        .text_size(theme.rems(0.75))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .child(note.author.clone()),
                )
                .child(div().text_size(theme.rems(0.8125)).child(note.body.clone()))
        });
        Some(
            ui::popover_surface(cx)
                .id(("pdf-thread", id as usize))
                .w(theme.rems(18.75))
                .flex()
                .flex_col()
                .gap(theme.rems(0.5))
                .p(theme.rems(0.75))
                .text_color(cx.theme().popover_foreground)
                // Presses inside the card stay inside it.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(notes)
                .child(Input::new(&draft.input).id("pdf-comment-input").small())
                .child(
                    h_flex()
                        .justify_between()
                        .gap(theme.rems(0.375))
                        .child(
                            h_flex()
                                .gap(theme.rems(0.25))
                                .when(!comment.notes.is_empty(), |this| {
                                    this.child(
                                        Button::new("pdf-resolve")
                                            .ghost()
                                            .small()
                                            .label("Resolve")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.resolve_comment(id, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("pdf-delete")
                                            .ghost()
                                            .small()
                                            .label("Delete")
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.delete_comment(id, window, cx)
                                            })),
                                    )
                                }),
                        )
                        .child(
                            Button::new("pdf-post")
                                .primary()
                                .small()
                                .label("Comment")
                                .on_click(cx.listener(|this, _, window, cx| this.post(window, cx))),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_page(&self, ix: usize, numbers: &[(usize, PdfComment)], cx: &mut Context<Self>) -> AnyElement {
        let page = &self.pages[ix];
        let width = px(PAGE_WIDTH * self.zoom);
        let height = width * (page.size.1 / page.size.0.max(1.));
        let colors = cx.theme().clone();
        let entity = cx.entity().downgrade();

        let probe = canvas(
            move |bounds, _, cx| {
                let _ = entity.update(cx, |this, cx| {
                    if this.page_bounds.len() <= ix {
                        this.page_bounds.resize(ix + 1, None);
                    }
                    // A page that moved may have come into view: look again.
                    if this.page_bounds[ix] != Some(bounds) {
                        this.page_bounds[ix] = Some(bounds);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let pins = numbers.iter().filter(|(_, c)| c.page == ix).map(|(number, comment)| {
            let id = comment.id;
            let open = self.open_comment == Some(id);
            div()
                .id(("pin", id as usize))
                .absolute()
                .left(width * comment.x - px(12.))
                .top(height * comment.y - px(12.))
                .size(px(24.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(12.))
                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                .bg(if open { colors.primary } else { gpui_kit::rgb(0xF2C94C).into() })
                .text_color(if open { colors.primary_foreground } else { gpui_kit::rgb(0x37352F).into() })
                .border_2()
                .border_color(gpui_kit::white())
                .shadow_md()
                .cursor_pointer()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| this.show_comment(id, window, cx)))
                .child(number.to_string())
        }).collect::<Vec<_>>();

        // A pin being placed has no number yet; show it as an empty dot.
        let new_pin = self
            .open_comment
            .and_then(|id| self.comments.iter().find(|c| c.id == id && c.page == ix && c.notes.is_empty()))
            .map(|comment| {
                div()
                    .absolute()
                    .left(width * comment.x - px(8.))
                    .top(height * comment.y - px(8.))
                    .size(px(16.))
                    .rounded_full()
                    .bg(colors.primary)
                    .border_2()
                    .border_color(gpui_kit::white())
            });

        let thread = self
            .open_comment
            .and_then(|id| self.comments.iter().find(|c| c.id == id && c.page == ix))
            .and_then(|comment| {
                let card = self.render_thread(comment, cx)?;
                // Beside the pin, kept inside the page horizontally.
                let left = (width * comment.x + px(16.)).min(width - px(300.)).max(px(0.));
                Some(div().absolute().left(left).top(height * comment.y + px(16.)).child(card))
            });

        div()
            .id(("pdf-page", ix))
            .relative()
            .flex_none()
            .w(width)
            .h(height)
            .bg(gpui_kit::white())
            .border_1()
            .border_color(colors.border)
            .shadow_sm()
            .when(self.placing, |this| this.cursor_crosshair())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.on_page_press(ix, event, window, cx)
                }),
            )
            .child(probe)
            .child(match page.image.clone() {
                Some(image) => img(image).size_full().into_any_element(),
                None => div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(gpui_kit::rgb(0x9B9A97))
                    .child(format!("Page {}", ix + 1))
                    .into_any_element(),
            })
            .children(pins)
            .children(new_pin)
            .children(thread)
            .into_any_element()
    }
}

impl Render for PdfView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Last frame's layout says which pages are on screen now.
        self.request_visible(cx);
        let colors = cx.theme().clone();
        let theme = cx.editor_theme().clone();
        let numbers = self.numbered_comments();
        let pages: Vec<AnyElement> = (0..self.pages.len()).map(|ix| self.render_page(ix, &numbers, cx)).collect();

        let body: AnyElement = if let (Some(error), true) = (self.error.clone(), self.pages.is_empty()) {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(colors.danger)
                .child(error)
                .into_any_element()
        } else if self.pages.is_empty() {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(colors.muted_foreground)
                .child("Opening…")
                .into_any_element()
        } else {
            v_flex()
                .id("pdf-pages")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .on_scroll_wheel(cx.listener(|_, _, _, cx| cx.notify()))
                .items_center()
                .py(theme.rems(1.5))
                .gap(theme.rems(1.5))
                .children(pages)
                .into_any_element()
        };

        v_flex()
            .id("pdf")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(colors.muted)
            .on_action(cx.listener(|this, _: &NextPage, _, cx| this.step_page(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousPage, _, cx| this.step_page(-1, cx)))
            .on_action(cx.listener(|this, _: &FirstPage, _, cx| this.scroll_to_page(0, cx)))
            .on_action(cx.listener(|this, _: &LastPage, _, cx| {
                let last = this.pages.len().saturating_sub(1);
                this.scroll_to_page(last, cx)
            }))
            .on_action(cx.listener(|this, _: &CancelPdf, window, cx| {
                if this.placing {
                    this.placing = false;
                    cx.notify();
                } else {
                    this.close_comment(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &PlaceComment, window, cx| this.start_placing(window, cx)))
            .on_action(cx.listener(|this, _: &editor_actions::NextComment, window, cx| {
                this.step_comment(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &editor_actions::PreviousComment, window, cx| {
                this.step_comment(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &editor_actions::ResolveComment, window, cx| {
                if let Some(id) = this.current() {
                    this.resolve_comment(id, window, cx)
                }
            }))
            .on_action(cx.listener(|this, _: &editor_actions::DeleteComment, window, cx| {
                if let Some(id) = this.current() {
                    this.delete_comment(id, window, cx)
                }
            }))
            // The app's zoom keys zoom the pages here, not the text size.
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.step_zoom(1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.step_zoom(-1, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.set_zoom(1.0, cx)))
            .child(self.render_toolbar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_round_trip_beside_the_pdf() {
        let dir = std::env::temp_dir().join(format!("riffly-pdf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let pdf = dir.join("report.pdf");
        assert!(load_comments(&pdf).is_empty());

        let comments = vec![
            PdfComment {
                id: 1,
                page: 2,
                x: 0.25,
                y: 0.5,
                notes: vec![Note { author: "You".into(), body: "check this".into() }],
                resolved: false,
            },
            // An empty pin (still being written) is not kept.
            PdfComment { id: 2, page: 0, x: 0.1, y: 0.1, notes: vec![], resolved: false },
        ];
        save_comments(&pdf, &comments).unwrap();
        assert!(sidecar_path(&pdf).ends_with("report.pdf.riffly-comments.json"));
        assert_eq!(load_comments(&pdf), vec![comments[0].clone()]);

        // With nothing left to keep, the file goes away.
        save_comments(&pdf, &[]).unwrap();
        assert!(!sidecar_path(&pdf).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
