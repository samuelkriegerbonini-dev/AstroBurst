export interface TileLevel {
  level: number;
  width: number;
  height: number;
  cols: number;
  rows: number;
  scale_factor: number;
}

export interface TileResult {
  tile_size: number;
  original_width: number;
  original_height: number;
  levels: TileLevel[];
  base_dir: string;
}
