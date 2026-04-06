#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryMode {
    Clamp,
    Wrap,
    Reflect,
}

#[inline]
pub fn clamp_index(idx: i64, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if idx < 0 {
        0
    } else if idx >= len as i64 {
        len - 1
    } else {
        idx as usize
    }
}

#[inline]
pub fn wrap_index(idx: i64, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let l = len as i64;
    let m = idx % l;
    if m < 0 { (m + l) as usize } else { m as usize }
}

#[inline]
pub fn reflect_index(idx: i64, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    if len == 1 {
        return 0;
    }
    let l = len as i64;
    let period = 2 * (l - 1);
    let mut m = idx % period;
    if m < 0 {
        m += period;
    }
    if m < l {
        m as usize
    } else {
        (period - m) as usize
    }
}

#[inline]
pub fn resolve_index(idx: i64, len: usize, mode: BoundaryMode) -> usize {
    match mode {
        BoundaryMode::Clamp => clamp_index(idx, len),
        BoundaryMode::Wrap => wrap_index(idx, len),
        BoundaryMode::Reflect => reflect_index(idx, len),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clamp_in_range() {
        assert_eq!(clamp_index(3, 10), 3);
    }

    #[test]
    fn test_clamp_negative() {
        assert_eq!(clamp_index(-5, 10), 0);
    }

    #[test]
    fn test_clamp_overflow() {
        assert_eq!(clamp_index(15, 10), 9);
    }

    #[test]
    fn test_clamp_zero_len() {
        assert_eq!(clamp_index(0, 0), 0);
    }

    #[test]
    fn test_clamp_len_one() {
        assert_eq!(clamp_index(5, 1), 0);
        assert_eq!(clamp_index(-3, 1), 0);
    }

    #[test]
    fn test_wrap_positive() {
        assert_eq!(wrap_index(12, 10), 2);
    }

    #[test]
    fn test_wrap_negative() {
        assert_eq!(wrap_index(-1, 10), 9);
        assert_eq!(wrap_index(-11, 10), 9);
    }

    #[test]
    fn test_wrap_zero_len() {
        assert_eq!(wrap_index(5, 0), 0);
    }

    #[test]
    fn test_reflect_in_range() {
        assert_eq!(reflect_index(3, 10), 3);
    }

    #[test]
    fn test_reflect_negative() {
        assert_eq!(reflect_index(-1, 10), 1);
        assert_eq!(reflect_index(-2, 10), 2);
    }

    #[test]
    fn test_reflect_overflow() {
        assert_eq!(reflect_index(10, 10), 8);
        assert_eq!(reflect_index(11, 10), 7);
    }

    #[test]
    fn test_reflect_zero_len() {
        assert_eq!(reflect_index(3, 0), 0);
    }

    #[test]
    fn test_reflect_len_one() {
        assert_eq!(reflect_index(5, 1), 0);
        assert_eq!(reflect_index(-3, 1), 0);
    }

    #[test]
    fn test_resolve_dispatch() {
        assert_eq!(resolve_index(-1, 10, BoundaryMode::Clamp), 0);
        assert_eq!(resolve_index(-1, 10, BoundaryMode::Wrap), 9);
        assert_eq!(resolve_index(-1, 10, BoundaryMode::Reflect), 1);
    }
}
