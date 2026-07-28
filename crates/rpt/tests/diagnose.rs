//! End-to-end checks that a bad input is *diagnosed*, not merely rejected.
//!
//! The failure a user hits most is pointing the tool at the wrong file. Each case here asserts the
//! message names the file, says what it is instead, and never leaks the CFB library's internal
//! complaint as the whole answer.

use std::io::Write as _;

use rpt::Rpt;

/// Build an OLE2 compound file carrying `streams` (name → bytes) but no `Contents`.
fn compound_file_without_contents(streams: &[(&str, &[u8])]) -> Vec<u8> {
    let mut comp = cfb::CompoundFile::create(std::io::Cursor::new(Vec::new()))
        .expect("create a compound file");
    for (name, bytes) in streams {
        let mut s = comp
            .create_stream(format!("/{name}"))
            .expect("create a stream");
        s.write_all(bytes).expect("write the stream");
    }
    comp.flush().expect("flush");
    comp.into_inner().into_inner()
}

/// Write `bytes` to a uniquely-named temp file, open it, and return the full error chain. Asserts
/// the failure is a diagnosis rather than a raw lower-layer complaint.
fn open_err(name: &str, bytes: Vec<u8>) -> String {
    let path = std::env::temp_dir().join(format!("rpt-diagnose-{name}.rpt"));
    std::fs::write(&path, &bytes).expect("write the test input");
    let err = Rpt::open(&path).expect_err("this input is not a report");
    let msg = rpt::error_chain(&err);
    std::fs::remove_file(&path).ok();
    assert!(
        matches!(err, rpt::Error::NotAReport(_)),
        "expected a NotAReport diagnosis, got: {msg}"
    );
    msg
}

#[test]
fn plain_text_is_named_as_such_and_the_file_is_identified() {
    let msg = open_err("plain-text", b"id,name\n1,foo\n".to_vec());
    assert!(msg.contains("is not a Crystal Reports report"), "{msg}");
    assert!(msg.contains("looks like plain text"), "{msg}");
    assert!(
        msg.contains(".rpt`"),
        "the message must name the file: {msg}"
    );
    // The CFB library's message is what this layer exists to replace.
    assert!(!msg.contains("Invalid CFB file"), "{msg}");
}

#[test]
fn another_ole2_document_is_told_apart_from_a_report_by_its_streams() {
    // A compound file is a *container* format — Word and Excel documents are OLE2 too. What makes a
    // file a report is the `Contents` stream, so its absence is the diagnosis, and the streams that
    // are present are the evidence of what the file really is.
    let msg = open_err(
        "other-ole2",
        compound_file_without_contents(&[("WordDocument", b"..."), ("1Table", b"...")]),
    );
    assert!(msg.contains("no `Contents` stream"), "{msg}");
    assert!(msg.contains("WordDocument"), "{msg}");
    assert!(msg.contains("1Table"), "{msg}");
}

#[test]
fn an_undecodable_contents_stream_says_so_rather_than_yielding_an_empty_report() {
    // A `Contents` stream that will not decrypt/inflate leaves no report definition at all. Opening
    // must fail: a silently empty report would look authoritative.
    let msg = open_err(
        "bad-contents",
        compound_file_without_contents(&[("Contents", &[0xff; 64][..])]),
    );
    assert!(
        msg.contains("`Contents` stream could not be decoded"),
        "{msg}"
    );
    assert!(
        msg.contains("rpt streams"),
        "the hint names the next step: {msg}"
    );
}

#[test]
fn a_truncated_compound_file_keeps_the_container_message_as_the_cause() {
    // This one really is (the start of) a compound file, so the CFB layer's complaint is the useful
    // detail — the diagnosis frames it rather than replacing it.
    let mut bytes = compound_file_without_contents(&[("Contents", b"x")]);
    bytes.truncate(700);
    let msg = open_err("truncated", bytes);
    assert!(msg.contains("truncated"), "{msg}");
    assert!(
        msg.contains("transferred completely"),
        "the hint is missing: {msg}"
    );
}
