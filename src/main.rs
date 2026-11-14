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

fn main() {
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

        // Files named on the command line open in tabs; with none, the demo
        // document does.
        let files: Vec<std::path::PathBuf> = std::env::args_os()
            .skip(1)
            .map(std::path::PathBuf::from)
            .collect();

        // The document wears the template's own palette unless the reader
        // asks for the framework's: `RIFFLY_PALETTE=kit`.
        if std::env::var("RIFFLY_PALETTE").as_deref() != Ok("kit")
            && let Err(error) = editor::theme::apply_template_palette(cx)
        {
            eprintln!("the template palette could not be loaded: {error}");
        }

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1100.), px(860.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| {
                    let mut workspace = Workspace::new(config_dir.clone(), cx);
                    if files.is_empty() {
                        let editor = workspace.add_tab(demo_document(), None, window, cx);
                        editor.update(cx, |editor, cx| open_demo_state(editor, window, cx));
                    } else {
                        for file in files {
                            workspace.open_path(file, window, cx);
                        }
                    }
                    workspace.guard_window_close(window, cx);
                    workspace
                });
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
