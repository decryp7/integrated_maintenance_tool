import { describe, expect, it } from 'vitest';
import { CmdError } from '../core/cmd-error';
import { AlignEvidence, VideoRecord } from '../core/generated/video';
import {
  alignedByLabel,
  confidenceLabel,
  evidenceText,
  formatOffsetSeconds,
  formatVideoTime,
  frameAlignedMs,
  frameName,
  fromInputValue,
  hasVideoExtension,
  parseOffsetSeconds,
  removalOf,
  removalText,
  selectionNoteText,
  selectionText,
  toFailure,
  toInputValue,
  userAlignmentText,
} from './video-format';

describe('video times', () => {
  it('formats a time in the video', () => {
    expect(formatVideoTime(0)).toBe('00:00');
    expect(formatVideoTime(12_900)).toBe('00:12');
    expect(formatVideoTime(12_345, true)).toBe('00:12.345');
    expect(formatVideoTime(3_723_000)).toBe('1:02:03');
    expect(formatVideoTime(3_723_004, true)).toBe('1:02:03.004');
    expect(formatVideoTime(-5)).toBe('00:00');
  });

  it('names a frame and places it on the logs clock', () => {
    expect(frameName(2, { videoMs: 75_000 })).toBe('Frame 3 at 01:15');
    expect(frameAlignedMs({ alignment: { startMs: 1000 } } as VideoRecord, { videoMs: 500 })).toBe(1500);
  });

  it('recognises video file names', () => {
    expect(['a.MP4', 'b.m4v', 'c.mov', 'd.mkv', 'e.webm', 'f.avi'].every(hasVideoExtension)).toBe(true);
    expect(hasVideoExtension('app.log')).toBe(false);
    expect(hasVideoExtension('noextension')).toBe(false);
  });

  it('parses and prints offsets', () => {
    expect(parseOffsetSeconds('1.5')).toBe(1500);
    expect(parseOffsetSeconds(' -30 ')).toBe(-30_000);
    expect(parseOffsetSeconds('2,25')).toBe(2250);
    expect(parseOffsetSeconds('')).toBeNull();
    expect(parseOffsetSeconds('1e3')).toBeNull();
    expect(formatOffsetSeconds(1500)).toBe('+1.5');
    expect(formatOffsetSeconds(-30_000)).toBe('-30');
  });
});

describe('date-time inputs', () => {
  it('round-trips in UTC and in the local zone', () => {
    const ms = Date.UTC(2026, 9, 14, 10, 42, 7);
    expect(toInputValue(ms, 'utc')).toBe('2026-10-14T10:42:07');
    expect(fromInputValue('2026-10-14T10:42:07', 'utc')).toBe(ms);
    const local = toInputValue(ms, 'local');
    expect(fromInputValue(local, 'local')).toBe(ms);
    expect(fromInputValue('2026-10-14T10:42', 'utc')).toBe(Date.UTC(2026, 9, 14, 10, 42, 0));
  });

  it('refuses what is not a date and time', () => {
    expect(fromInputValue('', 'utc')).toBeNull();
    expect(fromInputValue('yesterday', 'local')).toBeNull();
    expect(fromInputValue('99999999-01-01T00:00:00', 'utc')).toBeNull();
  });
});

describe('alignment in words', () => {
  const base: AlignEvidence = { kind: 'container', createdMs: Date.UTC(2026, 9, 14, 10, 40, 0) };

  it('says how and how sure', () => {
    expect(alignedByLabel('clock')).toContain('clock');
    for (const by of ['user', 'clock', 'fileName', 'container', 'fileTime', 'none'] as const) expect(alignedByLabel(by)).not.toBe('');
    expect(['low', 'medium', 'high'].map((c) => confidenceLabel(c as 'low'))).toEqual(['low', 'medium', 'high']);
  });

  it('builds the evidence sentence of every kind', () => {
    expect(evidenceText({ kind: 'clock', frameMs: 12_000, text: '10:42:07', hasDate: false }, 'utc')).toBe(
      'The clock in the frame at 00:12 read 10:42:07; the date comes from the file.',
    );
    expect(evidenceText({ kind: 'clock', frameMs: 12_000, text: '14 Oct 10:42:07', hasDate: true }, 'utc')).toContain('with its date');
    expect(evidenceText({ kind: 'userStart' }, 'utc')).toContain('You set');
    expect(evidenceText({ kind: 'userOffset', offsetMs: -2500, base }, 'utc')).toContain('-2.5 s');
    expect(evidenceText({ kind: 'userOffset', offsetMs: -2500, base }, 'utc')).toContain('created at 14 Oct 10:40:00');
    expect(evidenceText({ kind: 'fileName', text: '2026-10-14 10-40-00' }, 'utc')).toContain('2026-10-14 10-40-00');
    expect(evidenceText({ kind: 'fileTime', modifiedMs: Date.UTC(2026, 9, 14, 11, 0, 0) }, 'utc')).toContain('11:00:00');
    expect(evidenceText({ kind: 'nothing' }, 'utc')).toContain('No time was found');
  });

  it('describes what the user set', () => {
    expect(userAlignmentText({ kind: 'auto' }, 'utc')).toBe('Automatic');
    expect(userAlignmentText({ kind: 'start', startMs: Date.UTC(2026, 9, 14, 10, 0, 0) }, 'utc')).toBe('Start set to 14 Oct 10:00:00.000');
    expect(userAlignmentText({ kind: 'offset', offsetMs: 1500 }, 'utc')).toBe('Moved by +1.5 s');
  });
});

describe('selection in words', () => {
  it('says how frames were chosen', () => {
    expect(selectionText({ mode: { mode: 'changes', threshold: 0.1 }, cap: 30 })).toBe('Frames where the picture changed (threshold 0.1), at most 30.');
    expect(selectionText({ mode: { mode: 'interval', everyMs: 10_000 }, cap: 12 })).toBe('One frame every 00:10, at most 12.');
  });

  it('explains each note', () => {
    expect(selectionNoteText({ kind: 'thresholdRaised', from: 0.1, to: 0.25 })).toContain('raised from 0.1 to 0.25');
    expect(selectionNoteText({ kind: 'spreadEvenly', leftOut: 1 })).toContain('1 change was left out');
    expect(selectionNoteText({ kind: 'spreadEvenly', leftOut: 95 })).toContain('95 changes were left out');
    expect(selectionNoteText({ kind: 'noChanges' })).toContain('Nothing changed');
  });
});

describe('removal', () => {
  const record = {
    included: true,
    frames: [{ note: 'n', description: 'd' }, { note: null, description: 'd' }, { note: '', description: null }],
  } as unknown as VideoRecord;

  it('counts what is deleted and says so, copy included', () => {
    const r = removalOf(record);
    expect(r).toEqual({ frames: 3, notes: 1, descriptions: 2, included: true });
    const t = removalText('a.mp4', r);
    expect(t).toContain('3 frames, 1 notes and 2 descriptions');
    expect(t).toContain('copy of the video kept in the project goes too');
    expect(t).toContain('original video file is not touched');
    expect(removalText('a.mp4', { ...r, included: false })).not.toContain('copy of the video');
  });
});

describe('failures', () => {
  it('keeps the engine hint and points the AI codes to Settings', () => {
    const f = toFailure(new CmdError({ code: 'noVision', message: 'Text only.', hint: 'Add image support.', retryable: false }));
    expect(f).toEqual({ message: 'Text only.', hint: 'Add image support.', aiSetup: true, retryable: false });
    expect(toFailure(new CmdError({ code: 'notFound', message: 'x', retryable: true })).aiSetup).toBe(false);
    expect(toFailure(new Error('plain'))).toEqual({ message: 'plain', hint: null, aiSetup: false, retryable: true });
    expect(toFailure('odd').message).toBe('odd');
  });
});
