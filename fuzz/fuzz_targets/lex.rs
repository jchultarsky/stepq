//! Lexes arbitrary bytes and decodes every string literal.
//!
//! Invariants: no panic; tokens are non-empty, in order, non-overlapping
//! and inside the input.

#![no_main]

use libfuzzer_sys::fuzz_target;
use stepq::p21::{Lexer, TokenKind, decode_string};

fuzz_target!(|data: &[u8]| {
    let mut previous_end = 0;
    for token in Lexer::new(data) {
        let Ok(token) = token else {
            break;
        };
        let span = token.span;
        assert!(span.start >= previous_end, "tokens overlap: {span:?}");
        assert!(span.start < span.end, "empty token: {span:?}");
        assert!(span.end <= data.len(), "token past end of input: {span:?}");
        previous_end = span.end;
        if token.kind == TokenKind::String {
            let _ = decode_string(data, span);
        }
    }
});
