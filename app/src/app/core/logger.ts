import { ErrorHandler, inject, Injectable, Provider } from '@angular/core';
import { debug as tauriDebug, error as tauriError, info as tauriInfo, warn as tauriWarn } from '@tauri-apps/plugin-log';
import { isTauri } from './platform';

/** Structured properties attached to a log event (PascalCase names, Serilog style). */
export type LogProps = Record<string, string | number | boolean | null | undefined>;

/** Where UI diagnostics go. `category` plays the role of Serilog's SourceContext (e.g. "ui.timeline"). */
@Injectable({ providedIn: 'root', useFactory: createLogger })
export abstract class Logger {
  abstract debug(category: string, message: string, props?: LogProps): void;
  abstract info(category: string, message: string, props?: LogProps): void;
  abstract warn(category: string, message: string, props?: LogProps): void;
  abstract error(category: string, message: string, props?: LogProps): void;
}

const MAX_TEXT = 500;

/**
 * Replaces absolute file-system paths (POSIX and Windows) with "<path>" and caps the length,
 * so log events never carry full paths. URLs are left alone.
 */
export function redactPaths(text: string): string {
  const redacted = text
    .replace(/\b[A-Za-z]:\\[^\s'"<>|]+/g, '<path>')
    .replace(/\\\\[^\s'"<>|]+/g, '<path>')
    .replace(/(?<![\w:/.])\/(?:[^\s/'"()<>]+\/)+[^\s/'"()<>:]*/g, '<path>');
  return redacted.length > MAX_TEXT ? `${redacted.slice(0, MAX_TEXT)}…` : redacted;
}

/** Converts properties to the string key/values the log plugin accepts, with SourceContext set. */
export function toKeyValues(category: string, props: LogProps = {}): Record<string, string> {
  const out: Record<string, string> = { SourceContext: category };
  for (const [k, v] of Object.entries(props)) if (v !== undefined && v !== null) out[k] = String(v);
  return out;
}

/** Describes any thrown value as text. */
export function describeError(e: unknown): string {
  if (typeof e === 'string') return e;
  if (e instanceof Error) return e.message;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

type PluginFn = (message: string, options?: { keyValues?: Record<string, string | undefined> }) => Promise<void>;

/** The subset of @tauri-apps/plugin-log used here (injectable for tests). */
export interface LogPlugin {
  debug: PluginFn;
  info: PluginFn;
  warn: PluginFn;
  error: PluginFn;
}

const TAURI_LOG: LogPlugin = { debug: tauriDebug, info: tauriInfo, warn: tauriWarn, error: tauriError };

/** Sends events to the Rust log (tauri-plugin-log), which writes them as CLEF alongside the backend's. */
export class TauriLogger extends Logger {
  constructor(private readonly plugin: LogPlugin = TAURI_LOG) {
    super();
  }
  private send(fn: PluginFn, category: string, message: string, props?: LogProps): void {
    // Logging must never break the UI.
    fn(message, { keyValues: toKeyValues(category, props) }).catch(() => undefined);
  }
  debug(category: string, message: string, props?: LogProps): void {
    this.send(this.plugin.debug, category, message, props);
  }
  info(category: string, message: string, props?: LogProps): void {
    this.send(this.plugin.info, category, message, props);
  }
  warn(category: string, message: string, props?: LogProps): void {
    this.send(this.plugin.warn, category, message, props);
  }
  error(category: string, message: string, props?: LogProps): void {
    this.send(this.plugin.error, category, message, props);
  }
}

/** Browser development and tests: the devtools console. */
export class ConsoleLogger extends Logger {
  debug(category: string, message: string, props?: LogProps): void {
    console.debug(`[${category}] ${message}`, props ?? {});
  }
  info(category: string, message: string, props?: LogProps): void {
    console.info(`[${category}] ${message}`, props ?? {});
  }
  warn(category: string, message: string, props?: LogProps): void {
    console.warn(`[${category}] ${message}`, props ?? {});
  }
  error(category: string, message: string, props?: LogProps): void {
    console.error(`[${category}] ${message}`, props ?? {});
  }
}

export function createLogger(): Logger {
  return isTauri() ? new TauriLogger() : new ConsoleLogger();
}

/** Rust log inside Tauri, the console in a plain browser. */
export function provideLogger(): Provider {
  return { provide: Logger, useFactory: createLogger };
}

/** Logs uncaught errors (message and stack, paths redacted) before the default console output. */
@Injectable()
export class LoggingErrorHandler implements ErrorHandler {
  private readonly logger = inject(Logger);

  handleError(error: unknown): void {
    const stack = error instanceof Error ? error.stack : undefined;
    this.logger.error('ui.error', redactPaths(describeError(error)), { Stack: stack ? redactPaths(stack) : undefined });
    console.error(error);
  }
}

export function provideErrorLogging(): Provider {
  return { provide: ErrorHandler, useClass: LoggingErrorHandler };
}

export interface FailureLoggingOptions {
  /** Log category (default "ui.backend"). */
  category?: string;
  /** Called with the operation name and the error after logging, before the rejection is rethrown. Must not throw (errors it throws are ignored). */
  onFailure?: (command: string, error: unknown) => void;
}

/**
 * Decorates a promise-returning service so every rejected call is logged (operation name and
 * redacted error only), reported to `onFailure`, and then rethrown unchanged.
 */
export function withFailureLogging<T extends object>(target: T, logger: Logger, options: FailureLoggingOptions = {}): T {
  const { category = 'ui.backend', onFailure } = options;
  return new Proxy(target, {
    get(obj, prop, receiver) {
      const value = Reflect.get(obj, prop, receiver);
      if (typeof value !== 'function') return value;
      return (...args: unknown[]) => {
        const result = (value as (...a: unknown[]) => unknown).apply(obj, args);
        if (!(result instanceof Promise)) return result;
        return result.catch((e: unknown) => {
          logger.warn(category, 'Backend command failed', { Command: String(prop), Error: redactPaths(describeError(e)) });
          try {
            onFailure?.(String(prop), e);
          } catch {
            /* reporting must not replace the original error */
          }
          throw e;
        });
      };
    },
  });
}
