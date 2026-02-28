use std::collections::HashMap;

use crate::utils::constants::BLOCK_SIZE;

#[derive(Debug, Clone)]
pub struct HduHeader {
    #[allow(dead_code)]
    pub cards: Vec<(String, String)>,
    pub index: HashMap<String, String>,
}

impl HduHeader {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.index.get(key).map(|s| s.as_str())
    }

    pub fn get_i64(&self, key: &str) -> Option<i64> {
        self.index.get(key)?.trim().parse().ok()
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.index.get(key)?.trim().parse().ok()
    }

    pub fn data_byte_count(&self) -> usize {
        let naxis = self.get_i64("NAXIS").unwrap_or(0);
        if naxis == 0 {
            return 0;
        }
        let bitpix = self.get_i64("BITPIX").unwrap_or(0);
        let bytes_per_pixel = (bitpix.unsigned_abs() / 8) as usize;
        let mut total: usize = 1;
        for i in 1..=naxis {
            total *= self.get_i64(&format!("NAXIS{}", i)).unwrap_or(1) as usize;
        }
        total * bytes_per_pixel
    }

    pub fn padded_data_bytes(&self) -> usize {
        let raw = self.data_byte_count();
        ((raw + BLOCK_SIZE - 1) / BLOCK_SIZE) * BLOCK_SIZE
    }

    pub fn header_blocks(&self) -> usize {
        let total_cards = self.cards.len() + 1;
        let cards_per_block = BLOCK_SIZE / 80;
        (total_cards + cards_per_block - 1) / cards_per_block
    }

    pub fn data_offset(&self, header_start: usize) -> usize {
        header_start + self.header_blocks() * BLOCK_SIZE
    }
}
