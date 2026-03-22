export interface HeaderData {
  file_name: string;
  file_path: string;
  total_cards: number;
  cards: Array<{ key: string; value: string }>;
  categories: Record<string, Record<string, string>>;
  filter_detection: {
    filter: string;
    filter_id: string;
    hubble_channel: string;
    confidence: string;
    matched_keyword: string;
    matched_value: string;
  } | null;
  filename_hint: string | null;
}
