// A release build is a windowed app: no console window opens beside it.
// Debug builds keep the console, where `eprintln!` diagnostics go.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use gpui_kit::assets::AllAssets;
use gpui_kit::component::Root;
use gpui_kit::*;
use riffly::editor::block::{BlockAttrs, BlockContent, types};
use riffly::editor::mark::{HighlightColor, Mark, MarkKind, MarkList};
use riffly::editor::{self, RifflyEditor};
use riffly::workspace::{self, Workspace};

fn marked_paragraph() -> BlockContent {
    let text = "Bold, italic, underline, strike, code and a link.";
    let span = |needle: &str| {
        let start = text.find(needle).expect("sample text");
        start..start + needle.len()
    };
    BlockContent::paragraph(text).with_marks(MarkList::from_marks(vec![
        Mark::new(MarkKind::Bold, span("Bold")),
        Mark::new(MarkKind::Italic, span("italic")),
        Mark::new(MarkKind::Underline, span("underline")),
        Mark::new(MarkKind::Strike, span("strike")),
        Mark::new(MarkKind::Code, span("code")),
        Mark::new(MarkKind::Highlight(Some(HighlightColor::Yellow)), span("and")),
        Mark::new(MarkKind::Link("https://tiptap.dev".into()), span("a link")),
    ]))
}

fn demo_document() -> Vec<BlockContent> {
    let code = "fn greet(name: &str) {\n    println!(\"hello, {name}\");\n}";

    vec![
        BlockContent::new(types::HEADING, "Welcome to Riffly").with_attrs(BlockAttrs::level(1)),
        BlockContent::paragraph(
            "Riffly is a block editor for quickly reading, reviewing and fixing \
             markdown, like the files an AI just wrote. Open one with Ctrl+O, \
             split the window with Ctrl+\\ and press F1 for every shortcut.",
        ),
        BlockContent::new(types::HEADING, "Inline formatting").with_attrs(BlockAttrs::level(2)),
        marked_paragraph(),
        BlockContent::new(types::HEADING, "Lists").with_attrs(BlockAttrs::level(2)),
        BlockContent::new(types::BULLET_LIST, "Type - or * to start a bullet list"),
        BlockContent::new(types::BULLET_LIST, "Tab nests, Shift-Tab lifts").with_indent(1),
        BlockContent::new(types::ORDERED_LIST, "Numbered lists start with 1."),
        BlockContent::new(types::TASK_LIST, "Finish the editor").with_attrs(BlockAttrs {
            checked: true,
            ..Default::default()
        }),
        BlockContent::new(types::TASK_LIST, "Ship it"),
        BlockContent::new(types::BLOCKQUOTE, "Type / anywhere to insert a block."),
        BlockContent::new(types::CODE_BLOCK, code).with_attrs(BlockAttrs::language("rust")),
        BlockContent::new(types::HEADING, "Tables").with_attrs(BlockAttrs::level(2)),
        editor::table_content(&[
            &["Name", "Role", "Notes"],
            &["Ada", "Author", "Wrote the first program"],
            &["Grace", "Compiler", "Coined the bug"],
        ]),
        BlockContent::new(types::HORIZONTAL_RULE, ""),
        BlockContent::new(types::CALLOUT, "Callouts hold a note worth keeping."),
        BlockContent::paragraph(""),
    ]
}

/// Development aid: `RIFFLY_DEMO=toolbar` or `RIFFLY_DEMO=slash` starts the
/// window with that surface open, so it can be reviewed without driving the
/// pointer.
fn open_demo_state(editor: &mut RifflyEditor, window: &mut Window, cx: &mut Context<RifflyEditor>) {
    if std::env::var("RIFFLY_DEMO").is_ok() {
        window.activate_window();
    }
    // The gutter aid is its own switch, so it cannot ride along with a demo
    // that was asked for something else.
    if std::env::var("RIFFLY_GUTTER").as_deref() == Ok("always") {
        editor.show_gutter_always();
    }
    match std::env::var("RIFFLY_DEMO").as_deref() {
        Ok("toolbar") => editor.select_text_in_block(3, 0..12, window, cx),
        Ok("focus") => editor.select_text_in_block(3, 5..5, window, cx),
        Ok("caret") => {
            // Review aid: put the caret at a byte offset inside block 1, so
            // two runs can be compared pixel by pixel.
            let at: usize = std::env::var("RIFFLY_CARET")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(0);
            editor.select_text_in_block(1, at..at, window, cx);
        }
        Ok("comment") => {
            editor.select_text_in_block(3, 0..12, window, cx);
            editor.add_comment(window, cx);
        }
        Ok("slash") => {
            editor.focus_last_block(window, cx);
            editor.open_slash_menu(window, cx);
        }
        _ => {}
    }
}

/// The saved place as window bounds, fitted onto today's screens.
fn restore_bounds(placement: workspace::settings::Placement, cx: &App) -> Option<WindowBounds> {
    let screens: Vec<Bounds<Pixels>> = cx
        .displays()
        .iter()
        .map(|display| display.visible_bounds())
        .collect();
    let bounds = placement.fit(&screens)?;
    Some(if placement.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    })
}

/// A first window: 1100 × 860, or smaller on a smaller screen, centred.
fn default_bounds(cx: &App) -> WindowBounds {
    let (width, height) = cx
        .primary_display()
        .map(|display| {
            let area = display.visible_bounds();
            (f32::from(area.size.width) * 0.9, f32::from(area.size.height) * 0.9)
        })
        .unwrap_or((1100., 860.));
    WindowBounds::centered(size(px(width.min(1100.)), px(height.min(860.))), cx)
}

/// `--register` puts Riffly on the right-click menu and the "Open with" and
/// Default apps lists for Markdown, HTML and PDF files (this user only);
/// `--unregister` takes it all back out. Both exit without opening a window.
fn shell_command() -> Option<i32> {
    let flag = std::env::args().nth(1)?;
    let result = match flag.as_str() {
        "--register" => std::env::current_exe()
            .map_err(anyhow::Error::from)
            .and_then(|exe| riffly::shell::register(&exe))
            .map(|()| {
                if std::env::args().any(|arg| arg == "--open-settings") {
                    riffly::shell::open_default_apps_settings();
                }
            }),
        "--unregister" => riffly::shell::unregister(),
        _ => return None,
    };
    Some(match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("riffly {flag}: {error:#}");
            1
        }
    })
}

fn main() {
    if let Some(code) = shell_command() {
        std::process::exit(code);
    }
    let app = gpui_kit::application().with_assets(AllAssets);

    app.run(move |cx| {
        gpui_kit::init(cx);

        // Review aid: `RIFFLY_THEME=dark` starts in the dark palette, and
        // `RIFFLY_FONT_SIZE=<px>` changes the base the document is measured
        // in — the whole editor scales with it, headings and gutters and
        // tables together.
        // Otherwise the palette is the one picked last time, or the
        // system's when none has been picked.
        let config_dir = workspace::default_config_dir();
        let saved = workspace::settings::Settings::load(config_dir.as_deref()).appearance;
        let dark = match std::env::var("RIFFLY_THEME").as_deref() {
            Ok("dark") => true,
            Ok(_) => false,
            Err(_) => match saved {
                Some(workspace::settings::Appearance::Dark) => true,
                Some(workspace::settings::Appearance::Light) => false,
                None => matches!(
                    cx.window_appearance(),
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                ),
            },
        };
        gpui_kit::component::Theme::change(
            if dark {
                gpui_kit::component::ThemeMode::Dark
            } else {
                gpui_kit::component::ThemeMode::Light
            },
            None,
            cx,
        );
        if let Ok(size) = std::env::var("RIFFLY_FONT_SIZE")
            && let Ok(size) = size.parse::<f32>()
        {
            cx.global_mut::<gpui_kit::component::Theme>().font_size = px(size);
        }

        editor::init(cx);
        editor::theme::init_appearance_actions(cx);
        workspace::init(cx);
        riffly::pdf::init(cx);

        // Files named on the command line open in tabs; with none, the
        // welcome document does the first time and an empty page after.
        let files: Vec<std::path::PathBuf> = std::env::args_os()
            .skip(1)
            .map(std::path::PathBuf::from)
            .collect();

        // The document wears the template's own palette unless the reader
        // asks for the framework's: `RIFFLY_PALETTE=kit`.
        let color_theme = workspace::settings::Settings::load(config_dir.as_deref())
            .color_theme
            .unwrap_or_else(|| "riffly".into());
        if std::env::var("RIFFLY_PALETTE").as_deref() != Ok("kit")
            && let Err(error) = editor::theme::apply_color_theme(&color_theme, cx)
        {
            eprintln!("the template palette could not be loaded: {error}");
        }

        // Where the window was last time, if that is still on a screen. On
        // Windows the system's own record of it is put back once the window
        // exists (it knows each monitor's scale); the window opens hidden
        // until then so it does not jump.
        let placement = workspace::settings::Settings::load(config_dir.as_deref()).window;
        riffly::log::write(format!(
            "start {} from {:?}; settings {:?}; saved place {:?}",
            env!("CARGO_PKG_VERSION"),
            std::env::current_exe().ok(),
            config_dir,
            placement.and_then(|p| p.native)
        ));
        let native = placement
            .and_then(|placement| placement.native)
            .filter(riffly::placement::is_on_a_monitor);
        let window_bounds = placement
            .and_then(|placement| restore_bounds(placement, cx))
            .unwrap_or_else(|| default_bounds(cx));
        let options = WindowOptions {
            window_bounds: Some(window_bounds),
            show: native.is_none(),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    let mut workspace = Workspace::new(config_dir.clone(), cx);
                    let first_run = workspace.take_first_run();
                    if files.is_empty() && (first_run || std::env::var("RIFFLY_DEMO").is_ok()) {
                        let editor = workspace.add_tab(demo_document(), None, window, cx);
                        editor.update(cx, |editor, cx| open_demo_state(editor, window, cx));
                    } else if files.is_empty() {
                        workspace.new_tab(window, cx);
                    } else {
                        for file in files {
                            workspace.open_path(file, window, cx);
                        }
                    }
                    workspace.guard_window_close(window, cx);
                    workspace
                });
                // Shown by putting it in place; if that fails, show it anyway.
                if let Some(native) = native {
                    let applied = riffly::placement::apply(window, &native);
                    riffly::log::write(format!("restored place {native:?}: {applied}"));
                    if !applied {
                        window.activate_window();
                    }
                }
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
