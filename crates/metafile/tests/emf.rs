//! Integration tests for the EMF parser, built from synthetic metafiles (the record-construction
//! pattern follows Wine's `dlls/gdi32/tests/metafile.c`). Output is checked in the metafile's own
//! device space — no consumer target box is involved.

use metafile::{
    collect_emf, parse_emf, BitmapFormat, Color, Error, Feature, Font, Point, Primitive, Recording,
    Rect,
};

// EMF record type numbers used by the tests (MS-EMF §2.1.1).
const EMR_HEADER: u32 = 1;
const EMR_POLYBEZIER: u32 = 2;
const EMR_EOF: u32 = 14;
const EMR_MOVETOEX: u32 = 27;
const EMR_SAVEDC: u32 = 33;
const EMR_RESTOREDC: u32 = 34;
const EMR_SETWORLDTRANSFORM: u32 = 35;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_ELLIPSE: u32 = 42;
const EMR_RECTANGLE: u32 = 43;
const EMR_LINETO: u32 = 54;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTW: u32 = 84;
const EMR_POLYGON16: u32 = 86;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETWINDOWORGEX: u32 = 10;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_COMMENT: u32 = 70;
const EMR_SETDIBITSTODEVICE: u32 = 80;
const EMR_STRETCHDIBITS: u32 = 81;

/// A 40-byte `BITMAPINFOHEADER` for a 1×1 image at `bit_count` bpp and `compression` (0 = BI_RGB).
fn bmih(bit_count: u16, compression: u32) -> Vec<u8> {
    let mut b = vec![0u8; 40];
    b[0..4].copy_from_slice(&40u32.to_le_bytes()); // biSize
    b[4..8].copy_from_slice(&1i32.to_le_bytes()); // biWidth
    b[8..12].copy_from_slice(&1i32.to_le_bytes()); // biHeight
    b[12..14].copy_from_slice(&1u16.to_le_bytes()); // biPlanes
    b[14..16].copy_from_slice(&bit_count.to_le_bytes());
    b[16..20].copy_from_slice(&compression.to_le_bytes());
    b
}

/// A `STRETCHDIBITS` payload placing a DIB (`bmi` + `bits`) at dest `(x, y)` with extent `(cx, cy)`.
/// The `off*` fields are record-relative (they include the 8-byte header `record()` prepends), per
/// MS-EMF; payload byte N is at record offset N + 8.
fn stretchdibits(x: i32, y: i32, cx: i32, cy: i32, bmi: &[u8], bits: &[u8]) -> Vec<u8> {
    let mut p = vec![0u8; 72];
    p[16..20].copy_from_slice(&x.to_le_bytes()); // xDest   (rec 24)
    p[20..24].copy_from_slice(&y.to_le_bytes()); // yDest   (rec 28)
    let off_bmi = 72u32 + 8; // DIB is appended at payload offset 72 → record offset 80
    p[40..44].copy_from_slice(&off_bmi.to_le_bytes()); // offBmiSrc  (rec 48)
    p[44..48].copy_from_slice(&(bmi.len() as u32).to_le_bytes()); // cbBmiSrc (52)
    p[48..52].copy_from_slice(&(off_bmi + bmi.len() as u32).to_le_bytes()); // offBitsSrc (56)
    p[52..56].copy_from_slice(&(bits.len() as u32).to_le_bytes()); // cbBitsSrc (60)
    p[64..68].copy_from_slice(&cx.to_le_bytes()); // cxDest  (rec 72)
    p[68..72].copy_from_slice(&cy.to_le_bytes()); // cyDest  (rec 76)
    p.extend_from_slice(bmi);
    p.extend_from_slice(bits);
    p
}

/// An 88-byte `ENHMETAHEADER` with `rclBounds = (0,0,99,99)` and the ` EMF` signature. `bad_sig`
/// corrupts the signature to exercise the reject path.
fn header(bad_sig: bool) -> Vec<u8> {
    let mut h = vec![0u8; 88];
    h[0..4].copy_from_slice(&EMR_HEADER.to_le_bytes());
    h[4..8].copy_from_slice(&88u32.to_le_bytes());
    h[16..20].copy_from_slice(&99i32.to_le_bytes()); // rclBounds.right
    h[20..24].copy_from_slice(&99i32.to_le_bytes()); // rclBounds.bottom
    let sig: &[u8] = if bad_sig { b"XXXX" } else { b" EMF" };
    h[40..44].copy_from_slice(sig);
    h
}

/// A generic record: `iType`, `nSize` (= 8 + payload), then the payload.
fn record(itype: u32, payload: &[u8]) -> Vec<u8> {
    let size = 8 + payload.len();
    let mut r = Vec::with_capacity(size);
    r.extend_from_slice(&itype.to_le_bytes());
    r.extend_from_slice(&(size as u32).to_le_bytes());
    r.extend_from_slice(payload);
    r
}

fn eof() -> Vec<u8> {
    record(EMR_EOF, &[0u8; 12])
}

fn rectl(l: i32, t: i32, r: i32, b: i32) -> Vec<u8> {
    let mut v = Vec::new();
    for x in [l, t, r, b] {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

fn i32s(vals: &[i32]) -> Vec<u8> {
    let mut v = Vec::new();
    for x in vals {
        v.extend_from_slice(&x.to_le_bytes());
    }
    v
}

fn collect(records: &[Vec<u8>]) -> Recording {
    let mut emf = header(false);
    for r in records {
        emf.extend_from_slice(r);
    }
    emf.extend(eof());
    collect_emf(&emf).expect("valid EMF")
}

#[test]
fn header_bounds_reported() {
    let rec = collect(&[]);
    let h = rec.header.expect("header");
    assert_eq!(
        h.bounds,
        Rect {
            left: 0.0,
            top: 0.0,
            right: 99.0,
            bottom: 99.0
        }
    );
}

#[test]
fn rectangle_in_device_space() {
    let rec = collect(&[record(EMR_RECTANGLE, &rectl(10, 10, 50, 50))]);
    assert_eq!(rec.primitives.len(), 1);
    let Primitive::Rectangle { bounds, .. } = &rec.primitives[0] else {
        panic!("expected a rectangle, got {:?}", rec.primitives[0]);
    };
    assert_eq!(bounds.left, 10.0);
    assert_eq!(bounds.top, 10.0);
    assert_eq!(bounds.right, 50.0);
    assert_eq!(bounds.bottom, 50.0);
}

#[test]
fn brush_then_polygon16_is_closed_and_red() {
    // COLORREF 0x00BBGGRR with r=255.
    let red = Color::from_colorref(0x0000_00FF);

    let mut brush = 1u32.to_le_bytes().to_vec(); // ihBrush = 1
    brush.extend_from_slice(&0u32.to_le_bytes()); // lbStyle = BS_SOLID
    brush.extend_from_slice(&0x0000_00FFu32.to_le_bytes()); // lbColor = red
    brush.extend_from_slice(&0u32.to_le_bytes()); // lbHatch

    let mut poly = rectl(0, 0, 99, 99); // rclBounds (unused)
    poly.extend_from_slice(&3u32.to_le_bytes()); // 3 points
    for (x, y) in [(10i16, 10i16), (90, 10), (50, 90)] {
        poly.extend_from_slice(&x.to_le_bytes());
        poly.extend_from_slice(&y.to_le_bytes());
    }

    let rec = collect(&[
        record(EMR_CREATEBRUSHINDIRECT, &brush),
        record(EMR_SELECTOBJECT, &1u32.to_le_bytes()),
        record(EMR_POLYGON16, &poly),
    ]);
    assert_eq!(rec.primitives.len(), 1);
    let Primitive::Polygon { points, brush, .. } = &rec.primitives[0] else {
        panic!("expected a polygon, got {:?}", rec.primitives[0]);
    };
    assert_eq!(points.len(), 3);
    assert_eq!(points[0], Point::new(10.0, 10.0));
    assert_eq!(brush.map(|b| b.color), Some(red));
}

#[test]
fn bad_signature_is_not_a_metafile() {
    let mut emf = header(true);
    emf.extend(record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)));
    emf.extend(eof());
    assert_eq!(
        parse_emf(&emf, &mut Recording::default()),
        Err(Error::NotAMetafile)
    );
}

#[test]
fn unknown_record_is_skipped_not_fatal() {
    let rec = collect(&[
        record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)),
        record(9999, &[0u8; 24]),
        record(EMR_ELLIPSE, &rectl(0, 0, 20, 20)),
    ]);
    assert_eq!(rec.primitives.len(), 2);
    assert!(matches!(rec.primitives[0], Primitive::Rectangle { .. }));
    assert!(matches!(rec.primitives[1], Primitive::Ellipse { .. }));
}

#[test]
fn truncated_stream_errors() {
    let mut emf = header(false);
    // A rectangle claiming 24 bytes but supplying only 10 — it overruns the buffer.
    let mut rec = EMR_RECTANGLE.to_le_bytes().to_vec();
    rec.extend_from_slice(&24u32.to_le_bytes());
    rec.extend_from_slice(&[0u8; 2]);
    emf.extend(rec);
    // The error names *where* the stream ran out, not merely that it did — for a stream of vector
    // commands the offset is the diagnosis.
    let err =
        parse_emf(&emf, &mut Recording::default()).expect_err("a truncated record must error");
    assert!(matches!(err, Error::UnexpectedEof { .. }), "{err:?}");
    assert!(err.offset().is_some_and(|o| o > 0), "{err}");
    assert!(err.to_string().contains("at byte"), "{err}");
}

#[test]
fn lineto_uses_current_position() {
    let rec = collect(&[
        record(EMR_MOVETOEX, &i32s(&[20, 20])),
        record(EMR_LINETO, &i32s(&[80, 80])),
    ]);
    assert_eq!(rec.primitives.len(), 1);
    let Primitive::Polyline { points, .. } = &rec.primitives[0] else {
        panic!("expected a polyline, got {:?}", rec.primitives[0]);
    };
    assert_eq!(points, &[Point::new(20.0, 20.0), Point::new(80.0, 80.0)]);
}

#[test]
fn world_transform_scales_coordinates() {
    // XFORM that doubles both axes.
    let mut xform = Vec::new();
    for v in [2.0f32, 0.0, 0.0, 2.0, 0.0, 0.0] {
        xform.extend_from_slice(&v.to_le_bytes());
    }
    let rec = collect(&[
        record(EMR_SETWORLDTRANSFORM, &xform),
        record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)),
    ]);
    let Primitive::Rectangle { bounds, .. } = &rec.primitives[0] else {
        panic!("expected a rectangle");
    };
    assert_eq!(bounds.left, 20.0);
    assert_eq!(bounds.right, 100.0);
}

#[test]
fn window_viewport_maps_coordinates() {
    // window ext 100, viewport ext 200 → 2× scale; window org 0.
    let rec = collect(&[
        record(EMR_SETWINDOWORGEX, &i32s(&[0, 0])),
        record(EMR_SETWINDOWEXTEX, &i32s(&[100, 100])),
        record(EMR_SETVIEWPORTEXTEX, &i32s(&[200, 200])),
        record(EMR_RECTANGLE, &rectl(10, 10, 50, 50)),
    ]);
    let Primitive::Rectangle { bounds, .. } = &rec.primitives[0] else {
        panic!("expected a rectangle");
    };
    assert_eq!(bounds.left, 20.0);
    assert_eq!(bounds.right, 100.0);
}

#[test]
fn savedc_restoredc_restores_pen() {
    // Create a red pen, select it, save, select stock null pen, restore → red pen active again.
    let mut pen = 1u32.to_le_bytes().to_vec(); // ihPen = 1
    pen.extend_from_slice(&0u32.to_le_bytes()); // PS_SOLID
    pen.extend_from_slice(&i32s(&[2, 0])); // width POINTL
    pen.extend_from_slice(&0x0000_00FFu32.to_le_bytes()); // red

    let null_pen = 0x8000_0008u32; // NULL_PEN stock handle
    let rec = collect(&[
        record(EMR_CREATEPEN, &pen),
        record(EMR_SELECTOBJECT, &1u32.to_le_bytes()),
        record(EMR_SAVEDC, &[]),
        record(EMR_SELECTOBJECT, &null_pen.to_le_bytes()),
        record(EMR_RESTOREDC, &(-1i32).to_le_bytes()),
        record(EMR_RECTANGLE, &rectl(0, 0, 10, 10)),
    ]);
    let Primitive::Rectangle { pen, .. } = &rec.primitives[0] else {
        panic!("expected a rectangle");
    };
    let pen = pen.expect("pen restored");
    assert_eq!(pen.color, Color::from_colorref(0x0000_00FF));
}

#[test]
fn font_selection_carries_into_text() {
    // EXTCREATEFONTINDIRECTW: ihFont (8), then LOGFONT at 12.
    let mut font_rec = 1u32.to_le_bytes().to_vec(); // ihFont = 1
    let mut logfont = vec![0u8; 12 + 92]; // LOGFONT is 92 bytes (with 32×u16 face)
    logfont[0..4].copy_from_slice(&(-24i32).to_le_bytes()); // lfHeight
    logfont[16..20].copy_from_slice(&700i32.to_le_bytes()); // lfWeight = bold
    logfont[20] = 1; // lfItalic
                     // lfFaceName at offset 28 (within LOGFONT), i.e. 28 in this slice: "Ar"
    for (i, u) in "Arial".encode_utf16().enumerate() {
        let at = 28 + i * 2;
        logfont[at..at + 2].copy_from_slice(&u.to_le_bytes());
    }
    font_rec.extend_from_slice(&logfont);

    // ExtTextOutW record with a reference point and a short string.
    let text = "Hi";
    let utf16: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    // Payload begins at record offset 8. Build fields relative to record start.
    let mut payload = vec![0u8; 64]; // through offDxScale; string appended after
                                     // rclBounds (payload 0..16) left as 0.
                                     // EMRTEXT.ptlReference at record offset 36 → payload offset 28.
    payload[28..32].copy_from_slice(&30i32.to_le_bytes()); // ref x
    payload[32..36].copy_from_slice(&40i32.to_le_bytes()); // ref y
    payload[36..40].copy_from_slice(&(text.len() as u32).to_le_bytes()); // nChars (rec off 44)
    let off_string = 8 + payload.len(); // string placed right after the fixed fields
    payload[40..44].copy_from_slice(&(off_string as u32).to_le_bytes()); // offString (rec off 48)
    payload.extend_from_slice(&utf16);

    let rec = collect(&[
        record(EMR_EXTCREATEFONTINDIRECTW, &font_rec),
        record(EMR_SELECTOBJECT, &1u32.to_le_bytes()),
        record(EMR_EXTTEXTOUTW, &payload),
    ]);
    assert_eq!(rec.primitives.len(), 1);
    let Primitive::Text {
        text,
        position,
        font,
        ..
    } = &rec.primitives[0]
    else {
        panic!("expected text, got {:?}", rec.primitives[0]);
    };
    assert_eq!(text, "Hi");
    assert_eq!(*position, Point::new(30.0, 40.0));
    assert_eq!(font.face, "Arial");
    assert_eq!(font.weight, 700);
    assert!(font.italic);
    assert_eq!(font.height, -24.0);
}

#[test]
fn bezier_is_flattened_to_a_polyline() {
    // POLYBEZIER (32-bit points): anchor + one cubic triplet = 4 points.
    let mut payload = rectl(0, 0, 99, 99); // rclBounds
    payload.extend_from_slice(&4u32.to_le_bytes()); // 4 points
    for (x, y) in [(0i32, 0i32), (0, 50), (50, 50), (50, 0)] {
        payload.extend_from_slice(&x.to_le_bytes());
        payload.extend_from_slice(&y.to_le_bytes());
    }
    let rec = collect(&[record(EMR_POLYBEZIER, &payload)]);
    let Primitive::Polyline { points, .. } = &rec.primitives[0] else {
        panic!("expected a polyline, got {:?}", rec.primitives[0]);
    };
    // Flattened: the anchor plus many interpolated points, staying within the control hull.
    assert!(points.len() > 4);
    assert_eq!(points[0], Point::new(0.0, 0.0));
    assert!(points.iter().all(|p| p.x >= 0.0 && p.x <= 50.0));
}

#[test]
fn default_font_is_reasonable() {
    let f = Font::default();
    assert_eq!(f.weight, 400);
    assert!(f.face.is_empty());
}

#[test]
fn stretchdibits_reconstructs_a_bmp() {
    let bmi = bmih(24, 0); // 1×1, 24bpp, BI_RGB
    let bits = vec![0x10u8, 0x20, 0x30, 0x00]; // one padded BGR pixel
    let rec = collect(&[record(
        EMR_STRETCHDIBITS,
        &stretchdibits(5, 5, 10, 10, &bmi, &bits),
    )]);
    assert_eq!(rec.primitives.len(), 1);
    let Primitive::Image { bounds, bitmap } = &rec.primitives[0] else {
        panic!("expected an image, got {:?}", rec.primitives[0]);
    };
    assert_eq!((bounds.left, bounds.top), (5.0, 5.0));
    assert_eq!((bounds.right, bounds.bottom), (15.0, 15.0));
    assert_eq!(bitmap.format, BitmapFormat::Bmp);
    // A valid .bmp: "BM", then the file header points past the 14-byte header + 40-byte BMIH.
    assert!(bitmap.bytes.starts_with(b"BM"));
    let pixel_offset = u32::from_le_bytes(bitmap.bytes[10..14].try_into().unwrap());
    assert_eq!(pixel_offset, 14 + 40);
    assert_eq!(bitmap.bytes.len(), 14 + bmi.len() + bits.len());
}

#[test]
fn bi_png_dib_is_passed_through() {
    // biCompression = BI_PNG (5): the "bits" are a whole PNG file, carried verbatim.
    let bmi = bmih(0, 5);
    let png = b"\x89PNG\r\n\x1a\nsynthetic".to_vec();
    let rec = collect(&[record(
        EMR_STRETCHDIBITS,
        &stretchdibits(0, 0, 8, 8, &bmi, &png),
    )]);
    let Primitive::Image { bitmap, .. } = &rec.primitives[0] else {
        panic!("expected an image, got {:?}", rec.primitives[0]);
    };
    assert_eq!(bitmap.format, BitmapFormat::Png);
    assert_eq!(bitmap.bytes, png); // verbatim, no BMP header prepended
}

#[test]
fn setdibitstodevice_uses_source_extent_as_dest() {
    // SETDIBITSTODEVICE has no dest extent: dest box is xDest,yDest + cxSrc,cySrc.
    let bmi = bmih(24, 0);
    let bits = vec![0u8, 0, 0, 0];
    let mut p = vec![0u8; 76];
    p[16..20].copy_from_slice(&2i32.to_le_bytes()); // xDest (rec 24)
    p[20..24].copy_from_slice(&3i32.to_le_bytes()); // yDest (rec 28)
    p[32..36].copy_from_slice(&7i32.to_le_bytes()); // cxSrc (rec 40) → dest width
    p[36..40].copy_from_slice(&9i32.to_le_bytes()); // cySrc (rec 44) → dest height
    let off_bmi = 76u32 + 8; // DIB appended at payload offset 76 → record offset 84
    p[40..44].copy_from_slice(&off_bmi.to_le_bytes()); // offBmiSrc (rec 48)
    p[44..48].copy_from_slice(&(bmi.len() as u32).to_le_bytes()); // cbBmiSrc (52)
    p[48..52].copy_from_slice(&(off_bmi + bmi.len() as u32).to_le_bytes()); // offBitsSrc (56)
    p[52..56].copy_from_slice(&(bits.len() as u32).to_le_bytes()); // cbBitsSrc (60)
    p.extend_from_slice(&bmi);
    p.extend_from_slice(&bits);

    let rec = collect(&[record(EMR_SETDIBITSTODEVICE, &p)]);
    let Primitive::Image { bounds, .. } = &rec.primitives[0] else {
        panic!("expected an image, got {:?}", rec.primitives[0]);
    };
    assert_eq!((bounds.left, bounds.top), (2.0, 3.0));
    assert_eq!((bounds.right, bounds.bottom), (9.0, 12.0)); // 2+7, 3+9
}

#[test]
fn rop_only_blit_without_bitmap_draws_nothing() {
    // cbBmiSrc = 0 (a pattern/ROP fill, no source DIB): no image emitted, parse still succeeds.
    let mut p = vec![0u8; 72];
    p[16..20].copy_from_slice(&5i32.to_le_bytes());
    // offsets 40..44 (cbBmiSrc within payload) left zero.
    let rec = collect(&[record(EMR_STRETCHDIBITS, &p)]);
    assert!(rec.primitives.is_empty());
}

#[test]
fn emf_plus_comment_is_detected() {
    // An EMR_COMMENT whose data starts with "EMF+" flags embedded GDI+ content.
    let mut data = b"EMF+".to_vec();
    data.extend_from_slice(&[0u8; 8]); // arbitrary EMF+ record bytes we don't interpret
    let mut payload = (data.len() as u32).to_le_bytes().to_vec(); // cbData (rec 8)
    payload.extend_from_slice(&data);
    let rec = collect(&[record(EMR_COMMENT, &payload)]);
    assert_eq!(rec.unsupported, vec![Feature::EmfPlus]);
    assert!(rec.primitives.is_empty());
}

#[test]
fn plain_comment_is_not_flagged() {
    let mut payload = 4u32.to_le_bytes().to_vec();
    payload.extend_from_slice(b"junk");
    let rec = collect(&[record(EMR_COMMENT, &payload)]);
    assert!(rec.unsupported.is_empty());
}
