mod api;
mod models;
#[cfg(test)]
mod tests;

pub use api::ClassyApi;
pub use models::{ClassySearchResultItem, ClassySearchResults};
