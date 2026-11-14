//! The shell around the editor: tabs, opening and saving markdown files, the
//! outline and the recent-files list — driven through the real window.

use std::path::{Path, PathBuf};

use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{Action as _, AnyWindowHandle, AppContext as _, Entity, TestAppContext, px, size};
use riffly::editor;
use riffly::workspace::{
    self, ActivateTab, ClosePane, EqualizePanes, FocusOutline, MoveTabForward, MoveTabToNextPane,
    MoveTabToPreviousPane, NextTab, SaveFile, SidebarConfirm, SidebarDown, SplitRight,
    ToggleShortcuts, WidenPane, Workspace,
};

struct Harness {
    workspace: Entity<Workspace>,
    window: AnyWindowHandle,
    dir: PathBuf,
}

fn setup(cx: &mut TestAppContext, name: &str) -> Harness {
    cx.update(gpui_kit::init);
    cx.update(editor::init);
    cx.update(workspace::init);
    cx.update(riffly::pdf::init);

    let dir = std::env::temp_dir().join(format!("riffly-ws-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("config");

    let mut workspace = None;
    let handle = cx.open_window(size(px(1100.), px(800.)), |window, cx| {
        let view = cx.new(|cx| {
            let mut workspace = Workspace::new(Some(config), cx);
            workspace.new_tab(window, cx);
            workspace
        });
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    let harness = Harness {
        workspace: workspace.unwrap(),
        window: handle.into(),
        dir,
    };
    harness.ui(cx, |_, _, _| {});
    harness
}

impl Harness {
    fn ui<R>(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&Entity<Workspace>, &mut gpui_kit::Window, &mut gpui_kit::App) -> R,
    ) -> R {
        let workspace = self.workspace.clone();
        let result = cx
            .update_window(self.window, |_, window, cx| {
                window.render_frame(cx);
                let result = f(&workspace, window, cx);
                window.render_frame(cx);
                result
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    fn file(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }

    fn open(&self, cx: &mut TestAppContext, path: &Path) {
        let path = path.to_path_buf();
        self.ui(cx, |workspace, window, cx| {
            workspace.update(cx, |workspace, cx| workspace.open_path(path, window, cx))
        });
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[gpui_kit::test]
fn opening_a_file_replaces_the_empty_tab_and_lists_its_headings(cx: &mut TestAppContext) {
    let h = setup(cx, "open");
    let path = h.file("notes.md", "# Notes\n\nIntro\n\n## First\n\ntext\n\n### Deeper\n");
    h.open(cx, &path);

    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.tab_count(), 1, "the untouched tab gives way");
        assert_eq!(workspace.tab_titles()[0].as_ref(), "notes.md");
        assert!(!workspace.tab_is_dirty(0));
        assert_eq!(workspace.recent_files().len(), 1);
        assert!(workspace.recent_files()[0].ends_with("notes.md"));

        let editor = workspace.active_editor().unwrap();
        let outline: Vec<(u8, String)> = editor
            .read(cx)
            .outline()
            .into_iter()
            .map(|item| (item.level, item.text))
            .collect();
        assert_eq!(
            outline,
            vec![
                (1, "Notes".to_string()),
                (2, "First".to_string()),
                (3, "Deeper".to_string())
            ]
        );
    });
}

#[gpui_kit::test]
fn opening_an_open_file_brings_its_tab_forward(cx: &mut TestAppContext) {
    let h = setup(cx, "reopen");
    let a = h.file("a.md", "# A\n");
    let b = h.file("b.md", "# B\n");
    h.open(cx, &a);
    h.open(cx, &b);
    h.open(cx, &a);

    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.tab_count(), 2);
        assert_eq!(workspace.tab_titles()[workspace.active_index()].as_ref(), "a.md");
        assert!(workspace.recent_files()[0].ends_with("a.md"));
    });
}

#[gpui_kit::test]
fn an_edit_marks_the_tab_and_saving_writes_markdown(cx: &mut TestAppContext) {
    let h = setup(cx, "save");
    let path = h.file("doc.md", "# Title\n\n- item\n");
    h.open(cx, &path);

    h.ui(cx, |workspace, window, cx| {
        let editor = workspace.read(cx).active_editor().unwrap();
        editor.update(cx, |editor, cx| {
            editor.select_text_in_block(1, 4..4, window, cx);
            editor.insert_content(" two", window, cx);
        });
    });
    cx.update(|cx| assert!(h.workspace.read(cx).tab_is_dirty(0)));

    h.ui(cx, |_, window, cx| window.dispatch_action(SaveFile.boxed_clone(), cx));

    cx.update(|cx| assert!(!h.workspace.read(cx).tab_is_dirty(0)));
    let written = std::fs::read_to_string(&path).unwrap();
    assert_eq!(written, "# Title\n\n- item two\n");
}

#[gpui_kit::test]
fn tabs_cycle_and_close(cx: &mut TestAppContext) {
    let h = setup(cx, "tabs");
    let a = h.file("a.md", "a\n");
    let b = h.file("b.md", "b\n");
    h.open(cx, &a);
    h.open(cx, &b);
    cx.update(|cx| assert_eq!(h.workspace.read(cx).active_index(), 1));

    h.ui(cx, |_, window, cx| window.dispatch_action(NextTab.boxed_clone(), cx));
    cx.update(|cx| assert_eq!(h.workspace.read(cx).active_index(), 0));

    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_tab(0, window, cx))
    });
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.tab_count(), 1);
        assert_eq!(workspace.tab_titles()[0].as_ref(), "b.md");
    });

    // Closing the last tab leaves an empty one, never an empty window.
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_tab(0, window, cx))
    });
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.tab_count(), 1);
        assert_eq!(workspace.tab_titles()[0].as_ref(), "Untitled");
    });
}

fn titles(h: &Harness, cx: &mut TestAppContext) -> Vec<Vec<String>> {
    cx.update(|cx| {
        h.workspace
            .read(cx)
            .pane_tab_titles()
            .into_iter()
            .map(|pane| pane.into_iter().map(|t| t.to_string()).collect())
            .collect()
    })
}

#[gpui_kit::test]
fn splitting_moves_the_shown_tab_into_a_pane_on_the_right(cx: &mut TestAppContext) {
    let h = setup(cx, "split");
    let a = h.file("a.md", "# A\n");
    let b = h.file("b.md", "# B\n");
    h.open(cx, &a);
    h.open(cx, &b);

    h.ui(cx, |_, window, cx| window.dispatch_action(SplitRight.boxed_clone(), cx));
    assert_eq!(titles(&h, cx), vec![vec!["a.md"], vec!["b.md"]]);
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.active_pane_index(), 1, "the new pane takes the keyboard");
        let outline = workspace.active_editor().unwrap().read(cx).outline();
        assert_eq!(outline[0].text, "B", "the outline follows the active pane");
    });

    // Opening a file that is shown in the other pane goes to that pane.
    h.open(cx, &a);
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.pane_count(), 2);
        assert_eq!(workspace.active_pane_index(), 0);
    });
}

#[gpui_kit::test]
fn splitting_a_lone_tab_opens_an_empty_document_beside_it(cx: &mut TestAppContext) {
    let h = setup(cx, "split-lone");
    let a = h.file("a.md", "# A\n");
    h.open(cx, &a);
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.split_right(window, cx))
    });
    assert_eq!(titles(&h, cx), vec![vec!["a.md"], vec!["Untitled"]]);
}

#[gpui_kit::test]
fn closing_a_panes_last_tab_closes_the_pane(cx: &mut TestAppContext) {
    let h = setup(cx, "close-pane");
    let a = h.file("a.md", "# A\n");
    let b = h.file("b.md", "# B\n");
    h.open(cx, &a);
    h.open(cx, &b);
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.split_right(window, cx))
    });
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_tab(0, window, cx))
    });
    assert_eq!(titles(&h, cx), vec![vec!["a.md"]]);
    cx.update(|cx| assert_eq!(h.workspace.read(cx).active_pane_index(), 0));
}

#[gpui_kit::test]
fn the_appearance_toggle_is_remembered(cx: &mut TestAppContext) {
    let h = setup(cx, "appearance");
    let before = cx.update(|cx| h.workspace.read(cx).is_dark(cx));
    h.ui(cx, |workspace, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.toggle_appearance(cx))
    });
    let after = cx.update(|cx| h.workspace.read(cx).is_dark(cx));
    assert_ne!(before, after);

    let saved = workspace::settings::Settings::load(Some(&h.dir.join("config")));
    let expected = if after {
        workspace::settings::Appearance::Dark
    } else {
        workspace::settings::Appearance::Light
    };
    assert_eq!(saved.appearance, Some(expected));
}

fn act(h: &Harness, cx: &mut TestAppContext, action: impl gpui_kit::Action) {
    h.ui(cx, |_, window, cx| window.dispatch_action(action.boxed_clone(), cx));
}

fn widths(h: &Harness, cx: &mut TestAppContext) -> Vec<f32> {
    cx.update(|cx| h.workspace.read(cx).pane_widths())
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
}

#[gpui_kit::test]
fn panes_split_in_half_and_resize(cx: &mut TestAppContext) {
    let h = setup(cx, "widths");
    let a = h.file("a.md", "# A\n");
    let b = h.file("b.md", "# B\n");
    h.open(cx, &a);
    h.open(cx, &b);
    act(&h, cx, SplitRight);
    assert!(close(&widths(&h, cx), &[0.5, 0.5]));

    // Dragging the divider sets it; it stops short of squeezing a pane away.
    h.ui(cx, |workspace, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.set_divider(0, 0.7, 0.1, cx))
    });
    assert!(close(&widths(&h, cx), &[0.7, 0.3]));
    h.ui(cx, |workspace, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.set_divider(0, 0.99, 0.1, cx))
    });
    assert!(close(&widths(&h, cx), &[0.9, 0.1]));

    act(&h, cx, EqualizePanes);
    assert!(close(&widths(&h, cx), &[0.5, 0.5]));

    // The right pane is active after the split; widening it takes from the left.
    act(&h, cx, WidenPane);
    assert!(close(&widths(&h, cx), &[0.45, 0.55]));

    // Closing a pane hands its width back.
    act(&h, cx, ClosePane);
    assert!(close(&widths(&h, cx), &[1.0]));
    assert_eq!(titles(&h, cx), vec![vec!["a.md"]]);
}

#[gpui_kit::test]
fn tabs_move_between_panes_by_keyboard(cx: &mut TestAppContext) {
    let h = setup(cx, "move-keys");
    let a = h.file("a.md", "# A\n");
    let b = h.file("b.md", "# B\n");
    let c = h.file("c.md", "# C\n");
    h.open(cx, &a);
    h.open(cx, &b);
    h.open(cx, &c);

    // With nothing on the right, moving right splits.
    act(&h, cx, MoveTabToNextPane);
    assert_eq!(titles(&h, cx), vec![vec!["a.md", "b.md"], vec!["c.md"]]);
    // And back again; the emptied pane closes.
    act(&h, cx, MoveTabToPreviousPane);
    assert_eq!(titles(&h, cx), vec![vec!["a.md", "b.md", "c.md"]]);

    // Ctrl+1 goes to the first tab, Ctrl+9 to the last.
    act(&h, cx, ActivateTab(0));
    cx.update(|cx| assert_eq!(h.workspace.read(cx).active_index(), 0));
    act(&h, cx, ActivateTab(usize::MAX));
    cx.update(|cx| assert_eq!(h.workspace.read(cx).active_index(), 2));

    act(&h, cx, ActivateTab(0));
    act(&h, cx, MoveTabForward);
    assert_eq!(titles(&h, cx), vec![vec!["b.md", "a.md", "c.md"]]);
}

#[gpui_kit::test]
fn the_outline_can_be_walked_by_keyboard(cx: &mut TestAppContext) {
    let h = setup(cx, "outline-keys");
    let doc = h.file("doc.md", "# One\n\ntext\n\n## Two\n\nmore\n\n## Three\n");
    h.open(cx, &doc);

    act(&h, cx, FocusOutline);
    cx.update(|cx| assert_eq!(h.workspace.read(cx).sidebar_cursor(), Some(0)));
    act(&h, cx, SidebarDown);
    act(&h, cx, SidebarDown);
    act(&h, cx, SidebarConfirm);
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.sidebar_cursor(), None);
        let editor = workspace.active_editor().unwrap();
        let editor = editor.read(cx);
        let current = editor.current_heading().unwrap();
        let text = editor.outline().into_iter().find(|i| i.id == current).unwrap().text;
        assert_eq!(text, "Three");
    });
}

#[gpui_kit::test]
fn f1_shows_and_hides_the_shortcuts(cx: &mut TestAppContext) {
    let h = setup(cx, "f1");
    act(&h, cx, ToggleShortcuts);
    cx.update(|cx| assert!(h.workspace.read(cx).shortcuts_are_shown()));
    act(&h, cx, ToggleShortcuts);
    cx.update(|cx| assert!(!h.workspace.read(cx).shortcuts_are_shown()));
}

#[gpui_kit::test]
fn an_html_file_opens_as_a_page_with_its_headings(cx: &mut TestAppContext) {
    let h = setup(cx, "html");
    let path = h.file(
        "page.html",
        "<html><head><title>x</title><style>h1{color:red}</style></head><body><h1>Report</h1>         <p>Plain and <strong>bold</strong>.</p><h2>Details</h2><ul><li>item</li></ul></body></html>",
    );
    h.open(cx, &path);
    cx.update(|cx| {
        let workspace = h.workspace.read(cx);
        assert_eq!(workspace.tab_titles()[0].as_ref(), "page.html");
        assert!(workspace.active_editor().is_none(), "a page, not a document to edit");
        let page = workspace.active_html().expect("an HTML tab");
        let headings: Vec<_> = page.read(cx).headings().iter().map(|h| (h.level, h.text.clone())).collect();
        assert_eq!(headings, vec![(1, "Report".to_string()), (2, "Details".to_string())]);
    });
    // The page is shown as it is; nothing is ever written back.
    h.ui(cx, |_, window, cx| window.dispatch_action(SaveFile.boxed_clone(), cx));
    assert!(std::fs::read_to_string(&path).unwrap().contains("<style>h1{color:red}</style>"));
}

#[cfg(windows)]
#[gpui_kit::test]
fn a_pdf_opens_in_a_viewer_and_keeps_its_comments_beside_it(cx: &mut TestAppContext) {
    let h = setup(cx, "pdf");
    let path = h.dir.join("paper.pdf");
    std::fs::write(&path, riffly::pdf::render::sample_pdf(&[(612, 792), (612, 792), (612, 792)]))
        .unwrap();
    h.open(cx, &path);
    // Opening happens in the background; let it finish.
    for _ in 0..20 {
        cx.run_until_parked();
        if cx.update(|cx| h.workspace.read(cx).active_pdf().is_some_and(|p| !p.read(cx).is_loading())) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let pdf = cx.update(|cx| h.workspace.read(cx).active_pdf()).expect("a PDF tab");
    cx.update(|cx| {
        let view = pdf.read(cx);
        assert_eq!(view.error(), None);
        assert_eq!(view.page_count(), 3);
        assert_eq!(h.workspace.read(cx).tab_titles()[0].as_ref(), "paper.pdf");
    });

    // Comment on page 2, the way a double-click would.
    h.ui(cx, |_, window, cx| {
        pdf.update(cx, |pdf, cx| {
            pdf.add_comment_at(1, 0.5, 0.25, window, cx);
        })
    });
    for ch in "too vague".chars() {
        let ch = ch.to_string();
        h.ui(cx, |_, window, cx| window.input(&ch, cx));
    }
    h.ui(cx, |_, window, cx| window.press("enter", cx));

    let saved = riffly::pdf::load_comments(&path);
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].page, 1);
    assert_eq!(saved[0].notes[0].body, "too vague");

    // Esc closes it; Ctrl+Alt+N brings it back; Ctrl+Alt+R resolves it.
    h.ui(cx, |_, window, cx| window.press("escape", cx));
    cx.update(|cx| assert_eq!(pdf.read(cx).open_comment(), None));
    h.ui(cx, |_, window, cx| window.press("secondary-alt-n", cx));
    cx.update(|cx| assert_eq!(pdf.read(cx).open_comment(), Some(saved[0].id)));
    h.ui(cx, |_, window, cx| window.press("secondary-alt-r", cx));
    assert!(riffly::pdf::load_comments(&path)[0].resolved);

    // Reopening the PDF reads the comments back.
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.close_tab(0, window, cx))
    });
    h.open(cx, &path);
    let pdf = cx.update(|cx| h.workspace.read(cx).active_pdf()).unwrap();
    cx.update(|cx| assert_eq!(pdf.read(cx).comments().len(), 1));
}

#[gpui_kit::test]
fn closing_remembers_where_the_window_was(cx: &mut TestAppContext) {
    let h = setup(cx, "placement");
    let expected = h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, _| workspace.remember_window(window));
        window.window_bounds()
    });
    let saved = workspace::settings::Settings::load(Some(&h.dir.join("config")))
        .window
        .expect("the placement is saved");
    let bounds = match expected {
        gpui_kit::WindowBounds::Windowed(bounds) => bounds,
        gpui_kit::WindowBounds::Maximized(bounds) | gpui_kit::WindowBounds::Fullscreen(bounds) => bounds,
    };
    assert_eq!(saved.width, f32::from(bounds.size.width));
    assert_eq!(saved.height, f32::from(bounds.size.height));
    assert_eq!(saved.x, f32::from(bounds.origin.x));
    assert_eq!(saved.y, f32::from(bounds.origin.y));
}

#[gpui_kit::test]
fn resizing_the_window_saves_its_place_without_closing(cx: &mut TestAppContext) {
    let h = setup(cx, "resize");
    h.ui(cx, |workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.guard_window_close(window, cx))
    });
    let config = h.dir.join("config");
    assert!(workspace::settings::Settings::load(Some(&config)).window.is_none());

    let visual = gpui_kit::VisualTestContext::from_window(h.window, cx);
    visual.simulate_resize(size(px(1234.), px(777.)));
    // Saved once the window has settled, not on every step of a drag.
    cx.executor().advance_clock(std::time::Duration::from_millis(100));
    cx.run_until_parked();
    assert!(workspace::settings::Settings::load(Some(&config)).window.is_none());
    cx.executor().advance_clock(std::time::Duration::from_millis(500));
    cx.run_until_parked();

    let saved = workspace::settings::Settings::load(Some(&config))
        .window
        .expect("the place is saved");
    assert_eq!((saved.width, saved.height), (1234., 777.));
}

#[gpui_kit::test]
fn the_welcome_document_is_for_the_first_start_only(cx: &mut TestAppContext) {
    let h = setup(cx, "welcome");
    let first = h.ui(cx, |workspace, _, cx| workspace.update(cx, |w, _| w.take_first_run()));
    let second = h.ui(cx, |workspace, _, cx| workspace.update(cx, |w, _| w.take_first_run()));
    assert!(first, "a fresh install welcomes");
    assert!(!second, "and only once");
    assert!(workspace::settings::Settings::load(Some(&h.dir.join("config"))).welcomed);
}

#[gpui_kit::test]
fn colour_themes_switch_and_are_remembered(cx: &mut TestAppContext) {
    use gpui_kit::component::{Theme, ThemeMode};
    let h = setup(cx, "themes");
    cx.update(|cx| riffly::editor::theme::apply_template_palette(cx).unwrap());

    // Every theme loads, light and dark, and sets its own page colour.
    for theme in riffly::editor::theme::COLOR_THEMES {
        h.ui(cx, |workspace, _, cx| {
            workspace.update(cx, |workspace, cx| workspace.set_color_theme(theme.id, cx))
        });
        let light = cx.update(|cx| {
            Theme::change(ThemeMode::Light, None, cx);
            Theme::global(cx).background
        });
        let dark = cx.update(|cx| {
            Theme::change(ThemeMode::Dark, None, cx);
            Theme::global(cx).background
        });
        assert_ne!(light, dark, "{} has two sides", theme.name);
    }

    h.ui(cx, |workspace, _, cx| {
        workspace.update(cx, |workspace, cx| workspace.set_color_theme("nord", cx))
    });
    cx.update(|cx| {
        Theme::change(ThemeMode::Dark, None, cx);
        assert_eq!(Theme::global(cx).background, gpui_kit::rgb(0x2e3440).into(), "Nord's page");
    });
    let saved = workspace::settings::Settings::load(Some(&h.dir.join("config")));
    assert_eq!(saved.color_theme.as_deref(), Some("nord"));

    // Ctrl+Alt+T moves on to the next one.
    h.ui(cx, |workspace, _, cx| workspace.update(cx, |workspace, cx| workspace.next_color_theme(cx)));
    cx.update(|cx| assert_eq!(h.workspace.read(cx).color_theme().id, "one"));
}
