import './style.css';
import { Engine } from './engine';
import type { WorkerDocument } from './protocol';
import type { CaseReport } from './types';

const app = document.getElementById('app');
if (!app) throw new Error('The #app element is missing from index.html.');

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text?: string,
  className?: string,
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag);
  // textContent keeps file names and findings from being treated as HTML.
  if (text !== undefined) node.textContent = text;
  if (className) node.className = className;
  return node;
}

async function readDocuments(files: File[]): Promise<WorkerDocument[]> {
  return Promise.all(
    files.map(async (file, index) => {
      const bytes = new Uint8Array(await file.arrayBuffer());
      return {
        id: `doc-${index + 1}`,
        name: file.name,
        source: bytes,
        pages: [{ image: bytes, lines: [] }],
      };
    }),
  );
}

function renderReport(report: CaseReport): HTMLElement {
  const section = el('section');
  section.append(el('p', report.overview));

  for (const failure of report.failures) {
    section.append(
      el('p', `Could not run ${failure.step} on ${failure.document}: ${failure.message}`, 'meta'),
    );
  }

  const list = el('ol');
  for (const { finding, explanation } of report.findings) {
    const item = el('li');
    item.append(el('strong', explanation.headline));
    item.append(
      el(
        'div',
        `${finding.document}, page ${finding.page + 1}, region at ` +
          `${Math.round(finding.region.x)}, ${Math.round(finding.region.y)}`,
        'meta',
      ),
    );

    // Which checks fired, and what each one measured.
    item.append(el('div', `Checks: ${finding.detectors.join(', ')}`, 'meta'));
    for (const member of finding.members) {
      item.append(el('div', `${member.detector}: ${member.summary}`, 'meta'));
    }

    list.append(item);
  }
  section.append(list);
  section.append(el('p', `Audit log: ${report.audit.entries.length} entries`, 'meta'));
  return section;
}

const engine = new Engine();

const input = el('input');
input.type = 'file';
input.multiple = true;
input.accept = 'image/png,image/jpeg';

const button = el('button', 'Analyse');
const status = el('p', '', 'meta');
const output = el('div');

const controls = el('div', undefined, 'controls');
controls.append(input, button);

app.append(
  el('h1', 'Veridoc'),
  el('p', 'Document forensics that runs in your browser. Files never leave this device.'),
  controls,
  status,
  output,
);

button.addEventListener('click', async () => {
  const files = Array.from(input.files ?? []);
  if (files.length === 0) {
    status.textContent = 'Choose at least one image first.';
    return;
  }

  button.disabled = true;
  status.textContent = 'Analysing...';
  output.replaceChildren();

  try {
    const report = await engine.analyze(await readDocuments(files));
    status.textContent = `Finished ${files.length} document(s).`;
    output.append(renderReport(report));
  } catch (error) {
    status.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    button.disabled = false;
  }
});