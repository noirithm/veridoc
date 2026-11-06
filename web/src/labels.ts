import type { DetectorKind, Priority, Status } from './types';

export const DETECTOR_LABELS: Record<DetectorKind, string> = {
  error_level: 'Error level analysis',
  noise: 'Noise analysis',
  copy_move: 'Copy-move check',
  typography: 'Typography check',
  signature: 'Signature check',
  pdf_structure: 'PDF structure check',
  ocr_confidence: 'OCR confidence check',
  cross_document: 'Cross-document comparison',
  ink_marks: 'Pen mark check',
};

export const PRIORITY_LABELS: Record<Priority, string> = {
  high: 'High',
  medium: 'Medium',
  low: 'Low',
};

export const STATUS_LABELS: Record<Status, string> = {
  observed: 'Observed',
  inconsistent: 'Inconsistent',
  potential_manipulation: 'Potential manipulation',
};