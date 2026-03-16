import { FILE_STATUS } from "../../utils/constants";

export type FileStatus = (typeof FILE_STATUS)[keyof typeof FILE_STATUS];

export interface QueueStats {
  total: number;
  done: number;
  failed: number;
  totalBytes: number;
}
