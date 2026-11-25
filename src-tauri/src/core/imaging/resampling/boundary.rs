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
}
