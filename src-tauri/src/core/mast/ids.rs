use super::errors::bad_input;
use super::types::MastCmdError;

pub const REQUEST_ID_MAX: usize = 512;
pub const DATASET_ID_MAX: usize = 200;

fn request_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, ':' | '.' | '_' | '-')
}

fn dataset_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

pub fn validate_request_id(value: &str) -> Result<(), MastCmdError> {
    if value.is_empty() || value.len() > REQUEST_ID_MAX || !value.chars().all(request_id_char) {
        return Err(bad_input("requestId", "use 1 to 512 letters, digits or : . _ -"));
    }
    Ok(())
}

fn validate_component(field: &str, value: &str) -> Result<(), MastCmdError> {
    if value.is_empty() || value.len() > DATASET_ID_MAX || !value.chars().all(dataset_char) {
        return Err(bad_input(field, "use 1 to 200 letters, digits or . _ -"));
    }
    if value.starts_with('.') || value.contains("..") {
        return Err(bad_input(field, "must not start with a dot or contain .."));
    }
    Ok(())
}

pub fn validate_dataset_id(value: &str) -> Result<(), MastCmdError> {
    validate_component("datasetId", value)
}

pub fn validate_filename(value: &str) -> Result<(), MastCmdError> {
    validate_component("filename", value)
}

pub fn path_component(value: &str) -> Result<&str, MastCmdError> {
    validate_dataset_id(value)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::MastErrorCode;

    #[test]
    fn ids_follow_the_contract_patterns() {
        for good in ["g1:search", "g12:thumb:jw02739-o001_t001_nircam", "token:save", "a.b_c-d", &"x".repeat(512)] {
            assert!(validate_request_id(good).is_ok(), "{good}");
        }
        for bad in ["", "g1 search", "g1/../x", "g1:\u{e9}", "g1:{x}", &"x".repeat(513)] {
            let error = validate_request_id(bad).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad}");
            assert!(error.message.starts_with("requestId: "), "{}", error.message);
        }
        for good in ["jw02739-o001_t001_nircam", "jw02739001002_02101_00001", "ICK909030", "a.b", &"d".repeat(200)] {
            assert!(validate_dataset_id(good).is_ok(), "{good}");
            assert_eq!(path_component(good).unwrap(), good);
        }
        for bad in ["", ".hidden", "a..b", "..", "a/b", "a\\b", "a:b", "C:", "a b", &"d".repeat(201)] {
            let error = validate_dataset_id(bad).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{bad}");
            assert!(error.message.starts_with("datasetId: "), "{}", error.message);
            assert!(path_component(bad).is_err(), "{bad}");
        }
        assert!(validate_filename("jw02739-o001_t001_nircam_clear-f187n_i2d_thumb.jpg").is_ok());
        let error = validate_filename("../x.jpg").unwrap_err();
        assert!(error.message.starts_with("filename: "), "{}", error.message);
    }
}
