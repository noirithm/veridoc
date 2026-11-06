import './style.css';
import { el } from './dom';
import { Engine } from './engine';
import type { WorkerDocument } from './protocol';
import { createViewer } from './viewer';

const app = document.getElementById('app');
if (!app) throw new Error('The #app element is missing from index.html.');

interface Upload {
  documents: WorkerDocument[];
  /** Object URL of each document's image, keyed by document id. */
  images: Map<string, string>;
}

async function readUpload(files: File[]): Promise<Upload> {
  const images = new Map<string, string>();

  const documents = await Promise.all(
    files.map(async (file, index) => {
      const id = `doc-${index + 1}`;
      const bytes = new Uint8Array(await file.arrayBuffer());
      images.set(id, URL.createObjectURL(file));
      return { id, name: file.name, source: bytes, pages: [{ image: bytes, lines: [] }] };
    }),
  );
  return { documents, images };
}

function release(images: Map<string, string>): void {
  for (const url of images.values()) {
    URL.revokeObjectURL(url);
  }
}

const engine = new Engine();
let shown = new Map<string, string>();

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

  let upload: Upload | undefined;
  try {
    upload = await readUpload(files);
    const report = await engine.analyze(upload.documents);

    release(shown);
    shown = upload.images;
    output.replaceChildren(createViewer({ report, images: shown }));
    status.textContent = `Finished ${files.length} document(s).`;
  } catch (error) {
    if (upload && upload.images !== shown) release(upload.images);
    status.textContent = error instanceof Error ? error.message : String(error);
  } finally {
    button.disabled = false;
  }
});