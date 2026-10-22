import { inject, Injectable } from '@angular/core';
import { ExplainNode, InferredRegex } from '../../core/api.types';
import { DebriefBackend } from '../../core/backend';

/** Regex inference and explanation. The engine is the only implementation; this is a thin seam for the views. */
@Injectable({ providedIn: 'root' })
export class RegexApi {
  private readonly backend = inject(DebriefBackend);

  infer(lines: string[]): Promise<InferredRegex> {
    return this.backend.inferRegex(lines);
  }

  explain(pattern: string): Promise<ExplainNode[]> {
    return this.backend.explainRegex(pattern);
  }
}
