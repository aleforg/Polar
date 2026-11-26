use std::{
    collections::VecDeque,
    ffi::CString,
    future::Future,
    io,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::Arc,
};

use gpui_kit::{App, Asset, Global, ImageCacheError, ImageSource, RenderImage};
use objc2::{AnyThread, rc::Retained, runtime::AnyObject};
use objc2_app_kit::NSImage;
use objc2_core_foundation::{CFRetained, CFType};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
    CGImageByteOrderInfo, kCGColorSpaceSRGB,
};
use objc2_foundation::{NSDictionary, NSMutableDictionary, NSNumber, NSRect, NSString, NSURL};

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C-unwind" {
    fn CGImageSourceCreateWithURL(
        url: &NSURL,
        options: Option<&NSDictionary<NSString, AnyObject>>,
    ) -> Option<NonNull<CFType>>;
    fn CGImageSourceCreateThumbnailAtIndex(
        source: &CFType,
        index: usize,
        options: &NSDictionary<NSString, AnyObject>,
    ) -> Option<NonNull<CGImage>>;
    static kCGImageSourceCreateThumbnailFromImageAlways: &'static NSString;
    static kCGImageSourceCreateThumbnailWithTransform: &'static NSString;
    static kCGImageSourceShouldCache: &'static NSString;
    static kCGImageSourceShouldCacheImmediately: &'static NSString;
    static kCGImageSourceThumbnailMaxPixelSize: &'static NSString;
}

const LIST_PIXELS: i32 = 80;
const RECENT_THUMBNAILS: usize = 64;

fn file_url(path: &Path) -> Option<Retained<NSURL>> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: ImageIO receives an NSURL created from a live, NUL-terminated path.
    Some(unsafe {
        NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
            NonNull::new(path.as_ptr().cast_mut())?,
            false,
            None,
        )
    })
}

fn downsample(url: &NSURL, max_pixels: i32) -> Option<CFRetained<CGImage>> {
    let options = NSMutableDictionary::new();
    let off = NSNumber::numberWithBool(false);
    let on = NSNumber::numberWithBool(true);
    let max_pixels = NSNumber::new_i32(max_pixels);
    // SAFETY: NSURL, NSDictionary and NSString are toll-free bridged to the ImageIO
    // argument types. The dictionaries contain the documented keys and value types.
    // Both Create functions return owned Core Foundation objects.
    unsafe {
        options.insert(kCGImageSourceShouldCache, off.as_ref());
        let source = CFRetained::from_raw(CGImageSourceCreateWithURL(url, Some(&options))?);
        options.insert(kCGImageSourceCreateThumbnailFromImageAlways, on.as_ref());
        options.insert(kCGImageSourceCreateThumbnailWithTransform, on.as_ref());
        options.insert(kCGImageSourceShouldCacheImmediately, on.as_ref());
        options.insert(kCGImageSourceThumbnailMaxPixelSize, max_pixels.as_ref());
        CGImageSourceCreateThumbnailAtIndex(&source, 0, &options)
            .map(|image| CFRetained::from_raw(image))
    }
}

/// Loads an editor preview at its maximum display size on the current screen.
pub(crate) fn editor_image(
    path: &Path,
    screen_width: f64,
    preview_height: f64,
    display_scale: f64,
) -> Option<Retained<NSImage>> {
    let url = file_url(path)?;
    let original = NSImage::initByReferencingURL(NSImage::alloc(), &url);
    let size = original.size();
    if !size.width.is_finite() || !size.height.is_finite() || size.width <= 0. || size.height <= 0.
    {
        return None;
    }
    let scale = (screen_width / size.width)
        .min(preview_height / size.height)
        .min(1.);
    let max_pixels = (size.width.max(size.height) * scale * display_scale).ceil() as i32;
    Some(
        downsample(&url, max_pixels.max(1)).map_or(original, |image| {
            NSImage::initWithCGImage_size(NSImage::alloc(), &image, size)
        }),
    )
}

/// Uses display-sized thumbnails for static formats while preserving animated attachments.
pub(crate) fn list_image(path: PathBuf) -> ImageSource {
    if !matches!(
        path.extension().and_then(|ext| ext.to_str()),
        Some("jpg" | "png")
    ) {
        return path.into();
    }
    ImageSource::Custom(Arc::new(move |window, cx| {
        let evicted = cx.default_global::<RecentThumbnails>().touch(&path);
        if let Some(evicted) = evicted {
            if let Some(Ok(image)) = window.get_asset::<Thumbnail>(&evicted, cx) {
                cx.drop_image(image, Some(window));
            }
            cx.remove_asset::<Thumbnail>(&evicted);
        }
        window.use_asset::<Thumbnail>(&path, cx)
    }))
}

#[derive(Default)]
struct RecentThumbnails(VecDeque<PathBuf>);

impl Global for RecentThumbnails {}

impl RecentThumbnails {
    fn touch(&mut self, path: &Path) -> Option<PathBuf> {
        if let Some(index) = self.0.iter().position(|recent| recent == path) {
            if let Some(path) = self.0.remove(index) {
                self.0.push_back(path);
            }
            return None;
        }
        self.0.push_back(path.to_owned());
        if self.0.len() > RECENT_THUMBNAILS {
            self.0.pop_front()
        } else {
            None
        }
    }
}

struct Thumbnail;

impl Asset for Thumbnail {
    type Source = PathBuf;
    type Output = Result<Arc<RenderImage>, ImageCacheError>;

    #[expect(
        clippy::manual_async_fn,
        reason = "async fn would capture GPUI's non-Send App"
    )]
    fn load(path: PathBuf, _: &mut App) -> impl Future<Output = Self::Output> + Send + 'static {
        async move {
            objc2::rc::autoreleasepool(|_| {
                let image = file_url(&path)
                    .and_then(|url| downsample(&url, LIST_PIXELS))
                    .and_then(|image| render_image(&image))
                    .ok_or_else(|| {
                        io::Error::other("Impossibile caricare l’anteprima dell’immagine")
                    })?;
                Ok(Arc::new(image))
            })
        }
    }
}

fn render_image(image: &CGImage) -> Option<RenderImage> {
    let width = CGImage::width(Some(image));
    let height = CGImage::height(Some(image));
    if width == 0 || height == 0 || width > LIST_PIXELS as usize || height > LIST_PIXELS as usize {
        return None;
    }
    let mut bgra = vec![0; width * height * 4];
    // SAFETY: Core Graphics provides a static sRGB color space name.
    let colorspace = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))?;
    // SAFETY: The buffer owns width * height pixels with four bytes per pixel.
    // It is exclusively accessed through the context until that context is dropped.
    let context = unsafe {
        CGBitmapContextCreate(
            bgra.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(&colorspace),
            CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0,
        )?
    };
    CGContext::draw_image(
        Some(&context),
        NSRect::new(
            Default::default(),
            objc2_foundation::NSSize::new(width as f64, height as f64),
        ),
        Some(image),
    );
    drop(context);
    // GPUI expects straight alpha and BGRA, as produced by its regular image loaders.
    for pixel in bgra.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha > 0 && alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    let buffer = image::RgbaImage::from_raw(width as u32, height as u32, bgra)?;
    Some(RenderImage::new([image::Frame::new(buffer)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsamples_to_the_preview_size_with_the_expected_pixel_order() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("image-preview.png");
        image::RgbaImage::from_fn(320, 160, |_, y| {
            if y < 80 {
                image::Rgba([255, 0, 0, 128])
            } else {
                image::Rgba([0, 0, 255, 255])
            }
        })
        .save(&path)
        .unwrap();
        let url = file_url(&path).unwrap();
        let image = downsample(&url, LIST_PIXELS).unwrap();
        assert_eq!(CGImage::width(Some(&image)), 80);
        assert_eq!(CGImage::height(Some(&image)), 40);
        let rendered = render_image(&image).unwrap();
        let pixels = rendered.as_bytes(0).unwrap();
        assert_eq!(&pixels[..4], &[0, 0, 255, 128]);
        assert_eq!(&pixels[pixels.len() - 4..], &[255, 0, 0, 255]);
    }
}
