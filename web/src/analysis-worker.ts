import init, { Case } from '../pkg/veridoc_wasm.js';
import type { AnalyzeRequest, WorkerResponse } from './protocol';
import type { CaseReport } from './types';

// Start loading the WebAssembly module as soon as the worker starts.
const ready = init();

function reply(message: WorkerResponse): void {
  (self as unknown as Worker).postMessage(message);
}

function describe(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

self.addEventListener('message', async (event: MessageEvent<AnalyzeRequest>) => {
  const { id, documents } = event.data;

  try {
    await ready;

    const session = new Case();
    try {
      for (const document of documents) {
        session.addDocument(document.id, document.name, document.source);
        for (const page of document.pages) {
          session.addPage(document.id, page.image, JSON.stringify(page.lines));
        }
      }
      const report = JSON.parse(session.analyze()) as CaseReport;
      reply({ id, ok: true, report });
    } finally {
      // The Rust side owns this memory, so it has to be released by hand.
      session.free();
    }
  } catch (error) {
    reply({ id, ok: false, error: describe(error) });
  }
});