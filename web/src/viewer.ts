import { bulletList, el } from './dom';
import { DETECTOR_LABELS, PRIORITY_LABELS, STATUS_LABELS } from './labels';
import type { AssessedFinding, CaseReport } from './types';

export interface ViewerInput {
  report: CaseReport;
  /** Object URL of each document's page image, keyed by document id. */
  images: Map<string, string>;
}

// Each document is shown as a single page for now.
const SHOWN_PAGE = 0;

export function createViewer({ report, images }: ViewerInput): HTMLElement {
  const root = el('section', undefined, 'viewer');
  root.append(el('p', report.overview));

  for (const failure of report.failures) {
    root.append(
      el('p', `Could not run ${failure.step} on ${failure.document}: ${failure.message}`, 'meta'),
    );
  }

  const tabs = el('div', undefined, 'tabs');
  const frameHolder = el('div');
  const listHolder = el('div');
  const detail = el('div', undefined, 'detail');

  const side = el('div', undefined, 'side');
  side.append(listHolder, detail);

  const layout = el('div', undefined, 'layout');
  layout.append(frameHolder, side);

  root.append(tabs, layout, el('p', `Audit log: ${report.audit.entries.length} entries.`, 'meta'));

  let currentDocument = '';
  let selectedId: string | null = null;
  const boxes = new Map<string, HTMLElement>();
  const rows = new Map<string, HTMLElement>();

  const scrollBehavior: ScrollBehavior = window.matchMedia('(prefers-reduced-motion: reduce)').matches
    ? 'auto'
    : 'smooth';

  function findingsFor(documentId: string): AssessedFinding[] {
    return report.findings.filter(({ finding }) => finding.document === documentId);
  }

  function reveal(node: HTMLElement | undefined): void {
    node?.scrollIntoView({ block: 'nearest', inline: 'nearest', behavior: scrollBehavior });
  }

  function select(id: string | null): void {
    selectedId = id;

    for (const [key, node] of boxes) {
      node.classList.toggle('selected', key === id);
      node.setAttribute('aria-pressed', String(key === id));
    }
    for (const [key, node] of rows) {
      node.classList.toggle('selected', key === id);
      node.setAttribute('aria-pressed', String(key === id));
    }

    if (id) {
      reveal(boxes.get(id));
      reveal(rows.get(id));
    }
    renderDetail();
  }

  function renderTabs(): void {
    if (report.documents.length < 2) {
      tabs.replaceChildren();
      return;
    }

    const buttons = report.documents.map((summary) => {
      const active = summary.id === currentDocument;
      const tab = el('button', `${summary.name} (${findingsFor(summary.id).length})`, 'tab');
      tab.type = 'button';
      tab.classList.toggle('selected', active);
      tab.setAttribute('aria-pressed', String(active));
      tab.addEventListener('click', () => showDocument(summary.id));
      return tab;
    });
    tabs.replaceChildren(...buttons);
  }

  function renderStage(): void {
    const documentId = currentDocument;
    const summary = report.documents.find((entry) => entry.id === documentId);
    const url = images.get(documentId);
    if (!summary || !url) {
      frameHolder.replaceChildren(el('p', 'This document cannot be displayed.', 'meta'));
      return;
    }

    const frame = el('div', undefined, 'frame');
    const image = el('img');
    image.alt = `${summary.name}, page ${SHOWN_PAGE + 1}`;
    frame.append(image);

    // Boxes are positioned in percentages of the image's own size, so they
    // stay on the right spot however large the picture is drawn.
    let placed = false;
    const placeBoxes = (): void => {
      if (placed || documentId !== currentDocument || image.naturalWidth === 0) return;
      placed = true;

      const { naturalWidth: width, naturalHeight: height } = image;
      for (const { finding, explanation } of findingsFor(documentId)) {
        if (finding.page !== SHOWN_PAGE) continue;

        const { x, y, width: boxWidth, height: boxHeight } = finding.region;
        const box = el('button', undefined, `box ${finding.priority}`);
        box.type = 'button';
        box.setAttribute('aria-label', explanation.headline);
        box.setAttribute('aria-pressed', 'false');
        box.style.left = `${(x / width) * 100}%`;
        box.style.top = `${(y / height) * 100}%`;
        box.style.width = `${(boxWidth / width) * 100}%`;
        box.style.height = `${(boxHeight / height) * 100}%`;
        box.addEventListener('click', () => select(finding.id));

        boxes.set(finding.id, box);
        frame.append(box);
      }

      if (selectedId) {
        boxes.get(selectedId)?.classList.add('selected');
      }
    };

    image.addEventListener('load', placeBoxes);
    image.src = url;
    placeBoxes(); // a cached image may already be loaded
    frameHolder.replaceChildren(frame);
  }

  function renderList(): void {
    const items = findingsFor(currentDocument);
    if (items.length === 0) {
      listHolder.replaceChildren(
        el(
          'p',
          'No findings were recorded for this document. That does not show it is genuine, ' +
            'because the checks have known limits.',
          'meta',
        ),
      );
      return;
    }

    const ordered = el('ol', undefined, 'finding-list');
    for (const { finding } of items) {
      const checks = finding.detectors.map((kind) => DETECTOR_LABELS[kind]).join(' + ');
      const place = `${Math.round(finding.region.x)}, ${Math.round(finding.region.y)}`;

      const row = el('button', undefined, 'row');
      row.type = 'button';
      row.append(
        el('span', PRIORITY_LABELS[finding.priority], `badge ${finding.priority}`),
        el('span', STATUS_LABELS[finding.status], 'row-title'),
        el('span', checks, 'row-meta'),
        el('span', `score ${finding.confidence.toFixed(2)}, at ${place}`, 'row-meta'),
      );
      row.addEventListener('click', () => select(finding.id));
      rows.set(finding.id, row);

      const item = el('li');
      item.append(row);
      ordered.append(item);
    }
    listHolder.replaceChildren(ordered);
  }

  function renderDetail(): void {
    const selected = selectedId
      ? report.findings.find(({ finding }) => finding.id === selectedId)
      : undefined;

    if (!selected) {
      detail.replaceChildren(
        el('p', 'Select a finding, in the list or on the page, to see the evidence behind it.', 'meta'),
      );
      return;
    }

    const { finding, explanation } = selected;
    const { x, y, width, height } = finding.region;
    const location =
      `Page ${finding.page + 1}, ${Math.round(width)} x ${Math.round(height)} px ` +
      `at (${Math.round(x)}, ${Math.round(y)}).`;

    detail.replaceChildren(
      el('h3', explanation.headline),
      el('p', `${STATUS_LABELS[finding.status]}. ${location}`, 'meta'),
      el('h4', 'Observed'),
      bulletList(explanation.observed),
      el('h4', 'Inferred'),
      bulletList(explanation.inferred),
      el('h4', 'To verify'),
      bulletList(explanation.to_verify),
      el('h4', 'Evidence trail'),
      bulletList(
        finding.members.map(
          (member) =>
            `${DETECTOR_LABELS[member.detector]}, finding ${member.id}, score ${member.confidence.toFixed(2)}`,
        ),
      ),
      el('p', 'Scores rank findings for review. They are not probabilities.', 'meta'),
    );
  }

  function showDocument(documentId: string): void {
    currentDocument = documentId;
    boxes.clear();
    rows.clear();
    renderTabs();
    renderStage();
    renderList();
    select(null);
  }

  const first = report.documents[0];
  if (first) {
    showDocument(first.id);
  }
  return root;
}