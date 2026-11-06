import type { CaseReport, OcrLine } from './types';

export interface WorkerPage {
  /** PNG or JPEG bytes of the page. */
  image: Uint8Array;
  /** Text lines read from the page. */
  lines: OcrLine[];
}

export interface WorkerDocument {
  id: string;
  name: string;
  /** The original file, kept so its hash can be recorded. */
  source: Uint8Array;
  pages: WorkerPage[];
}

export interface AnalyzeRequest {
  id: number;
  documents: WorkerDocument[];
}

export type WorkerResponse =
  | { id: number; ok: true; report: CaseReport }
  | { id: number; ok: false; error: string };