export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export type Status = 'observed' | 'inconsistent' | 'potential_manipulation';
export type Priority = 'low' | 'medium' | 'high';
export type Relation = 'agree' | 'variation' | 'conflict';

export type DetectorKind =
  | 'error_level'
  | 'noise'
  | 'copy_move'
  | 'typography'
  | 'signature'
  | 'pdf_structure'
  | 'ocr_confidence'
  | 'cross_document'
  | 'ink_marks';

export type FieldKind =
  | 'name'
  | 'date_of_birth'
  | 'date'
  | 'amount'
  | 'pan_number'
  | 'aadhaar_number'
  | 'phone'
  | 'email';

export interface OcrWord {
  text: string;
  region: Rect;
  confidence: number;
}

export interface OcrLine {
  words: OcrWord[];
}

export interface Finding {
  id: string;
  document: string;
  page: number;
  region: Rect;
  detector: DetectorKind;
  status: Status;
  confidence: number;
  summary: string;
  detected_at_ms: number;
}

export interface FusedFinding {
  id: string;
  document: string;
  page: number;
  region: Rect;
  status: Status;
  confidence: number;
  priority: Priority;
  detectors: DetectorKind[];
  reason: string;
  members: Finding[];
}

export interface Explanation {
  headline: string;
  observed: string[];
  inferred: string[];
  to_verify: string[];
}

export interface AssessedFinding {
  finding: FusedFinding;
  explanation: Explanation;
}

export interface Field {
  id: string;
  document: string;
  page: number;
  kind: FieldKind;
  raw: string;
  value: string;
  region: Rect;
  confidence: number;
}

export interface Link {
  a: string;
  b: string;
  kind: FieldKind;
  relation: Relation;
}

export interface DocumentSummary {
  id: string;
  name: string;
  sha256: string;
  pages: number;
  fields: number;
}

export interface Failure {
  document: string;
  page: number;
  step: string;
  message: string;
}

export interface AuditEntry {
  index: number;
  timestamp_ms: number;
  actor: string;
  action: string;
  detail: string;
  prev_hash: string;
  hash: string;
}

export interface CaseReport {
  documents: DocumentSummary[];
  fields: Field[];
  graph: { nodes: Field[]; links: Link[] };
  findings: AssessedFinding[];
  overview: string;
  failures: Failure[];
  audit: { entries: AuditEntry[] };
}
