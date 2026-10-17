import { ErrorHandler, inject, Injectable, Injector } from '@angular/core';
import { LoggingErrorHandler } from '../core/logger';
import { ErrorSnapshotService } from './error-snapshot.service';

/**
 * Logs like `LoggingErrorHandler`, then asks for an error snapshot. Angular routes `window` errors and
 * unhandled promise rejections here (`provideBrowserGlobalErrorListeners`). Nothing in here may throw.
 */
@Injectable()
export class SnapshotErrorHandler implements ErrorHandler {
  private readonly logging = inject(LoggingErrorHandler);
  private readonly injector = inject(Injector);

  handleError(error: unknown): void {
    this.logging.handleError(error);
    try {
      this.injector.get(ErrorSnapshotService).unhandled(error);
    } catch {
      /* the snapshot feature must never turn one error into two */
    }
  }
}
