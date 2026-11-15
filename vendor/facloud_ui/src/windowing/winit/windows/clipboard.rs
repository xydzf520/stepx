use std::ops::Not;

use arboard::{self, Clipboard as WindowsClipboardInner};

use crate::{clipboard::ClipboardContent, Clipboard};

const CLIPBOARD_OCCUPIED_RETRIES: usize = 4;
const CLIPBOARD_OCCUPIED_BACKOFF: std::time::Duration = std::time::Duration::from_millis(5);

fn retry_on_clipboard_occupied<T>(
    mut op: impl FnMut() -> Result<T, arboard::Error>,
) -> Result<T, arboard::Error> {
    let mut retries = 0;
    loop {
        match op() {
            Err(arboard::Error::ClipboardOccupied) if retries < CLIPBOARD_OCCUPIED_RETRIES => {
                retries += 1;
                std::thread::sleep(CLIPBOARD_OCCUPIED_BACKOFF);
            }
            result => return result,
        }
    }
}

pub struct WindowsClipboard {
    inner: WindowsClipboardInner,
}

impl WindowsClipboard {
    pub fn new() -> Result<Self, arboard::Error> {
        Ok(Self {
            inner: retry_on_clipboard_occupied(WindowsClipboardInner::new)?,
        })
    }
}

impl Clipboard for WindowsClipboard {
    fn write(&mut self, contents: ClipboardContent) {
        if let Some(first_image) = contents.images.as_ref().and_then(|imgs| imgs.first()) {
            if let Some(arboard_image) =
                crate::clipboard_utils::decode_image_for_arboard(first_image)
            {
                if let Err(err) = retry_on_clipboard_occupied(|| {
                    self.inner.set().image(arboard::ImageData {
                        width: arboard_image.width,
                        height: arboard_image.height,
                        bytes: std::borrow::Cow::Borrowed(arboard_image.bytes.as_ref()),
                    })
                }) {
                    log::warn!("Unable to set clipboard image: {err:?}");
                }
                return;
            }
            log::warn!("Unable to decode ClipboardContent image for Windows clipboard write");
        }

        let set_result = if let Some(html) = &contents.html {
            retry_on_clipboard_occupied(|| self.inner.set().html(html, Some(&contents.plain_text)))
        } else {
            retry_on_clipboard_occupied(|| self.inner.set().text(&contents.plain_text))
        };

        if let Err(err) = set_result {
            if contents.html.is_some() {
                log::warn!("Unable to set clipboard HTML: {err:?}");
            } else {
                log::warn!("Unable to set clipboard text: {err:?}");
            }
        }
    }

    fn read(&mut self) -> ClipboardContent {
        let mut content = ClipboardContent {
            plain_text: retry_on_clipboard_occupied(|| self.inner.get().text()).unwrap_or_default(),
            ..Default::default()
        };

        // Try to get HTML content
        if let Ok(html) = retry_on_clipboard_occupied(|| self.inner.get().html()) {
            content.html = html.is_empty().not().then_some(html);
        }

        // Some environments provide HTML but do not provide a plaintext representation.
        // If that happens, derive a best-effort plaintext fallback from the HTML.
        if content.plain_text.trim().is_empty() {
            if let Some(html) = content.html.as_ref() {
                let derived = crate::clipboard_utils::strip_html_to_plain_text(html);
                if !derived.trim().is_empty() {
                    content.plain_text = derived;
                }
            }
        }

        // Get file paths.
        content.paths = retry_on_clipboard_occupied(|| self.inner.get().file_list())
            .ok()
            .map(|list| {
                list.into_iter()
                    .map(|p| p.to_string_lossy().to_string())
                    .collect()
            });

        // Try to get image content from clipboard (retrying transient ClipboardOccupied locks).
        content.images = match retry_on_clipboard_occupied(|| self.inner.get().image()) {
            Ok(arboard_image) => {
                let filename = crate::clipboard_utils::extract_filename_from_clipboard_content(
                    &content.html,
                    &content.plain_text,
                );
                crate::clipboard_utils::process_clipboard_image(&arboard_image, filename)
                    .map(|img| vec![img])
            }
            Err(arboard::Error::ContentNotAvailable) => None,
            Err(err) => {
                log::warn!("Unable to read image from clipboard: {err:?}");
                None
            }
        };

        content
    }
}

#[cfg(test)]
#[path = "clipboard_tests.rs"]
mod tests;
