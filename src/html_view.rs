//! An HTML tab: the page as a browser shows it, read-only.
//!
//! The page is drawn by WebView2, the Edge engine Windows ships, placed over
//! the tab's area as a child window. It is created the first time the tab
//! is drawn and destroyed with the tab, so the engine's memory is only spent
//! while an HTML page is open; the rest of Riffly is untouched by it.
//!
//! Keys pressed inside the page go to the page, so Riffly's own shortcuts
//! (closing tabs, switching panes, F1 …) are caught by a small script and
//! handed back to Riffly.

// Only Windows can show a page yet; elsewhere the page machinery sits idle.
#![cfg_attr(not(windows), allow(unused_imports, dead_code))]

use std::path::{Path, PathBuf};

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, Bounds, Context, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, SharedString,
    Styled as _, Window, canvas, div,
};

use crate::editor::block::types;
use crate::editor::theme::{ZoomIn, ZoomOut, ZoomReset};

pub const CONTEXT: &str = "HtmlView";

/// A heading of the page, as the outline lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PageHeading {
    pub level: u8,
    pub text: String,
}

/// Emitted when the page is ready or fails, so the shell can redraw.
pub struct PageChanged;

/// Keys a page hands back to Riffly: any with Ctrl or Alt, and F1. Copy,
/// select-all and find stay with the page.
const KEY_SCRIPT: &str = r#"
document.addEventListener('keydown', function (e) {
  var k = e.key;
  if (!(e.ctrlKey || e.altKey || k === 'F1')) return;
  if (e.ctrlKey && !e.altKey && !e.shiftKey && ['c','a','f','x','v'].indexOf(k.toLowerCase()) >= 0) return;
  if (['Control','Alt','Shift','Meta'].indexOf(k) >= 0) return;
  e.preventDefault();
  e.stopPropagation();
  window.ipc.postMessage(JSON.stringify({ key: k, code: e.code, ctrl: e.ctrlKey, alt: e.altKey, shift: e.shiftKey }));
}, true);
"#;

#[derive(serde::Deserialize)]
struct PageKey {
    key: String,
    code: String,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

/// The page key as a gpui keystroke, e.g. `ctrl-shift-w`.
fn keystroke(key: &PageKey) -> Option<String> {
    let name = match key.key.as_str() {
        "Tab" => "tab".to_string(),
        "PageDown" => "pagedown".into(),
        "PageUp" => "pageup".into(),
        "ArrowLeft" => "left".into(),
        "ArrowRight" => "right".into(),
        "ArrowUp" => "up".into(),
        "ArrowDown" => "down".into(),
        "Enter" => "enter".into(),
        "Escape" => "escape".into(),
        "Backspace" => "backspace".into(),
        "Delete" => "delete".into(),
        "Home" => "home".into(),
        "End" => "end".into(),
        f if f.len() <= 3 && f.starts_with('F') && f[1..].parse::<u8>().is_ok() => f.to_lowercase(),
        // With Shift held the key is the shifted character; the code says
        // which key it was (`Digit7`, `KeyW`, `Backslash` …).
        _ => match key.code.as_str() {
            code if code.starts_with("Key") => code[3..].to_lowercase(),
            code if code.starts_with("Digit") => code[5..].to_string(),
            "Backslash" | "IntlYen" | "IntlRo" => "\\".into(),
            "Minus" => "-".into(),
            "Equal" => "=".into(),
            "BracketLeft" => "[".into(),
            "BracketRight" => "]".into(),
            "Slash" => "/".into(),
            "Period" => ".".into(),
            "Comma" => ",".into(),
            _ => key.key.to_lowercase(),
        },
    };
    let mut parts = Vec::new();
    if key.ctrl {
        parts.push("ctrl");
    }
    if key.alt {
        parts.push("alt");
    }
    if key.shift {
        parts.push("shift");
    }
    let mut stroke = parts.join("-");
    if !stroke.is_empty() {
        stroke.push('-');
    }
    stroke.push_str(&name);
    Some(stroke)
}

/// A file path as a `file:///` URL the page can be loaded from, so its own
/// stylesheets and images, given relative to it, load too.
pub fn file_url(path: &Path) -> String {
    let absolute = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let text = absolute.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file:///");
    for byte in text.trim_start_matches('/').bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b':' | b'-' | b'_' | b'.' | b'~' => {
                url.push(byte as char)
            }
            _ => url.push_str(&format!("%{byte:02X}")),
        }
    }
    url
}

pub struct HtmlView {
    path: PathBuf,
    headings: Vec<PageHeading>,
    focus_handle: FocusHandle,
    error: Option<SharedString>,
    /// Shown while the shell shows this tab and nothing is laid over it.
    shown: bool,
    zoom: f64,
    #[cfg(windows)]
    page: Option<Page>,
}

#[cfg(windows)]
struct Page {
    webview: wry::WebView,
    // Kept alive as long as the webview it made.
    _context: wry::WebContext,
    bounds: Bounds<Pixels>,
    visible: bool,
}

impl EventEmitter<PageChanged> for HtmlView {}

impl Focusable for HtmlView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl HtmlView {
    pub fn open(path: PathBuf, cx: &mut Context<Self>) -> Self {
        // The outline: the page's headings, read the way the editor reads
        // HTML, in document order.
        let headings = std::fs::read(&path)
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            .map(|text| {
                crate::editor::html::parse(&text)
                    .into_iter()
                    .filter(|block| block.ty == types::HEADING)
                    .map(|block| PageHeading {
                        level: block.attrs.level.max(1),
                        text: block.text,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            path,
            headings,
            focus_handle: cx.focus_handle(),
            error: None,
            shown: true,
            zoom: 1.0,
            #[cfg(windows)]
            page: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn headings(&self) -> &[PageHeading] {
        &self.headings
    }

    pub fn error(&self) -> Option<&SharedString> {
        self.error.as_ref()
    }

    /// Whether the page's engine has been started.
    pub fn is_loaded(&self) -> bool {
        #[cfg(windows)]
        {
            self.page.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Show or hide the page; the shell hides it when its tab is not the
    /// one shown, or when something (the F1 sheet) is laid over it.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
        #[cfg(windows)]
        if let Some(page) = self.page.as_mut()
            && page.visible != shown
        {
            let _ = page.webview.set_visible(shown);
            page.visible = shown;
        }
    }

    /// Scroll the page to its n-th heading.
    pub fn reveal_heading(&mut self, index: usize) {
        #[cfg(windows)]
        if let Some(page) = &self.page {
            let _ = page.webview.evaluate_script(&format!(
                "(function(){{var h=document.querySelectorAll('h1,h2,h3,h4,h5,h6')[{index}];\
                 if(h)h.scrollIntoView({{behavior:'smooth',block:'start'}});}})()"
            ));
        }
        #[cfg(not(windows))]
        let _ = index;
    }

    fn set_zoom(&mut self, zoom: f64, cx: &mut Context<Self>) {
        self.zoom = zoom.clamp(0.5, 3.0);
        #[cfg(windows)]
        if let Some(page) = &self.page {
            let _ = page.webview.zoom(self.zoom);
        }
        cx.notify();
    }

    /// Give the page the keyboard, once it exists.
    pub fn focus_page(&self) {
        #[cfg(windows)]
        if let Some(page) = &self.page {
            let _ = page.webview.focus();
        }
    }

    /// Put the page where the tab's area landed this frame, starting the
    /// engine the first time.
    #[cfg(windows)]
    fn place(&mut self, bounds: Bounds<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        use wry::dpi::{LogicalPosition, LogicalSize};
        let rect = wry::Rect {
            position: LogicalPosition::new(f32::from(bounds.origin.x), f32::from(bounds.origin.y))
                .into(),
            size: LogicalSize::new(f32::from(bounds.size.width), f32::from(bounds.size.height))
                .into(),
        };
        if let Some(page) = self.page.as_mut() {
            if page.bounds != bounds {
                let _ = page.webview.set_bounds(rect);
                page.bounds = bounds;
            }
            if page.visible != self.shown {
                let _ = page.webview.set_visible(self.shown);
                page.visible = self.shown;
            }
            return;
        }
        if self.error.is_some() {
            return;
        }

        // Keys the page hands back arrive here and are pressed in Riffly.
        let (keys, received) = async_channel::unbounded::<String>();
        let data = std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("Riffly").join("WebView2"));
        let mut context = wry::WebContext::new(data);
        let built = wry::WebViewBuilder::new_with_web_context(&mut context)
            .with_url(file_url(&self.path))
            .with_bounds(rect)
            .with_visible(self.shown)
            .with_initialization_script(KEY_SCRIPT)
            .with_ipc_handler(move |request| {
                let _ = keys.try_send(request.body().clone());
            })
            .build_as_child(&*window);
        match built {
            Ok(webview) => {
                self.page = Some(Page {
                    webview,
                    _context: context,
                    bounds,
                    visible: self.shown,
                });
                crate::log::write(format!("html page opened: {}", self.path.display()));
                let focus = self.focus_handle.clone();
                cx.spawn_in(window, async move |this, cx| {
                    while let Ok(message) = received.recv().await {
                        let Ok(key) = serde_json::from_str::<PageKey>(&message) else { continue };
                        let Some(stroke) = keystroke(&key) else { continue };
                        let Ok(stroke) = gpui_kit::Keystroke::parse(&stroke) else { continue };
                        let alive = this.update_in(cx, |_, window, cx| {
                            // Pressed as if in the tab, so the shell's and the
                            // tab's bindings both see it.
                            window.focus(&focus, cx);
                            window.dispatch_keystroke(stroke, cx);
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            }
            Err(error) => {
                self.error = Some(format!("The page could not be shown: {error}").into());
                crate::log::write(format!("html page failed: {error}"));
            }
        }
        cx.emit(PageChanged);
    }
}

impl Render for HtmlView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().clone();
        let entity = cx.entity().downgrade();
        let probe = canvas(
            move |bounds, window, cx| {
                #[cfg(windows)]
                let _ = entity.update(cx, |this, cx| this.place(bounds, window, cx));
                #[cfg(not(windows))]
                let _ = (&entity, bounds, window, cx);
            },
            |_, _, _, _| {},
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        div()
            .id("html-view")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(colors.background)
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.set_zoom(this.zoom + 0.1, cx)))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| this.set_zoom(this.zoom - 0.1, cx)))
            .on_action(cx.listener(|this, _: &ZoomReset, _, cx| this.set_zoom(1.0, cx)))
            .child(probe)
            .children(self.error.clone().map(|error| {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(colors.danger)
                    .child(error)
            }))
            .when_not_windows(|this| {
                this.child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(colors.muted_foreground)
                        .child("HTML pages can be shown on Windows only for now"),
                )
            })
    }
}

/// `when` for the one platform difference in the view's layout.
trait WhenNotWindows: Sized {
    fn when_not_windows(self, f: impl FnOnce(Self) -> Self) -> Self;
}

impl<T> WhenNotWindows for T {
    fn when_not_windows(self, f: impl FnOnce(Self) -> Self) -> Self {
        if cfg!(windows) { self } else { f(self) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: &str, code: &str, ctrl: bool, alt: bool, shift: bool) -> PageKey {
        PageKey { key: key.into(), code: code.into(), ctrl, alt, shift }
    }

    #[test]
    fn page_keys_become_riffly_keystrokes() {
        assert_eq!(keystroke(&key("w", "KeyW", true, false, false)).unwrap(), "ctrl-w");
        assert_eq!(keystroke(&key("W", "KeyW", true, false, true)).unwrap(), "ctrl-shift-w");
        assert_eq!(keystroke(&key("Tab", "Tab", true, false, false)).unwrap(), "ctrl-tab");
        assert_eq!(keystroke(&key("\\", "Backslash", true, false, false)).unwrap(), "ctrl-\\");
        assert_eq!(keystroke(&key("&", "Digit7", true, false, true)).unwrap(), "ctrl-shift-7");
        assert_eq!(keystroke(&key("ArrowRight", "ArrowRight", true, true, false)).unwrap(), "ctrl-alt-right");
        assert_eq!(keystroke(&key("F1", "F1", false, false, false)).unwrap(), "f1");
    }

    #[test]
    fn paths_become_file_urls() {
        let url = file_url(Path::new(r"C:\Users\me\My Docs\page 1.html"));
        assert_eq!(url, "file:///C:/Users/me/My%20Docs/page%201.html");
    }
}
