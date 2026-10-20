import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';
import { invoke } from './activity/ipc';
import { CmdError, failureText, fromRejection, isCmdErrorDto, isRetryable } from './cmd-error';
import { errorText } from '../analysis/async-resource';
import { errorMessage } from '../tools/tool-errors';
import { toAiError } from '../ai/ai-backend';
import { toUpdateError } from '../updates/updates.types';
import { profilingErrorCode } from '../profiling/profiling-errors';

describe('CmdError', () => {
  afterEach(() => clearMocks());

  it('is what every command rejects with, through invoke', async () => {
    mockIPC((cmd) => {
      if (cmd === 'import_profile') throw { code: 'notGranted', message: 'choose a.toml again with Open', retryable: false, field: 'path' };
      if (cmd === 'plain') throw 'refused by Tauri';
      return 1;
    });
    const e = await invoke('import_profile', { path: '/a.toml' }).catch((x: unknown) => x);
    expect(e).toBeInstanceOf(CmdError);
    expect(e).toMatchObject({ code: 'notGranted', message: 'choose a.toml again with Open', retryable: false, field: 'path' });
    expect(await invoke('plain').catch((x: unknown) => x)).toBe('refused by Tauri');
    expect(await invoke('ok')).toBe(1);
  });

  it('is read by every error helper: message and next step shown, code and retryable used', () => {
    const e = new CmdError({ code: 'denied', message: 'Access is denied.', retryable: false, hint: 'Restart IMT as administrator.', line: 3, detail: 'd' });
    expect([e.line, e.detail, e.name]).toEqual([3, 'd', 'CmdError']);
    expect(failureText(e)).toBe('Access is denied. Restart IMT as administrator.');
    expect(errorText(e)).toBe(failureText(e));
    expect(errorMessage(e)).toBe(failureText(e));
    expect(isRetryable(e)).toBe(false);
    expect(isRetryable(new Error('x'))).toBe(true);
    expect(failureText(new CmdError({ code: 'busy', message: 'b', retryable: true }))).toBe('b');
    expect(failureText('s')).toBe('s');
    expect(failureText(new Error('m'))).toBe('m');
    expect(failureText({ a: 1 })).toBe('{"a":1}');
    expect(isCmdErrorDto({ code: 'x', message: 'y' })).toBe(false);
    expect(fromRejection(e)).toBe(e);
    // Domain helpers keep working on it.
    expect(toAiError(new CmdError({ code: 'noModel', message: 'set up first', retryable: false }))).toMatchObject({ code: 'noModel', message: 'set up first' });
    expect(toUpdateError(new CmdError({ code: 'offline', message: 'o', retryable: true, hint: 'Check the network.' }))).toEqual({ kind: 'offline', message: 'o', nextStep: 'Check the network.', detail: null });
    expect(profilingErrorCode(new CmdError({ code: 'notACapture', message: 'no', retryable: false }))).toBe('notACapture');
  });
});
