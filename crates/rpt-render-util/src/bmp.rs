//! Minimal BMP decoder for the render backend. Crystal stores embedded OLE bitmaps as uncompressed
//! Windows BMPs, which the backend's image library does not handle, so they decode here.

/// Decode an uncompressed (BI_RGB) 24- or 32-bit Windows BMP to top-down straight RGBA8 +
/// dimensions. Handles the common `BITMAPINFOHEADER` (or larger) layout Crystal emits for embedded
/// bitmaps; paletted, RLE, and bitfield BMPs return `None` (the caller then skips the image) rather
/// than being decoded approximately — a refused image is a visible gap, a mis-decoded one is not. A
/// 32-bit BI_RGB BMP's fourth byte is undefined (not alpha), so every pixel is forced opaque.
pub fn decode_bmp_rgba(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    // BITMAPFILEHEADER (14) + at least a BITMAPINFOHEADER (40).
    if data.len() < 54 || &data[0..2] != b"BM" {
        return None;
    }
    let u16le = |o: usize| u16::from_le_bytes([data[o], data[o + 1]]);
    let u32le = |o: usize| u32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);
    let i32le = |o: usize| i32::from_le_bytes([data[o], data[o + 1], data[o + 2], data[o + 3]]);

    let pixel_offset = u32le(10) as usize;
    let header_size = u32le(14);
    if header_size < 40 {
        return None;
    }
    let width = i32le(18);
    let height_raw = i32le(22);
    let bpp = u16le(28);
    let compression = u32le(30);
    if compression != 0 || width <= 0 || height_raw == 0 || !matches!(bpp, 24 | 32) {
        return None;
    }
    let w = width as usize;
    let top_down = height_raw < 0;
    let h = height_raw.unsigned_abs() as usize;
    let bytes_per_px = (bpp / 8) as usize;
    // Rows are padded to a 4-byte boundary.
    let stride = (w * bytes_per_px + 3) & !3;
    if pixel_offset.checked_add(stride.checked_mul(h)?)? > data.len() {
        return None;
    }

    let mut rgba = vec![0u8; w * h * 4];
    for row in 0..h {
        // Bottom-up BMPs store the first row last; map to a top-down destination.
        let src_row = if top_down { row } else { h - 1 - row };
        let src = pixel_offset + src_row * stride;
        for col in 0..w {
            let p = src + col * bytes_per_px;
            let d = (row * w + col) * 4;
            // BMP pixels are BGR(A); emit RGBA, forcing opaque (BI_RGB has no alpha channel).
            rgba[d] = data[p + 2];
            rgba[d + 1] = data[p + 1];
            rgba[d + 2] = data[p];
            rgba[d + 3] = 255;
        }
    }
    Some((rgba, w as u32, h as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal 24-bit BI_RGB (bottom-up) BMP for `w`x`h`, each pixel BGR from `px(x, y)` given
    /// as `(r, g, b)` — the same layout Crystal emits for an embedded bitmap.
    fn bmp_24(w: usize, h: usize, px: impl Fn(usize, usize) -> (u8, u8, u8)) -> Vec<u8> {
        let stride = (w * 3 + 3) & !3;
        let pixel_offset = 54usize;
        let mut data = vec![0u8; pixel_offset + stride * h];
        data[0..2].copy_from_slice(b"BM");
        data[10..14].copy_from_slice(&(pixel_offset as u32).to_le_bytes());
        data[14..18].copy_from_slice(&40u32.to_le_bytes()); // BITMAPINFOHEADER
        data[18..22].copy_from_slice(&(w as i32).to_le_bytes());
        data[22..26].copy_from_slice(&(h as i32).to_le_bytes()); // positive = bottom-up
        data[28..30].copy_from_slice(&24u16.to_le_bytes());
        for y in 0..h {
            let src_row = h - 1 - y; // bottom-up
            let base = pixel_offset + src_row * stride;
            for x in 0..w {
                let (r, g, b) = px(x, y);
                let o = base + x * 3;
                data[o] = b;
                data[o + 1] = g;
                data[o + 2] = r;
            }
        }
        data
    }

    #[test]
    fn decodes_24bit_bottom_up_bmp_to_rgba() {
        // 2x2 with a distinct color per pixel so orientation + channel order are both checked.
        let colors = [[(255, 0, 0), (0, 255, 0)], [(0, 0, 255), (10, 20, 30)]];
        let bmp = bmp_24(2, 2, |x, y| colors[y][x]);
        let (rgba, w, h) = decode_bmp_rgba(&bmp).expect("valid BMP decodes");
        assert_eq!((w, h), (2, 2));
        for (y, row) in colors.iter().enumerate() {
            for (x, &(r, g, b)) in row.iter().enumerate() {
                let d = (y * 2 + x) * 4;
                assert_eq!(
                    (rgba[d], rgba[d + 1], rgba[d + 2], rgba[d + 3]),
                    (r, g, b, 255),
                    "pixel ({x},{y}) channel order / orientation / opaque-alpha"
                );
            }
        }
    }

    #[test]
    fn decodes_top_down_bmp() {
        // A negative biHeight stores the first row first; the destination is top-down either way.
        let colors = [[(255, 0, 0), (0, 255, 0)], [(0, 0, 255), (10, 20, 30)]];
        let mut bmp = bmp_24(2, 2, |x, y| colors[1 - y][x]);
        bmp[22..26].copy_from_slice(&(-2i32).to_le_bytes());
        let (rgba, w, h) = decode_bmp_rgba(&bmp).expect("valid BMP decodes");
        assert_eq!((w, h), (2, 2));
        for (y, row) in colors.iter().enumerate() {
            for (x, &(r, g, b)) in row.iter().enumerate() {
                let d = (y * 2 + x) * 4;
                assert_eq!(
                    (rgba[d], rgba[d + 1], rgba[d + 2]),
                    (r, g, b),
                    "pixel ({x},{y})"
                );
            }
        }
    }

    #[test]
    fn rejects_non_bmp_and_compressed() {
        assert!(decode_bmp_rgba(b"not a bmp at all, definitely too short").is_none());
        // A well-formed header but with an RLE compression code is unsupported.
        let mut bmp = bmp_24(1, 1, |_, _| (1, 2, 3));
        bmp[30] = 1; // BI_RLE8 compression
        assert!(decode_bmp_rgba(&bmp).is_none());
    }

    #[test]
    fn rejects_unsupported_bit_depths() {
        // Paletted and 16-bit BMPs are refused outright: the pixel bytes are indices or packed
        // channels, so reading them as BGR(A) would produce a plausible-looking wrong image.
        for bpp in [1u16, 4, 8, 16] {
            let mut bmp = bmp_24(4, 4, |_, _| (1, 2, 3));
            bmp[28..30].copy_from_slice(&bpp.to_le_bytes());
            assert!(decode_bmp_rgba(&bmp).is_none(), "{bpp}bpp must be refused");
        }
    }

    #[test]
    fn rejects_truncated_pixel_data() {
        let mut bmp = bmp_24(8, 8, |_, _| (1, 2, 3));
        bmp.truncate(bmp.len() - 1);
        assert!(decode_bmp_rgba(&bmp).is_none());
    }
}
