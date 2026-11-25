mod contract;
mod fitting;
mod matching;
mod search;
#[cfg(test)]
mod test_suite;

use contract::{catalog_tiers, constants, types};
use fitting::{distortion, tan_fit, tan_projection, verification};
use matching::{detection, pairing, triangle_match};
use search::{centre_trial, search_plan, solve};

#[cfg(test)]
use test_suite::acceptance;
#[cfg(test)]
use test_suite::bench;
#[cfg(test)]
pub(crate) use test_suite::synthetic;
#[cfg(test)]
use test_suite::tests;

pub use catalog_tiers::*;
#[cfg(test)]
use centre_trial::*;
pub use constants::*;
pub use detection::*;
#[cfg(test)]
use distortion::*;
#[cfg(test)]
use pairing::*;
#[cfg(test)]
use search_plan::*;
pub use solve::*;
#[cfg(test)]
use tan_fit::*;
pub(crate) use tan_projection::*;
#[cfg(test)]
use triangle_match::*;
pub use types::*;
pub(crate) use verification::*;
