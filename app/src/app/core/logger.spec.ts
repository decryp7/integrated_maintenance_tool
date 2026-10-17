import { ErrorHandler } from '@angular/core';
import { TestBed } from '@angular/core/testing';
import { MemoryLogger } from '../../testing/helpers';
import {
  ConsoleLogger,
  createLogger,
  describeError,
  Logger,
  LoggingErrorHandler,
  provideErrorLogging,
  provideLogger,
  redactPaths,
  TauriLogger,
  toKeyValues,
  withFailureLogging,
} from './logger';

const plugin = vi.hoisted(() => ({
  debug: vi.fn(async () => undefined),
  info: vi.fn(async () => undefined),
  warn: vi.fn(async () => undefined),
  error: vi.fn(async (): Promise<void> => {
    throw new Error('ipc down');
  }),
}));

const setTauri = (on: boolean) => {
  const w = window as unknown as Record<string, unknown>;
  if (on) w['__TAURI_INTERNALS__'] = {};
  else delete w['__TAURI_INTERNALS__'];
};

describe('log helpers', () => {
  it('redacts absolute paths but keeps URLs and file names', () => {
    expect(redactPaths('Failed to open /var/log/app/orders.clef: denied')).toBe('Failed to open <path>: denied');
    expect(redactPaths('Cannot read C:\\Logs\\web01\\app.log')).toBe('Cannot read <path>');
    expect(redactPaths('share \\\\server\\logs\\a.log gone')).toBe('share <path> gone');
    expect(redactPaths('at http://localhost:4200/main.js:10:5')).toBe('at http://localhost:4200/main.js:10:5');
    expect(redactPaths('bad line in app.log')).toBe('bad line in app.log');
    expect(redactPaths('x'.repeat(600))).toHaveLength(501);
  });

  it('builds key/values with SourceContext and stringified props', () => {
    expect(toKeyValues('ui.x', { Events: 3, Ok: true, Gone: undefined, Nil: null, Name: 'n' })).toEqual({ SourceContext: 'ui.x', Events: '3', Ok: 'true', Name: 'n' });
    expect(toKeyValues('ui.y')).toEqual({ SourceContext: 'ui.y' });
  });

  it('describes any thrown value', () => {
    expect(describeError('s')).toBe('s');
    expect(describeError(new Error('e'))).toBe('e');
    expect(describeError({ code: 1 })).toBe('{"code":1}');
    const cyclic: Record<string, unknown> = {};
    cyclic['self'] = cyclic;
    expect(describeError(cyclic)).toBe('[object Object]');
  });
});

describe('loggers', () => {
  afterEach(() => setTauri(false));

  it('sends events to the Rust log with SourceContext and never throws', async () => {
    const l = new TauriLogger(plugin);
    l.debug('ui.a', 'd', { N: 1 });
    l.info('ui.a', 'i');
    l.warn('ui.a', 'w');
    l.error('ui.a', 'e');
    await Promise.resolve();
    expect(plugin.debug).toHaveBeenCalledWith('d', { keyValues: { SourceContext: 'ui.a', N: '1' } });
    expect(plugin.info).toHaveBeenCalledWith('i', { keyValues: { SourceContext: 'ui.a' } });
    expect(plugin.warn).toHaveBeenCalled();
    expect(plugin.error).toHaveBeenCalled();
  });

  it('writes to the console in the browser', () => {
    const spies = (['debug', 'info', 'warn', 'error'] as const).map((m) => vi.spyOn(console, m).mockImplementation(() => undefined));
    const l = new ConsoleLogger();
    l.debug('ui.a', 'd');
    l.info('ui.a', 'i', { X: 1 });
    l.warn('ui.a', 'w');
    l.error('ui.a', 'e');
    expect(spies[0]).toHaveBeenCalledWith('[ui.a] d', {});
    expect(spies[1]).toHaveBeenCalledWith('[ui.a] i', { X: 1 });
    spies.forEach((s) => s.mockRestore());
  });

  it('selects the implementation by host, by default and via provideLogger', () => {
    expect(createLogger()).toBeInstanceOf(ConsoleLogger);
    setTauri(true);
    expect(createLogger()).toBeInstanceOf(TauriLogger);
    TestBed.configureTestingModule({ providers: [provideLogger()] });
    expect(TestBed.inject(Logger)).toBeInstanceOf(TauriLogger);
    setTauri(false);
    TestBed.resetTestingModule();
    expect(TestBed.inject(Logger)).toBeInstanceOf(ConsoleLogger);
  });
});

describe('LoggingErrorHandler', () => {
  it('logs uncaught errors with a redacted stack', () => {
    const logger = new MemoryLogger();
    TestBed.configureTestingModule({ providers: [{ provide: Logger, useValue: logger }, provideErrorLogging()] });
    const handler = TestBed.inject(ErrorHandler);
    expect(handler).toBeInstanceOf(LoggingErrorHandler);
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const err = new Error('boom reading /home/me/logs/a.log');
    err.stack = 'Error: boom\n    at f (/home/me/src/x.ts:1:1)';
    handler.handleError(err);
    handler.handleError('plain');
    consoleError.mockRestore();
    expect(logger.entries[0]).toEqual({ level: 'error', category: 'ui.error', message: 'boom reading <path>', props: { Stack: 'Error: boom\n    at f (<path>:1:1)' } });
    expect(logger.entries[1]).toMatchObject({ message: 'plain', props: { Stack: undefined } });
  });
});

describe('withFailureLogging', () => {
  it('logs rejected calls, rethrows, and passes everything else through', async () => {
    const logger = new MemoryLogger();
    class Svc {
      prefix = 'v';
      ok(x: number) {
        return Promise.resolve(`${this.prefix}${x}`);
      }
      bad() {
        return Promise.reject('cannot open /etc/secret/file.log');
      }
      sync() {
        return 42;
      }
    }
    const svc = withFailureLogging(new Svc(), logger, 'ui.test');
    expect(svc).toBeInstanceOf(Svc);
    expect(svc.prefix).toBe('v');
    expect(await svc.ok(1)).toBe('v1');
    expect(svc.sync()).toBe(42);
    await expect(svc.bad()).rejects.toBe('cannot open /etc/secret/file.log');
    expect(logger.entries).toEqual([{ level: 'warn', category: 'ui.test', message: 'Backend command failed', props: { Command: 'bad', Error: 'cannot open <path>' } }]);
  });
});
