//! Peak heap use of one call, in a test binary of its own so no other test allocates meanwhile.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};

use image::{DynamicImage, ImageBuffer, ImageFormat, Rgba};
use image_resize::config::ResizeConfig;
use image_resize::processing::{self, Format, ImageMetadata};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            let live = LIVE.fetch_add(layout.size(), Relaxed) + layout.size();
            PEAK.fetch_max(live, Relaxed);
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) };
        LIVE.fetch_sub(layout.size(), Relaxed);
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

#[test]
fn flattening_a_16_bit_png_for_jpeg_keeps_at_most_the_source_and_one_rgba8_copy() {
    let (w, h) = (1000, 1000);
    let src = ImageBuffer::from_fn(w, h, |x, y| {
        Rgba([(x * 65) as u16, (y * 65) as u16, 0, 40000])
    });
    let mut png = Vec::new();
    DynamicImage::ImageRgba16(src)
        .write_to(&mut Cursor::new(&mut png), ImageFormat::Png)
        .unwrap();
    let meta = ImageMetadata {
        output_format: Some(Format::Jpeg),
        target_width: NonZeroU32::new(100),
        target_height: NonZeroU32::new(100),
        ..Default::default()
    };
    let params = processing::resolve_params(&ResizeConfig::default(), &meta, Format::Png).unwrap();

    let base = LIVE.load(Relaxed);
    PEAK.store(base, Relaxed);
    processing::process_image(&png, &params).unwrap();
    let peak = PEAK.load(Relaxed) - base;
    // The 8 B/px source and its 4 B/px RGBA8 copy, with room for the codecs' buffers;
    // keeping the source through the 3 B/px RGB output as well came to 15 B/px.
    let px = (w * h) as usize;
    assert!(peak < px * 27 / 2, "peak {peak} B for {px} px");
}
