//! Frame-to-frame reuse for render buffers: a pool of cleared strings for
//! owned span text, and vectors recycled across element lifetimes. With
//! both, a steady-state frame builds its rows without heap allocations.

use crate::terminal::Span;
use std::borrow::Cow;
use std::cell::RefCell;

thread_local! {
    /// Recycled span strings (cleared, capacity kept).
    static STRING_POOL: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Pop a cleared string from the pool (or start a fresh one).
pub(super) fn pooled_string() -> String {
    STRING_POOL.with(|pool| {
        let mut s = pool.borrow_mut().pop().unwrap_or_default();
        s.clear();
        s
    })
}

/// Format into a pooled string.
pub(super) fn pooled_fmt(args: std::fmt::Arguments<'_>) -> String {
    let mut s = pooled_string();
    let _ = std::fmt::Write::write_fmt(&mut s, args);
    s
}

/// A pooled copy of `text`.
pub(super) fn pooled_str(text: &str) -> String {
    let mut s = pooled_string();
    s.push_str(text);
    s
}

/// Hand painted spans' owned strings back to the pool and empty the vector.
pub(super) fn recycle_spans(spans: &mut Vec<Span<'_>>) {
    STRING_POOL.with(|pool| {
        let mut pool = pool.borrow_mut();
        for span in spans.drain(..) {
            if let Cow::Owned(mut text) = span.content {
                text.clear();
                pool.push(text);
            }
        }
    });
}

/// Reuse an emptied vector's allocation for elements of another lifetime:
/// `collect` from `vec::IntoIter` writes in place when the element layouts
/// match, which they do for one type at two lifetimes.
pub(super) fn recycle_vec<T, U>(mut vec: Vec<T>) -> Vec<U> {
    vec.clear();
    vec.into_iter().map(|_| -> U { unreachable!() }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::terminal::{RowSeg, Style};

    #[test]
    fn recycled_vectors_keep_their_allocation() {
        // Row painting relies on this to reuse one segment vector per row.
        let store: Vec<RowSeg<'static>> = Vec::with_capacity(16);
        let buffer = store.as_ptr() as usize;
        let text = String::from("cell");
        let spans = [Span::raw(text.as_str())];
        let mut segs: Vec<RowSeg<'_>> = recycle_vec(store);
        assert_eq!((segs.as_ptr() as usize, segs.capacity()), (buffer, 16));
        segs.push(RowSeg {
            x: 0,
            width: 4,
            style: Style::default(),
            line_style: Style::default(),
            spans: &spans,
        });
        let store: Vec<RowSeg<'static>> = recycle_vec(segs);
        assert!(store.is_empty());
        assert_eq!((store.as_ptr() as usize, store.capacity()), (buffer, 16));
    }

    #[test]
    fn recycled_span_strings_are_reused_cleared() {
        let mut owned = pooled_fmt(format_args!("{}-{}", 12, "ab"));
        owned.reserve(64);
        let capacity = owned.capacity();
        let mut spans = vec![Span::raw(owned), Span::raw("borrowed")];
        recycle_spans(&mut spans);
        assert!(spans.is_empty());
        let reused = pooled_string();
        assert!(reused.is_empty());
        assert_eq!(reused.capacity(), capacity);
    }
}
