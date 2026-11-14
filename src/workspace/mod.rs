//! The application shell around the editor: panes of tabs side by side, and
//! a sidebar with the active document's outline and the files opened lately.
//!
//! Documents are markdown files (see [`crate::editor::markdown`]). A tab
//! knows the file it came from and whether it has changed since it was last
//! written, and closing a changed tab — or the window — asks first.
//!
//! The window holds one or more panes in a row. Each pane has its own tabs
//! and shows one of them; a tab lives in exactly one pane, and dragging it
//! moves it, onto another pane's tabs or to a pane's right edge to split.

pub mod recent;
pub mod settings;

use std::path::{Path, PathBuf};

use gpui_kit::component::{ActiveTheme, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, KeyBinding, MouseButton, ParentElement as _,
    PathPromptOptions, PromptLevel, Render, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, actions, div,
};

use crate::editor::block::{BlockContent, BlockId};
use crate::editor::theme::{ActiveEditorTheme, ToggleAppearance};
use crate::editor::ui::icon;
use crate::editor::view::DocumentChanged;
use crate::editor::{RifflyEditor, html, markdown};
use crate::pdf::{CommentsChanged, PdfView};
use recent::RecentFiles;
use settings::{Appearance, Settings};

pub const CONTEXT: &str = "Workspace";

actions!(
    workspace,
    [
        NewTab,
        OpenFile,
        SaveFile,
        SaveFileAs,
        CloseTab,
        NextTab,
        PreviousTab,
        ToggleSidebar,
        SplitRight,
        ClosePane,
        FocusNextPane,
        FocusPreviousPane,
        MoveTabToNextPane,
        MoveTabToPreviousPane,
        MoveTabForward,
        MoveTabBackward,
        WidenPane,
        NarrowPane,
        EqualizePanes,
        FocusOutline,
        FocusRecent,
        ToggleShortcuts,
        SidebarDown,
        SidebarUp,
        SidebarConfirm,
        SidebarCancel,
        SidebarRemove,
        ClearRecent,
    ]
);

/// Bring the n-th tab of the active pane forward; past the end means the last.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, gpui_kit::Action)]
#[action(namespace = workspace, no_json)]
pub struct ActivateTab(pub usize);

/// Key context of the sidebar while it has the keyboard.
pub const SIDEBAR_CONTEXT: &str = "WorkspaceSidebar";

/// Every shortcut the application has, grouped as the cheat sheet (F1)
/// shows them. The workspace's own are bound in [`init`]; the editor's live
/// in `editor::actions` and `editor::theme`.
pub const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Files",
        &[
            ("Ctrl+N / Ctrl+T", "New tab"),
            ("Ctrl+O", "Open a file (.md, .html, .pdf)"),
            ("Ctrl+S", "Save"),
            ("Ctrl+Alt+S", "Save as"),
            ("Ctrl+W", "Close tab"),
        ],
    ),
    (
        "Tabs",
        &[
            ("Ctrl+Tab / Ctrl+PageDown", "Next tab"),
            ("Ctrl+Shift+Tab / Ctrl+PageUp", "Previous tab"),
            ("Ctrl+1 … Ctrl+8", "Go to tab 1–8"),
            ("Ctrl+9", "Go to the last tab"),
            ("Ctrl+Shift+PageDown", "Move tab right within the pane"),
            ("Ctrl+Shift+PageUp", "Move tab left within the pane"),
        ],
    ),
    (
        "Panes",
        &[
            ("Ctrl+\\", "Split right"),
            ("Ctrl+Shift+W", "Close pane"),
            ("Ctrl+Alt+→ / ←", "Next / previous pane"),
            ("Ctrl+Alt+Shift+→ / ←", "Move tab to the next / previous pane"),
            ("Ctrl+Alt+= / Ctrl+Alt+-", "Widen / narrow the pane"),
            ("Ctrl+Alt+E", "Make all panes equal"),
            ("Drag a divider", "Resize panes (double-click to equalize)"),
        ],
    ),
    (
        "Sidebar",
        &[
            ("Ctrl+Alt+B", "Show / hide the sidebar"),
            ("Ctrl+Shift+O", "Go to the outline"),
            ("Ctrl+Shift+R", "Go to recent files"),
            ("↑ / ↓, Enter", "Move and open, in the sidebar"),
            ("Delete", "Forget the selected recent file"),
            ("Shift+Delete", "Clear recent files"),
            ("Esc", "Back to the document"),
        ],
    ),
    (
        "View",
        &[
            ("Ctrl+Shift+L", "Light / dark mode"),
            ("Ctrl+= / Ctrl+-", "Zoom in / out"),
            ("Ctrl+0", "Reset zoom"),
            ("F1", "Show / hide this list"),
        ],
    ),
    (
        "Editing",
        &[
            ("/", "Block menu"),
            ("Ctrl+B / I / U / E", "Bold, italic, underline, code"),
            ("Ctrl+Shift+S", "Strikethrough"),
            ("Ctrl+Shift+H", "Highlight"),
            ("Ctrl+Shift+K", "Link"),
            ("Ctrl+R", "Clear formatting"),
            ("Ctrl+Alt+0 … 3", "Paragraph, heading 1–3 (in text)"),
            ("Ctrl+Shift+7 / 8 / 9", "Numbered, bullet, to-do list"),
            ("Ctrl+Shift+B", "Quote"),
            ("Ctrl+Alt+C", "Code block"),
            ("Ctrl+D", "Duplicate block"),
            ("Ctrl+Shift+↑ / ↓", "Move block up / down"),
            ("Ctrl+Shift+Backspace", "Delete block"),
            ("Ctrl+Shift+C", "Copy block"),
            ("Ctrl+Alt+H", "Divider"),
            ("Ctrl+Enter", "Tick a to-do / open a toggle"),
            ("Ctrl+. / Ctrl+,", "Superscript / subscript"),
            ("Ctrl+Shift+E / Ctrl+Shift+2", "Emoji / mention"),
            ("Ctrl+Shift+I", "Image"),
            ("Esc", "Select the block"),
            ("Ctrl+Z / Ctrl+Shift+Z", "Undo / redo"),
        ],
    ),
    (
        "Comments",
        &[
            ("Ctrl+Shift+M", "Comment on the selection"),
            ("Ctrl+Alt+M", "Open the comment here (or start one)"),
            ("Enter", "Post the comment or reply"),
            ("Ctrl+Alt+N / Ctrl+Alt+P", "Next / previous comment"),
            ("Ctrl+Alt+R", "Resolve the comment"),
            ("Ctrl+Alt+Shift+M", "Delete the comment"),
            ("Esc", "Close the comment"),
        ],
    ),
    (
        "PDF",
        &[
            ("Double-click a page", "Comment there"),
            ("Ctrl+Shift+M, then click", "Comment where you click"),
            ("Ctrl+Alt+N / Ctrl+Alt+P", "Next / previous comment"),
            ("Ctrl+Alt+R / Ctrl+Alt+Shift+M", "Resolve / delete the comment"),
            ("PageDown / PageUp", "Next / previous page"),
            ("Home / End", "First / last page"),
            ("Ctrl+= / Ctrl+- / Ctrl+0", "Zoom the pages"),
            ("Esc", "Close the comment"),
        ],
    ),
    (
        "Tables",
        &[
            ("Tab / Shift+Tab", "Next / previous cell"),
            ("Ctrl+Alt+Enter", "Add a row below"),
            ("Ctrl+Alt+Shift+Enter", "Add a row above"),
            ("Ctrl+Alt+] / Ctrl+Alt+[", "Add a column right / left"),
            ("Ctrl+Alt+Backspace", "Delete the row"),
            ("Ctrl+Alt+Shift+Backspace", "Delete the column"),
        ],
    ),
];

/// Key bindings for the shell. They sit outside the editor's own context,
/// so an editor binding for the same keys would win; none of these clash.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-n", NewTab, Some(CONTEXT)),
        KeyBinding::new("secondary-t", NewTab, Some(CONTEXT)),
        KeyBinding::new("secondary-o", OpenFile, Some(CONTEXT)),
        KeyBinding::new("secondary-s", SaveFile, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-s", SaveFileAs, Some(CONTEXT)),
        KeyBinding::new("secondary-w", CloseTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-tab", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-pagedown", NextTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some(CONTEXT)),
        KeyBinding::new("ctrl-pageup", PreviousTab, Some(CONTEXT)),
        KeyBinding::new("secondary-\\", SplitRight, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-w", ClosePane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-b", ToggleSidebar, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-right", FocusNextPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-left", FocusPreviousPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-shift-right", MoveTabToNextPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-shift-left", MoveTabToPreviousPane, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-pagedown", MoveTabForward, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-pageup", MoveTabBackward, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-=", WidenPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-+", WidenPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt--", NarrowPane, Some(CONTEXT)),
        KeyBinding::new("secondary-alt-e", EqualizePanes, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-o", FocusOutline, Some(CONTEXT)),
        KeyBinding::new("secondary-shift-r", FocusRecent, Some(CONTEXT)),
        KeyBinding::new("f1", ToggleShortcuts, Some(CONTEXT)),
        KeyBinding::new("down", SidebarDown, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("up", SidebarUp, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("enter", SidebarConfirm, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("escape", SidebarCancel, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("delete", SidebarRemove, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("shift-delete", ClearRecent, Some(SIDEBAR_CONTEXT)),
        KeyBinding::new("escape", ToggleShortcuts, Some("WorkspaceShortcuts")),
    ]);
    let mut tabs = Vec::new();
    for n in 1..=9usize {
        tabs.push(KeyBinding::new(
            &format!("secondary-{n}"),
            ActivateTab(if n == 9 { usize::MAX } else { n - 1 }),
            Some(CONTEXT),
        ));
    }
    cx.bind_keys(tabs);
}

/// `%APPDATA%\Riffly` on Windows, the XDG config directory elsewhere:
/// where the recent-files list and the settings live.
pub fn default_config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let dir = base.join("Riffly");
    // The app was called gpui-notion before; bring its settings and recent
    // files along the first time Riffly starts.
    let legacy = base.join("gpui-notion");
    if !dir.exists() && legacy.is_dir() {
        let _ = std::fs::rename(&legacy, &dir);
    }
    Some(dir)
}

type TabId = u64;
type PaneId = u64;

/// What a tab shows: a document to edit, or a PDF to read and comment on.
#[derive(Clone)]
enum Content {
    Doc(Entity<RifflyEditor>),
    Pdf(Entity<PdfView>),
}

/// How a document is written to disk, chosen by its file's extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Markdown,
    Html,
}

impl Format {
    pub fn of(path: &Path) -> Self {
        match extension(path).as_deref() {
            Some("html" | "htm" | "xhtml") => Self::Html,
            _ => Self::Markdown,
        }
    }
}

fn extension(path: &Path) -> Option<String> {
    path.extension().map(|ext| ext.to_string_lossy().to_lowercase())
}

fn is_pdf(path: &Path) -> bool {
    extension(path).as_deref() == Some("pdf")
}

struct Tab {
    id: TabId,
    content: Content,
    path: Option<PathBuf>,
    /// Changed since it was opened or last saved.
    dirty: bool,
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    fn editor(&self) -> Option<&Entity<RifflyEditor>> {
        match &self.content {
            Content::Doc(editor) => Some(editor),
            Content::Pdf(_) => None,
        }
    }

    fn pdf(&self) -> Option<&Entity<PdfView>> {
        match &self.content {
            Content::Pdf(pdf) => Some(pdf),
            Content::Doc(_) => None,
        }
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            Content::Doc(editor) => editor.focus_handle(cx),
            Content::Pdf(pdf) => pdf.focus_handle(cx),
        }
    }

    fn icon(&self) -> &'static str {
        match self.content {
            Content::Pdf(_) => "book-open",
            Content::Doc(_) => "file-text",
        }
    }

    fn title(&self) -> SharedString {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().to_string().into())
            .unwrap_or_else(|| "Untitled".into())
    }
}

struct Pane {
    id: PaneId,
    tabs: Vec<Tab>,
    active: usize,
    /// Share of the row's width, the shares of all panes summing to one.
    width: f32,
}

impl Pane {
    fn new(id: PaneId, width: f32) -> Self {
        Self {
            id,
            tabs: Vec::new(),
            active: 0,
            width,
        }
    }

    fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }
}

/// A divider between two panes being dragged; `left` is the pane before it.
#[derive(Clone)]
pub struct DraggedDivider {
    left: PaneId,
}

impl Render for DraggedDivider {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // The divider itself follows the pointer; nothing else needs drawing.
        div()
    }
}

/// No pane is narrower than this share of the row, or this many pixels.
const MIN_PANE_SHARE: f32 = 0.1;
const MIN_PANE_WIDTH: f32 = 180.;
/// How much Widen / Narrow moves a pane's edge.
const PANE_STEP: f32 = 0.05;

/// What the pointer carries while a tab is dragged.
#[derive(Clone)]
pub struct DraggedTab {
    id: TabId,
    title: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.editor_theme();
        h_flex()
            .gap(theme.rems(0.375))
            .px(theme.rems(0.625))
            .py(theme.rems(0.25))
            .rounded(theme.radius)
            .bg(cx.theme().popover)
            .border_1()
            .border_color(cx.theme().border)
            .shadow_md()
            .text_size(theme.ui_text_size)
            .text_color(cx.theme().popover_foreground)
            .child(icon("file-text", theme.rems(0.875), cx.theme().muted_foreground))
            .child(self.title.clone())
    }
}

pub struct Workspace {
    panes: Vec<Pane>,
    active_pane: usize,
    next_id: u64,
    config_dir: Option<PathBuf>,
    recent: RecentFiles,
    settings: Settings,
    show_sidebar: bool,
    /// The last thing that went wrong with a file, shown until the next try.
    error: Option<SharedString>,
    focus_handle: FocusHandle,
    /// A close of the window is going through its questions.
    closing: bool,
    /// Every question was answered; the window may go.
    close_confirmed: bool,
    /// The sidebar's own focus, for moving through it by keyboard.
    sidebar_focus: FocusHandle,
    /// The highlighted sidebar row: outline entries first, then recent files.
    sidebar_cursor: Option<usize>,
    /// The cheat sheet is open.
    show_shortcuts: bool,
    shortcuts_focus: FocusHandle,
    /// A divider is being dragged, so tab drop targets stay hidden.
    resizing: bool,
}

impl Workspace {
    /// An empty workspace keeping its recent files and settings in
    /// `config_dir` (nothing is stored when it is `None`). Call
    /// [`Workspace::add_tab`] or [`Workspace::open_path`] to give it a
    /// document; one with no tabs gets an empty one when it is first drawn.
    pub fn new(config_dir: Option<PathBuf>, cx: &mut Context<Self>) -> Self {
        let recent = RecentFiles::load(config_dir.as_ref().map(|dir| dir.join("recent.json")));
        let settings = Settings::load(config_dir.as_deref());
        Self {
            panes: vec![Pane::new(1, 1.0)],
            active_pane: 0,
            next_id: 2,
            config_dir,
            recent,
            settings,
            show_sidebar: true,
            error: None,
            focus_handle: cx.focus_handle(),
            closing: false,
            close_confirmed: false,
            sidebar_focus: cx.focus_handle(),
            sidebar_cursor: None,
            show_shortcuts: false,
            shortcuts_focus: cx.focus_handle(),
            resizing: false,
        }
    }

    fn fresh_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ------------------------------------------------------------- queries

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    pub fn active_pane_index(&self) -> usize {
        self.active_pane
    }

    /// Each pane's share of the row's width, left to right.
    pub fn pane_widths(&self) -> Vec<f32> {
        self.panes.iter().map(|pane| pane.width).collect()
    }

    pub fn shortcuts_are_shown(&self) -> bool {
        self.show_shortcuts
    }

    pub fn sidebar_cursor(&self) -> Option<usize> {
        self.sidebar_cursor
    }

    /// Tab titles of every pane, left to right.
    pub fn pane_tab_titles(&self) -> Vec<Vec<SharedString>> {
        self.panes
            .iter()
            .map(|pane| pane.tabs.iter().map(Tab::title).collect())
            .collect()
    }

    fn pane(&self) -> &Pane {
        &self.panes[self.active_pane]
    }

    /// Tabs in the active pane.
    pub fn tab_count(&self) -> usize {
        self.pane().tabs.len()
    }

    /// The active pane's shown tab.
    pub fn active_index(&self) -> usize {
        self.pane().active
    }

    pub fn active_editor(&self) -> Option<Entity<RifflyEditor>> {
        self.pane().active_tab().and_then(|tab| tab.editor().cloned())
    }

    /// The PDF the active pane shows, if it shows one.
    pub fn active_pdf(&self) -> Option<Entity<PdfView>> {
        self.pane().active_tab().and_then(|tab| tab.pdf().cloned())
    }

    /// Titles of the active pane's tabs.
    pub fn tab_titles(&self) -> Vec<SharedString> {
        self.pane().tabs.iter().map(Tab::title).collect()
    }

    pub fn tab_path(&self, ix: usize) -> Option<&Path> {
        self.pane().tabs.get(ix).and_then(|tab| tab.path.as_deref())
    }

    pub fn tab_is_dirty(&self, ix: usize) -> bool {
        self.pane().tabs.get(ix).is_some_and(|tab| tab.dirty)
    }

    pub fn has_unsaved_changes(&self) -> bool {
        self.panes.iter().flat_map(|pane| &pane.tabs).any(|tab| tab.dirty)
    }

    pub fn recent_files(&self) -> &[PathBuf] {
        self.recent.entries()
    }

    /// The pane and position a tab is at.
    fn locate(&self, id: TabId) -> Option<(usize, usize)> {
        self.panes.iter().enumerate().find_map(|(pane_ix, pane)| {
            pane.tabs
                .iter()
                .position(|tab| tab.id == id)
                .map(|tab_ix| (pane_ix, tab_ix))
        })
    }

    fn tab(&self, id: TabId) -> Option<&Tab> {
        let (pane, ix) = self.locate(id)?;
        self.panes[pane].tabs.get(ix)
    }

    fn tab_mut(&mut self, id: TabId) -> Option<&mut Tab> {
        let (pane, ix) = self.locate(id)?;
        self.panes[pane].tabs.get_mut(ix)
    }

    fn pane_index(&self, id: PaneId) -> Option<usize> {
        self.panes.iter().position(|pane| pane.id == id)
    }

    /// The active pane's shown tab, if nobody has touched it: no file, no
    /// edits, one empty paragraph. A file opened next replaces it.
    fn pristine_tab(&self, cx: &App) -> Option<TabId> {
        let tab = self.pane().active_tab()?;
        let editor = tab.editor()?.read(cx);
        (tab.path.is_none()
            && !tab.dirty
            && editor.block_count() == 1
            && editor.block_is_empty_paragraph(0))
        .then_some(tab.id)
    }

    // ---------------------------------------------------------- appearance

    pub fn is_dark(&self, cx: &App) -> bool {
        cx.theme().mode.is_dark()
    }

    /// Switch between the light and dark palettes and remember the choice.
    pub fn toggle_appearance(&mut self, cx: &mut Context<Self>) {
        crate::editor::theme::toggle_appearance(cx);
        self.settings.appearance = Some(if cx.theme().mode.is_dark() {
            Appearance::Dark
        } else {
            Appearance::Light
        });
        self.settings.save(self.config_dir.as_deref());
        cx.notify();
    }

    // ---------------------------------------------------------------- tabs

    /// Open `content` in a new tab after the active one in the active pane,
    /// and bring it forward.
    pub fn add_tab(
        &mut self,
        content: Vec<BlockContent>,
        path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<RifflyEditor> {
        let editor = cx.new(|cx| RifflyEditor::with_content(content, window, cx));
        let id = self.fresh_id();

        let subscriptions = vec![
            cx.subscribe(&editor, move |this, _, _: &DocumentChanged, cx| {
                if let Some(tab) = this.tab_mut(id) {
                    tab.dirty = true;
                }
                cx.notify();
            }),
            // The outline follows the caret, so redraw when the editor does.
            cx.observe(&editor, |_, _, cx| cx.notify()),
        ];

        self.insert_tab(
            Tab {
                id,
                content: Content::Doc(editor.clone()),
                path,
                dirty: false,
                _subscriptions: subscriptions,
            },
            window,
            cx,
        );
        editor
    }

    /// Open a PDF in a new tab after the active one and bring it forward.
    pub fn add_pdf_tab(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<PdfView> {
        let pdf = cx.new(|cx| PdfView::open(path.clone(), window, cx));
        let id = self.fresh_id();
        let subscriptions = vec![
            cx.subscribe(&pdf, |_, _, _: &CommentsChanged, cx| cx.notify()),
            cx.observe(&pdf, |_, _, cx| cx.notify()),
        ];
        self.insert_tab(
            Tab {
                id,
                content: Content::Pdf(pdf.clone()),
                path: Some(path),
                dirty: false,
                _subscriptions: subscriptions,
            },
            window,
            cx,
        );
        pdf
    }

    fn insert_tab(&mut self, tab: Tab, window: &mut Window, cx: &mut Context<Self>) {
        let pane = &mut self.panes[self.active_pane];
        let at = if pane.tabs.is_empty() {
            0
        } else {
            (pane.active + 1).min(pane.tabs.len())
        };
        pane.tabs.insert(at, tab);
        self.activate_at(self.active_pane, at, window, cx);
    }

    pub fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.add_tab(Vec::new(), None, window, cx);
    }

    /// Bring a tab of the active pane forward.
    pub fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.activate_at(self.active_pane, ix, window, cx);
    }

    /// Make a pane active and show one of its tabs, with the keyboard in it.
    fn activate_at(&mut self, pane: usize, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.panes.get(pane).and_then(|p| p.tabs.get(ix)) else {
            return;
        };
        let content = tab.content.clone();
        self.active_pane = pane;
        self.panes[pane].active = ix;
        match content {
            Content::Doc(editor) => editor.update(cx, |editor, cx| editor.focus_document(window, cx)),
            Content::Pdf(pdf) => window.focus(&pdf.focus_handle(cx), cx),
        }
        cx.notify();
    }

    fn activate_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        if let Some((pane, ix)) = self.locate(id) {
            self.activate_at(pane, ix, window, cx);
        }
    }

    /// Make a pane the active one, keeping the tab it shows.
    pub fn focus_pane(&mut self, pane: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.panes.get(pane).map(|p| p.active) {
            self.activate_at(pane, ix, window, cx);
        }
    }

    fn cycle(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let len = self.pane().tabs.len() as isize;
        if len == 0 {
            return;
        }
        let ix = (self.pane().active as isize + delta).rem_euclid(len) as usize;
        self.activate(ix, window, cx);
    }

    /// Close a tab of the active pane, asking first when it has unsaved
    /// changes.
    pub fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.pane().tabs.get(ix).map(|tab| tab.id) {
            self.close_tab_id(id, window, cx);
        }
    }

    fn close_tab_id(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tabs(vec![id], false, window, cx);
    }

    /// Take a tab out of its pane, returning it. An emptied pane goes away
    /// unless it is the last one, and its width goes to its neighbour.
    fn detach_tab(&mut self, id: TabId) -> Option<Tab> {
        let (pane_ix, ix) = self.locate(id)?;
        let pane = &mut self.panes[pane_ix];
        let tab = pane.tabs.remove(ix);
        if ix < pane.active || pane.active >= pane.tabs.len() {
            pane.active = pane.active.saturating_sub(1);
        }
        if pane.tabs.is_empty() && self.panes.len() > 1 {
            self.remove_pane_at(pane_ix);
        }
        Some(tab)
    }

    /// Drop an (empty) pane, its width going to the pane on its left, or
    /// on its right when it is the first.
    fn remove_pane_at(&mut self, pane_ix: usize) {
        let removed = self.panes.remove(pane_ix);
        let heir = pane_ix.saturating_sub(1).min(self.panes.len() - 1);
        self.panes[heir].width += removed.width;
        if self.active_pane > pane_ix || self.active_pane >= self.panes.len() {
            self.active_pane = self.active_pane.saturating_sub(1);
        }
        self.normalize_widths();
    }

    /// Insert an empty pane after `after`, sharing that pane's width.
    fn insert_pane_after(&mut self, after: usize) -> PaneId {
        let id = self.fresh_id();
        let half = self.panes[after].width / 2.;
        self.panes[after].width = half;
        self.panes.insert(after + 1, Pane::new(id, half));
        self.normalize_widths();
        id
    }

    fn normalize_widths(&mut self) {
        let total: f32 = self.panes.iter().map(|pane| pane.width.max(0.0)).sum();
        if total <= f32::EPSILON || !total.is_finite() {
            self.equalize_panes_inner();
            return;
        }
        for pane in &mut self.panes {
            pane.width = pane.width.max(0.0) / total;
        }
    }

    fn equalize_panes_inner(&mut self) {
        let share = 1.0 / self.panes.len() as f32;
        for pane in &mut self.panes {
            pane.width = share;
        }
    }

    /// Give every pane the same width.
    pub fn equalize_panes(&mut self, cx: &mut Context<Self>) {
        self.equalize_panes_inner();
        cx.notify();
    }

    /// Put the divider after pane `left` at `at`, a share of the row from its
    /// left edge, keeping both panes it separates at least `min` wide.
    pub fn set_divider(&mut self, left: usize, at: f32, min: f32, cx: &mut Context<Self>) {
        if left + 1 >= self.panes.len() {
            return;
        }
        let start: f32 = self.panes[..left].iter().map(|pane| pane.width).sum();
        let pair = self.panes[left].width + self.panes[left + 1].width;
        let min = min.min(pair / 2.);
        let width = (at - start).clamp(min, pair - min);
        self.panes[left].width = width;
        self.panes[left + 1].width = pair - width;
        cx.notify();
    }

    /// Grow (or, with a negative step, shrink) the active pane, taking the
    /// room from the others in proportion to their widths.
    pub fn resize_active_pane(&mut self, step: f32, cx: &mut Context<Self>) {
        let n = self.panes.len();
        if n < 2 {
            return;
        }
        let ix = self.active_pane;
        let old = self.panes[ix].width;
        let new = (old + step).clamp(MIN_PANE_SHARE, 1.0 - MIN_PANE_SHARE * (n - 1) as f32);
        let rest_old = 1.0 - old;
        let rest_new = 1.0 - new;
        for (other, pane) in self.panes.iter_mut().enumerate() {
            if other == ix {
                pane.width = new;
            } else if rest_old > f32::EPSILON {
                pane.width = pane.width / rest_old * rest_new;
            }
        }
        self.normalize_widths();
        cx.notify();
    }

    fn remove_tab(&mut self, id: TabId, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pane_ix, _)) = self.locate(id) else { return };
        let pane_id = self.panes[pane_ix].id;
        if self.detach_tab(id).is_none() {
            return;
        }
        if self.panes.len() == 1 && self.panes[0].tabs.is_empty() {
            self.active_pane = 0;
            self.new_tab(window, cx);
            return;
        }
        // Stay in the pane the tab left, if it is still there.
        let pane = self.pane_index(pane_id).unwrap_or(self.active_pane);
        self.focus_pane(pane, window, cx);
    }

    /// Move a tab into a pane, before the tab at `at` or at the end. Within
    /// its own pane this reorders it.
    pub fn move_tab(
        &mut self,
        id: TabId,
        to_pane: PaneId,
        at: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pane_index(to_pane).is_none() {
            return;
        }
        if let Some((from, ix)) = self.locate(id)
            && self.panes[from].id == to_pane
        {
            let pane = &mut self.panes[from];
            let tab = pane.tabs.remove(ix);
            let at = at.unwrap_or(pane.tabs.len() + 1);
            let at = if at > ix { at - 1 } else { at }.min(pane.tabs.len());
            pane.tabs.insert(at, tab);
            self.activate_at(from, at, window, cx);
            return;
        }
        let Some(tab) = self.detach_tab(id) else { return };
        let Some(target) = self.pane_index(to_pane) else { return };
        let pane = &mut self.panes[target];
        let at = at.unwrap_or(pane.tabs.len()).min(pane.tabs.len());
        pane.tabs.insert(at, tab);
        self.activate_at(target, at, window, cx);
    }

    /// Put a tab in a new pane to the right of `beside`.
    fn split_with(&mut self, id: TabId, beside: PaneId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.pane_index(beside) else { return };
        let Some((from, _)) = self.locate(id) else { return };
        // A pane's only tab dropped on its own edge has nowhere to go.
        if from == at && self.panes[from].tabs.len() == 1 {
            return;
        }
        let pane_id = self.insert_pane_after(at);
        self.move_tab(id, pane_id, None, window, cx);
    }

    /// Split the active pane: its shown tab moves into a new pane on the
    /// right, or, when it is the pane's only tab, the new pane starts with
    /// an empty document.
    pub fn split_right(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pane = self.pane();
        let beside = pane.id;
        match pane.active_tab() {
            Some(tab) if pane.tabs.len() > 1 => {
                let id = tab.id;
                self.split_with(id, beside, window, cx);
            }
            _ => {
                self.insert_pane_after(self.active_pane);
                self.active_pane += 1;
                self.new_tab(window, cx);
            }
        }
    }

    /// Move the shown tab to the pane on the right (`1`) or left (`-1`),
    /// opening a new pane there when there is none.
    pub fn move_tab_to_neighbour(
        &mut self,
        direction: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pane = self.pane();
        let Some(id) = pane.active_tab().map(|tab| tab.id) else { return };
        let lone = pane.tabs.len() == 1;
        let beside = pane.id;
        let target = self.active_pane as isize + direction;
        if target >= 0 && (target as usize) < self.panes.len() {
            let to = self.panes[target as usize].id;
            self.move_tab(id, to, None, window, cx);
            return;
        }
        if lone {
            return; // a lone tab at the edge stays where it is
        }
        if direction > 0 {
            self.split_with(id, beside, window, cx);
        } else {
            let pane_id = self.fresh_id();
            let half = self.panes[0].width / 2.;
            self.panes[0].width = half;
            self.panes.insert(0, Pane::new(pane_id, half));
            self.active_pane += 1;
            self.move_tab(id, pane_id, None, window, cx);
        }
    }

    /// Move the shown tab one place along its pane's tab bar.
    pub fn move_tab_within_pane(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        let pane_ix = self.active_pane;
        let pane = &mut self.panes[pane_ix];
        let len = pane.tabs.len() as isize;
        let to = pane.active as isize + delta;
        if len < 2 || to < 0 || to >= len {
            return;
        }
        pane.tabs.swap(pane.active, to as usize);
        self.activate_at(pane_ix, to as usize, window, cx);
    }

    /// Close every tab of the active pane, asking about unsaved ones; the
    /// pane goes with its last tab.
    pub fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<TabId> = self.pane().tabs.iter().map(|tab| tab.id).collect();
        self.close_tabs(ids, false, window, cx);
    }

    // ------------------------------------------------------------- sidebar

    /// How many rows the sidebar's keyboard cursor walks: the outline, then
    /// the recent files.
    fn sidebar_rows(&self, cx: &App) -> (usize, usize) {
        let outline = match (self.active_editor(), self.active_pdf()) {
            (Some(editor), _) => editor.read(cx).outline().len(),
            (None, Some(pdf)) => pdf.read(cx).page_count(),
            _ => 0,
        };
        (outline, self.recent.entries().len())
    }

    /// Give the sidebar the keyboard with its cursor on row `row`.
    fn focus_sidebar(&mut self, row: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let (outline, recent) = self.sidebar_rows(cx);
        let total = outline + recent;
        self.show_sidebar = true;
        self.sidebar_cursor = if total == 0 {
            None
        } else {
            Some(row.unwrap_or(0).min(total - 1))
        };
        window.focus(&self.sidebar_focus, cx);
        cx.notify();
    }

    /// Ctrl+Shift+O: the outline, at the heading the caret is under.
    pub fn focus_outline(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = match self.active_pdf() {
            Some(pdf) => Some(pdf.read(cx).current_page()),
            None => self.active_editor().and_then(|editor| {
                let editor = editor.read(cx);
                let current = editor.current_heading()?;
                editor.outline().iter().position(|item| item.id == current)
            }),
        };
        self.focus_sidebar(Some(row.unwrap_or(0)), window, cx);
    }

    /// Ctrl+Shift+R: the first recent file.
    pub fn focus_recent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (outline, _) = self.sidebar_rows(cx);
        self.focus_sidebar(Some(outline), window, cx);
    }

    fn move_sidebar_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let (outline, recent) = self.sidebar_rows(cx);
        let total = (outline + recent) as isize;
        if total == 0 {
            self.sidebar_cursor = None;
        } else {
            let at = self.sidebar_cursor.map(|row| row as isize).unwrap_or(-1) + delta;
            self.sidebar_cursor = Some(at.clamp(0, total - 1) as usize);
        }
        cx.notify();
    }

    /// Enter in the sidebar: jump to the heading, or open the file.
    pub fn confirm_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.sidebar_cursor else { return };
        let (outline, _) = self.sidebar_rows(cx);
        if row < outline {
            if let Some(editor) = self.active_editor() {
                let id = editor.read(cx).outline()[row].id;
                editor.update(cx, |editor, cx| editor.reveal_block(id, window, cx));
            } else if let Some(pdf) = self.active_pdf() {
                pdf.update(cx, |pdf, cx| pdf.scroll_to_page(row, cx));
                window.focus(&pdf.focus_handle(cx), cx);
            }
        } else if let Some(path) = self.recent.entries().get(row - outline).cloned() {
            self.open_path(path, window, cx);
            self.focus_pane(self.active_pane, window, cx);
        }
        self.sidebar_cursor = None;
        cx.notify();
    }

    fn leave_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar_cursor = None;
        self.focus_pane(self.active_pane, window, cx);
    }

    /// Delete in the sidebar: forget the highlighted recent file.
    fn remove_sidebar_row(&mut self, cx: &mut Context<Self>) {
        let Some(row) = self.sidebar_cursor else { return };
        let (outline, _) = self.sidebar_rows(cx);
        if row < outline {
            return;
        }
        if let Some(path) = self.recent.entries().get(row - outline).cloned() {
            self.recent.remove(&path);
        }
        self.move_sidebar_cursor(0, cx);
    }

    pub fn clear_recent(&mut self, cx: &mut Context<Self>) {
        self.recent.clear();
        self.move_sidebar_cursor(0, cx);
    }

    // ---------------------------------------------------------- cheat sheet

    pub fn toggle_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.show_shortcuts = !self.show_shortcuts;
        if self.show_shortcuts {
            window.focus(&self.shortcuts_focus, cx);
        } else {
            self.focus_pane(self.active_pane, window, cx);
        }
        cx.notify();
    }

    // --------------------------------------------------------------- files

    /// Open a markdown file in a tab, or bring its tab forward wherever it is.
    pub fn open_path(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        let path = std::path::absolute(&path).unwrap_or(path);
        let open = self
            .panes
            .iter()
            .flat_map(|pane| &pane.tabs)
            .find(|tab| tab.path.as_deref().is_some_and(|open| same_path(open, &path)))
            .map(|tab| tab.id);
        if let Some(id) = open {
            self.recent.touch(&path);
            self.activate_tab(id, window, cx);
            return;
        }

        if !path.is_file() {
            self.error = Some(format!("Couldn't open {}: it is not there", path.display()).into());
            self.recent.remove(&path);
            cx.notify();
            return;
        }
        let replace = self.pristine_tab(cx);
        if is_pdf(&path) {
            self.error = None;
            self.add_pdf_tab(path.clone(), window, cx);
        } else {
            let text = match std::fs::read(&path) {
                Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                Err(error) => {
                    self.error = Some(format!("Couldn't open {}: {error}", path.display()).into());
                    self.recent.remove(&path);
                    cx.notify();
                    return;
                }
            };
            self.error = None;
            let content = match Format::of(&path) {
                Format::Html => html::parse(&text),
                Format::Markdown => markdown::parse(&text),
            };
            self.add_tab(content, Some(path.clone()), window, cx);
        }
        if let Some(id) = replace
            && let Some((pane_ix, ix)) = self.locate(id)
        {
            let pane = &mut self.panes[pane_ix];
            pane.tabs.remove(ix);
            if ix < pane.active {
                pane.active -= 1;
            }
        }
        self.recent.touch(&path);
        cx.notify();
    }

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                for path in paths {
                    this.open_path(path, window, cx);
                }
            });
        })
        .detach();
    }

    /// Write a tab to its file, asking where when it has none or when
    /// `choose` is set. `close` closes the tab once it is written.
    fn save_tab(
        &mut self,
        id: TabId,
        choose: bool,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(tab) = self.tab(id) else { return };
        // A PDF is never written; its comments save themselves.
        if tab.pdf().is_some() {
            if close {
                self.remove_tab(id, window, cx);
            }
            return;
        }
        if let Some(path) = tab.path.clone()
            && !choose
        {
            if self.write_tab(id, path, cx) && close {
                self.remove_tab(id, window, cx);
            }
            return;
        }

        let Some(chosen) = self.save_dialog(id, cx) else { return };
        cx.spawn_in(window, async move |this, cx| {
            let Some(path) = chosen.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.write_tab(id, path, cx) && close {
                    this.remove_tab(id, window, cx);
                }
            });
        })
        .detach();
    }

    /// Ask where to save a tab, starting beside its file or in Documents.
    fn save_dialog(
        &self,
        id: TabId,
        cx: &App,
    ) -> Option<impl std::future::Future<Output = Option<PathBuf>> + use<>> {
        let tab = self.tab(id)?;
        let current = tab.path.clone();
        let directory = current
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .unwrap_or_else(default_directory);
        let suggested = current
            .as_deref()
            .and_then(Path::file_name)
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| tab.editor().map(|e| suggested_name(e, cx)).unwrap_or_default());
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        Some(async move { chosen.await.ok()?.ok()?.map(with_markdown_extension) })
    }

    fn write_tab(&mut self, id: TabId, path: PathBuf, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.tab(id).and_then(|tab| tab.editor().cloned()) else {
            return false;
        };
        let text = match Format::of(&path) {
            Format::Html => editor.read(cx).to_html(cx),
            Format::Markdown => editor.read(cx).to_markdown(cx),
        };
        match std::fs::write(&path, text) {
            Ok(()) => {
                self.error = None;
                self.recent.touch(&path);
                if let Some(tab) = self.tab_mut(id) {
                    tab.path = Some(path);
                    tab.dirty = false;
                }
                cx.notify();
                true
            }
            Err(error) => {
                self.error = Some(format!("Couldn't save {}: {error}", path.display()).into());
                cx.notify();
                false
            }
        }
    }

    fn save_active(&mut self, choose: bool, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(id) = self.pane().active_tab().map(|tab| tab.id) {
            self.save_tab(id, choose, false, window, cx);
        }
    }

    // --------------------------------------------------------- window close

    /// Ask about unsaved tabs before the window closes. Call once, after the
    /// workspace is in its window.
    pub fn guard_window_close(&self, window: &mut Window, cx: &mut Context<Self>) {
        // Weak, and taken without upgrading: this may run while the
        // workspace is still being built.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(this) = this.upgrade() else { return true };
            this.update(cx, |this, cx| {
                if this.close_confirmed || !this.has_unsaved_changes() {
                    return true;
                }
                this.confirm_close(window, cx);
                false
            })
        });
    }

    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids = self
            .panes
            .iter()
            .flat_map(|pane| &pane.tabs)
            .map(|tab| tab.id)
            .collect();
        self.close_tabs(ids, true, window, cx);
    }

    /// Close tabs, one question at a time: each unsaved one is brought
    /// forward and asked about (Save / Don't Save / Cancel). With `quit`,
    /// nothing is closed until every answer is in, and then the window goes;
    /// otherwise each tab closes as soon as it is settled. Cancel, or a save
    /// dialog dismissed, stops there.
    fn close_tabs(
        &mut self,
        ids: Vec<TabId>,
        quit: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.closing {
            return;
        }
        // Clean tabs need no question.
        let mut dirty = Vec::new();
        for id in ids {
            match self.tab(id) {
                Some(tab) if tab.dirty => dirty.push(id),
                Some(_) if !quit => self.remove_tab(id, window, cx),
                _ => {}
            }
        }
        if dirty.is_empty() {
            if quit {
                self.close_confirmed = true;
                window.remove_window();
                cx.quit();
            }
            return;
        }
        self.closing = true;

        cx.spawn_in(window, async move |this, cx| {
            let proceed = async {
                for id in dirty {
                    let asked = this.update_in(cx, |this, window, cx| {
                        let tab = this.tab(id)?;
                        if !tab.dirty {
                            if !quit {
                                this.remove_tab(id, window, cx);
                            }
                            return None;
                        }
                        let message = format!("Save changes to {}?", tab.title());
                        this.activate_tab(id, window, cx);
                        Some(window.prompt(
                            PromptLevel::Warning,
                            &message,
                            Some("Your changes will be lost if you don't save them."),
                            &["Save", "Don't Save", "Cancel"],
                            cx,
                        ))
                    });
                    let Ok(Some(answer)) = asked else { continue };
                    match answer.await {
                        Ok(0) => {}
                        Ok(1) => {
                            if !quit {
                                let _ = this.update_in(cx, |this, window, cx| {
                                    this.remove_tab(id, window, cx)
                                });
                            }
                            continue;
                        }
                        _ => return false,
                    }

                    let known = this
                        .update(cx, |this, _| this.tab(id).and_then(|tab| tab.path.clone()))
                        .ok()
                        .flatten();
                    let path = match known {
                        Some(path) => path,
                        None => {
                            let Ok(Some(dialog)) =
                                this.update(cx, |this, cx| this.save_dialog(id, cx))
                            else {
                                return false;
                            };
                            let Some(path) = dialog.await else {
                                return false;
                            };
                            path
                        }
                    };
                    let written = this
                        .update_in(cx, |this, window, cx| {
                            let written = this.write_tab(id, path, cx);
                            if written && !quit {
                                this.remove_tab(id, window, cx);
                            }
                            written
                        })
                        .unwrap_or(false);
                    if !written {
                        return false;
                    }
                }
                true
            }
            .await;

            let _ = this.update_in(cx, |this, window, cx| {
                this.closing = false;
                if proceed && quit {
                    this.close_confirmed = true;
                    window.remove_window();
                    cx.quit();
                }
            });
        })
        .detach();
    }

    // -------------------------------------------------------------- render

    fn render_tab_bar(&self, pane_ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.editor_theme().clone();
        let colors = cx.theme().clone();
        let pane = &self.panes[pane_ix];
        let pane_id = pane.id;
        // The accent marks which pane has the keyboard, so one pane needs none.
        let pane_is_active = pane_ix == self.active_pane && self.panes.len() > 1;
        let last_pane = pane_ix + 1 == self.panes.len();

        let tabs = pane.tabs.iter().enumerate().map(|(ix, tab)| {
            let shown = ix == pane.active;
            let id = tab.id;
            let title = tab.title();
            let tab_icon = tab.icon();
            h_flex()
                .id(("tab", id as usize))
                .h_full()
                .max_w(theme.rems(14.))
                .pl(theme.rems(0.75))
                .pr(theme.rems(0.375))
                .gap(theme.rems(0.375))
                .border_r_1()
                .border_color(colors.border)
                .relative()
                .cursor_pointer()
                .text_size(theme.ui_text_size)
                .when(shown, |this| {
                    this.bg(colors.tab_active)
                        .text_color(colors.tab_active_foreground)
                        // The tab the keyboard is in carries the accent.
                        .when(pane_is_active, |this| {
                            this.child(
                                div()
                                    .absolute()
                                    .top_0()
                                    .left_0()
                                    .right_0()
                                    .h(gpui_kit::px(2.))
                                    .bg(colors.primary),
                            )
                        })
                })
                .when(!shown, |this| {
                    this.bg(colors.tab)
                        .text_color(colors.muted_foreground)
                        .hover(|this| this.text_color(colors.foreground))
                })
                .on_click(cx.listener(move |this, _, window, cx| this.activate_tab(id, window, cx)))
                .on_mouse_down(
                    MouseButton::Middle,
                    cx.listener(move |this, _, window, cx| this.close_tab_id(id, window, cx)),
                )
                .on_drag(
                    DraggedTab {
                        id,
                        title: title.clone(),
                    },
                    |dragged, _, _, cx| cx.new(|_| dragged.clone()),
                )
                .drag_over::<DraggedTab>(move |style, _, _, cx| {
                    style.bg(cx.theme().drop_target)
                })
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                    if dragged.id == id {
                        return;
                    }
                    let at = this
                        .pane_index(pane_id)
                        .and_then(|pane| this.panes[pane].tabs.iter().position(|t| t.id == id));
                    this.move_tab(dragged.id, pane_id, at, window, cx);
                }))
                .child(icon(tab_icon, theme.rems(0.875), colors.muted_foreground))
                .child(div().min_w_0().truncate().child(title))
                .child(
                    div()
                        .id(("close-tab", id as usize))
                        .size(theme.rems(1.25))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .rounded(theme.radius)
                        .hover(|this| this.bg(colors.accent))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.close_tab_id(id, window, cx);
                        }))
                        .child(if tab.dirty {
                            // A changed tab shows a dot where its close
                            // button is, the way editors mark unsaved work.
                            div()
                                .size(theme.rems(0.5))
                                .rounded_full()
                                .bg(colors.foreground.opacity(0.6))
                                .into_any_element()
                        } else {
                            icon("x", theme.rems(0.75), colors.muted_foreground).into_any_element()
                        }),
                )
        });

        h_flex()
            .id(("tab-bar", pane_id as usize))
            .flex_none()
            .h(theme.rems(2.25))
            .w_full()
            .bg(colors.tab_bar)
            .border_b_1()
            .border_color(colors.border)
            .when(pane_ix == 0 && !self.show_sidebar, |this| {
                this.child(
                    toolbar_icon(("show-sidebar", 0usize), "panel-left", "Show sidebar", cx).on_click(
                        cx.listener(|this, _, _, cx| {
                            this.show_sidebar = true;
                            cx.notify();
                        }),
                    ),
                )
            })
            .child(
                h_flex()
                    .id(("tabs", pane_id as usize))
                    .h_full()
                    .min_w_0()
                    .overflow_x_scroll()
                    .children(tabs),
            )
            .child(
                toolbar_icon(("new-tab", pane_id as usize), "plus", "New tab", cx).on_click(
                    cx.listener(move |this, _, window, cx| {
                        if let Some(pane) = this.pane_index(pane_id) {
                            this.active_pane = pane;
                        }
                        this.new_tab(window, cx)
                    }),
                ),
            )
            // The empty end of the bar takes a dropped tab as this pane's last.
            .child(
                div()
                    .id(("tab-bar-end", pane_id as usize))
                    .flex_1()
                    .h_full()
                    .drag_over::<DraggedTab>(|style, _, _, cx| style.bg(cx.theme().drop_target))
                    .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                        this.move_tab(dragged.id, pane_id, None, window, cx)
                    })),
            )
            .when(last_pane, |this| {
                this.children(self.error.clone().map(|error| {
                    div()
                        .px(theme.rems(0.75))
                        .max_w(theme.rems(24.))
                        .text_size(theme.ui_text_size)
                        .text_color(colors.danger)
                        .truncate()
                        .child(error)
                }))
            })
            .child(
                toolbar_icon(("split", pane_id as usize), "columns-2", "Split right", cx).on_click(
                    cx.listener(move |this, _, window, cx| {
                        if let Some(pane) = this.pane_index(pane_id) {
                            this.focus_pane(pane, window, cx);
                        }
                        this.split_right(window, cx)
                    }),
                ),
            )
    }

    fn render_pane(&self, pane_ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = &self.panes[pane_ix];
        let pane_id = pane.id;
        let editor: Option<AnyElement> = pane.active_tab().map(|tab| match &tab.content {
            Content::Doc(editor) => editor.clone().into_any_element(),
            Content::Pdf(pdf) => pdf.clone().into_any_element(),
        });
        let dragging = cx.has_active_drag() && !self.resizing;

        v_flex()
            .id(("pane", pane_id as usize))
            .w(gpui_kit::relative(pane.width))
            .min_w_0()
            .h_full()
            // A press anywhere in a pane makes it the active one.
            .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                if let Some(pane) = this.pane_index(pane_id)
                    && this.active_pane != pane
                {
                    this.active_pane = pane;
                    cx.notify();
                }
            }))
            .child(self.render_tab_bar(pane_ix, cx))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .children(editor)
                    .when(dragging, |this| {
                        // While a tab is carried, the pane's right edge
                        // splits and the rest of it takes the tab.
                        this.child(
                            div()
                                .id(("drop-into", pane_id as usize))
                                .absolute()
                                .top_0()
                                .left_0()
                                .h_full()
                                .w(gpui_kit::relative(0.65))
                                .drag_over::<DraggedTab>(|style, _, _, cx| {
                                    style.bg(cx.theme().drop_target)
                                })
                                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                                    this.move_tab(dragged.id, pane_id, None, window, cx)
                                })),
                        )
                        .child(
                            div()
                                .id(("drop-split", pane_id as usize))
                                .absolute()
                                .top_0()
                                .right_0()
                                .h_full()
                                .w(gpui_kit::relative(0.35))
                                .drag_over::<DraggedTab>(|style, _, _, cx| {
                                    style
                                        .bg(cx.theme().drop_target)
                                        .border_l_2()
                                        .border_color(cx.theme().primary)
                                })
                                .on_drop(cx.listener(move |this, dragged: &DraggedTab, window, cx| {
                                    this.split_with(dragged.id, pane_id, window, cx)
                                })),
                        )
                    }),
            )
    }

    fn render_shortcuts(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.editor_theme().clone();
        let colors = cx.theme().clone();
        let groups = SHORTCUTS.iter().map(|(group, rows)| {
            v_flex()
                .gap(theme.rems(0.25))
                .child(
                    div()
                        .pb(theme.rems(0.25))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(colors.muted_foreground)
                        .child(*group),
                )
                .children(rows.iter().map(|(keys, what)| {
                    h_flex()
                        .gap(theme.rems(1.))
                        .justify_between()
                        .child(div().child(*what))
                        .child(
                            div()
                                .flex_none()
                                .px(theme.rems(0.375))
                                .rounded(theme.radius)
                                .bg(colors.muted)
                                .font_family("monospace")
                                .text_size(theme.ui_text_size * 0.9)
                                .child(*keys),
                        )
                }))
        });

        div()
            .id("shortcuts-backdrop")
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(gpui_kit::black().opacity(0.35))
            .on_click(cx.listener(|this, _, window, cx| this.toggle_shortcuts(window, cx)))
            .child(
                div()
                    .id("shortcuts")
                    .track_focus(&self.shortcuts_focus)
                    .key_context("WorkspaceShortcuts")
                    .w(theme.rems(52.))
                    .max_w(gpui_kit::relative(0.92))
                    .max_h(gpui_kit::relative(0.88))
                    .overflow_y_scroll()
                    .p(theme.rems(1.25))
                    .rounded(theme.radius * 2.)
                    .bg(colors.popover)
                    .text_color(colors.popover_foreground)
                    .border_1()
                    .border_color(colors.border)
                    .shadow_lg()
                    .text_size(theme.ui_text_size)
                    // Clicks inside the sheet stay inside it.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .justify_between()
                            .pb(theme.rems(0.75))
                            .child(
                                div()
                                    .text_size(theme.ui_text_size * 1.3)
                                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                    .child("Keyboard shortcuts"),
                            )
                            .child(
                                div()
                                    .text_color(colors.muted_foreground)
                                    .child("Esc or F1 to close"),
                            ),
                    )
                    .child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap_x(theme.rems(2.))
                            .gap_y(theme.rems(1.25))
                            .children(groups),
                    ),
            )
    }

    /// The line between pane `left` and the next, with a wider grip to drag.
    /// Double-click it to make all panes equal.
    fn render_divider(&self, left: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let left_id = self.panes[left].id;
        let colors = cx.theme().clone();
        div()
            .relative()
            .flex_none()
            .w(gpui_kit::px(1.))
            .h_full()
            .bg(colors.border)
            .child(
                div()
                    .id(("divider", left_id as usize))
                    .absolute()
                    .top_0()
                    .left(gpui_kit::px(-3.))
                    .w(gpui_kit::px(7.))
                    .h_full()
                    .cursor_col_resize()
                    .hover(|this| this.bg(colors.primary.opacity(0.35)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui_kit::MouseDownEvent, _, cx| {
                            this.resizing = true;
                            if event.click_count >= 2 {
                                this.equalize_panes(cx);
                            }
                        }),
                    )
                    .on_drag(DraggedDivider { left: left_id }, |dragged, _, _, cx| {
                        cx.new(|_| dragged.clone())
                    }),
            )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.editor_theme().clone();
        let colors = cx.theme().clone();

        let (outline, current) = match self.pane().active_tab().and_then(Tab::editor) {
            Some(editor) => {
                let editor = editor.read(cx);
                (editor.outline(), editor.current_heading())
            }
            None => (Vec::new(), None),
        };
        let pdf = self.active_pdf();
        let editor = self.active_editor();
        let min_level = outline.iter().map(|item| item.level).min().unwrap_or(1);

        let cursor = self.sidebar_cursor;
        let outline_len = match &pdf {
            Some(pdf) => pdf.read(cx).page_count(),
            None => outline.len(),
        };
        let outline_rows: Vec<AnyElement> = outline
            .into_iter()
            .enumerate()
            .map(|(row, item)| {
                let BlockId(raw) = item.id;
                let id = item.id;
                let editor = editor.clone();
                let selected = current == Some(item.id);
                let empty = item.text.trim().is_empty();
                let label: SharedString = if empty {
                    "Untitled heading".into()
                } else {
                    item.text.replace('\n', " ").into()
                };
                sidebar_row(("outline", raw as usize), selected, cursor == Some(row), cx)
                    .pl(theme.rems(0.5 + 0.875 * (item.level - min_level) as f32))
                    .when(item.level == min_level, |this| {
                        this.font_weight(gpui_kit::FontWeight::MEDIUM)
                    })
                    .when(empty, |this| this.text_color(colors.muted_foreground))
                    .on_click(move |_, window, cx| {
                        if let Some(editor) = &editor {
                            editor.update(cx, |editor, cx| editor.reveal_block(id, window, cx));
                        }
                    })
                    .child(div().min_w_0().truncate().child(label))
                    .into_any_element()
            })
            .collect();

        let shown_path = self.pane().active_tab().and_then(|tab| tab.path.clone());
        let recent_rows: Vec<AnyElement> = self
            .recent
            .entries()
            .iter()
            .enumerate()
            .map(|(ix, path)| {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                let folder = path
                    .parent()
                    .map(|parent| parent.display().to_string())
                    .unwrap_or_default();
                let open = shown_path
                    .as_deref()
                    .is_some_and(|shown| same_path(shown, path));
                let target = path.clone();
                sidebar_row(("recent", ix), open, cursor == Some(outline_len + ix), cx)
                    .h(theme.rems(2.5))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_path(target.clone(), window, cx)
                    }))
                    .child(icon(
                        if is_pdf(path) { "book-open" } else { "file-text" },
                        theme.rems(0.875),
                        colors.muted_foreground,
                    ))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(div().truncate().child(name))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme.ui_text_size * 0.85)
                                    .text_color(colors.muted_foreground)
                                    .child(folder),
                            ),
                    )
                    .into_any_element()
            })
            .collect();

        // A PDF lists its pages where a document lists its headings, and its
        // comments below them.
        let (page_rows, comment_rows): (Vec<AnyElement>, Vec<AnyElement>) = match &pdf {
            Some(pdf_entity) => {
                let view = pdf_entity.read(cx);
                let current_page = view.current_page();
                let open = view.open_comment();
                let pages = (0..view.page_count())
                    .map(|page| {
                        let target = pdf_entity.clone();
                        sidebar_row(("pdf-page-row", page), page == current_page, cursor == Some(page), cx)
                            .on_click(move |_, window, cx| {
                                target.update(cx, |pdf, cx| pdf.scroll_to_page(page, cx));
                                window.focus(&target.focus_handle(cx), cx);
                            })
                            .child(format!("Page {}", page + 1))
                            .into_any_element()
                    })
                    .collect();
                let comments = view
                    .numbered_comments()
                    .into_iter()
                    .map(|(number, comment)| {
                        let target = pdf_entity.clone();
                        let id = comment.id;
                        let text = comment
                            .notes
                            .first()
                            .map(|note| note.body.replace('\n', " "))
                            .unwrap_or_default();
                        sidebar_row(("pdf-comment-row", id as usize), open == Some(id), false, cx)
                            .h(theme.rems(2.5))
                            .on_click(move |_, window, cx| {
                                target.update(cx, |pdf, cx| pdf.show_comment(id, window, cx));
                            })
                            .child(
                                div()
                                    .flex_none()
                                    .size(theme.rems(1.25))
                                    .rounded_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(theme.ui_text_size * 0.8)
                                    .bg(gpui_kit::rgb(0xF2C94C))
                                    .text_color(gpui_kit::rgb(0x37352F))
                                    .child(number.to_string()),
                            )
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .child(div().truncate().child(text))
                                    .child(
                                        div()
                                            .text_size(theme.ui_text_size * 0.85)
                                            .text_color(colors.muted_foreground)
                                            .child(format!(
                                                "Page {} · {} {}",
                                                comment.page + 1,
                                                comment.notes.len(),
                                                if comment.notes.len() == 1 { "note" } else { "notes" }
                                            )),
                                    ),
                            )
                            .into_any_element()
                    })
                    .collect();
                (pages, comments)
            }
            None => (Vec::new(), Vec::new()),
        };
        let is_pdf_tab = pdf.is_some();
        let has_comments = !comment_rows.is_empty();
        let has_outline = !outline_rows.is_empty();
        let has_recent = !recent_rows.is_empty();
        let dark = self.is_dark(cx);

        v_flex()
            .id("sidebar")
            .track_focus(&self.sidebar_focus)
            .key_context(SIDEBAR_CONTEXT)
            .on_action(cx.listener(|this, _: &SidebarDown, _, cx| this.move_sidebar_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &SidebarUp, _, cx| this.move_sidebar_cursor(-1, cx)))
            .on_action(cx.listener(|this, _: &SidebarConfirm, window, cx| {
                this.confirm_sidebar(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SidebarCancel, window, cx| {
                this.leave_sidebar(window, cx)
            }))
            .on_action(cx.listener(|this, _: &SidebarRemove, _, cx| this.remove_sidebar_row(cx)))
            .on_action(cx.listener(|this, _: &ClearRecent, _, cx| this.clear_recent(cx)))
            .flex_none()
            .w(theme.rems(16.))
            .h_full()
            .bg(colors.sidebar)
            .text_color(colors.sidebar_foreground)
            .border_r_1()
            .border_color(colors.sidebar_border)
            .text_size(theme.ui_text_size)
            .child(
                h_flex()
                    .flex_none()
                    .h(theme.rems(2.25))
                    .px(theme.rems(0.25))
                    .border_b_1()
                    .border_color(colors.sidebar_border)
                    .child(
                        toolbar_icon("sidebar-new", "file-plus", "New tab", cx)
                            .on_click(cx.listener(|this, _, window, cx| this.new_tab(window, cx))),
                    )
                    .child(
                        toolbar_icon("sidebar-open", "folder-open", "Open", cx).on_click(
                            cx.listener(|this, _, window, cx| this.prompt_open(window, cx)),
                        ),
                    )
                    .child(
                        toolbar_icon("sidebar-save", "save", "Save", cx).on_click(
                            cx.listener(|this, _, window, cx| this.save_active(false, window, cx)),
                        ),
                    )
                    .child(div().flex_1())
                    .child(
                        toolbar_icon("show-shortcuts", "keyboard", "Keyboard shortcuts (F1)", cx)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_shortcuts(window, cx)
                            })),
                    )
                    .child(
                        toolbar_icon(
                            "toggle-appearance",
                            if dark { "sun" } else { "moon" },
                            if dark { "Light mode" } else { "Dark mode" },
                            cx,
                        )
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_appearance(cx))),
                    )
                    .child(
                        toolbar_icon("hide-sidebar", "panel-left", "Hide sidebar", cx).on_click(
                            cx.listener(|this, _, _, cx| {
                                this.show_sidebar = false;
                                cx.notify();
                            }),
                        ),
                    ),
            )
            .child(
                v_flex()
                    .id("sidebar-sections")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(theme.rems(0.375))
                    .pb(theme.rems(0.75))
                    .when(is_pdf_tab, |this| {
                        this.child(section_header("book-open", "Pages", cx))
                            .children(page_rows)
                            .child(section_header("message-square", "Comments", cx))
                            .children(comment_rows)
                            .when(!has_comments, |this| {
                                this.child(placeholder("Double-click a page to comment.", cx))
                            })
                    })
                    .when(!is_pdf_tab, |this| {
                        this.child(section_header("list-tree", "Outline", cx))
                            .children(outline_rows)
                            .when(!has_outline, |this| {
                                this.child(placeholder("Headings appear here.", cx))
                            })
                    })
                    .child(
                        section_header("clock", "Recent", cx).when(has_recent, |this| {
                            this.child(div().flex_1()).child(
                                div()
                                    .id("clear-recent")
                                    .px(theme.rems(0.375))
                                    .rounded(theme.radius)
                                    .cursor_pointer()
                                    .hover(|this| this.bg(colors.sidebar_accent))
                                    .child("Clear")
                                    .on_click(cx.listener(|this, _, _, cx| this.clear_recent(cx))),
                            )
                        }),
                    )
                    .children(recent_rows)
                    .when(!has_recent, |this| {
                        this.child(placeholder("Files you open or save appear here.", cx))
                    }),
            )
    }
}

fn with_markdown_extension(mut path: PathBuf) -> PathBuf {
    if path.extension().is_none() {
        path.set_extension("md");
    }
    path
}

fn same_path(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

/// Where a save dialog starts: Documents if there is one, else home.
fn default_directory() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let documents = home.join("Documents");
    if documents.is_dir() { documents } else { home }
}

/// A file name for an unsaved document: its first heading, else "Untitled".
fn suggested_name(editor: &Entity<RifflyEditor>, cx: &App) -> String {
    let title = editor
        .read(cx)
        .outline()
        .into_iter()
        .map(|item| item.text)
        .find(|text| !text.trim().is_empty())
        .unwrap_or_else(|| "Untitled".to_string());
    let clean: String = title
        .chars()
        .map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '-' } else { c })
        .collect();
    format!("{}.md", clean.trim())
}

fn toolbar_icon(
    id: impl Into<gpui_kit::ElementId>,
    name: &'static str,
    tooltip: &'static str,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.editor_theme();
    div()
        .id(id)
        .flex_none()
        .size(theme.rems(1.75))
        .mx(theme.rems(0.125))
        .flex()
        .items_center()
        .justify_center()
        .rounded(theme.radius)
        .cursor_pointer()
        .hover(|this| this.bg(cx.theme().accent))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(tooltip).build(window, cx))
        .child(icon(name, theme.rems(1.), cx.theme().muted_foreground))
}

fn sidebar_row(
    id: impl Into<gpui_kit::ElementId>,
    selected: bool,
    keyboard: bool,
    cx: &App,
) -> gpui_kit::Stateful<gpui_kit::Div> {
    let theme = cx.editor_theme();
    let colors = cx.theme();
    let row = h_flex()
        .id(id)
        .w_full()
        .min_h(theme.rems(1.75))
        .px(theme.rems(0.5))
        .gap(theme.rems(0.5))
        .rounded(theme.radius)
        .cursor_pointer()
        .border_1()
        .border_color(if keyboard {
            colors.primary
        } else {
            gpui_kit::transparent_black()
        });
    if selected {
        row.bg(colors.sidebar_accent)
            .text_color(colors.sidebar_accent_foreground)
    } else {
        row.hover(|this| this.bg(colors.sidebar_accent.opacity(0.6)))
    }
}

fn section_header(icon_name: &'static str, label: &'static str, cx: &App) -> gpui_kit::Div {
    let theme = cx.editor_theme();
    h_flex()
        .w_full()
        .mt(theme.rems(0.75))
        .mb(theme.rems(0.25))
        .px(theme.rems(0.5))
        .gap(theme.rems(0.375))
        .text_size(theme.ui_text_size * 0.85)
        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
        .text_color(cx.theme().muted_foreground)
        .child(icon(icon_name, theme.rems(0.875), cx.theme().muted_foreground))
        .child(label)
}

fn placeholder(text: &'static str, cx: &App) -> gpui_kit::Div {
    let theme = cx.editor_theme();
    div()
        .px(theme.rems(0.5))
        .py(theme.rems(0.25))
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.panes.iter().all(|pane| pane.tabs.is_empty()) {
            self.active_pane = 0;
            self.new_tab(window, cx);
        }
        // The pane with the keyboard in it is the active one, however the
        // keyboard got there.
        if let Some(pane) = self.panes.iter().position(|pane| {
            pane.active_tab()
                .is_some_and(|tab| tab.focus_handle(cx).contains_focused(window, cx))
        }) {
            self.active_pane = pane;
        }
        if let Some(tab) = self.pane().active_tab() {
            let mark = if tab.dirty { "● " } else { "" };
            window.set_window_title(&format!("{mark}{} — Riffly", tab.title()));
        }

        let mut panes: Vec<AnyElement> = Vec::with_capacity(self.panes.len() * 2);
        for ix in 0..self.panes.len() {
            if ix > 0 {
                panes.push(self.render_divider(ix - 1, cx).into_any_element());
            }
            panes.push(self.render_pane(ix, cx).into_any_element());
        }

        h_flex()
            .id("workspace")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, _: &OpenFile, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| {
                this.save_active(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SaveFileAs, window, cx| {
                this.save_active(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let ix = this.pane().active;
                this.close_tab(ix, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, window, cx| this.cycle(1, window, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, window, cx| this.cycle(-1, window, cx)))
            .on_action(cx.listener(|this, _: &SplitRight, window, cx| this.split_right(window, cx)))
            .on_action(cx.listener(|this, _: &FocusNextPane, window, cx| {
                let next = (this.active_pane + 1) % this.panes.len();
                this.focus_pane(next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| {
                this.show_sidebar = !this.show_sidebar;
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &ToggleAppearance, _, cx| this.toggle_appearance(cx)))
            .on_action(cx.listener(|this, _: &ClosePane, window, cx| this.close_pane(window, cx)))
            .on_action(cx.listener(|this, _: &FocusPreviousPane, window, cx| {
                let len = this.panes.len();
                let previous = (this.active_pane + len - 1) % len;
                this.focus_pane(previous, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabToNextPane, window, cx| {
                this.move_tab_to_neighbour(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabToPreviousPane, window, cx| {
                this.move_tab_to_neighbour(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabForward, window, cx| {
                this.move_tab_within_pane(1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabBackward, window, cx| {
                this.move_tab_within_pane(-1, window, cx)
            }))
            .on_action(cx.listener(|this, _: &WidenPane, _, cx| this.resize_active_pane(PANE_STEP, cx)))
            .on_action(cx.listener(|this, _: &NarrowPane, _, cx| {
                this.resize_active_pane(-PANE_STEP, cx)
            }))
            .on_action(cx.listener(|this, _: &EqualizePanes, _, cx| this.equalize_panes(cx)))
            .on_action(cx.listener(|this, _: &FocusOutline, window, cx| this.focus_outline(window, cx)))
            .on_action(cx.listener(|this, _: &FocusRecent, window, cx| this.focus_recent(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleShortcuts, window, cx| {
                this.toggle_shortcuts(window, cx)
            }))
            .on_action(cx.listener(|this, action: &ActivateTab, window, cx| {
                let len = this.pane().tabs.len();
                if len > 0 {
                    this.activate(action.0.min(len - 1), window, cx);
                }
            }))
            // A drag that ends anywhere ends a resize.
            .capture_any_mouse_up(cx.listener(|this, _, _, cx| {
                if this.resizing {
                    this.resizing = false;
                    cx.notify();
                }
            }))
            .relative()
            .when(self.show_sidebar, |this| this.child(self.render_sidebar(cx)))
            .child(
                h_flex()
                    .id("panes")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .on_drag_move(cx.listener(
                        |this, event: &gpui_kit::DragMoveEvent<DraggedDivider>, _, cx| {
                            let left_id = event.drag(cx).left;
                            let Some(left) = this.pane_index(left_id) else { return };
                            let width = f32::from(event.bounds.size.width);
                            if width <= 0. {
                                return;
                            }
                            let x = f32::from(event.event.position.x - event.bounds.left()) / width;
                            let min = MIN_PANE_SHARE.max(MIN_PANE_WIDTH / width);
                            this.set_divider(left, x, min, cx);
                        },
                    ))
                    .children(panes),
            )
            .when(self.show_shortcuts, |this| this.child(self.render_shortcuts(cx)))
    }
}
