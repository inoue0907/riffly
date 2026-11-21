//! Video shown inside a document: a markdown image whose source is a video,
//! such as `![clip](data:video/mp4;base64,…)` or `![clip](run.mp4)`.
//!
//! GPUI draws images, not video, so the system decoder (Media Foundation)
//! turns the video into frames on a thread of its own and hands them over
//! one at a time; the view shows the latest. Only one frame is held at once,
//! however long the video. The first frame stands in as the poster; a click
//! plays and pauses, and the video starts over when it ends.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{
    Context, InteractiveElement as _, IntoElement, ParentElement as _, Render, RenderImage,
    StatefulInteractiveElement as _, Styled as _, Task, Window, div, img, prelude::FluentBuilder as _,
};

use crate::editor::theme::ActiveEditorTheme as _;

/// Extensions read as video when a markdown image points at a file.
const VIDEO_EXTENSIONS: [&str; 4] = ["mp4", "m4v", "mov", "webm"];

/// Whether an image source is in fact a video.
pub fn is_video(src: &str) -> bool {
    if let Some(rest) = src.strip_prefix("data:") {
        return rest.starts_with("video/");
    }
    Path::new(src)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|v| v.eq_ignore_ascii_case(ext)))
}

/// The bytes of a video source: the payload of a base64 `data:` URL, or the
/// file it names.
pub fn video_bytes(src: &str) -> Result<Vec<u8>, String> {
    use base64::Engine as _;

    if let Some(rest) = src.strip_prefix("data:") {
        let (meta, payload) = rest.split_once(',').ok_or("The data URL has no comma")?;
        if !meta.ends_with(";base64") {
            return Err("Only base64 data URLs can hold a video".into());
        }
        // Long payloads are often wrapped; the line breaks are not data.
        let payload: String = payload.chars().filter(|c| !c.is_whitespace()).collect();
        return base64::engine::general_purpose::STANDARD
            .decode(payload)
            .map_err(|err| format!("The video's base64 is damaged ({err})"));
    }
    let path = src.strip_prefix("file:///").unwrap_or(src);
    std::fs::read(path).map_err(|err| format!("Could not read {path} ({err})"))
}

/// What the decoding thread hands the view.
enum Message {
    Frame(RenderImage),
    Failed(String),
}

/// One video, playing or paused.
pub struct VideoView {
    frame: Option<Arc<RenderImage>>,
    /// Frames replaced since the last draw, whose textures can be let go.
    stale: Vec<Arc<RenderImage>>,
    error: Option<String>,
    playing: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    _receive: Task<()>,
}

impl VideoView {
    pub fn new(src: &str, cx: &mut Context<Self>) -> Self {
        let playing = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        // Two frames in flight at most: the thread waits for the view rather
        // than piling frames up when drawing falls behind.
        let (tx, rx) = async_channel::bounded::<Message>(2);

        match video_bytes(src) {
            Ok(bytes) => {
                let (playing, stop) = (playing.clone(), stop.clone());
                let spawned = std::thread::Builder::new()
                    .name("riffly-video".into())
                    .spawn(move || {
                        if let Err(err) = decode::run(bytes, &tx, &playing, &stop) {
                            let _ = tx.send_blocking(Message::Failed(err));
                        }
                    });
                if let Err(err) = spawned {
                    crate::log::write(format!("video thread: {err}"));
                }
            }
            Err(err) => {
                let _ = tx.try_send(Message::Failed(err));
            }
        }

        let receive = cx.spawn(async move |this, cx| {
            while let Ok(message) = rx.recv().await {
                let alive = this.update(cx, |this, cx| {
                    match message {
                        Message::Frame(frame) => {
                            if let Some(old) = this.frame.replace(Arc::new(frame)) {
                                this.stale.push(old);
                            }
                        }
                        Message::Failed(err) => this.error = Some(err),
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        Self {
            frame: None,
            stale: Vec::new(),
            error: None,
            playing,
            stop,
            _receive: receive,
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing.load(Ordering::Relaxed)
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        self.playing.fetch_xor(true, Ordering::Relaxed);
        cx.notify();
    }
}

impl Drop for VideoView {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl Render for VideoView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A frame that has been replaced is not drawn again; give its texture back.
        for old in self.stale.drain(..) {
            let _ = window.drop_image(old);
        }
        let theme = cx.editor_theme().clone();

        let Some(frame) = self.frame.clone() else {
            let text = match &self.error {
                Some(err) => format!("This video cannot be played. {err}"),
                None => "Loading video…".to_string(),
            };
            return div()
                .id("video")
                .w_full()
                .h(theme.rems(7.5))
                .rounded(theme.radius)
                .border_1()
                .border_color(cx.theme().border)
                .flex()
                .items_center()
                .justify_center()
                .gap(theme.rems(0.375))
                .text_color(cx.theme().muted_foreground)
                .child(crate::editor::ui::icon("video", theme.rems(1.125), cx.theme().muted_foreground))
                .child(text)
                .into_any_element();
        };

        let playing = self.is_playing();
        // The box takes the column's width and the video's proportions; an
        // image sized by width alone keeps its own pixel height and would be
        // letterboxed in a box of the wrong shape.
        let size = frame.size(0);
        let ratio = size.width.0 as f32 / size.height.0.max(1) as f32;
        div()
            .id("video")
            .relative()
            .w_full()
            .aspect_ratio(ratio)
            .cursor_pointer()
            .child(img(frame).size_full().rounded(theme.radius))
            .when(!playing, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(theme.rems(3.))
                                .rounded_full()
                                .bg(gpui_kit::black().opacity(0.55))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(crate::editor::ui::icon(
                                    "play",
                                    theme.rems(1.5),
                                    gpui_kit::white(),
                                )),
                        ),
                )
            })
            .on_click(cx.listener(|this, _, _, cx| this.toggle(cx)))
            .into_any_element()
    }
}

#[cfg(windows)]
mod decode {
    //! Media Foundation's source reader, decoding to 32-bit BGR frames.

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    use gpui_kit::RenderImage;
    use windows::Win32::Media::MediaFoundation::*;
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};
    use windows::Win32::UI::Shell::SHCreateMemStream;
    use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PROPVARIANT_0, PROPVARIANT_0_0};
    use windows::Win32::System::Variant::VT_I8;
    use windows::core::{GUID, Interface as _};

    use super::Message;

    const VIDEO: u32 = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

    /// Decode `bytes` until told to stop, sending each frame as its time comes.
    pub(super) fn run(
        bytes: Vec<u8>,
        tx: &async_channel::Sender<Message>,
        playing: &AtomicBool,
        stop: &AtomicBool,
    ) -> Result<(), String> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED)
                .ok()
                .map_err(|err| err.message())?;
            let result = match MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET) {
                Ok(()) => {
                    let played = play(&bytes, tx, playing, stop).map_err(|err| err.message());
                    let _ = MFShutdown();
                    played
                }
                Err(err) => Err(err.message()),
            };
            CoUninitialize();
            match result {
                Ok(()) => Ok(()),
                // The view went away while a frame was on its way.
                Err(_) if stop.load(Ordering::Relaxed) || tx.is_closed() => Ok(()),
                Err(err) => Err(err),
            }
        }
    }

    unsafe fn play(
        bytes: &[u8],
        tx: &async_channel::Sender<Message>,
        playing: &AtomicBool,
        stop: &AtomicBool,
    ) -> windows::core::Result<()> {
        unsafe {
            let stream = SHCreateMemStream(Some(bytes)).ok_or_else(windows::core::Error::from_thread)?;
            let byte_stream = MFCreateMFByteStreamOnStream(&stream)?;
            let mut attributes = None;
            MFCreateAttributes(&mut attributes, 1)?;
            let attributes = attributes.ok_or_else(windows::core::Error::empty)?;
            // Lets the reader convert whatever the decoder gives into RGB32.
            attributes.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1)?;
            let reader = MFCreateSourceReaderFromByteStream(&byte_stream, &attributes)?;

            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(VIDEO, true)?;
            let wanted = MFCreateMediaType()?;
            wanted.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
            wanted.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)?;
            reader.SetCurrentMediaType(VIDEO, None, &wanted)?;

            let actual = reader.GetCurrentMediaType(VIDEO)?;
            let size = actual.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let (coded_width, coded_height) = ((size >> 32) as u32, size as u32);
            // A decoder pads the picture to whole macroblocks; the aperture
            // says which part of it is the picture.
            let (crop_x, crop_y, width, height) = aperture(&actual)
                .filter(|&(x, y, w, h)| w > 0 && h > 0 && x + w <= coded_width && y + h <= coded_height)
                .unwrap_or((0, 0, coded_width, coded_height));
            let stride_hint = actual
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .map(|stride| stride as i32)
                .unwrap_or(coded_width as i32 * 4);

            // Where the clock started, counted so that `start + time` is when
            // the frame stamped `time` is due.
            let mut start: Option<Instant> = None;
            let mut shown_any = false;
            loop {
                if stop.load(Ordering::Relaxed) || tx.is_closed() {
                    return Ok(());
                }
                // Paused once the poster is up: wait, and restart the clock
                // from the current frame when play resumes.
                if shown_any && !playing.load(Ordering::Relaxed) {
                    start = None;
                    std::thread::sleep(Duration::from_millis(30));
                    continue;
                }

                let mut flags = 0u32;
                let mut time = 0i64;
                let mut sample = None;
                reader.ReadSample(VIDEO, 0, None, Some(&mut flags), Some(&mut time), Some(&mut sample))?;
                if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                    if !shown_any {
                        return Err(windows::core::Error::new(
                            windows::core::HRESULT(0x80004005u32 as i32),
                            "The video has no frames",
                        ));
                    }
                    // From the top again.
                    // A position is a VT_I8 in 100 ns units; zeroed, its
                    // value is already 0.
                    let zero = PROPVARIANT {
                        Anonymous: PROPVARIANT_0 {
                            Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                                vt: VT_I8,
                                ..Default::default()
                            }),
                        },
                    };
                    reader.SetCurrentPosition(&GUID::zeroed(), &zero)?;
                    start = None;
                    continue;
                }
                let Some(sample) = sample else { continue };

                let due = Duration::from_nanos(time.max(0) as u64 * 100);
                if shown_any {
                    let begun = *start.get_or_insert_with(|| Instant::now() - due);
                    let at = begun + due;
                    let now = Instant::now();
                    if at > now {
                        std::thread::sleep(at - now);
                    } else if now - at > Duration::from_millis(250) {
                        // Far behind (the machine was busy): catch the clock up
                        // rather than racing through the backlog.
                        start = Some(now - due);
                    }
                }

                let frame = to_frame(&sample, coded_width, coded_height, stride_hint, (crop_x, crop_y, width, height))?;
                if tx.send_blocking(Message::Frame(frame)).is_err() {
                    return Ok(());
                }
                shown_any = true;
            }
        }
    }

    /// The picture's part of the decoded frame, as x, y, width and height.
    unsafe fn aperture(media: &IMFMediaType) -> Option<(u32, u32, u32, u32)> {
        unsafe {
            let mut area = MFVideoArea::default();
            let bytes = std::slice::from_raw_parts_mut(
                (&mut area as *mut MFVideoArea).cast::<u8>(),
                std::mem::size_of::<MFVideoArea>(),
            );
            for key in [&MF_MT_MINIMUM_DISPLAY_APERTURE, &MF_MT_GEOMETRIC_APERTURE] {
                if media.GetBlob(key, bytes, None).is_ok() {
                    return Some((
                        area.OffsetX.value.max(0) as u32,
                        area.OffsetY.value.max(0) as u32,
                        area.Area.cx.max(0) as u32,
                        area.Area.cy.max(0) as u32,
                    ));
                }
            }
            None
        }
    }

    /// Copy a decoded sample into an image GPUI can draw (BGRA, top row first).
    unsafe fn to_frame(
        sample: &IMFSample,
        _coded_width: u32,
        coded_height: u32,
        stride_hint: i32,
        (crop_x, crop_y, width, height): (u32, u32, u32, u32),
    ) -> windows::core::Result<RenderImage> {
        unsafe {
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut pixels = vec![0u8; width as usize * height as usize * 4];

            let mut copy_rows = |first_row: *const u8, pitch: isize, available: usize| {
                for row in 0..height as usize {
                    let source_row = row + crop_y as usize;
                    if source_row >= coded_height as usize {
                        break;
                    }
                    let offset = source_row as isize * pitch + crop_x as isize * 4;
                    // The rows are walked by pitch, which is negative for a
                    // picture stored bottom row first.
                    if offset < -(available as isize) || offset as usize > available && pitch > 0 {
                        break;
                    }
                    let source = std::slice::from_raw_parts(first_row.offset(offset), width as usize * 4);
                    let target = &mut pixels[row * width as usize * 4..(row + 1) * width as usize * 4];
                    target.copy_from_slice(source);
                }
            };

            if let Ok(buffer_2d) = buffer.cast::<IMF2DBuffer>() {
                let mut first_row = std::ptr::null_mut();
                let mut pitch = 0i32;
                buffer_2d.Lock2D(&mut first_row, &mut pitch)?;
                copy_rows(first_row, pitch as isize, pitch.unsigned_abs() as usize * coded_height as usize);
                buffer_2d.Unlock2D()?;
            } else {
                let mut data = std::ptr::null_mut();
                let mut length = 0u32;
                buffer.Lock(&mut data, None, Some(&mut length))?;
                let pitch = stride_hint as isize;
                let first_row = if pitch < 0 {
                    data.offset((coded_height as isize - 1) * -pitch)
                } else {
                    data
                };
                copy_rows(first_row, pitch, length as usize);
                buffer.Unlock()?;
            }

            // RGB32 leaves the fourth byte undefined; the picture is opaque.
            for pixel in pixels.as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
            let image = image::RgbaImage::from_raw(width, height, pixels)
                .ok_or_else(windows::core::Error::empty)?;
            Ok(RenderImage::new([image::Frame::new(image)]))
        }
    }
}

#[cfg(not(windows))]
mod decode {
    use std::sync::atomic::AtomicBool;

    use super::Message;

    pub(super) fn run(
        _: Vec<u8>,
        _: &async_channel::Sender<Message>,
        _: &AtomicBool,
        _: &AtomicBool,
    ) -> Result<(), String> {
        Err("Video is played with the Windows decoder, which this system does not have.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_sources_are_told_from_images() {
        assert!(is_video("data:video/mp4;base64,AAAA"));
        assert!(is_video("clips/run.MP4"));
        assert!(is_video("C:/videos/run.webm"));
        assert!(!is_video("data:image/png;base64,AAAA"));
        assert!(!is_video("figure.png"));
    }

    #[test]
    fn a_base64_data_url_gives_its_bytes() {
        assert_eq!(video_bytes("data:video/mp4;base64,AAEC\nAw==").unwrap(), vec![0, 1, 2, 3]);
        assert!(video_bytes("data:video/mp4,raw").is_err());
    }
}
