const GOLDEN_GAMMA: u64 = 0x9E37_79B9_7F4A_7C15;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SeedPurpose {
    Field,
    Noise,
    Flat,
    Frame,
    Cosmic,
}

impl SeedPurpose {
    pub const ALL: [SeedPurpose; 5] = [Self::Field, Self::Noise, Self::Flat, Self::Frame, Self::Cosmic];

    fn tag(self) -> u64 {
        let bytes: [u8; 8] = match self {
            Self::Field => *b"field\0\0\0",
            Self::Noise => *b"noise\0\0\0",
            Self::Flat => *b"flat\0\0\0\0",
            Self::Frame => *b"frame\0\0\0",
            Self::Cosmic => *b"cosmic\0\0",
        };
        u64::from_le_bytes(bytes)
    }
}

pub fn splitmix64(state: u64) -> u64 {
    let mut z = state.wrapping_add(GOLDEN_GAMMA);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub fn derive_seed(user_seed: u64, purpose: SeedPurpose, frame_index: u32) -> u64 {
    let scoped = splitmix64(splitmix64(user_seed) ^ purpose.tag());
    splitmix64(scoped ^ u64::from(frame_index))
}

pub fn combine_seeds(user_seed: u64, extra: Option<u64>) -> u64 {
    match extra {
        None => user_seed,
        Some(extra) => splitmix64(splitmix64(user_seed) ^ splitmix64(extra).rotate_left(32)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn purposes_of_one_seed_are_independent_streams() {
        for seed in [0u64, 1, 42, 7919, u64::MAX] {
            let mut seen = HashMap::new();
            for purpose in SeedPurpose::ALL {
                let derived = derive_seed(seed, purpose, 0);
                assert_ne!(derived, seed, "{purpose:?} reused the raw seed {seed}");
                if let Some(other) = seen.insert(derived, purpose) {
                    panic!("seed {seed}: {purpose:?} and {other:?} share {derived}");
                }
            }
        }
    }

    #[test]
    fn frame_seeds_never_collide_with_other_seeds_across_the_slider_range() {
        let mut owners: HashMap<u64, (u64, &str, u32)> = HashMap::new();
        for seed in 0..=10_000u64 {
            let field = derive_seed(seed, SeedPurpose::Field, 0);
            if let Some(prev) = owners.insert(field, (seed, "field", 0)) {
                panic!("field seed of {seed} collides with {prev:?}");
            }
            for frame in 0..64u32 {
                let noise = derive_seed(seed, SeedPurpose::Noise, frame);
                if let Some(prev) = owners.insert(noise, (seed, "noise", frame)) {
                    panic!("noise seed of ({seed}, frame {frame}) collides with {prev:?}");
                }
            }
        }
        assert_ne!(derive_seed(0, SeedPurpose::Noise, 1), derive_seed(7919, SeedPurpose::Noise, 0));
        assert_ne!(derive_seed(0, SeedPurpose::Flat, 1), derive_seed(1, SeedPurpose::Flat, 0));
    }

    #[test]
    fn a_legacy_noise_seed_is_mixed_in_rather_than_used_raw() {
        assert_eq!(combine_seeds(42, None), 42);
        let mixed = combine_seeds(42, Some(1042));
        assert_ne!(mixed, 1042);
        assert_ne!(mixed, 42);
        assert_ne!(mixed, combine_seeds(42, Some(1043)));
        assert_ne!(mixed, combine_seeds(43, Some(1042)));
        assert_eq!(mixed, combine_seeds(42, Some(1042)));
    }
}
