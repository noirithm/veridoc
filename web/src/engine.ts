import type { AnalyzeRequest, WorkerDocument, WorkerResponse } from './protocol';
import type { CaseReport } from './types';

interface Pending {
  resolve: (report: CaseReport) => void;
  reject: (error: Error) => void;
}

/** Runs analyses in a background worker and hands back the results as promises. */
export class Engine {
  private readonly worker: Worker;
  private readonly pending = new Map<number, Pending>();
  private nextId = 1;

  constructor() {
    this.worker = new Worker(new URL('./analysis-worker.ts', import.meta.url), {
      type: 'module',
    });

    this.worker.addEventListener('message', (event: MessageEvent<WorkerResponse>) => {
      const response = event.data;
      const request = this.pending.get(response.id);
      if (!request) return;

      this.pending.delete(response.id);
      if (response.ok) {
        request.resolve(response.report);
      } else {
        request.reject(new Error(response.error));
      }
    });

    this.worker.addEventListener('error', () => {
      this.failAll(new Error('The analysis worker stopped unexpectedly.'));
    });
  }

  analyze(documents: WorkerDocument[]): Promise<CaseReport> {
    const id = this.nextId++;
    const request: AnalyzeRequest = { id, documents };

    return new Promise<CaseReport>((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.worker.postMessage(request);
    });
  }

  dispose(): void {
    this.worker.terminate();
    this.failAll(new Error('The engine was shut down.'));
  }

  private failAll(error: Error): void {
    for (const request of this.pending.values()) {
      request.reject(error);
    }
    this.pending.clear();
  }
}