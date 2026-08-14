use image::RgbImage;
use std::path::Path;

/// Decodes a webp file directly into a scaled RGB buffer of the target size
/// using libwebp's built-in scaler. Skips the pure-Rust decoder and the
/// separate resize pass entirely, keeping webp → quadrant well under the
/// display budget even at large terminal sizes.
pub fn decode_scaled(path: &Path, out_w: u32, out_h: u32) -> Option<RgbImage> {
    let data = std::fs::read(path).ok()?;
    let (w, h) = (out_w as i32, out_h as i32);
    if w <= 0 || h <= 0 {
        return None;
    }
    unsafe {
        let mut config = std::mem::zeroed::<libwebp_sys::WebPDecoderConfig>();
        if !libwebp_sys::WebPInitDecoderConfig(&mut config) {
            return None;
        }
        config.options.use_scaling = 1;
        config.options.scaled_width = w;
        config.options.scaled_height = h;
        config.output.colorspace = libwebp_sys::WEBP_CSP_MODE::MODE_RGB;

        let ret = libwebp_sys::WebPDecode(data.as_ptr(), data.len(), &mut config);
        if ret != libwebp_sys::VP8StatusCode::VP8_STATUS_OK {
            libwebp_sys::WebPFreeDecBuffer(&mut config.output);
            return None;
        }
        let buf = &config.output;
        let width = buf.width as usize;
        let height = buf.height as usize;
        let stride = buf.u.RGBA.stride as usize;
        let mut raw = Vec::with_capacity(width * height * 3);
        let src = std::slice::from_raw_parts(buf.u.RGBA.rgba, stride * height);
        for row in 0..height {
            let start = row * stride;
            raw.extend_from_slice(&src[start..start + width * 3]);
        }
        libwebp_sys::WebPFreeDecBuffer(&mut config.output);
        RgbImage::from_raw(width as u32, height as u32, raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::RgbImage;

    fn encode_webp_lossless(rgb: &RgbImage) -> Vec<u8> {
        unsafe {
            let mut out: *mut u8 = std::ptr::null_mut();
            let len = libwebp_sys::WebPEncodeLosslessRGB(
                rgb.as_raw().as_ptr(),
                rgb.width() as i32,
                rgb.height() as i32,
                rgb.width() as i32 * 3,
                &mut out,
            );
            assert!(len > 0 && !out.is_null(), "WebPEncodeLosslessRGB failed");
            let bytes = std::slice::from_raw_parts(out, len).to_vec();
            libwebp_sys::WebPFree(out as *mut std::ffi::c_void);
            bytes
        }
    }

    fn make_webp(width: u32, height: u32) -> (Vec<u8>, image::RgbImage) {
        let rgb = RgbImage::from_fn(width, height, |x, y| {
            let (r, g, b) = ((x * 255 / width) as u8, (y * 255 / height) as u8, 128);
            image::Rgb([r, g, b])
        });
        let bytes = encode_webp_lossless(&rgb);
        (bytes, rgb)
    }

    #[test]
    fn decode_scaled_matches_resized_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("splash.webp");
        let (bytes, rgb) = make_webp(640, 360);
        std::fs::write(&path, &bytes).unwrap();

        let out = decode_scaled(&path, 32, 18).expect("decode should succeed");
        assert_eq!(out.dimensions(), (32, 18));

        // Compare against a direct Triangle resize of the source.
        let expected = image::imageops::resize(&rgb, 32, 18, image::imageops::FilterType::Triangle);
        for (got, exp) in out.pixels().zip(expected.pixels()) {
            let d = |a: u8, b: u8| (a as i32 - b as i32).abs();
            assert!(
                d(got.0[0], exp.0[0]) <= 4
                    && d(got.0[1], exp.0[1]) <= 4
                    && d(got.0[2], exp.0[2]) <= 4,
                "pixel mismatch: got {:?} expected {:?}",
                got.0,
                exp.0,
            );
        }
    }

    #[test]
    fn decode_scaled_rejects_bad_input() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.webp");
        std::fs::write(&path, b"not a webp").unwrap();
        assert!(decode_scaled(&path, 32, 18).is_none());
    }
}
