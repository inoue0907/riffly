//! Turning PDF pages into pictures.
//!
//! On Windows this is the system's own PDF engine (`Windows.Data.Pdf`), so
//! nothing has to be shipped beside the executable. Elsewhere a PDF cannot
//! be opened yet, and [`Document::open`] says so.

use std::path::Path;

/// Width pages are drawn at, before Windows scales it by the display's
/// scale factor: sharp at the widths the viewer shows them, without holding
/// more than it needs.
pub const RENDER_WIDTH: u32 = 1100;

#[cfg(windows)]
mod imp {
    use std::path::Path;

    use anyhow::{Context as _, Result};
    use windows::Data::Pdf::{PdfDocument, PdfPageRenderOptions};
    use windows::Storage::StorageFile;
    use windows::Storage::Streams::{DataReader, InMemoryRandomAccessStream};
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::HSTRING;

    /// WinRT needs the calling thread in an apartment; background threads
    /// join the process-wide multithreaded one. Doing it twice is harmless.
    fn enter_apartment() {
        // SAFETY: initialising COM on the current thread has no
        // preconditions; a second call only returns S_FALSE.
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    }

    #[derive(Clone)]
    pub struct Document {
        pdf: PdfDocument,
    }

    impl Document {
        pub fn open(path: &Path) -> Result<Self> {
            enter_apartment();
            let path = std::path::absolute(path)?;
            let file = StorageFile::GetFileFromPathAsync(&HSTRING::from(path.as_os_str()))?
                .join()
                .context("the file could not be read")?;
            let pdf = PdfDocument::LoadFromFileAsync(&file)?
                .join()
                .context("this is not a PDF Windows can open (is it password-protected?)")?;
            Ok(Self { pdf })
        }

        /// Each page's size in points, width by height.
        pub fn page_sizes(&self) -> Result<Vec<(f32, f32)>> {
            enter_apartment();
            (0..self.pdf.PageCount()?)
                .map(|ix| {
                    let size = self.pdf.GetPage(ix)?.Size()?;
                    Ok((size.Width, size.Height))
                })
                .collect()
        }

        /// One page as PNG bytes, `width` pixels wide.
        pub fn render(&self, page: usize, width: u32) -> Result<Vec<u8>> {
            enter_apartment();
            let page = self.pdf.GetPage(page as u32)?;
            let stream = InMemoryRandomAccessStream::new()?;
            let options = PdfPageRenderOptions::new()?;
            options.SetDestinationWidth(width)?;
            page.RenderWithOptionsToStreamAsync(&stream, &options)?.join()?;

            let size = stream.Size()? as u32;
            let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0)?)?;
            reader.LoadAsync(size)?.join()?;
            let mut bytes = vec![0u8; size as usize];
            reader.ReadBytes(&mut bytes)?;
            Ok(bytes)
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::Path;

    use anyhow::{Result, bail};

    #[derive(Clone)]
    pub struct Document;

    impl Document {
        pub fn open(_: &Path) -> Result<Self> {
            bail!("PDF files can be viewed on Windows only for now")
        }

        pub fn page_sizes(&self) -> Result<Vec<(f32, f32)>> {
            Ok(Vec::new())
        }

        pub fn render(&self, _: usize, _: u32) -> Result<Vec<u8>> {
            bail!("PDF files can be viewed on Windows only for now")
        }
    }
}

pub use imp::Document;

/// Open a PDF and read its page sizes in one go, for a background task.
pub fn open(path: &Path) -> anyhow::Result<(Document, Vec<(f32, f32)>)> {
    let document = Document::open(path)?;
    let sizes = document.page_sizes()?;
    Ok((document, sizes))
}

/// A tiny, valid PDF with one page per entry of `pages` (width, height in
/// points), each saying "Page n". For tests.
#[doc(hidden)]
pub fn sample_pdf(pages: &[(u32, u32)]) -> Vec<u8> {
    let mut objects: Vec<String> = Vec::new();
    let n = pages.len();
    // 1: catalog, 2: pages, 3: font, then a page and a content stream each.
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 4 + i * 2)).collect();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    objects.push(format!("<< /Type /Pages /Kids [{}] /Count {} >>", kids.join(" "), n));
    objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into());
    for (i, (w, h)) in pages.iter().enumerate() {
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] \
             /Resources << /Font << /F1 3 0 R >> >> /Contents {} 0 R >>",
            5 + i * 2
        ));
        let text = format!("BT /F1 24 Tf 20 {} Td (Page {}) Tj ET", h.saturating_sub(40), i + 1);
        objects.push(format!("<< /Length {} >>\nstream\n{text}\nendstream", text.len()));
    }
    let mut out = b"%PDF-1.4\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1)
            .as_bytes(),
    );
    out
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn draws_every_page_of_a_pdf() {
        let dir = std::env::temp_dir().join(format!("riffly-render-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sample.pdf");
        std::fs::write(&path, sample_pdf(&[(612, 792), (400, 300)])).unwrap();

        let (document, sizes) = open(&path).expect("the sample opens");
        assert_eq!(sizes.len(), 2);
        // Sizes come in device-independent pixels (96 per inch), not
        // points; what the viewer relies on is the shape.
        assert!((sizes[1].0 / sizes[1].1 - 4. / 3.).abs() < 0.01, "{sizes:?}");
        assert!((sizes[0].1 / sizes[0].0 - 792. / 612.).abs() < 0.01, "{sizes:?}");

        let png = document.render(1, 800).expect("the page draws");
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "it is a PNG");
        // IHDR holds the width and height, big-endian, at bytes 16..24.
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        // Windows scales the width by the display's scale factor.
        assert!(width >= 800, "{width}");
        assert!((width as f32 / height as f32 - 4. / 3.).abs() < 0.01, "{width}x{height}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_a_pdf_says_so() {
        let dir = std::env::temp_dir().join(format!("riffly-render-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("broken.pdf");
        std::fs::write(&path, b"not a pdf").unwrap();
        assert!(open(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
