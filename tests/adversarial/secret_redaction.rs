//! Invariant: formatted tracing output cannot leak common secret shapes.

use std::io::Write;

use santh_tracing::RedactingWriter;

#[test]
fn secret_redaction() {
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        writer
            .write_all(b"token=ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ\npassword=hunter2")
            .expect("Fix: redacting writer must accept in-memory bytes.");
        writer
            .flush()
            .expect("Fix: redacting writer must flush partial lines.");
    }

    let rendered =
        String::from_utf8(output).expect("Fix: redacted tracing output must remain UTF-8.");
    assert!(
        !rendered.contains("hunter2") && !rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"),
        "Fix: tracing output must redact passwords and tokens: {rendered}"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "Fix: tracing output must include a redaction marker."
    );
}

#[test]
fn newline_less_stream_is_bounded_and_still_redacts() {
    // A stream that writes megabytes with NO newline would, before the fix, grow
    // the internal `pending` buffer without limit (OOM). The bounded buffer must
    // force-flush the head instead. We prove it observably: because a redacting
    // writer normally emits NOTHING until a newline or an explicit flush, seeing
    // output appear after only newline-less writes proves the forced flush
    // engaged - and the secret written at the start must still be redacted in it.
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        // Secret first, then a large newline-less filler exceeding the 1 MiB cap.
        writer
            .write_all(b"token=ghp_abcdefghijklmnopqrstuvwxyzABCDEFGHIJ ")
            .expect("Fix: redacting writer must accept the secret bytes.");
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..64 {
            // 4 MiB total, no newline anywhere.
            writer
                .write_all(&chunk)
                .expect("Fix: redacting writer must accept newline-less bytes.");
        }
        // Intentionally NO flush(): RedactingWriter has no Drop-flush, so any bytes
        // in `output` after the writer is dropped came ONLY from the forced
        // mid-stream flush the buffer cap triggers. Without the OOM guard a
        // newline-less stream emits nothing until an explicit flush, so `output`
        // would be empty here.
    }

    assert!(
        !output.is_empty(),
        "newline-less stream past the cap must force-flush its head (buffer stayed unbounded?)"
    );

    let rendered = String::from_utf8_lossy(&output);
    assert!(
        !rendered.contains("ghp_abcdefghijklmnopqrstuvwxyz"),
        "the secret must be redacted even when it precedes a huge newline-less run"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "forced-flush output must include the redaction marker"
    );
}

#[test]
fn multiline_pem_private_key_is_redacted_as_a_block() {
    // A PEM private key spans multiple lines. Block-oriented redaction catches
    // it once its lines are buffered together as complete lines; the previous
    // per-line scan forwarded each line separately and never matched the
    // BEGIN...END framing, leaking the key body.
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        writer
            .write_all(
                b"-----BEGIN PRIVATE KEY-----\nMIIBVAIBADANBgkqhkiG9w0\nZZsecretkeymaterialZZ\n-----END PRIVATE KEY-----\n",
            )
            .expect("Fix: redacting writer must accept the PEM bytes.");
        writer.flush().expect("Fix: redacting writer must flush.");
    }
    let rendered = String::from_utf8(output).expect("Fix: redacted output must remain UTF-8.");
    assert!(
        !rendered.contains("MIIBVAIBADANBgkqhkiG9w0") && !rendered.contains("secretkeymaterial"),
        "Fix: the multi-line PEM key body must be redacted: {rendered}"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "Fix: PEM redaction must include a redaction marker: {rendered}"
    );
}


#[test]
fn long_single_line_pem_is_redacted_across_forced_flushes() {
    // A hostile or malformed stream may write a PEM private key as a single
    // line with no newlines. The 1 MiB pending cap forces the writer to flush
    // the head before the trailing `-----END ... -----` arrives; without
    // stateful redaction the BEGIN marker would be in the flushed head while
    // the base64 body leaks because `redact_secrets` cannot match without END.
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        writer
            .write_all(b"-----BEGIN PRIVATE KEY-----")
            .expect("must accept PEM opener");
        // 4 MiB of base64-like filler with no newlines, exceeding MAX_PENDING.
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..64 {
            writer
                .write_all(&chunk)
                .expect("must accept newline-less PEM body");
        }
        // Finally deliver the closer.
        writer
            .write_all(b"-----END PRIVATE KEY-----\n")
            .expect("must accept PEM closer");
        // Explicit flush to clear the remaining tail.
        writer.flush().expect("must flush");
    }

    let rendered = String::from_utf8_lossy(&output);
    assert!(
        !rendered.contains("xxxxxxxxxxxxxxxx"),
        "PEM body must not appear in the clear: {rendered}"
    );
    assert!(
        !rendered.contains("-----END PRIVATE KEY-----"),
        "PEM closer must also be redacted: {rendered}"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "PEM block must be replaced with the redaction marker: {rendered}"
    );
}
#[test]
fn multiline_pem_key_streamed_across_multiple_writes_is_redacted() {
    // Proving test: streaming a multi-line PEM key line-by-line across multiple
    // `write_all` calls (under the 1MB MAX_PENDING cap) must immediately engage
    // stateful redaction on the opener, preventing `forward_complete_lines` from
    // emitting key lines in the clear before the closer arrives.
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        writer
            .write_all(b"Header info line\n")
            .expect("must accept header");
        writer
            .write_all(b"-----BEGIN RSA PRIVATE KEY-----\n")
            .expect("must accept opener");
        writer
            .write_all(b"MIIEvgIBADANBgkqhkiG9w0BAQEFAASCBKgwggSkAgEAAoIBAQC...\n")
            .expect("must accept key body line 1");
        writer
            .write_all(b"ZZsecret_rsa_payload_bytes_hereZZ\n")
            .expect("must accept key body line 2");
        writer
            .write_all(b"-----END RSA PRIVATE KEY-----\n")
            .expect("must accept closer");
        writer
            .write_all(b"Footer status line\n")
            .expect("must accept footer");
        writer.flush().expect("must flush");
    }

    let rendered = String::from_utf8(output).expect("output must remain UTF-8");
    assert!(
        rendered.contains("Header info line"),
        "Header before PEM opener must be preserved: {rendered}"
    );
    assert!(
        !rendered.contains("MIIEvgIBADANBgkqhkiG9w0")
            && !rendered.contains("secret_rsa_payload_bytes_here"),
        "PEM key body lines streamed across writes must not leak: {rendered}"
    );
    assert!(
        rendered.contains("[REDACTED]"),
        "Streamed PEM key block must be replaced with redaction marker: {rendered}"
    );
    assert!(
        rendered.contains("Footer status line"),
        "Footer after PEM closer must be preserved: {rendered}"
    );
}

#[test]
fn trailing_text_after_pem_closer_on_flush_is_preserved_and_flushed() {
    // Proving test: calling `flush()` on a buffer containing a PEM secret
    // followed by trailing text must redact the secret AND flush the trailing text,
    // rather than dropping or leaving the trailing text unflushed in pending.
    let mut output = Vec::new();
    {
        let mut writer = RedactingWriter::new(&mut output);
        writer
            .write_all(b"-----BEGIN PRIVATE KEY-----\nsecret_bytes\n-----END PRIVATE KEY----- trailing_log_data")
            .expect("must accept PEM and trailing text");
        writer.flush().expect("must flush");
    }

    let rendered = String::from_utf8_lossy(&output);
    assert!(
        rendered.contains("[REDACTED]"),
        "PEM secret must be redacted on flush: {rendered}"
    );
    assert!(
        rendered.contains("trailing_log_data"),
        "Trailing text after PEM closer must be flushed completely to output: {rendered}"
    );
}
