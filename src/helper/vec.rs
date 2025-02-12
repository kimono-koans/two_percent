use std::vec::Vec;

#[inline]
pub fn concat<T: Sized>(lhs: &mut Vec<T>, rhs: &mut [T]) {
    append_elements(lhs, rhs as _);
}

/// Appends elements to `self` from other buffer.
#[inline]
fn append_elements<T: Sized>(lhs: &mut Vec<T>, rhs: *const [T]) {
    let count = rhs.len();
    lhs.reserve(count);
    let len = lhs.len();
    let new_len = len + count;

    unsafe {
        std::ptr::copy_nonoverlapping(rhs as *const T, lhs.as_mut_ptr().add(len), count);
        lhs.set_len(new_len);
    }
}
