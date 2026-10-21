import { describe, expect, it } from 'vitest';
import { headlineText } from './dump-headline';

describe('headlineText', () => {
  it('words an exception with or without its location', () => {
    expect(headlineText({ type: 'exception', name: 'ACCESS_VIOLATION', location: 'ntdll.dll!RtlFreeHeap+0x1d' })).toBe('ACCESS_VIOLATION in ntdll.dll!RtlFreeHeap+0x1d');
    expect(headlineText({ type: 'exception', name: 'CLR_EXCEPTION', location: null })).toBe('CLR_EXCEPTION');
  });

  it('counts threads of a dump without an exception', () => {
    expect(headlineText({ type: 'noException', threads: 1 })).toBe('1 thread, no exception');
    expect(headlineText({ type: 'noException', threads: 1200 })).toBe('1,200 threads, no exception');
  });

  it('formats UMDH sizes with the shared units rules', () => {
    expect(headlineText({ type: 'umdhSnapshot', stacks: 412, bytes: 40_055_603 })).toBe('412 allocation stacks, 38 MB');
    expect(headlineText({ type: 'umdhSnapshot', stacks: 1, bytes: 1536 })).toBe('1 allocation stack, 1.5 KB');
    expect(headlineText({ type: 'umdhDiff', stacks: 3, deltaBytes: 2 << 20 })).toBe('+2 MB across 3 allocation stacks');
    expect(headlineText({ type: 'umdhDiff', stacks: 1, deltaBytes: -(5 << 20) })).toBe('-5 MB across 1 allocation stack');
  });
});
