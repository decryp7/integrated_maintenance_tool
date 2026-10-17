import { ErrorHandler, inject, Injectable, Provider } from '@angular/core';
import { debug as tauriDebug, error as tauriError, info as tauriInfo, warn as tauriWarn } from '@tauri-apps/plugin-log';
import { isTauri } from './platform';
import { activity, errorCode } from './activity/runtime';
import { ACTIVITY_TRACKED, backendCallSettled, backendCallStarted } from './activity/backend-activity';
import { isUserError } from './activity/severity';

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

/**
 * Adds what correlates an event with the user's activity: the current operation's id and this window's label
 * (unless the caller set them). Every UI log event goes through it, so an error inside an operation carries
 * its `OperationId`.
 */
export function withActivityContext(props: LogProps = {}): LogProps {
  const op = activity.currentOperationId();
  return { ...(op ? { OperationId: op } : {}), Window: activity.window, ...props };
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
    try {
      fn(message, { keyValues: toKeyValues(category, withActivityContext(props)) }).catch(() => undefined);
    } catch {
      /* the plugin is unavailable */
    }
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

/**
 * Browser development and tests: the devtools console. Without Rust to attach them, errors carry this window's
 * latest activity entries (`Breadcrumbs`) here.
 */
export class ConsoleLogger extends Logger {
  debug(category: string, message: string, props?: LogProps): void {
    console.debug(`[${category}] ${message}`, withActivityContext(props));
  }
  info(category: string, message: string, props?: LogProps): void {
    console.info(`[${category}] ${message}`, withActivityContext(props));
  }
  warn(category: string, message: string, props?: LogProps): void {
    console.warn(`[${category}] ${message}`, withActivityContext(props));
  }
  error(category: string, message: string, props?: LogProps): void {
    const crumbs = activity.breadcrumbs();
    console.error(`[${category}] ${message}`, withActivityContext(crumbs.length ? { Breadcrumbs: JSON.stringify(crumbs), ...props } : props));
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
    // Pending coalesced changes (a query being typed) are part of what led here.
    activity.flushCoalesced();
    this.logger.error('ui.error', redactPaths(describeError(error)), {
      ErrorType: errorType(error),
      ErrorCode: errorCode(error),
      View: activity.view(),
      Stack: stack ? redactPaths(stack) : undefined,
    });
    console.error(error);
  }
}

/** The kind of a thrown value (TypeError, a custom error class, string, ...). */
export function errorType(e: unknown): string {
  if (e instanceof Error) return e.constructor?.name || e.name || 'Error';
  if (e === null) return 'null';
  return typeof e;
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
 * redacted error only), reported to `onFailure`, and then rethrown unchanged. It is also where every backend
 * call meets the activity log (`core/activity/backend-activity.ts`): a call inside an operation carries its id;
 * a change made outside any operation becomes one.
 */
export function withFailureLogging<T extends object>(target: T, logger: Logger, options: FailureLoggingOptions = {}): T {
  const { category = 'ui.backend', onFailure } = options;
  return new Proxy(target, {
    get(obj, prop, receiver) {
      if (prop === ACTIVITY_TRACKED) return true;
      const value = Reflect.get(obj, prop, receiver);
      if (typeof value !== 'function') return value;
      return (...args: unknown[]) => {
        const op = typeof prop === 'string' ? backendCallStarted(prop, args) : null;
        let result: unknown;
        try {
          result = (value as (...a: unknown[]) => unknown).apply(obj, args);
        } catch (e) {
          backendCallSettled(op, e);
          throw e;
        }
        if (!(result instanceof Promise)) {
          backendCallSettled(op);
          return result;
        }
        return result.then(
          (v: unknown) => {
            backendCallSettled(op);
            return v;
          },
          (e: unknown) => {
            backendCallSettled(op, e);
            throw e;
          },
        ).catch((e: unknown) => {
          // A mistake the user can fix is a warning; anything else is an unexpected error.
          const log = isUserError(e) ? logger.warn.bind(logger) : logger.error.bind(logger);
          log(category, 'Backend command failed', { Command: String(prop), ErrorCode: errorCode(e), Error: redactPaths(describeError(e)) });
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
