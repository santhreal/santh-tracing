//! A [`std::io::Write`] adapter that redacts secrets from log output.
//!
//! The Santh safe-defaults contract requires that secrets are never written
//! to logs. `RedactingWriter` wraps any writer and runs each complete line
//! through [`santh_error::redact_secrets`] - the single, canonical redaction
//! routine shared across the ecosystem - before forwarding it downstream.
//! Partial (newline-less) input is buffered and redacted on [`flush`](std::io::Write::flush)
//! so a secret split across writes is never emitted in the clear.
//!
//! For multi-line secrets that are deliberately written without newlines
//! (or whose closing marker has not yet arrived), the writer tracks whether
//! it is inside an unterminated secret opener. The buffered bytes from the
//! opener onward are kept redacted until the matching closer is seen, so an
//! oversized single-line PEM block cannot have its head force-flushed in the
//! clear before the trailing `-----END ... -----` arrives.

use std::io::{self, Write};

use santh_error::redact_secrets;

/// Hard cap on the un-forwarded `pending` buffer. A stream that writes a very
/// long run with NO newline (malformed or hostile input) would otherwise grow
/// `pending` without bound and OOM the process (Law 7: unbounded allocation is a
/// production bug). Once the buffer exceeds this, its head is redacted and
/// forwarded, retaining only a raw [`OVERLAP`] tail.
const MAX_PENDING: usize = 1024 * 1024;

/// Raw tail retained across a forced (newline-less) flush so that a secret split
/// across the forced-flush boundary is still redacted once the rest of it
/// arrives. Sized far larger than any single-line secret shape (tokens, keys and
/// URL credentials are at most a few hundred bytes). Multi-line PEM bodies are
/// handled by the stateful redaction path, not by the overlap tail alone.
const OVERLAP: usize = 8 * 1024;

/// The string all secret replacements use. Kept in one place so the writer's
/// manual stateful redaction and [`santh_error::redact_secrets`] agree.
const REDACTED: &str = "[REDACTED]";

/// PEM begin markers. The key-type prefix (RSA / DSA / EC / OPENSSH, or none)
/// is baked into each constant, so detection is a plain substring search with
/// no regex engine and no fallible constructor to unwrap.
const PEM_BEGIN_MARKERS: [&str; 5] = [
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN DSA PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
];

/// PEM end markers, mirroring [`PEM_BEGIN_MARKERS`].
const PEM_END_MARKERS: [&str; 5] = [
    "-----END PRIVATE KEY-----",
    "-----END RSA PRIVATE KEY-----",
    "-----END DSA PRIVATE KEY-----",
    "-----END EC PRIVATE KEY-----",
    "-----END OPENSSH PRIVATE KEY-----",
];

/// Leftmost occurrence of any `marker` in `text`, as a byte range.
fn find_marker(text: &str, markers: &[&str]) -> Option<std::ops::Range<usize>> {
    markers
        .iter()
        .filter_map(|marker| text.find(marker).map(|start| start..start + marker.len()))
        .min_by_key(|range| range.start)
}

/// Wraps a writer and redacts known secret shapes from every line written
/// through it. Redaction is line-oriented: complete lines are redacted and
/// forwarded immediately; a trailing partial line is held until the next
/// newline or until [`flush`](Write::flush).
///
/// Stateful redaction is used only for known multi-line secret shapes (PEM
/// private keys) that may be streamed without newlines. When such an opener is
/// detected without its closer, the writer stays in the redacting state and
/// withholds the buffered body from the downstream writer until the closer is
/// seen or the buffer is explicitly flushed.
pub struct RedactingWriter<W: Write> {
    inner: W,
    pending: Vec<u8>,
    /// Byte offset in `pending` where an unterminated multi-line secret begins.
    /// `None` means no unterminated opener is currently being tracked.
    secret_start: Option<usize>,
}

impl<W: Write> RedactingWriter<W> {
    /// Wrap `inner`, redacting secrets from everything written through it.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            secret_start: None,
        }
    }

    /// Redact and forward every complete (newline-terminated) line currently
    /// buffered, leaving any trailing partial line pending.
    ///
    /// The whole block of complete lines (up to and including the last newline)
    /// is drained and redacted in a single pass. Draining line-by-line from the
    /// front was O(N^2) (each `drain(..=i)` shifts all trailing bytes left); a
    /// single drain of the block is O(N). Redacting the block as one string is
    /// also strictly safer than the old per-line redaction: a multi-line secret
    /// (e.g. a PEM `BEGIN...END PRIVATE KEY` block) that per-line scanning could
    /// not match is redacted when it lies within the forwarded block, and no
    /// single-line secret is ever missed because `redact_secrets` operates on
    /// the full text.
    ///
    /// If the writer is currently inside an unterminated multi-line secret, the
    /// bytes are held until the matching closer is seen. A newline inside such
    /// a secret does not cause a premature split.
    fn forward_complete_lines(&mut self) -> io::Result<()> {
        let Some(last_newline) = self.pending.iter().rposition(|&b| b == b'\n') else {
            return Ok(());
        };

        // If a tracked secret starts at or before the last newline, we cannot
        // safely split the secret body across lines. Wait for the closer or for
        // the buffer-bound path to flush the whole region as redacted.
        if let Some(start) = self.secret_start {
            if start <= last_newline {
                return Ok(());
            }
        }

        let block_len = last_newline + 1;
        let block: Vec<u8> = self.pending.drain(..block_len).collect();

        if let Some(start) = &mut self.secret_start {
            *start = start.saturating_sub(block_len);
        }

        let redacted = redact_secrets(&String::from_utf8_lossy(&block));
        self.inner.write_all(redacted.as_bytes())
    }

    /// Bound the pending buffer so a newline-less stream cannot grow it without
    /// limit. When `pending` exceeds [`MAX_PENDING`], its head is redacted and
    /// forwarded, retaining only an [`OVERLAP`] tail.
    ///
    /// If the head contains the start of an unterminated multi-line secret and
    /// the matching closer has not yet arrived, the bytes from the opener to the
    /// cut point are replaced with [`REDACTED`] and the writer records that the
    /// remaining tail is still inside the secret. The next writes will stay
    /// redacted until the closer is seen.
    fn bound_pending_buffer(&mut self) -> io::Result<()> {
        if self.pending.len() <= MAX_PENDING {
            return Ok(());
        }

        let cut = self.pending.len() - OVERLAP;

        // Detect a freshly opened multi-line secret in the head if we are not
        // already inside one.
        if self.secret_start.is_none() {
            let head_text = String::from_utf8_lossy(&self.pending[..cut]);
            if let Some(m) = find_marker(&head_text, &PEM_BEGIN_MARKERS) {
                let after = &head_text[m.start..];
                if find_marker(after, &PEM_END_MARKERS).is_none() {
                    self.secret_start = Some(m.start);
                }
            }
        }

        if self.secret_start.is_some() {
            self.flush_secret_region(cut)?;
        } else {
            let head: Vec<u8> = self.pending.drain(..cut).collect();
            let redacted = redact_secrets(&String::from_utf8_lossy(&head));
            self.inner.write_all(redacted.as_bytes())?;
        }

        // The buffer may still exceed the cap if the remaining tail is huge and
        // we are inside a secret that has not closed. Recurse until under cap.
        self.bound_pending_buffer()
    }

    /// Redact and forward the head of `pending` up to `cut` while inside a
    /// tracked multi-line secret. If the closer is found in `pending[..cut]`,
    /// the whole secret block is replaced with [`REDACTED`], the prefix before
    /// the opener is normally redacted, and the tracking state is cleared. If the
    /// closer is not found, the region from the opener to `cut` is redacted and
    /// the tail is left pending with `secret_start` reset to the beginning.
    fn flush_secret_region(&mut self, cut: usize) -> io::Result<()> {
        let Some(start) = self.secret_start else {
            return Ok(());
        };

        let text = String::from_utf8_lossy(&self.pending);
        if let Some(end_match) = find_marker(&text[start..], &PEM_END_MARKERS) {
            // Closer arrived inside the head. Redact the whole secret block.
            let end_abs = start + end_match.end;
            let prefix: Vec<u8> = self.pending.drain(..start).collect();
            self.inner.write_all(redact_secrets(&String::from_utf8_lossy(&prefix)).as_bytes())?;
            self.inner.write_all(REDACTED.as_bytes())?;
            // Remove the redacted region from pending. After the prefix drain,
            // pending begins at the original `start`, so `end_abs - start` bytes
            // are removed.
            let redacted_len = end_abs - start;
            let _ = self.pending.drain(..redacted_len);
            self.secret_start = None;
        } else if start >= cut {
            // Defensive: the tracked opener sits inside the retained OVERLAP
            // tail, so the head up to `cut` precedes the secret and can be
            // forwarded with normal redaction. This branch is not reachable
            // through the current call paths (the opener is always recorded
            // below `cut`), but it keeps `cut - start` below from ever
            // underflowing if a future caller changes the invariant.
            let head: Vec<u8> = self.pending.drain(..cut).collect();
            self.inner
                .write_all(redact_secrets(&String::from_utf8_lossy(&head)).as_bytes())?;
            self.secret_start = Some(start - cut);
        } else {
            // No closer yet. Redact from the opener to the cut point.
            let prefix: Vec<u8> = self.pending.drain(..start).collect();
            self.inner.write_all(redact_secrets(&String::from_utf8_lossy(&prefix)).as_bytes())?;
            self.inner.write_all(REDACTED.as_bytes())?;
            let _ = self.pending.drain(..cut - start);
            // The remaining tail is still inside the unterminated secret.
            self.secret_start = Some(0);
        }
        Ok(())
    }
}

impl<W: Write> Write for RedactingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buf);
        self.forward_complete_lines()?;
        // Guard against unbounded growth on a newline-less stream (OOM).
        self.bound_pending_buffer()?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() {
            if let Some(start) = self.secret_start {
                let text = String::from_utf8_lossy(&self.pending);
                if let Some(end_match) = find_marker(&text[start..], &PEM_END_MARKERS) {
                    // Closer is in the remaining buffer. Redact only the secret
                    // region; anything after the closer stays pending.
                    let end_abs = start + end_match.end;
                    let prefix: Vec<u8> = self.pending.drain(..start).collect();
                    self.inner.write_all(redact_secrets(&String::from_utf8_lossy(&prefix)).as_bytes())?;
                    self.inner.write_all(REDACTED.as_bytes())?;
                    let _ = self.pending.drain(..(end_abs - start));
                } else {
                    // Flush while still inside a secret: redact everything from
                    // the opener onward. The prefix before the opener is still
                    // normally redacted.
                    let prefix: Vec<u8> = self.pending.drain(..start).collect();
                    self.inner.write_all(redact_secrets(&String::from_utf8_lossy(&prefix)).as_bytes())?;
                    self.inner.write_all(REDACTED.as_bytes())?;
                    self.pending.clear();
                }
                self.secret_start = None;
            } else {
                let redacted = redact_secrets(&String::from_utf8_lossy(&self.pending));
                self.inner.write_all(redacted.as_bytes())?;
                self.pending.clear();
            }
        }
        self.inner.flush()
    }
}
